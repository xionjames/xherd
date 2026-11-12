//! Host companion commands (R-703, R-704; design §8.7): `xherd open`,
//! `attach`, `shell` and `companion`, run on the computer where the user
//! works. Apps come from `~/.config/xherd/open.json`
//! (`{"apps": {"idea": ["idea", "{dir}"]}}`), else the known editors found on
//! `PATH`. Only those commands ever run, with `{dir}` replaced, plus two
//! kinds the companion builds or checks itself: the terminal (R-707), whose
//! command is `xherd shell <agent>` in a fixed list of terminal apps, and
//! user commands from Settings (R-706), run only after the user confirmed that
//! exact argv here, once (hashes in `open-approved.json`).

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::Client;
use crate::docker::config_dir;

/// Editors offered when `open.json` lists none (each if found on `PATH`).
const KNOWN: &[(&str, &str)] = &[
    ("code", "code"),
    ("cursor", "cursor"),
    ("idea", "idea"),
    ("zed", "zed"),
    ("subl", "subl"),
];

fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| {
            std::fs::metadata(d.join(bin)).is_ok_and(|m| {
                use std::os::unix::fs::PermissionsExt;
                m.is_file() && m.permissions().mode() & 0o111 != 0
            })
        })
    })
}

/// App name → argv with `{dir}`.
pub fn apps(config: Option<&str>, found: impl Fn(&str) -> bool) -> BTreeMap<String, Vec<String>> {
    let configured: BTreeMap<String, Vec<String>> = config
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .and_then(|v| serde_json::from_value(v["apps"].clone()).ok())
        .unwrap_or_default();
    let configured: BTreeMap<_, _> = configured
        .into_iter()
        .filter(|(_, argv)| !argv.is_empty())
        .collect();
    if !configured.is_empty() {
        return configured;
    }
    KNOWN
        .iter()
        .filter(|(_, bin)| found(bin))
        .map(|(name, bin)| (name.to_string(), vec![bin.to_string(), "{dir}".into()]))
        .collect()
}

fn host_apps() -> BTreeMap<String, Vec<String>> {
    let text = std::fs::read_to_string(config_dir().join("open.json")).ok();
    apps(text.as_deref(), on_path)
}

fn launch(argv: &[String], dir: &str) -> Result<(), String> {
    launch_with(argv, dir, "")
}

/// Run `argv` detached, with `{dir}` and `{agent}` replaced.
fn launch_with(argv: &[String], dir: &str, agent: &str) -> Result<(), String> {
    let argv: Vec<String> = argv
        .iter()
        .map(|a| a.replace("{dir}", dir).replace("{agent}", agent))
        .collect();
    std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("{}: {e}", argv[0]))
}

fn agent(client: &Client, who: &str) -> Result<Value, String> {
    client
        .call("GET", &format!("/agents/{who}"), None)
        .map(|v| v["agent"].clone())
}

/// The xherd container's user id when the daemon runs in Docker (from
/// `xherd docker init`'s `.env` and a running `xherd` container).
fn docker_uid() -> Option<String> {
    let env = std::fs::read_to_string(config_dir().join(".env")).ok()?;
    let uid = env
        .lines()
        .find_map(|l| l.strip_prefix("XHERD_UID="))?
        .trim()
        .to_string();
    let running = std::process::Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", "xherd"])
        .output()
        .ok()
        .is_some_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true");
    running.then_some(uid)
}

