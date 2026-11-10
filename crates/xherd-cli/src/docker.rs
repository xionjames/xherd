//! Host-side Docker commands (R-606, design §8.3, §10.2):
//! `xherd docker init` writes the compose file and `.env` for the detected
//! engine; `doctor` checks what the same-path setup relies on, each failure
//! with its fix; `update` pauses live agents, pulls and restarts the
//! container, waits for it and resumes them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use xherd_core::exec::{Cmd, run};
use xherd_core::paths::write_atomic;

pub const DEFAULT_IMAGE: &str = "ghcr.io/xherd/xherd:1";
const SERVICE: &str = "xherd";
/// Docker Desktop and OrbStack forward the host's SSH agent here.
const VM_SSH_SOCK: &str = "/run/host-services/ssh-auth.sock";

/// `~/.config/xherd` (or `$XDG_CONFIG_HOME/xherd`).
pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("xherd")
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    DockerDesktop,
    OrbStack,
    Colima,
    Podman,
    /// Docker Engine on Linux (or anything else).
    Other,
}

/// From `docker info` (`OperatingSystem`, `Name`) and the context endpoint.
pub fn detect_engine(os: &str, name: &str, endpoint: &str) -> Engine {
    let all = format!("{os} {name} {endpoint}").to_lowercase();
    if all.contains("docker desktop") {
        Engine::DockerDesktop
    } else if all.contains("orbstack") {
        Engine::OrbStack
    } else if all.contains("colima") {
        Engine::Colima
    } else if all.contains("podman") {
        Engine::Podman
    } else {
        Engine::Other
    }
}

/// The SSH agent socket to mount (R-682), or why there is none.
pub fn ssh_socket(engine: Engine, host_sock: Option<&str>) -> Result<String, String> {
    match engine {
        Engine::DockerDesktop | Engine::OrbStack => Ok(VM_SSH_SOCK.into()),
        Engine::Other if cfg!(target_os = "linux") => host_sock
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                "SSH_AUTH_SOCK is not set: start an ssh-agent and run `xherd docker init` again"
                    .into()
            }),
        Engine::Colima => Err(
            "xherd does not set up SSH agent forwarding for Colima yet; use HTTPS remotes".into(),
        ),
        _ => Err(
            "this engine does not forward the host's SSH agent; use HTTPS remotes or mount keys"
                .into(),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setup {
    pub image: String,
    pub workspace: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub port: u16,
    pub ssh_sock: Option<String>,
    /// More host folders mounted at the same path (R-605).
    pub extra_paths: Vec<PathBuf>,
}

pub fn env_file(s: &Setup) -> String {
    let mut out = format!(
        "XHERD_IMAGE={}\nXHERD_WORKSPACE={}\nXHERD_UID={}\nXHERD_GID={}\nXHERD_PORT={}\n",
        s.image,
        s.workspace.display(),
        s.uid,
        s.gid,
        s.port
    );
    if let Some(sock) = &s.ssh_sock {
        out.push_str(&format!("XHERD_SSH_SOCK={sock}\n"));
    }
    out
}

/// Compose file (design §8.3). The port is published on the host's loopback
/// only: the daemon treats requests from the gateway as local.
pub fn compose_file(s: &Setup) -> String {
    let mut vols = vec![
        "      - ${XHERD_WORKSPACE}:${XHERD_WORKSPACE}".to_string(),
        "      - xherd-home:/home/xherd".to_string(),
        "      - xherd-data:/var/lib/xherd-daemon".to_string(),
    ];
    for p in &s.extra_paths {
        vols.push(format!("      - {0}:{0}", p.display()));
    }
    if s.ssh_sock.is_some() {
        vols.push("      - ${XHERD_SSH_SOCK}:/run/host-ssh-agent.sock".into());
    }
    let extra = if s.extra_paths.is_empty() {
        String::new()
    } else {
        let list: Vec<String> = s
            .extra_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        format!("      XHERD_EXTRA_PATHS: \"{}\"\n", list.join(":"))
    };
    format!(
        "# Written by `xherd docker init`; .env beside it holds the values.\n\
services:\n  \
  {SERVICE}:\n    \
    image: ${{XHERD_IMAGE}}\n    \
    container_name: {SERVICE}\n    \
    restart: unless-stopped\n    \
    environment:\n      \
      XHERD_WORKSPACE: ${{XHERD_WORKSPACE}}\n      \
      XHERD_UID: ${{XHERD_UID}}\n      \
      XHERD_GID: ${{XHERD_GID}}\n\
{extra}    \
    ports: [\"127.0.0.1:${{XHERD_PORT}}:4740\"]\n    \
    volumes:\n\
{vols}\n\
volumes:\n  \
  xherd-home: {{}}\n  \
  xherd-data: {{}}\n",
        vols = vols.join("\n")
    )
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

async fn cmd(argv: &[&str], timeout: u64) -> Result<String, String> {
    let o = run(Cmd::new(argv.iter().copied()).timeout(Duration::from_secs(timeout)))
        .await
        .map_err(|e| e.to_string())?;
    if o.success() {
        Ok(o.stdout_str().trim().to_string())
    } else {
        let err = o.stderr.trim();
        Err(if err.is_empty() {
            format!("{} failed", argv.join(" "))
        } else {
            err.lines().last().unwrap_or(err).to_string()
        })
    }
}

/// `docker compose` (the CLI plugin), else the standalone `docker-compose`
/// (Homebrew and some Linux packages install only that).
fn compose_bin() -> &'static [&'static str] {
    static BIN: std::sync::OnceLock<&'static [&'static str]> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        let plugin = std::process::Command::new("docker")
            .args(["compose", "version"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if plugin {
            &["docker", "compose"]
        } else {
            &["docker-compose"]
        }
    })
}

