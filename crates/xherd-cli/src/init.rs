//! `xherd init`: the container entrypoint (design §8.4), run as root under
//! tini. Two users (R-1203): the agents' user has the host's UID/GID (R-603)
//! and its home on the `xherd-home` volume (R-604); the daemon has its own
//! (`xherd-daemon`) with its data on another volume, unreadable by agents.
//! Init prepares both homes, the forwarded SSH agent (R-682), git and SSH
//! config, starts `xherd runner` as the agents' user (it runs what the daemon
//! needs done as that user and keeps their tmux server), then drops to the
//! daemon's user and replaces itself with `xherd daemon`. Nothing stays root.

use std::net::Ipv4Addr;
use std::os::unix::fs::chown;
use std::path::{Path, PathBuf};
use std::time::Duration;

use xherd_core::container::{
    AGENT_HOME, DAEMON_DATA, DAEMON_HOME, DAEMON_USER, QUEUE, RUN_DIR, RUNNER_SOCK, SHARED,
    TMUX_SOCK,
};
use xherd_core::exec::{Cmd, run};
use xherd_core::paths::write_atomic;

pub const HOME: &str = AGENT_HOME;
/// Image copies of the version managers, copied into the home volume once.
const SEED: &str = "/opt/xherd/seed";
const KNOWN_HOSTS: &str = "/opt/xherd/known_hosts";
/// Where compose mounts the host's SSH agent socket (root-owned).
pub const HOST_SSH_SOCK: &str = "/run/host-ssh-agent.sock";
/// The socket the user reaches it through (a proxy owned by the user).
pub const SSH_SOCK: &str = "/run/xherd/ssh-agent.sock";

/// Default IPv4 gateway from `/proc/net/route` (published ports reach the
/// container from it, so the daemon treats it as the local host).
pub fn default_gateway(route: &str) -> Option<Ipv4Addr> {
    route.lines().skip(1).find_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.get(1) != Some(&"00000000") {
            return None;
        }
        let gw = u32::from_str_radix(f.get(2)?, 16).ok()?;
        (gw != 0).then(|| Ipv4Addr::from(gw.to_le_bytes()))
    })
}

/// `text` with `line` first, unless some line already equals it.
pub fn with_first_line(text: &str, line: &str) -> String {
    if text.lines().any(|l| l.trim() == line) {
        return text.to_string();
    }
    format!("{line}\n{text}")
}

/// `~/.gitconfig` including xherd's managed file (kept if already there).
pub fn gitconfig_with_include(text: &str, managed: &str) -> String {
    let line = format!("path = {managed}");
    if text.lines().any(|l| l.trim() == line) {
        return text.to_string();
    }
    format!("[include]\n\t{line}\n{text}")
}

/// Claude Code's first-run screens marked done (design §8.4 step 6). Only
/// that key is touched; the daemon trusts each agent worktree itself.
pub fn claude_onboarded(text: Option<&str>) -> Option<String> {
    let mut v: serde_json::Map<String, serde_json::Value> = match text {
        None => serde_json::Map::new(),
        Some(t) => serde_json::from_str(t).ok()?,
    };
    if v.get("hasCompletedOnboarding") == Some(&true.into()) {
        return None;
    }
    v.insert("hasCompletedOnboarding".into(), true.into());
    Some(serde_json::to_string_pretty(&v).expect("json"))
}

fn env_id(name: &str) -> Result<u32, String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => v
            .parse()
            .map_err(|_| format!("{name}={v:?} is not a number")),
        _ => Ok(1000),
    }
}

async fn sh(argv: &[&str]) -> Result<String, String> {
    let o = run(Cmd::new(argv.iter().copied()).timeout(Duration::from_secs(300)))
        .await
        .map_err(|e| e.to_string())?;
    if o.success() {
        Ok(o.stdout_str())
    } else {
        Err(format!("{}: {}", argv.join(" "), o.stderr.trim()))
    }
}

struct Who {
    uid: u32,
    gid: u32,
    name: String,
}