pub fn open(client: &Client, who: &str, app: Option<String>) -> i32 {
    let a = match agent(client, who) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let Some(dir) = a["workdir"].as_str() else {
        eprintln!("{who} has no worktree");
        return 2;
    };
    let apps = host_apps();
    let pick = match &app {
        Some(n) => apps.get_key_value(n.as_str()),
        None => apps.iter().next(),
    };
    let Some((name, argv)) = pick else {
        eprintln!(
            "no app {}; configure apps in {} as {{\"apps\": {{\"idea\": [\"idea\", \"{{dir}}\"]}}}}",
            app.as_deref().unwrap_or("found"),
            config_dir().join("open.json").display()
        );
        return 2;
    };
    match launch(argv, dir) {
        Ok(()) => {
            println!("opened {dir} in {name}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

/// Attach to the agent's session (native tmux, or inside the container).
/// Replaces this process: it owns the terminal.
pub fn attach(client: &Client, who: &str, shell: bool) -> i32 {
    let a = match agent(client, who) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    let name = a["name"].as_str().unwrap_or(who).to_string();
    let dir = a["workdir"].as_str().unwrap_or(".").to_string();
    let target = format!("={name}");
    let err = match (docker_uid(), shell) {
        (Some(uid), false) => std::process::Command::new("docker")
            .args([
                "exec",
                "-it",
                "-u",
                &uid,
                "xherd",
                "tmux",
                "-S",
                xherd_core::container::TMUX_SOCK,
                "attach",
                "-t",
                &target,
            ])
            .exec(),
        (Some(uid), true) => std::process::Command::new("docker")
            .args(["exec", "-it", "-u", &uid, "-w", &dir, "xherd", "bash", "-l"])
            .exec(),
        (None, false) => {
            let cfg =
                xherd_core::config::Config::load(&xherd_core::paths::Paths::from_env().config())
                    .map(|(c, _)| c)
                    .unwrap_or_default();
            let mut c = std::process::Command::new("tmux");
            if let Some(s) = cfg.tmux_socket {
                c.args(["-L", &s]);
            }
            c.args(["attach", "-t", &target]).exec()
        }
        (None, true) => {
            if !Path::new(&dir).is_dir() {
                eprintln!("{dir} is not a folder here");
                return 2;
            }
            let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            std::process::Command::new(sh).current_dir(&dir).exec()
        }
    };
    eprintln!("xherd: {err}");
    1
}

/// Agent names reach a terminal's command line: plain characters only.
fn plain_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `'…'` for `sh`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Terminals on Linux (and other non-macOS Unix): id = binary, and the
/// arguments that come before the command to run. Order = preference when
/// the desktop names no default. Keep the ids in `config::TERMINAL_APPS`.
const UNIX_TERMINALS: &[(&str, &[&str])] = &[
    ("ghostty", &["-e"]),
    ("konsole", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("wezterm", &["start", "--"]),
    ("xfce4-terminal", &["-x"]),
    ("foot", &[]),
    ("xterm", &["-e"]),
    ("xdg-terminal-exec", &[]),
    ("x-terminal-emulator", &["-e"]),
];

/// Terminals on macOS: id and the app bundle that shows it is installed
/// (`Terminal` always is).
const MAC_TERMINALS: &[(&str, &str)] = &[
    ("terminal", "Terminal.app"),
    ("iterm", "iTerm.app"),
    ("ghostty", "Ghostty.app"),
];

/// The terminal a KDE Plasma `kdeglobals` names (`[General]`
/// `TerminalApplication=ghostty` or `TerminalService=com.mitchellh.ghostty.desktop`),
/// as one of `UNIX_TERMINALS`.
fn kde_terminal(kdeglobals: &str) -> Option<&'static str> {
    let mut general = false;
    let mut found = None;
    for l in kdeglobals.lines().map(str::trim) {
        if l.starts_with('[') {
            general = l == "[General]";
            continue;
        }
        let Some((k, v)) = l.split_once('=').filter(|_| general) else {
            continue;
        };
        let v = v.trim();
        let id = match k.trim() {
            "TerminalApplication" => {
                let bin = v.split_whitespace().next().unwrap_or("");
                let bin = bin.rsplit('/').next().unwrap_or(bin);
                UNIX_TERMINALS.iter().find(|(id, _)| *id == bin)
            }
            "TerminalService" => UNIX_TERMINALS
                .iter()
                .find(|(id, _)| v.to_ascii_lowercase().contains(id)),
            _ => None,
        };
        // TerminalApplication wins over TerminalService.
        if let Some((id, _)) = id
            && (found.is_none() || k.trim() == "TerminalApplication")
        {
            found = Some(*id);
        }
    }
    found
}

/// The desktop's own default terminal on Linux: `$TERMINAL`, KDE's
/// `kdeglobals`, then GNOME's `gsettings`. `None` when none names a known one.
fn desktop_terminal() -> Option<&'static str> {
    let known = |s: &str| {
        let bin = s.trim().trim_matches('\'').split_whitespace().next()?;
        let bin = bin.rsplit('/').next().unwrap_or(bin);
        UNIX_TERMINALS
            .iter()
            .find(|(id, _)| *id == bin)
            .map(|(id, _)| *id)
    };
    if let Some(t) = std::env::var("TERMINAL").ok().and_then(|t| known(&t)) {
        return Some(t);
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    if let Some(t) = std::fs::read_to_string(config.join("kdeglobals"))
        .ok()
        .and_then(|t| kde_terminal(&t))
    {
        return Some(t);
    }
    let out = std::process::Command::new("gsettings")
        .args([
            "get",
            "org.gnome.desktop.default-applications.terminal",
            "exec",
        ])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    known(&String::from_utf8_lossy(&out.stdout))
}

/// The terminals installed here, the default one first (R-707).
fn host_terminals() -> Vec<String> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        return MAC_TERMINALS
            .iter()
            .filter(|(id, app)| {
                *id == "terminal"
                    || Path::new("/Applications").join(app).exists()
                    || home
                        .as_ref()
                        .is_some_and(|h| h.join("Applications").join(app).exists())
            })
            .map(|(id, _)| id.to_string())
            .collect();
    }
    let mut found: Vec<String> = UNIX_TERMINALS
        .iter()
        .filter(|(id, _)| on_path(id))
        .map(|(id, _)| id.to_string())
        .collect();
    if let Some(d) = desktop_terminal().filter(|d| found.iter().any(|f| f == d)) {
        found.retain(|f| f != d);
        found.insert(0, d.to_string());
    }
    found
}

/// The argv that opens `app` with a shell in the agent's worktree (R-707):
/// `<xherd> shell <agent>`, in one of a fixed list of terminal apps (the
/// macOS ones when `mac`). `None` for an unknown app or a name that is not
/// plain.
pub fn terminal_argv(app: &str, xherd: &str, agent: &str, mac: bool) -> Option<Vec<String>> {
    if !plain_name(agent) {
        return None;
    }
    if !mac {
        let (bin, pre) = UNIX_TERMINALS.iter().find(|(id, _)| *id == app)?;
        let mut v = vec![bin.to_string()];
        v.extend(pre.iter().map(|s| s.to_string()));
        v.extend([xherd.to_string(), "shell".into(), agent.to_string()]);
        return Some(v);
    }
    let cmd = format!("{} shell {agent}", sh_quote(xherd));
    // AppleScript string literal.
    let lit = format!("\"{}\"", cmd.replace('\\', "\\\\").replace('"', "\\\""));
    let osa = |app: &str, line: String| {
        [
            "osascript".to_string(),
            "-e".into(),
            format!("tell application \"{app}\""),
            "-e".into(),
            "activate".into(),
            "-e".into(),
            line,
            "-e".into(),
            "end tell".into(),
        ]
        .to_vec()
    };
    let direct = |pre: &[&str]| {
        let mut v: Vec<String> = pre.iter().map(|s| s.to_string()).collect();
        v.extend([xherd.to_string(), "shell".into(), agent.to_string()]);
        v
    };
    Some(match app {
        "terminal" => osa("Terminal", format!("do script {lit}")),
        "iterm" => osa(
            "iTerm",
            format!("create window with default profile command {lit}"),
        ),
        "ghostty" => direct(&["open", "-na", "Ghostty", "--args", "-e"]),
        _ => return None,
    })
}

/// A user command's fingerprint: the sha256 of its argv as JSON.
fn argv_hash(argv: &[String]) -> String {
    xherd_core::hash::content_sha256(serde_json::to_string(argv).unwrap_or_default().as_bytes())
}

fn approved_file() -> std::path::PathBuf {
    config_dir().join("open-approved.json")
}

fn approved() -> Vec<String> {
    std::fs::read_to_string(approved_file())
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| serde_json::from_value(v["approved"].clone()).ok())
        .unwrap_or_default()
}

/// Has the user confirmed this argv on this computer? Asks once on the
/// companion's terminal (R-706); without one, the answer is no.
fn confirmed(app: &str, argv: &[String]) -> Result<(), String> {
    let hash = argv_hash(argv);
    let mut known = approved();
    if known.contains(&hash) {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err(format!(
            "refused {app}: a new command from Settings must be confirmed here; run `xherd companion` in a terminal"
        ));
    }
    print!(
        "Open in → {app} wants to run on this computer:\n  {}\nRun it now and from now on? [y/N] ",
        argv.iter()
            .map(|a| sh_quote(a))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if !matches!(line.trim(), "y" | "Y" | "yes") {
        return Err(format!("refused {app}: not confirmed"));
    }
    known.push(hash);
    let body = serde_json::json!({ "approved": known });
    let path = approved_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    xherd_core::paths::write_atomic(&path, body.to_string().as_bytes(), Some(0o600))
        .map_err(|e| format!("{}: {e}", path.display()))
}

/// One request from the daemon: the terminal, a user command, or an app.
fn handle(
    apps: &BTreeMap<String, Vec<String>>,
    terminals: &[String],
    r: &Value,
) -> Result<String, String> {
    let (Some(app), Some(dir)) = (r["app"].as_str(), r["dir"].as_str()) else {
        return Err("ignored a malformed request".into());
    };
    let agent = r["agent"].as_str().unwrap_or("");
    if app == "terminal" {
        let mac = cfg!(target_os = "macos");
        let fallback = if mac {
            "terminal"
        } else {
            "x-terminal-emulator"
        };
        // A choice made for another OS (Settings is shared) falls back to
        // this computer's default.
        let term = r["terminal"]
            .as_str()
            .filter(|t| terminal_argv(t, "xherd", "a", mac).is_some())
            .or(terminals.first().map(String::as_str))
            .unwrap_or(fallback);
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let argv = terminal_argv(term, &exe.to_string_lossy(), agent, mac)
            .ok_or_else(|| format!("ignored a terminal request for {term} / {agent:?}"))?;
        launch(&argv, dir)?;
        return Ok(format!("opened a shell for {agent} in {term}"));
    }
    if app.starts_with("cmd:") {
        let argv: Vec<String> = serde_json::from_value(r["argv"].clone())
            .ok()
            .filter(|a: &Vec<String>| a.first().is_some_and(|p| !p.is_empty()))
            .ok_or_else(|| format!("ignored {app}: no command"))?;
        confirmed(app, &argv)?;
        launch_with(&argv, dir, agent)?;
        return Ok(format!("opened {dir} with {app}"));
    }
    let argv = apps
        .get(app)
        .ok_or_else(|| format!("ignored a request for {app}: not offered here"))?;
    launch(argv, dir)?;
    Ok(format!("opened {dir} in {app}"))
}

/// Poll the daemon for Open-in requests from the browser and run them
/// (R-704, R-706, R-707). Runs until interrupted.
pub fn companion(client: &Client) -> i32 {
    let apps = host_apps();
    let terminals = host_terminals();
    let names: Vec<&str> = apps.keys().map(String::as_str).collect();
    println!(
        "companion connected to {}, offering the terminal ({}){}{}",
        client.url,
        if terminals.is_empty() {
            "none found".to_string()
        } else {
            terminals.join(", ")
        },
        if names.is_empty() { "" } else { ", " },
        names.join(", ")
    );
    if names.is_empty() {
        println!(
            "(none of {} is on PATH; list your apps in {})",
            KNOWN.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "),
            config_dir().join("open.json").display()
        );
    }
    let path = format!(
        "/companion/next?wait=25&features=terminal,commands&apps={}&terminals={}",
        names.join(","),
        terminals.join(",")
    );
    loop {
        match client.call("GET", &path, None) {
            Ok(v) if v["request"].is_null() => {}
            Ok(v) => match handle(&apps, &terminals, &v["request"]) {
                Ok(m) => println!("{m}"),
                Err(e) => eprintln!("{e}"),
            },
            Err(e) => {
                eprintln!("{e}; retrying");
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_apps_win_else_known_ones_on_path() {
        let a = apps(
            Some(r#"{"apps": {"idea": ["idea", "{dir}"], "bad": []}}"#),
            |_| true,
        );
        assert_eq!(a.keys().collect::<Vec<_>>(), ["idea"]);
        let a = apps(None, |b| b == "code" || b == "zed");
        assert_eq!(a.keys().collect::<Vec<_>>(), ["code", "zed"]);
        assert_eq!(a["code"], ["code", "{dir}"]);
        assert!(apps(Some("not json"), |_| false).is_empty());
    }

    #[test]
    fn terminal_commands_are_built_in_and_quoted() {
        let t = terminal_argv("terminal", "/opt/x y/xherd", "SC-1", true).unwrap();
        assert_eq!(t[0], "osascript");
        assert_eq!(t[6], r#"do script "'/opt/x y/xherd' shell SC-1""#);
        let g = terminal_argv("ghostty", "/b/xherd", "SC-1", true).unwrap();
        assert_eq!(
            g.join(" "),
            "open -na Ghostty --args -e /b/xherd shell SC-1"
        );
        let quote = terminal_argv("iterm", r#"/a"b/xherd"#, "SC-1", true).unwrap();
        assert!(
            quote[6].ends_with(r#""'/a\"b/xherd' shell SC-1""#),
            "{}",
            quote[6]
        );
        assert!(terminal_argv("xterm", "/b/xherd", "SC-1", true).is_none());
        assert!(terminal_argv("terminal", "/b/xherd", "a;rm -rf ~", true).is_none());
        assert!(terminal_argv("terminal", "/b/xherd", "", true).is_none());
    }

    #[test]
    fn linux_terminals_run_the_binary_directly() {
        let argv = |app| terminal_argv(app, "/b/xherd", "SC-1", false).map(|v| v.join(" "));
        assert_eq!(argv("ghostty").unwrap(), "ghostty -e /b/xherd shell SC-1");
        assert_eq!(argv("konsole").unwrap(), "konsole -e /b/xherd shell SC-1");
        assert_eq!(
            argv("gnome-terminal").unwrap(),
            "gnome-terminal -- /b/xherd shell SC-1"
        );
        assert_eq!(argv("kitty").unwrap(), "kitty /b/xherd shell SC-1");
        assert!(argv("terminal").is_none());
        assert!(argv("iterm").is_none());
        assert!(terminal_argv("konsole", "/b/xherd", "a b", false).is_none());
    }

    #[test]
    fn kde_names_its_default_terminal() {
        let g = "[General]\nTerminalApplication=ghostty\nTerminalService=org.kde.konsole.desktop\n";
        assert_eq!(kde_terminal(g), Some("ghostty"));
        let g = "[General]\nTerminalService=com.mitchellh.ghostty.desktop\n";
        assert_eq!(kde_terminal(g), Some("ghostty"));
        let g = "[General]\nTerminalApplication=/usr/bin/kitty --single-instance\n";
        assert_eq!(kde_terminal(g), Some("kitty"));
        assert_eq!(kde_terminal("[KDE]\nTerminalApplication=kitty\n"), None);
        assert_eq!(kde_terminal("[General]\nTerminalApplication=st\n"), None);
    }

    #[test]
    fn argv_hash_changes_with_any_argument() {
        let a = argv_hash(&["idea".into(), "{dir}".into()]);
        assert_eq!(a, argv_hash(&["idea".into(), "{dir}".into()]));
        assert_ne!(a, argv_hash(&["idea".into(), "{dir}".into(), "-x".into()]));
        assert_ne!(
            argv_hash(&["a b".into()]),
            argv_hash(&["a".into(), "b".into()])
        );
    }
}
