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
use xherd_plugin::toolchain::{ENV_FILE, PLAN_FILE, Plan, Step, resolved_dir, step_env};

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

/// `s` as one single-quoted `sh` word.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The `toolchain.env` text for a resolved plan: a POSIX `sh` script that
/// prepends the resolved directories to `PATH`, exports the variables and, when
/// `prev_bash_env` is set, sources the `BASH_ENV` that `exec` replaced.
fn env_file(r: &Resolved, prev_bash_env: Option<&str>) -> String {
    let mut s = String::from(
        "# Written by `xherd toolchain exec`; source it with `. ./.xherd/toolchain.env`.\n",
    );
    if !r.path.is_empty() {
        let dirs: Vec<String> = r
            .path
            .iter()
            .map(|p| sh_quote(&p.to_string_lossy()))
            .collect();
        s.push_str(&format!("PATH={}:\"$PATH\"\nexport PATH\n", dirs.join(":")));
    }
    for (k, v) in &r.env {
        let mut chars = k.chars();
        let valid = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if valid {
            s.push_str(&format!("export {k}={}\n", sh_quote(v)));
        }
    }
    if let Some(prev) = prev_bash_env.filter(|p| !p.is_empty()) {
        let q = sh_quote(prev);
        s.push_str(&format!("if [ -f {q} ]; then . {q}; fi\n"));
    }
    s
}

/// Write `.xherd/toolchain.env` (R-652) and return its absolute path, or
/// `None` (after printing why) when it could not be written.
fn write_env_file(r: &Resolved) -> Option<PathBuf> {
    let prev = std::env::var("BASH_ENV").ok();
    let file = Path::new(ENV_FILE);
    let saved =
        xherd_core::paths::write_atomic(file, env_file(r, prev.as_deref()).as_bytes(), None)
            .and_then(|()| std::env::current_dir().map(|d| d.join(file)));
    saved.map_err(|e| eprintln!("xherd: {ENV_FILE}: {e}")).ok()
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
    let env_path = write_env_file(&r);
    let mut cmd = std::process::Command::new(program);
    cmd.args(&argv[1..])
        .env("PATH", new_path(&r.path))
        .envs(r.env);
    // Shells the runtime starts (bash, sh) source the file, so login profiles
    // that reset `PATH` do not hide the resolved tools.
    if let Some(p) = env_path {
        cmd.env("BASH_ENV", &p).env("ENV", &p);
    }
    let err = cmd.exec();
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

    /// Source `text` with `sh` (`PATH=/usr/bin:/bin`) and print what `script` shows.
    async fn sourced(text: &str, script: &str) -> String {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("toolchain.env");
        std::fs::write(&f, text).unwrap();
        let o = run(Cmd::new(vec![
            "sh".into(),
            "-c".into(),
            format!(". {}\n{script}", f.display()),
        ])
        .env("PATH", "/usr/bin:/bin"))
        .await
        .unwrap();
        assert!(o.success(), "{}", o.stderr);
        o.stdout_str()
    }

    fn resolved(path: &[&str], env: &[(&str, &str)]) -> Resolved {
        Resolved {
            path: path.iter().map(PathBuf::from).collect(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            complete: true,
        }
    }

    #[tokio::test]
    async fn env_file_prepends_dirs_and_exports_variables() {
        let text = env_file(
            &resolved(&["/a/bin", "/b dir/bin"], &[("T_HOME", "/a")]),
            None,
        );
        let out = sourced(&text, "echo \"$PATH\"; echo \"$T_HOME\"").await;
        assert_eq!(out, "/a/bin:/b dir/bin:/usr/bin:/bin\n/a\n");
    }

    #[tokio::test]
    async fn env_file_quotes_values_and_skips_invalid_names() {
        let text = env_file(
            &resolved(
                &["/it's/bin"],
                &[("Q", "a'b $x"), ("1BAD", "x"), ("A-B", "y")],
            ),
            None,
        );
        assert!(!text.contains("1BAD") && !text.contains("A-B"));
        let out = sourced(&text, "echo \"$Q\"; echo \"$PATH\"").await;
        assert_eq!(out, "a'b $x\n/it's/bin:/usr/bin:/bin\n");
    }

    #[tokio::test]
    async fn env_file_chains_the_previous_bash_env() {
        let d = tempfile::tempdir().unwrap();
        let prev = d.path().join("prev.sh");
        std::fs::write(&prev, "PREV=yes\n").unwrap();
        let prev = prev.to_string_lossy().to_string();
        let r = resolved(&[], &[]);
        let out = sourced(&env_file(&r, Some(&prev)), "echo \"$PREV\"").await;
        assert_eq!(out, "yes\n");
        // A missing previous file is not an error.
        let missing = format!("{prev}.none");
        let out = sourced(&env_file(&r, Some(&missing)), "echo ok").await;
        assert_eq!(out, "ok\n");
    }

    #[tokio::test]
    async fn env_file_of_a_partly_failed_plan_keeps_what_resolved() {
        let mut r = resolved(&["/a/bin"], &[]);
        r.complete = false;
        let out = sourced(&env_file(&r, None), "echo \"$PATH\"").await;
        assert_eq!(out, "/a/bin:/usr/bin:/bin\n");
        let out = sourced(&env_file(&Resolved::default(), None), "echo \"$PATH\"").await;
        assert_eq!(out, "/usr/bin:/bin\n");
    }

    #[tokio::test]
    async fn bash_login_shell_finds_tools_through_bash_env() {
        let have_bash = run(Cmd::new(["bash", "-c", "true"].map(String::from).to_vec()))
            .await
            .is_ok_and(|o| o.success());
        if !have_bash {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("tc/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let tool = bin.join("t");
        std::fs::write(&tool, "#!/bin/sh\necho from-toolchain\n").unwrap();
        std::fs::set_permissions(&tool, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let env = d.path().join("toolchain.env");
        std::fs::write(
            &env,
            env_file(&resolved(&[&bin.to_string_lossy()], &[]), None),
        )
        .unwrap();
        let home = d.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".profile"), "PATH=/usr/bin:/bin\n").unwrap();
        std::fs::write(home.join(".bash_profile"), "PATH=/usr/bin:/bin\n").unwrap();
        let o = run(
            Cmd::new(["bash", "-l", "-c", "t"].map(String::from).to_vec())
                .env("HOME", home.to_string_lossy().to_string())
                .env("PATH", "/usr/bin:/bin")
                .env("BASH_ENV", env.to_string_lossy().to_string()),
        )
        .await
        .unwrap();
        assert!(o.success(), "{}", o.stderr);
        assert_eq!(o.stdout_str(), "from-toolchain\n");
    }

    #[test]
    fn path_entries_go_first() {
        let p = new_path(&[PathBuf::from("/a/bin")]);
        assert!(p.to_string_lossy().starts_with("/a/bin:"));
    }
}