/// The group and user with the host's ids, created when missing (an existing
/// one with those ids is reused under its own name).
async fn ensure_user(uid: u32, gid: u32) -> Result<Who, String> {
    if uid == 0 {
        eprintln!(
            "xherd init: XHERD_UID=0: agents run as root; files they write are root-owned on the host"
        );
        return Ok(Who {
            uid,
            gid,
            name: "root".into(),
        });
    }
    if sh(&["getent", "group", &gid.to_string()]).await.is_err() {
        sh(&["groupadd", "-g", &gid.to_string(), "xherd"]).await?;
    }
    let name = match sh(&["getent", "passwd", &uid.to_string()]).await {
        Ok(line) => line.split(':').next().unwrap_or("xherd").to_string(),
        Err(_) => {
            sh(&[
                "useradd",
                "-u",
                &uid.to_string(),
                "-g",
                &gid.to_string(),
                "-d",
                HOME,
                "-M",
                "-s",
                "/bin/bash",
                "xherd",
            ])
            .await?;
            "xherd".into()
        }
    };
    Ok(Who { uid, gid, name })
}

fn owned_by(p: &Path, uid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).is_ok_and(|m| m.uid() == uid)
}

async fn chown_r(p: &Path, w: &Who) -> Result<(), String> {
    let p = p.to_string_lossy();
    sh(&["chown", "-R", &format!("{}:{}", w.uid, w.gid), &p])
        .await
        .map(drop)
}

/// Write a file in the home as the user.
fn write_as(p: &Path, text: &str, w: &Who) -> Result<(), String> {
    let err = |e: std::io::Error| format!("{}: {e}", p.display());
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
        // create_dir_all makes missing parents (e.g. `~/.config`) as root, and
        // then gh cannot save its login there: give every directory between
        // the home and the file to the user, also on existing volumes.
        for d in dir
            .ancestors()
            .take_while(|d| d.starts_with(HOME) && *d != Path::new(HOME))
        {
            if !owned_by(d, w.uid) {
                chown(d, Some(w.uid), Some(w.gid)).map_err(err)?;
            }
        }
    }
    write_atomic(p, text.as_bytes(), Some(0o644)).map_err(err)?;
    chown(p, Some(w.uid), Some(w.gid)).map_err(err)
}

async fn prepare_home(w: &Who) -> Result<(), String> {
    let home = Path::new(HOME);
    std::fs::create_dir_all(home).map_err(|e| format!("{HOME}: {e}"))?;
    // A new volume is root-owned; a changed host UID leaves the old owner.
    if !owned_by(home, w.uid) {
        eprintln!("xherd init: giving {HOME} to {}:{}", w.uid, w.gid);
        chown_r(home, w).await?;
    }
    for e in std::fs::read_dir(SEED).into_iter().flatten().flatten() {
        let dst = home.join(e.file_name());
        if !dst.exists() {
            let src = e.path();
            sh(&["cp", "-a", &src.to_string_lossy(), &dst.to_string_lossy()]).await?;
            chown_r(&dst, w).await?;
        }
    }

    // SSH: xherd's known hosts (R-683) through an include, user config kept.
    let xdir = home.join(".config/xherd");
    if let Ok(k) = std::fs::read_to_string(KNOWN_HOSTS) {
        write_as(&xdir.join("known_hosts"), &k, w)?;
    }
    let include = format!("Include {}", xdir.join("ssh_config").display());
    write_as(
        &xdir.join("ssh_config"),
        &format!(
            "Host *\n  UserKnownHostsFile ~/.ssh/known_hosts {}\n",
            xdir.join("known_hosts").display()
        ),
        w,
    )?;
    let ssh_cfg = home.join(".ssh/config");
    let old = std::fs::read_to_string(&ssh_cfg).unwrap_or_default();
    write_as(&ssh_cfg, &with_first_line(&old, &include), w)?;
    let _ = std::fs::set_permissions(
        home.join(".ssh"),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    );

    // git: an include of xherd's managed file, which the daemon rewrites from
    // Settings → Git access in its own shared folder (read-only for this user).
    let managed = Path::new(SHARED).join("gitconfig");
    let gc = home.join(".gitconfig");
    let old = std::fs::read_to_string(&gc).unwrap_or_default();
    write_as(
        &gc,
        &gitconfig_with_include(&old, &managed.to_string_lossy()),
        w,
    )?;

    let claude = home.join(".claude.json");
    if let Some(t) = claude_onboarded(std::fs::read_to_string(&claude).ok().as_deref()) {
        write_as(&claude, &t, w)?;
    }
    Ok(())
}

