//! `xherd toolchain exec -- <runtime argv>` (R-652, design §8.6): runs in the
//! agent's tmux window before the runtime. It carries out the plan the daemon
//! wrote to `.xherd/toolchain.json` (resolve each tool; in the container,
//! install missing versions with a live log), then replaces itself with the
//! runtime, `PATH` and variables set. Manager scripts are only sourced inside
//! the plan's commands, so the runtime gets plain `PATH` entries and no global
//! default is switched. Results are cached per plan in
//! `.xherd/toolchain.resolved.json`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use xherd_core::exec::{Cmd, run};
use xherd_plugin::toolchain::{PLAN_FILE, Plan, Step, resolved_dir, step_env};

const CACHE_FILE: &str = ".xherd/toolchain.resolved.json";
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Default, PartialEq)]
struct Resolved {
    path: Vec<PathBuf>,
    env: Vec<(String, String)>,
    /// Every step succeeded (only then is the result cached).
    complete: bool,
}

fn step_cmd(step: &Step, argv: &[String]) -> Cmd {
    let mut c = Cmd::new(argv.to_vec());
    for k in &step.pass_env {
        if let Ok(v) = std::env::var(k) {
            c = c.env(k.clone(), v);
        }
    }
    c
}

/// `Ok(dir)` when the version is there (`dir`: what goes on `PATH`).
async fn resolve(step: &Step) -> Result<Option<PathBuf>, String> {
    let Some(argv) = &step.resolve else {
        return Ok(None);
    };
    match run(step_cmd(step, argv).timeout(RESOLVE_TIMEOUT)).await {
        Ok(o) if o.success() => Ok(resolved_dir(&o.stdout_str())),
        Ok(o) => Err(o.stderr.trim().lines().last().unwrap_or("").to_string()),
        Err(e) => Err(e.to_string()),
    }
}

async fn provide(step: &Step, install: bool) -> Result<Option<PathBuf>, String> {
    let first = resolve(step).await;
    let (Err(why), Some(argv)) = (&first, &step.install) else {
        return first;
    };
    if !install {
        let why = if why.is_empty() {
            String::new()
        } else {
            format!(" ({why})")
        };
        return Err(format!(
            "not installed{why}; xherd installs toolchains only in its container, so install it yourself"
        ));
    }
    eprintln!(
        "xherd: installing {} {} (from {})…",
        step.tool, step.version, step.file
    );
    let cmd = step_cmd(step, argv)
        .timeout(INSTALL_TIMEOUT)
        .inherit_output();
    match run(cmd).await {
        Ok(o) if o.success() => resolve(step).await,
        Ok(o) if o.timed_out => Err("install timed out".into()),
        Ok(o) => Err(format!("install failed (exit {:?})", o.code)),
        Err(e) => Err(e.to_string()),
    }
}

async fn carry_out(plan: &Plan) -> Resolved {
    let mut out = Resolved {
        complete: true,
        ..Resolved::default()
    };
    for step in &plan.steps {
        match provide(step, plan.install).await {
            Ok(dir) => {
                out.env.extend(step_env(step, dir.as_deref()));
                out.path.extend(dir);
            }
            Err(e) => {
                out.complete = false;
                eprintln!(
                    "xherd: {} {} (from {}): {e}; starting without it",
                    step.tool, step.version, step.file
                )
            }
        }
    }
    out
}

/// The cached result for this exact plan, if its directories still exist.
fn cached(key: &str) -> Option<Resolved> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(CACHE_FILE).ok()?).ok()?;
    if v["key"] != key {
        return None;
    }
    let path: Vec<PathBuf> = v["path"]
        .as_array()?
        .iter()
        .map(|p| p.as_str().map(PathBuf::from))
        .collect::<Option<_>>()?;
    let env: Vec<(String, String)> = v["env"]
        .as_array()?
        .iter()
        .map(|e| Some((e[0].as_str()?.to_string(), e[1].as_str()?.to_string())))
        .collect::<Option<_>>()?;
    path.iter().all(|p| p.is_dir()).then_some(Resolved {
        path,
        env,
        complete: true,
    })
}