/// `<compose> -f <file> --env-file <env> …`.
fn compose_argv<'a>(file: &'a str, env: &'a str, rest: &[&'a str]) -> Vec<&'a str> {
    let mut v = compose_bin().to_vec();
    v.extend(["-f", file, "--env-file", env]);
    v.extend_from_slice(rest);
    v
}

async fn engine() -> Result<Engine, String> {
    let info = cmd(
        &[
            "docker",
            "info",
            "--format",
            "{{.OperatingSystem}}|{{.Name}}",
        ],
        20,
    )
    .await
    .map_err(|e| {
        format!(
            "Docker is not reachable ({e}); start your engine (Docker Desktop, OrbStack, Colima…)"
        )
    })?;
    let endpoint = cmd(
        &[
            "docker",
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
        ],
        10,
    )
    .await
    .unwrap_or_default();
    let (os, name) = info.split_once('|').unwrap_or((&info, ""));
    Ok(detect_engine(os, name, &endpoint))
}

fn ids() -> (u32, u32) {
    (
        nix::unistd::getuid().as_raw(),
        nix::unistd::getgid().as_raw(),
    )
}

pub struct InitArgs {
    pub workspace: Option<PathBuf>,
    pub image: Option<String>,
    pub port: u16,
    pub extra_paths: Vec<PathBuf>,
    pub force: bool,
}

pub fn init(a: InitArgs) -> i32 {
    let dir = config_dir();
    let (compose, env) = (dir.join("compose.yaml"), dir.join(".env"));
    if compose.exists() && !a.force {
        eprintln!("{} exists; pass --force to rewrite it", compose.display());
        return 2;
    }
    let engine = match rt().block_on(engine()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("xherd docker init: {e}");
            return 2;
        }
    };
    let ws = a.workspace.unwrap_or_else(|| home().join("xherd"));
    if let Err(e) = std::fs::create_dir_all(ws.join("worktrees")) {
        eprintln!("xherd docker init: {}: {e}", ws.display());
        return 2;
    }
    let abs = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let ssh = ssh_socket(engine, std::env::var("SSH_AUTH_SOCK").ok().as_deref());
    let (uid, gid) = ids();
    let s = Setup {
        image: a.image.unwrap_or_else(|| DEFAULT_IMAGE.into()),
        // Same path on both sides (design §8.2): the path as the host shows it.
        workspace: abs(&ws),
        uid,
        gid,
        port: a.port,
        ssh_sock: ssh.as_ref().ok().cloned(),
        extra_paths: a.extra_paths.iter().map(|p| abs(p)).collect(),
    };
    let write = |p: &Path, text: String| {
        write_atomic(p, text.as_bytes(), Some(0o600)).map_err(|e| format!("{}: {e}", p.display()))
    };
    if let Err(e) = write(&env, env_file(&s)).and_then(|_| write(&compose, compose_file(&s))) {
        eprintln!("xherd docker init: {e}");
        return 2;
    }
    println!("engine     {engine:?}");
    println!(
        "workspace  {} (same path in the container)",
        s.workspace.display()
    );
    match &ssh {
        Ok(sock) => println!("ssh agent  {sock}"),
        Err(why) => println!("ssh agent  none: {why}"),
    }
    println!("wrote      {}", compose.display());
    println!("           {}", env.display());
    println!(
        "\nStart it:   {} -f {} --env-file {} up -d",
        compose_bin().join(" "),
        compose.display(),
        env.display()
    );
    println!(
        "Then open   http://127.0.0.1:{}/   and run `xherd docker doctor`.",
        s.port
    );
    0
}