/// Expose the root-owned forwarded agent socket to the user (R-682) through
/// socat. A long-running helper: spawned directly, reaped by tini.
fn ssh_proxy(w: &Who) -> Option<String> {
    if !Path::new(HOST_SSH_SOCK).exists() {
        eprintln!(
            "xherd init: no SSH agent forwarded ({HOST_SSH_SOCK}); SSH remotes need keys or HTTPS"
        );
        return None;
    }
    let _ = std::fs::create_dir_all("/run/xherd");
    let listen = format!(
        "UNIX-LISTEN:{SSH_SOCK},fork,unlink-early,user={},group={},mode=600",
        w.uid, w.gid
    );
    let connect = format!("UNIX-CONNECT:{HOST_SSH_SOCK}");
    match std::process::Command::new("socat")
        .args(["-d", &listen, &connect])
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => Some(SSH_SOCK.into()),
        Err(e) => {
            eprintln!("xherd init: SSH agent proxy (socat): {e}");
            None
        }
    }
}

pub fn init() -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("xherd init: {e}");
            return 1;
        }
    };
    match rt.block_on(prepare()) {
        Ok(plan) => exec_daemon(&plan),
        Err(e) => {
            eprintln!("xherd init: {e}");
            1
        }
    }
}

/// The two users, their folders and what the daemon is started with.
struct Plan {
    agent: Who,
    daemon: Who,
    /// The environment of the daemon.
    daemon_env: Vec<(String, String)>,
}

/// The daemon's user, created on the first start (its ids come from 10001 up,
/// never the host's); its own group, and no shell.
async fn ensure_daemon_user(agent: &Who) -> Result<Who, String> {
    if let Ok(line) = sh(&["getent", "passwd", DAEMON_USER]).await {
        let f: Vec<&str> = line.trim().split(':').collect();
        if let (Some(uid), Some(gid)) = (
            f.get(2).and_then(|v| v.parse().ok()),
            f.get(3).and_then(|v| v.parse().ok()),
        ) {
            return Ok(Who {
                uid,
                gid,
                name: DAEMON_USER.into(),
            });
        }
    }
    let mut id = 10001u32;
    loop {
        let s = id.to_string();
        if id != agent.uid
            && id != agent.gid
            && sh(&["getent", "passwd", &s]).await.is_err()
            && sh(&["getent", "group", &s]).await.is_err()
        {
            break;
        }
        id += 1;
    }
    let s = id.to_string();
    sh(&["groupadd", "-g", &s, DAEMON_USER]).await?;
    sh(&[
        "useradd",
        "-u",
        &s,
        "-g",
        &s,
        "-d",
        DAEMON_HOME,
        "-M",
        "-s",
        "/usr/sbin/nologin",
        DAEMON_USER,
    ])
    .await?;
    Ok(Who {
        uid: id,
        gid: id,
        name: DAEMON_USER.into(),
    })
}

fn chmod(p: &Path, mode: u32) -> Result<(), String> {
    std::fs::set_permissions(p, std::os::unix::fs::PermissionsExt::from_mode(mode))
        .map_err(|e| format!("{}: {e}", p.display()))
}

/// The daemon's folders: `data` only for it (0700), `shared` readable by the
/// agents' user (what the daemon writes for them), the home traversable only.
async fn prepare_daemon(d: &Who) -> Result<(), String> {
    let home = Path::new(DAEMON_HOME);
    std::fs::create_dir_all(home).map_err(|e| format!("{DAEMON_HOME}: {e}"))?;
    // A new volume is root-owned.
    if !owned_by(home, d.uid) {
        eprintln!("xherd init: giving {DAEMON_HOME} to {}:{}", d.uid, d.gid);
        chown_r(home, d).await?;
    }
    chmod(home, 0o711)?;
    for (dir, mode) in [(DAEMON_DATA, 0o700), (SHARED, 0o755)] {
        let p = Path::new(dir);
        std::fs::create_dir_all(p).map_err(|e| format!("{dir}: {e}"))?;
        chown(p, Some(d.uid), Some(d.gid)).map_err(|e| format!("{dir}: {e}"))?;
        chmod(p, mode)?;
    }
    let gc = Path::new(SHARED).join("gitconfig");
    if !gc.exists() {
        write_atomic(&gc, b"[safe]\n\tdirectory = *\n", Some(0o644))
            .map_err(|e| format!("{}: {e}", gc.display()))?;
        chown(&gc, Some(d.uid), Some(d.gid)).map_err(|e| format!("{}: {e}", gc.display()))?;
    }
    Ok(())
}