fn new_path(entries: &[PathBuf]) -> std::ffi::OsString {
    let old = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(entries.iter().cloned().chain(std::env::split_paths(&old))).unwrap_or(old)
}

pub fn exec(argv: Vec<String>) -> i32 {
    let Some(program) = argv.first() else {
        eprintln!("xherd toolchain exec: nothing to run");
        return 64;
    };
    let plan: Plan = match std::fs::read(PLAN_FILE) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            eprintln!("xherd: {PLAN_FILE}: {e}; starting without toolchains");
            Plan::default()
        }),
        Err(_) => Plan::default(),
    };
    let key = xherd_core::hash::content_sha256(&serde_json::to_vec(&plan).expect("json"));
    let r = match cached(&key) {
        Some(r) => r,
        None => {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("xherd: {e}");
                    return 1;
                }
            };
            let r = rt.block_on(carry_out(&plan));
            let v = json!({"key": key, "path": r.path, "env": r.env});
            let file = Path::new(CACHE_FILE);
            let saved = if r.complete {
                xherd_core::paths::write_atomic(file, v.to_string().as_bytes(), None)
            } else {
                std::fs::remove_file(file).or(Ok(()))
            };
            if let Err(e) = saved {
                eprintln!("xherd: {CACHE_FILE}: {e}");
            }
            r
        }
    };
    // Replace this process with the runtime (not `exec::run`: it must own the
    // terminal, like the browser terminal's tmux client).
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(program)
        .args(&argv[1..])
        .env("PATH", new_path(&r.path))
        .envs(r.env)
        .exec();
    eprintln!("xherd: cannot start {program}: {err}");
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(resolve: &str, install: Option<&str>) -> Step {
        Step {
            plugin: "p".into(),
            tool: "t".into(),
            version: "1".into(),
            file: ".t".into(),
            resolve: Some(vec!["sh".into(), "-c".into(), resolve.into()]),
            install: install.map(|i| vec!["sh".into(), "-c".into(), i.into()]),
            env: [("T_HOME".to_string(), "{home}".to_string())].into(),
            pass_env: vec![],
        }
    }

    #[tokio::test]
    async fn installs_missing_versions_only_when_allowed() {
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("t/1/bin");
        let res = format!("test -d {0} && echo {0}", bin.display());
        let inst = format!("mkdir -p {}", bin.display());
        let s = step(&res, Some(&inst));

        // Native: never installs.
        let native = Plan {
            steps: vec![s.clone()],
            install: false,
        };
        let r = carry_out(&native).await;
        assert!(r.path.is_empty() && r.env.is_empty() && !r.complete);
        assert!(!bin.exists());

        let container = Plan {
            steps: vec![s],
            install: true,
        };
        let r = carry_out(&container).await;
        assert!(r.complete);
        assert_eq!(r.path, std::slice::from_ref(&bin));
        assert_eq!(
            r.env,
            [(
                "T_HOME".to_string(),
                bin.parent().unwrap().display().to_string()
            )]
        );
    }

    #[tokio::test]
    async fn steps_without_resolve_only_set_env() {
        let mut s = step("true", None);
        s.resolve = None;
        s.env = [("GOTOOLCHAIN".to_string(), "auto".to_string())].into();
        let r = carry_out(&Plan {
            steps: vec![s],
            install: true,
        })
        .await;
        assert!(r.path.is_empty());
        assert_eq!(r.env, [("GOTOOLCHAIN".to_string(), "auto".to_string())]);
    }

    #[test]
    fn path_entries_go_first() {
        let p = new_path(&[PathBuf::from("/a/bin")]);
        assert!(p.to_string_lossy().starts_with("/a/bin:"));
    }
}