/// `KEY=value` lines of the `.env` file.
fn read_env(p: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(p)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub struct Finding {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub fix: Option<String>,
}

/// `docker port` output → whether every binding is on loopback.
pub fn port_on_loopback(out: &str) -> bool {
    let binds: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    !binds.is_empty()
        && binds.iter().all(|b| {
            let addr = b.rsplit_once(':').map_or(*b, |(a, _)| a);
            matches!(addr, "127.0.0.1" | "[::1]" | "::1")
        })
}

async fn doctor_findings() -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |name, ok, detail: String, fix: Option<&str>| {
        out.push(Finding {
            name,
            ok,
            detail,
            fix: fix.map(str::to_string),
        });
        ok
    };
    let dir = config_dir();
    let (compose, envf) = (dir.join("compose.yaml"), dir.join(".env"));
    let cf = compose.to_string_lossy().into_owned();
    let ef = envf.to_string_lossy().into_owned();

    let engine = engine().await;
    if !push(
        "engine",
        engine.is_ok(),
        engine
            .as_ref()
            .map_or_else(Clone::clone, |e| format!("{e:?}")),
        Some("start Docker Desktop, OrbStack, Colima or your engine"),
    ) {
        return out;
    }
    if !push(
        "compose file",
        compose.exists(),
        cf.clone(),
        Some("xherd docker init"),
    ) {
        return out;
    }
    let env = read_env(&envf);
    let get = |k: &str| {
        env.iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let up_fix = compose_argv(&cf, &ef, &["up", "-d"]).join(" ");
    let id = cmd(
        &["docker", "inspect", "-f", "{{.State.Status}}", SERVICE],
        20,
    )
    .await;
    let running = matches!(&id, Ok(s) if s == "running");
    if !push(
        "container running",
        running,
        id.unwrap_or_else(|e| e),
        Some(&up_fix),
    ) {
        return out;
    }

    let port = cmd(&["docker", "port", SERVICE, "4740"], 10).await;
    let lo = port.as_ref().is_ok_and(|p| port_on_loopback(p));
    push(
        "port on 127.0.0.1 only",
        lo,
        port.unwrap_or_else(|e| e),
        Some(
            "publish the port as \"127.0.0.1:4740:4740\" (xherd docker init --force writes it so); \
              on any other address, every machine that reaches it gets full rights",
        ),
    );

    let health = cmd(
        &[
            "curl",
            "-fsS",
            &format!("http://127.0.0.1:{}/api/v1/healthz", get("XHERD_PORT")),
        ],
        10,
    )
    .await;
    push(
        "daemon answers",
        health.is_ok(),
        health.unwrap_or_else(|e| e),
        Some("docker logs xherd"),
    );

    // Same path, both directions, and ownership (design §8.2, R-603).
    let ws = PathBuf::from(get("XHERD_WORKSPACE"));
    let marker = ws.join(format!(".xherd-doctor-{}", std::process::id()));
    let m = marker.to_string_lossy().into_owned();
    let seen = std::fs::write(&marker, "host").is_ok()
        && cmd(&["docker", "exec", SERVICE, "cat", &m], 10)
            .await
            .is_ok_and(|t| t == "host");
    push(
        "host files seen in the container (same path)",
        seen,
        ws.display().to_string(),
        Some(
            "mount the workspace at the same path: XHERD_WORKSPACE in .env, then xherd docker init --force",
        ),
    );
    let uid = get("XHERD_UID");
    let back = format!("{m}.c");
    let wrote = cmd(
        &[
            "docker",
            "exec",
            "-u",
            &uid,
            SERVICE,
            "sh",
            "-c",
            "echo container > \"$1\"",
            "_",
            &back,
        ],
        10,
    )
    .await;
    let owner = std::fs::metadata(&back)
        .ok()
        .map(|md| std::os::unix::fs::MetadataExt::uid(&md));
    let own_ok = wrote.is_ok() && owner == Some(ids().0);
    push(
        "container files owned by you on the host",
        own_ok,
        match (&wrote, owner) {
            (Err(e), _) => e.clone(),
            (Ok(_), Some(o)) => format!("owner uid {o}"),
            (Ok(_), None) => "file not visible on the host".into(),
        },
        Some("XHERD_UID/XHERD_GID in .env must be your ids (`id -u`, `id -g`)"),
    );
    let _ = std::fs::remove_file(&marker);
    let _ = std::fs::remove_file(&back);

    let has_sock = !get("XHERD_SSH_SOCK").is_empty();
    let ssh = cmd(
        &[
            "docker",
            "exec",
            "-u",
            &uid,
            SERVICE,
            "sh",
            "-c",
            "SSH_AUTH_SOCK=/run/xherd/ssh-agent.sock ssh-add -l; test $? -le 1",
        ],
        10,
    )
    .await;
    push(
        "SSH agent reachable",
        has_sock && ssh.is_ok(),
        if has_sock {
            ssh.unwrap_or_else(|e| e)
        } else {
            "not forwarded".into()
        },
        Some("forward your SSH agent (see `xherd docker init` output) or use HTTPS remotes"),
    );
    out
}

pub fn doctor() -> i32 {
    let findings = rt().block_on(doctor_findings());
    let mut failed = 0;
    for f in &findings {
        println!(
            "{} {:<46} {}",
            if f.ok { "ok  " } else { "FAIL" },
            f.name,
            f.detail
        );
        if !f.ok {
            failed += 1;
            if let Some(fix) = &f.fix {
                println!("     fix: {fix}");
            }
        }
    }
    i32::from(failed > 0)
}

/// Pause live agents, pull, restart, wait for the daemon, resume them
/// (design §10.2; backups before updates arrive with backups, M4).
pub fn update(client: &crate::Client, force: bool, pull: bool) -> i32 {
    let dir = config_dir();
    let (cf, ef) = (
        dir.join("compose.yaml").to_string_lossy().into_owned(),
        dir.join(".env").to_string_lossy().into_owned(),
    );
    let drained = dir.join("drained.json");
    let live: Vec<String> = match client.call("GET", "/agents", None) {
        Ok(v) => v["agents"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|a| {
                matches!(
                    a["status"].as_str(),
                    Some("running" | "waiting-input" | "idle" | "blocked" | "starting")
                )
            })
            .filter_map(|a| a["name"].as_str().map(str::to_string))
            .collect(),
        Err(e) if force => {
            eprintln!("daemon not reachable ({e}); updating anyway");
            Vec::new()
        }
        Err(e) => {
            eprintln!("xherd docker update: {e} (pass --force to update anyway)");
            return 2;
        }
    };
    if !live.is_empty() {
        println!(
            "pausing {} live agent(s): {} (their sessions end with the container)",
            live.len(),
            live.join(", ")
        );
        let _ = write_atomic(
            &drained,
            serde_json::to_vec(&live).expect("json").as_slice(),
            None,
        );
        let mut asked = Vec::new();
        for a in &live {
            match client.call("POST", &format!("/agents/{a}/pause"), None) {
                Ok(_) => asked.push(a.as_str()),
                Err(e) => eprintln!("  {a}: {e}"),
            }
        }
        // A pause waits for each agent's handoff note (R-026, up to
        // `pauseTimeoutSec`); the container must not go before it is written.
        if !asked.is_empty() {
            println!("waiting for the handoff notes…");
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3600 + 30);
        let mut pending = asked;
        while !pending.is_empty() && std::time::Instant::now() < deadline {
            pending.retain(|a| {
                client
                    .call("GET", &format!("/agents/{a}"), None)
                    .is_ok_and(|v| {
                        matches!(
                            v["agent"]["status"].as_str().or(v["status"].as_str()),
                            Some("running" | "waiting-input" | "idle")
                        )
                    })
            });
            if !pending.is_empty() {
                std::thread::sleep(Duration::from_secs(2));
            }
        }
        for a in &pending {
            eprintln!("  {a}: no handoff note yet; going on");
        }
    }
    let resume = |live: &[String]| {
        for a in live {
            match client.call("POST", &format!("/agents/{a}/resume"), None) {
                Ok(_) => println!("resumed {a}"),
                Err(e) => eprintln!("  {a}: {e}"),
            }
        }
    };
    let steps: Vec<&[&str]> = if pull {
        vec![&["pull"], &["up", "-d"]]
    } else {
        vec![&["up", "-d"]]
    };
    for rest in steps {
        let argv = compose_argv(&cf, &ef, rest);
        println!("$ {}", argv.join(" "));
        let ok = rt().block_on(async {
            run(Cmd::new(argv.iter().copied())
                .timeout(Duration::from_secs(30 * 60))
                .inherit_output())
            .await
            .is_ok_and(|o| o.success())
        });
        if !ok && rest[0] == "pull" {
            // Nothing was restarted: the running container keeps going.
            eprintln!(
                "xherd docker update: `{}` failed; nothing changed (a locally built image: --no-pull)",
                argv.join(" ")
            );
            resume(&live);
            let _ = std::fs::remove_file(&drained);
            return 1;
        }
        if !ok {
            eprintln!(
                "xherd docker update: `{}` failed; paused agents are listed in {}",
                argv.join(" "),
                drained.display()
            );
            return 1;
        }
    }
    print!("waiting for the daemon");
    let up = (0..90).any(|_| {
        std::thread::sleep(Duration::from_secs(1));
        print!(".");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        client.call("GET", "/healthz", None).is_ok()
    });
    println!();
    if !up {
        eprintln!(
            "the daemon did not answer within 90 s: `docker logs xherd`; paused agents are listed in {}",
            drained.display()
        );
        return 1;
    }
    resume(&live);
    let _ = std::fs::remove_file(&drained);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(ssh: Option<&str>) -> Setup {
        Setup {
            image: "ghcr.io/xherd/xherd:1".into(),
            workspace: "/Users/me/xherd".into(),
            uid: 501,
            gid: 20,
            port: 4740,
            ssh_sock: ssh.map(str::to_string),
            extra_paths: vec![],
        }
    }

    #[test]
    fn engines_and_ssh_sockets() {
        assert_eq!(
            detect_engine("Docker Desktop", "docker-desktop", ""),
            Engine::DockerDesktop
        );
        assert_eq!(detect_engine("OrbStack", "orbstack", ""), Engine::OrbStack);
        assert_eq!(
            detect_engine(
                "Ubuntu 24.04",
                "colima",
                "unix:///Users/me/.colima/default/docker.sock"
            ),
            Engine::Colima
        );
        assert_eq!(
            detect_engine("Debian GNU/Linux 13", "box", "unix:///var/run/docker.sock"),
            Engine::Other
        );
        assert_eq!(
            ssh_socket(Engine::DockerDesktop, None).unwrap(),
            VM_SSH_SOCK
        );
        assert!(ssh_socket(Engine::Colima, Some("/tmp/a")).is_err());
    }

    #[test]
    fn compose_publishes_on_loopback_and_mounts_same_path() {
        let c = compose_file(&setup(Some(VM_SSH_SOCK)));
        assert!(c.contains("ports: [\"127.0.0.1:${XHERD_PORT}:4740\"]"));
        assert!(c.contains("- ${XHERD_WORKSPACE}:${XHERD_WORKSPACE}"));
        assert!(c.contains("- ${XHERD_SSH_SOCK}:/run/host-ssh-agent.sock"));
        assert!(c.contains("xherd-home: {}"));
        assert!(c.contains("xherd-data:/var/lib/xherd-daemon"));
        assert!(c.contains("xherd-data: {}"));
        let no_ssh = compose_file(&setup(None));
        assert!(!no_ssh.contains("XHERD_SSH_SOCK"));
        let mut s = setup(None);
        s.extra_paths = vec!["/Volumes/code".into()];
        let c = compose_file(&s);
        assert!(c.contains("- /Volumes/code:/Volumes/code"));
        assert!(c.contains("XHERD_EXTRA_PATHS: \"/Volumes/code\""));

        let e = env_file(&setup(Some(VM_SSH_SOCK)));
        assert!(e.contains("XHERD_WORKSPACE=/Users/me/xherd\nXHERD_UID=501\nXHERD_GID=20\n"));
        assert!(e.ends_with(&format!("XHERD_SSH_SOCK={VM_SSH_SOCK}\n")));
    }

    #[test]
    fn port_binding() {
        assert!(port_on_loopback("127.0.0.1:4740\n"));
        assert!(!port_on_loopback("0.0.0.0:4740\n[::]:4740\n"));
        assert!(!port_on_loopback("127.0.0.1:4740\n0.0.0.0:4741\n"));
        assert!(!port_on_loopback(""));
    }
}