/// Sockets of the runner, the tmux server and the SSH proxy: the agents'
/// user's, in the host's group, which the daemon's user is in.
fn prepare_run(agent: &Who) -> Result<(), String> {
    let p = Path::new(RUN_DIR);
    std::fs::create_dir_all(p).map_err(|e| format!("{RUN_DIR}: {e}"))?;
    chown(p, Some(agent.uid), Some(agent.gid)).map_err(|e| format!("{RUN_DIR}: {e}"))?;
    chmod(p, 0o2770)
}

/// `worktrees/` and `repos/` are made by the agents' user (git) and by the
/// daemon (folders, renames): group-writable with the host's group. The
/// daemon's user must also get into the workspace (group execute).
fn prepare_workspace(ws: &Path, agent: &Who) {
    use std::os::unix::fs::MetadataExt;
    if let Ok(m) = std::fs::metadata(ws) {
        let mode = m.mode() & 0o7777;
        if mode & 0o010 == 0
            && let Err(e) = chmod(ws, mode | 0o010)
        {
            eprintln!("xherd init: {e}; the daemon may not get into the workspace");
        }
    }
    for sub in ["worktrees", "repos"] {
        let p = ws.join(sub);
        let made = std::fs::create_dir_all(&p)
            .map_err(|e| format!("{}: {e}", p.display()))
            .and_then(|()| {
                chown(&p, Some(agent.uid), Some(agent.gid))
                    .map_err(|e| format!("{}: {e}", p.display()))
            })
            .and_then(|()| chmod(&p, 0o2775));
        if let Err(e) = made {
            eprintln!("xherd init: {e}");
        }
    }
}

/// `xherd runner` as the agents' user: their commands and files for the
/// daemon, and the tmux server their sessions live in. A long-running helper
/// like socat: spawned directly, not waited for.
fn start_runner(agent: &Who, env: &[(String, String)]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new("setpriv")
        .args([
            "--reuid",
            &agent.uid.to_string(),
            "--regid",
            &agent.gid.to_string(),
            "--groups",
            &agent.gid.to_string(),
            "--",
        ])
        .arg(exe)
        .args(["runner", "--socket", RUNNER_SOCK, "--mode", "660"])
        .args(["--tmux", TMUX_SOCK, "--tmux-allow", DAEMON_USER])
        .current_dir(AGENT_HOME)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("starting the runner (setpriv): {e}"))
}

async fn prepare() -> Result<Plan, String> {
    if !nix::unistd::Uid::effective().is_root() {
        return Err("must run as root (it is the container entrypoint)".into());
    }
    let ws = std::env::var("XHERD_WORKSPACE").unwrap_or_default();
    let ws = PathBuf::from(ws);
    if !ws.is_absolute() || !ws.is_dir() {
        return Err(format!(
            "XHERD_WORKSPACE={} is not a mounted folder; mount the host workspace at the same path (design §8.2)",
            ws.display()
        ));
    }
    let agent = ensure_user(env_id("XHERD_UID")?, env_id("XHERD_GID")?).await?;
    let daemon = ensure_daemon_user(&agent).await?;
    // The host's group in /etc/group too, so a shell started later for this
    // user (`docker exec -u xherd-daemon`) has what the daemon process has.
    let _ = sh(&["usermod", "-aG", &agent.gid.to_string(), DAEMON_USER]).await;
    prepare_daemon(&daemon).await?;
    prepare_home(&agent).await?;
    prepare_run(&agent)?;
    prepare_workspace(&ws, &agent);

    let agent_path = std::env::var("PATH").unwrap_or_default();
    let mut agent_env = vec![
        ("HOME".to_string(), AGENT_HOME.to_string()),
        ("USER".into(), agent.name.clone()),
        ("LOGNAME".into(), agent.name.clone()),
        ("SHELL".into(), "/bin/bash".into()),
        // Hooks the daemon cannot take wait in a folder both users write.
        ("XHERD_QUEUE".into(), QUEUE.into()),
    ];
    let mut daemon_env = vec![
        ("HOME".to_string(), DAEMON_HOME.to_string()),
        ("USER".into(), daemon.name.clone()),
        ("LOGNAME".into(), daemon.name.clone()),
        ("SHELL".into(), "/bin/sh".into()),
        // System folders only: the agents' PATH has folders they can write,
        // and the daemon runs programs by name.
        (
            "PATH".into(),
            "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".into(),
        ),
        ("XHERD_HOME".into(), DAEMON_DATA.into()),
        ("XHERD_QUEUE".into(), QUEUE.into()),
        ("XHERD_SHARED".into(), SHARED.into()),
        ("XHERD_RUNNER".into(), RUNNER_SOCK.into()),
        ("XHERD_TMUX_SOCKET".into(), TMUX_SOCK.into()),
        ("XHERD_AGENT_HOME".into(), AGENT_HOME.into()),
        ("XHERD_AGENT_PATH".into(), agent_path),
    ];
    if let Some(sock) = ssh_proxy(&agent) {
        agent_env.push(("SSH_AUTH_SOCK".into(), sock.clone()));
        daemon_env.push(("SSH_AUTH_SOCK".into(), sock));
    }
    let route = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    if let Some(gw) = default_gateway(&route) {
        daemon_env.push(("XHERD_LOCAL_PEERS".into(), gw.to_string()));
    }
    start_runner(&agent, &agent_env)?;
    Ok(Plan {
        agent,
        daemon,
        daemon_env,
    })
}

