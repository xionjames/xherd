//! The real `xherd` binary: hook/report contract and an end-to-end run of the
//! daemon with a private tmux server.

use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_xherd");

fn xherd(home: &Path, cwd: &Path, args: &[&str], env: &[(&str, &str)], stdin: &str) -> Output {
    let mut c = Command::new(BIN);
    c.args(args)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .env("HOME", home)
        .env("XHERD_HOME", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in env {
        c.env(k, v);
    }
    let mut child = c.spawn().unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn queue_lines(home: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(home.join("hook-queue.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn hook_is_silent_and_queues_when_daemon_is_down() {
    let home = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    let down = format!("http://127.0.0.1:{}", free_port());

    // Not under xherd: nothing happens, exit 0, no output.
    let o = xherd(home.path(), wt.path(), &["hook", "stop"], &[], "{}");
    assert!(o.status.success() && o.stdout.is_empty() && o.stderr.is_empty());
    // Garbage arguments still exit 0.
    let o = xherd(home.path(), wt.path(), &["hook"], &[], "");
    assert!(o.status.success() && o.stdout.is_empty());

    std::fs::create_dir(wt.path().join(".xherd")).unwrap();
    std::fs::write(
        wt.path().join(".xherd/agent.json"),
        r#"{"id":"A1","name":"SC-1"}"#,
    )
    .unwrap();
    let sub = wt.path().join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let env = [("XHERD_URL", down.as_str()), ("XHERD_AGENT_TOKEN", "xh_x")];
    let o = xherd(
        home.path(),
        &sub,
        &["hook", "user-prompt-submit"],
        &env,
        r#"{"prompt":"hi","session_id":"s1"}"#,
    );
    assert!(o.status.success() && o.stdout.is_empty(), "{o:?}");
    let o = xherd(home.path(), &sub, &["hook", "stop"], &env, "not json");
    assert!(o.status.success() && o.stdout.is_empty());

    let q = queue_lines(home.path());
    assert_eq!(q.len(), 2);
    assert_eq!(q[0]["jobId"], "A1");
    assert_eq!(q[0]["event"], "user-prompt-submit");
    assert_eq!(q[0]["payload"]["session_id"], "s1");
    assert_eq!(q[1]["payload"]["raw"], "not json");
    assert!(
        q[1]["seq"].as_u64() > q[0]["seq"].as_u64(),
        "monotonic seq from .xherd/seq"
    );
}

#[test]
fn report_exit_codes() {
    let home = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    let o = xherd(
        home.path(),
        wt.path(),
        &["report", "--state", "idle"],
        &[],
        "",
    );
    assert_eq!(o.status.code(), Some(4));
    let o = xherd(
        home.path(),
        wt.path(),
        &["report", "--state", "sleepy"],
        &[],
        "",
    );
    assert_eq!(o.status.code(), Some(64));
    let o = xherd(
        home.path(),
        wt.path(),
        &["report"],
        &[("XHERD_AGENT_ID", "A1")],
        "",
    );
    assert_eq!(o.status.code(), Some(64));

    let down = format!("http://127.0.0.1:{}", free_port());
    let env = [("XHERD_AGENT_ID", "A1"), ("XHERD_URL", down.as_str())];
    let args = [
        "report",
        "--state",
        "blocked",
        "--message",
        "need a key",
        "--seq",
        "7",
    ];
    let o = xherd(home.path(), wt.path(), &args, &env, "");
    assert_eq!(o.status.code(), Some(0));
    let q = queue_lines(home.path());
    assert_eq!(
        (q[0]["event"].as_str(), q[0]["seq"].as_u64()),
        (Some("blocked"), Some(7))
    );
    assert_eq!(q[0]["payload"]["message"], "need a key");
}

struct Proc(Child, String);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
        let _ = Command::new("tmux")
            .args(["-L", &self.1, "kill-server"])
            .output();
    }
}

#[test]
fn end_to_end_replay_and_live_hooks() {
    if Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    let port = free_port();
    let sock = format!("xherd-e2e-{}", std::process::id());
    std::fs::write(
        home.path().join("config.json"),
        serde_json::json!({"port": port, "tmuxSocket": sock}).to_string(),
    )
    .unwrap();

    // Seed a running agent with a live session and an agent token, as M1's `xherd new` will.
    let token = "xh_e2e_token";
    let id = {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let store = xherd_store::Store::open(
                &home.path().join("xherd.db"),
                &home.path().join("backups"),
            )
            .unwrap();
            let mut a = xherd_core::Job::new("SC-1", 1);
            a.status = xherd_core::Status::Running;
            store.put_job(&a).await.unwrap();
            store
                .insert_token(
                    "t1",
                    "agent",
                    Some(&a.id),
                    &["hooks", "xap"],
                    &xherd_daemon::hash_token(token),
                    1,
                )
                .await
                .unwrap();
            a.id
        })
    };
    let ok = Command::new("tmux")
        .args([
            "-L",
            &sock,
            "new-session",
            "-d",
            "-s",
            "SC-1",
            "sleep",
            "600",
        ])
        .status()
        .unwrap();
    assert!(ok.success());

    // While the daemon is down, a stop is queued.
    let url = format!("http://127.0.0.1:{port}");
    let env = [
        ("XHERD_JOB_ID", id.as_str()),
        ("XHERD_URL", url.as_str()),
        ("XHERD_JOB_TOKEN", token),
    ];
    xherd(
        home.path(),
        wt.path(),
        &["hook", "stop"],
        &env,
        r#"{"message":"done"}"#,
    );
    assert_eq!(queue_lines(home.path()).len(), 1);

    let child = Command::new(BIN)
        .arg("daemon")
        .env("XHERD_HOME", home.path())
        .env("XHERD_LOG", "warn")
        // The container exports its own queue file; the test replays its own.
        .env_remove("XHERD_QUEUE")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _daemon = Proc(child, sock.clone());
    let start = Instant::now();
    loop {
        let o = xherd(home.path(), wt.path(), &["status"], &[], "");
        if o.status.success() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "daemon did not come up"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let ls = || String::from_utf8(xherd(home.path(), wt.path(), &["ls"], &[], "").stdout).unwrap();
    assert!(
        ls().contains("idle"),
        "queued stop replayed on start: {}",
        ls()
    );
    assert!(!home.path().join("hook-queue.jsonl").exists());

    // Live hook: delivered directly, nothing queued, and fast.
    let t = Instant::now();
    let o = xherd(
        home.path(),
        wt.path(),
        &["hook", "user-prompt-submit"],
        &env,
        r#"{"prompt":"go","session_id":"sess-9"}"#,
    );
    let took = t.elapsed();
    assert!(o.status.success() && o.stdout.is_empty());
    assert!(!home.path().join("hook-queue.jsonl").exists());
    let out = ls();
    assert!(out.contains("running") && out.contains("sess-9"), "{out}");
    eprintln!("live hook round trip (process start included): {took:?}");

    // A wrong token is rejected, not queued.
    let bad = [
        ("XHERD_JOB_ID", id.as_str()),
        ("XHERD_URL", url.as_str()),
        ("XHERD_JOB_TOKEN", "nope"),
    ];
    xherd(home.path(), wt.path(), &["hook", "stop"], &bad, "{}");
    assert!(!home.path().join("hook-queue.jsonl").exists());

    // XAP: accepted (0), refused (2), refused with --json on stdout.
    let o = xherd(home.path(), wt.path(), &["ask", "which db?"], &env, "");
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert!(ls().contains("question: which db?"), "{}", ls());
    // The same question again changes nothing and says why.
    let o = xherd(home.path(), wt.path(), &["ask", "which db?"], &env, "");
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert_eq!(
        String::from_utf8_lossy(&o.stdout).trim(),
        "ask: no change (this question is already asked)",
        "{o:?}"
    );
    let o = xherd(home.path(), wt.path(), &["approve", "SC-1"], &[], "");
    assert_eq!(o.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("answer it with"),
        "{o:?}"
    );
    let o = xherd(home.path(), wt.path(), &["task", "start", "T01"], &env, "");
    assert_eq!(o.status.code(), Some(2), "{o:?}");
    assert!(String::from_utf8_lossy(&o.stderr).contains("no_flow"));
    let o = xherd(
        home.path(),
        wt.path(),
        &["--json", "stage", "enter", "spec"],
        &env,
        "",
    );
    assert_eq!(o.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        (v["ok"].as_bool(), v["code"].as_str()),
        (Some(false), Some("no_flow"))
    );

    // User actions through the CLI end the tmux session.
    let o = xherd(home.path(), wt.path(), &["pause", "SC-1"], &[], "");
    assert!(
        String::from_utf8_lossy(&o.stdout).contains("paused"),
        "{o:?}"
    );
    let has = Command::new("tmux")
        .args(["-L", &sock, "has-session", "-t", "=SC-1"])
        .output()
        .unwrap();
    assert!(!has.status.success(), "session ended");
}

#[test]
fn xap_exit_codes_without_daemon() {
    let home = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    let down = format!("http://127.0.0.1:{}", free_port());

    // Not under xherd.
    let o = xherd(home.path(), wt.path(), &["gate", "open", "spec"], &[], "");
    assert_eq!(o.status.code(), Some(4));

    let env = [
        ("XHERD_JOB_ID", "A1"),
        ("XHERD_URL", down.as_str()),
        ("XHERD_JOB_TOKEN", "t"),
    ];
    // Needs an answer: unreachable.
    let o = xherd(home.path(), wt.path(), &["gate", "open", "spec"], &env, "");
    assert_eq!(o.status.code(), Some(3), "{o:?}");
    assert!(queue_lines(home.path()).is_empty());
    // Report-only: queued.
    let o = xherd(
        home.path(),
        wt.path(),
        &["task", "done", "T01", "--evidence", "tests pass"],
        &env,
        "",
    );
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    let q = queue_lines(home.path());
    assert_eq!(
        (q[0]["cmd"].as_str(), q[0]["args"]["id"].as_str()),
        (Some("task.done"), Some("T01"))
    );
    assert_eq!(q[0]["jobId"], "A1");
    // Usage errors.
    let o = xherd(home.path(), wt.path(), &["task", "done", "T01"], &env, "");
    assert_eq!(o.status.code(), Some(64));
    let o = xherd(
        home.path(),
        wt.path(),
        &["issue", "add", "--title", "x", "--severity", "huge"],
        &env,
        "",
    );
    assert_eq!(o.status.code(), Some(64));
}

/// R7: `xherd terminal-handler install|uninstall` renders the installer from
/// the saved config and runs it for this user, with no daemon running.
#[test]
fn terminal_handler_install_and_uninstall() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().canonicalize().unwrap();
    let bin = home.join("fake-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = home.join("tools.log");
    for t in ["xdg-mime", "update-desktop-database", "curl"] {
        let p = bin.join(t);
        std::fs::write(
            &p,
            format!("#!/bin/sh\necho \"{t} $*\" >> '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let workspace = home.join("ws");
    std::fs::write(
        home.join("config.json"),
        serde_json::json!({"port": 47999, "workspace": workspace}).to_string(),
    )
    .unwrap();
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let env = [("PATH", path.as_str()), ("XHERD_TERM_OS", "linux")];
    let handler = home.join(".local/bin/xherd-open-terminal");
    let desktop = home.join(".local/share/applications/xherd-term.desktop");
    let conf = home.join(".config/xherd/terminal-handler.conf");

    let o = xherd(&home, &home, &["terminal-handler", "install"], &env, "");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(handler.is_file() && desktop.is_file());
    let root = workspace.join("worktrees");
    let conf_text = std::fs::read_to_string(&conf).unwrap();
    assert!(
        conf_text.contains(&format!("root={}", root.display())),
        "{conf_text}"
    );
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("xdg-mime default xherd-term.desktop"),
        "{calls}"
    );
    assert!(
        calls.contains("http://127.0.0.1:47999/api/v1/open-in/terminal-handler"),
        "{calls}"
    );

    let o = xherd(&home, &home, &["terminal-handler", "uninstall"], &env, "");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(!handler.exists() && !desktop.exists() && !conf.exists());

    // Inside the container it refuses: the handler belongs on the user's computer.
    let mut env = env.to_vec();
    env.push(("XHERD_SANDBOX", "1"));
    let o = xherd(&home, &home, &["terminal-handler", "install"], &env, "");
    assert_eq!(o.status.code(), Some(2));
    assert!(!handler.exists());
}

/// #141: a job's session has `XHERD_JOB_*` and `.xherd/job.json`; the old
/// names (the test above) still work for sessions started before.
#[test]
fn hook_reads_the_job_names() {
    let home = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    let down = format!("http://127.0.0.1:{}", free_port());
    std::fs::create_dir(wt.path().join(".xherd")).unwrap();
    std::fs::write(
        wt.path().join(".xherd/job.json"),
        r#"{"id":"J1","name":"sc-1"}"#,
    )
    .unwrap();
    let env = [("XHERD_URL", down.as_str()), ("XHERD_JOB_TOKEN", "xh_x")];
    let o = xherd(home.path(), wt.path(), &["hook", "stop"], &env, "{}");
    assert!(o.status.success() && o.stdout.is_empty(), "{o:?}");
    let q = queue_lines(home.path());
    assert_eq!(q.len(), 1);
    assert_eq!(q[0]["jobId"], "J1");
    // The environment's id wins over the file's.
    let env = [
        ("XHERD_URL", down.as_str()),
        ("XHERD_JOB_TOKEN", "xh_x"),
        ("XHERD_JOB_ID", "J2"),
    ];
    xherd(home.path(), wt.path(), &["hook", "stop"], &env, "{}");
    assert_eq!(queue_lines(home.path())[1]["jobId"], "J2");
}

/// A local git repository holding `files` (path, content), as a `file://` url.
fn plugin_repo(files: &[(&str, String)]) -> (tempfile::TempDir, String) {
    let repo = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let p = repo.path().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    for args in [
        vec!["init", "--quiet", "-b", "main"],
        vec!["add", "--all"],
        vec!["commit", "--quiet", "-m", "x"],
    ] {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(&args)
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
    }
    let url = format!("file://{}", repo.path().display());
    (repo, url)
}

fn manifest(name: &str) -> String {
    serde_json::json!({"name": name, "kind": "host", "version": "1.0.0"}).to_string()
}

/// `xherd plugin …` against a daemon run in this process, which alone may
/// take `file://` sources (the binary never does).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_install_update_remove_and_ls() {
    let home = tempfile::tempdir().unwrap();
    let paths = xherd_core::paths::Paths::at(home.path());
    paths.ensure().unwrap();
    let ws = home.path().join("ws");
    xherd_core::config::Config {
        workspace: Some(ws.to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let mut o = xherd_daemon::Options::new(
        std::sync::Arc::new(xherd_daemon::FakeDriver::default()),
        std::sync::Arc::new(xherd_daemon::SystemClock),
    );
    o.scheduler = None;
    o.allow_local_plugin_sources = true;
    let d = xherd_daemon::Daemon::start(paths, o).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(xherd_api::serve(d, listener, std::future::pending()));
    let cwd = tempfile::tempdir().unwrap();
    let env = [("XHERD_URL", url.as_str())];
    let run = |args: &[&str]| {
        let o = xherd(home.path(), cwd.path(), args, &env, "");
        (
            o.status.code(),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    };

    let (repo, src) = plugin_repo(&[("xherd-plugin.json", manifest("gitea"))]);

    // No terminal and no --yes: nothing is installed.
    let (code, _, err) = run(&["plugin", "install", &src]);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("--yes"), "{err}");
    assert!(!run(&["plugin", "ls"]).1.contains("gitea"));

    // --yes installs, and `ls` shows where it came from.
    let (code, out, err) = run(&["plugin", "install", &src, "--yes"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.contains("gitea 1.0.0") && out.contains("xherd daemon restart"),
        "{out}"
    );
    let sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "--short=7", "HEAD"])
            .current_dir(repo.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    // Not loaded yet: the running daemon says a restart is needed.
    let (_, ls, _) = run(&["plugin", "ls"]);
    assert!(
        ls.contains("Restart needed") && ls.contains("built-in"),
        "{ls}"
    );
    // A daemon started afterwards (as after a restart) lists it with its source.
    let mut o2 = xherd_daemon::Options::new(
        std::sync::Arc::new(xherd_daemon::FakeDriver::default()),
        std::sync::Arc::new(xherd_daemon::SystemClock),
    );
    o2.scheduler = None;
    let d2 = xherd_daemon::Daemon::start(xherd_core::paths::Paths::at(home.path()), o2)
        .await
        .unwrap();
    let l2 = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url2 = format!("http://{}", l2.local_addr().unwrap());
    tokio::spawn(xherd_api::serve(d2, l2, std::future::pending()));
    let o = xherd(
        home.path(),
        cwd.path(),
        &["plugin", "ls"],
        &[("XHERD_URL", url2.as_str())],
        "",
    );
    let ls = String::from_utf8_lossy(&o.stdout);
    let line = ls.lines().find(|l| l.starts_with("gitea")).unwrap();
    assert!(line.ends_with(&format!("{src}@{}", sha.trim())), "{ls}");
    assert!(!ls.contains("Restart needed"), "{ls}");

    // Installing it again is refused.
    let (code, _, err) = run(&["plugin", "install", &src, "--yes"]);
    assert_eq!(code, Some(2));
    assert!(
        err.contains("plugin_exists") || err.contains("already"),
        "{err}"
    );

    // Update shows the manifest diff first.
    std::fs::write(
        repo.path().join("xherd-plugin.json"),
        serde_json::json!({"name": "gitea", "kind": "host", "version": "1.1.0"}).to_string(),
    )
    .unwrap();
    for args in [vec!["add", "--all"], vec!["commit", "--quiet", "-m", "v2"]] {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(&args)
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(out.status.success());
    }
    let (code, _, err) = run(&["plugin", "update", "gitea"]);
    assert_eq!(code, Some(2), "{err}");
    let (code, out, err) = run(&["plugin", "update", "gitea", "--yes"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.contains("-  \"version\": \"1.0.0\"") && out.contains("+  \"version\": \"1.1.0\""),
        "{out}"
    );
    assert!(out.contains("Updated gitea to 1.1.0"), "{out}");

    // Remove.
    let (code, _, _) = run(&["plugin", "remove", "gitea"]);
    assert_eq!(code, Some(2));
    let (code, out, err) = run(&["plugin", "remove", "gitea", "--forget", "--yes"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(
        out.contains("Removed gitea with its settings and secrets"),
        "{out}"
    );
    // Installed and removed again: the loaded set is as it was at start.
    assert!(!run(&["plugin", "ls"]).1.contains("Restart needed"));
    let (code, _, err) = run(&["plugin", "remove", "github", "--yes"]);
    assert_eq!(code, Some(2), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plugin_install_with_several_plugins_asks_for_a_path() {
    let home = tempfile::tempdir().unwrap();
    let paths = xherd_core::paths::Paths::at(home.path());
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(home.path().join("ws").to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let mut o = xherd_daemon::Options::new(
        std::sync::Arc::new(xherd_daemon::FakeDriver::default()),
        std::sync::Arc::new(xherd_daemon::SystemClock),
    );
    o.scheduler = None;
    o.allow_local_plugin_sources = true;
    let d = xherd_daemon::Daemon::start(paths, o).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(xherd_api::serve(d, listener, std::future::pending()));
    let cwd = tempfile::tempdir().unwrap();
    let env = [("XHERD_URL", url.as_str())];
    let (_repo, src) = plugin_repo(&[
        ("plugins/a/xherd-plugin.json", manifest("alpha")),
        ("plugins/b/xherd-plugin.json", manifest("beta")),
    ]);
    let o = xherd(
        home.path(),
        cwd.path(),
        &["plugin", "install", &src, "--yes"],
        &env,
        "",
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{o:?}");
    assert!(
        err.contains("--path") && err.contains("plugins/a") && err.contains("plugins/b"),
        "{err}"
    );
    let o = xherd(
        home.path(),
        cwd.path(),
        &["plugin", "install", &src, "--path", "plugins/b", "--yes"],
        &env,
        "",
    );
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert!(String::from_utf8_lossy(&o.stdout).contains("beta 1.0.0"));
}
