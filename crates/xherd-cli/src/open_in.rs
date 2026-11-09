//! Host companion commands (R-703, R-704; design §8.7): `xherd open`,
//! `attach`, `shell` and `companion`, run on the computer where the user
//! works. Apps come from `~/.config/xherd/open.json`
//! (`{"apps": {"idea": ["idea", "{dir}"]}}`), else the known editors found on
//! `PATH`. Only those commands ever run, with `{dir}` replaced.

use std::collections::BTreeMap;
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
    let argv: Vec<String> = argv.iter().map(|a| a.replace("{dir}", dir)).collect();
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

/// Poll the daemon for Open-in requests from the browser and run the
/// matching app (R-704). Runs until interrupted.
pub fn companion(client: &Client) -> i32 {
    let apps = host_apps();
    if apps.is_empty() {
        eprintln!(
            "no apps: none of {} is on PATH; list yours in {}",
            KNOWN.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", "),
            config_dir().join("open.json").display()
        );
        return 2;
    }
    let names: Vec<&str> = apps.keys().map(String::as_str).collect();
    println!(
        "companion connected to {}, offering {}",
        client.url,
        names.join(", ")
    );
    let path = format!("/companion/next?wait=25&apps={}", names.join(","));
    loop {
        match client.call("GET", &path, None) {
            Ok(v) => {
                let r = &v["request"];
                let (Some(app), Some(dir)) = (r["app"].as_str(), r["dir"].as_str()) else {
                    continue;
                };
                match apps.get(app) {
                    Some(argv) => match launch(argv, dir) {
                        Ok(()) => println!("opened {dir} in {app}"),
                        Err(e) => eprintln!("{e}"),
                    },
                    None => eprintln!("ignored a request for {app}: not offered here"),
                }
            }
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
}