fn exec_daemon(p: &Plan) -> i32 {
    use nix::unistd::{Gid, Uid, setgid, setuid};
    use std::os::unix::process::CommandExt;
    let (d, a) = (&p.daemon, &p.agent);
    // The daemon is in the host's group too: that is how it shares `.xherd/`
    // folders and the runner's sockets with the agents' user.
    let groups = [Gid::from_raw(d.gid), Gid::from_raw(a.gid)];
    // Only the container (Linux) runs `init`; macOS lacks `setgroups`.
    #[cfg(target_os = "linux")]
    let res = nix::unistd::setgroups(&groups);
    #[cfg(not(target_os = "linux"))]
    let res: nix::Result<()> = Err(nix::errno::Errno::ENOTSUP);
    if let Err(e) = res
        .and_then(|_| setgid(groups[0]))
        .and_then(|_| setuid(Uid::from_raw(d.uid)))
    {
        eprintln!("xherd init: dropping privileges: {e}");
        return 1;
    }
    // Files the daemon makes in shared folders are group-writable.
    nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o002));
    let exe = std::env::current_exe().unwrap_or_else(|_| "xherd".into());
    let err = std::process::Command::new(exe)
        .arg("daemon")
        .current_dir(DAEMON_HOME)
        .envs(p.daemon_env.iter().map(|(k, v)| (k, v)))
        .exec();
    eprintln!("xherd init: starting the daemon: {err}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_from_route_table() {
        let route = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\n\
                     eth0\t00000000\t010011AC\t0003\t0\t0\t0\t00000000\n\
                     eth0\t000011AC\t00000000\t0001\t0\t0\t0\t0000FFFF\n";
        assert_eq!(default_gateway(route), Some(Ipv4Addr::new(172, 17, 0, 1)));
        assert_eq!(default_gateway("Iface\tDestination\n"), None);
    }

    #[test]
    fn includes_are_added_once() {
        let inc = "Include /h/.config/xherd/ssh_config";
        let once = with_first_line("Host x\n  User y\n", inc);
        assert!(once.starts_with(inc));
        assert_eq!(with_first_line(&once, inc), once);

        let g = gitconfig_with_include("[user]\n\tname = me\n", "/h/x/gitconfig");
        assert!(g.starts_with("[include]\n\tpath = /h/x/gitconfig\n[user]"));
        assert_eq!(gitconfig_with_include(&g, "/h/x/gitconfig"), g);
    }

    #[test]
    fn claude_onboarding_is_set_once_and_keeps_other_keys() {
        let t = claude_onboarded(Some(r#"{"projects": {}}"#)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["hasCompletedOnboarding"], true);
        assert!(v["projects"].is_object());
        assert_eq!(claude_onboarded(Some(&t)), None);
        assert!(claude_onboarded(None).is_some());
        // Not JSON: left alone.
        assert_eq!(claude_onboarded(Some("nope")), None);
    }
}
