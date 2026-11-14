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
    assert_eq!(q[0]["agentId"], "A1");
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
            let mut a = xherd_core::Agent::new("SC-1", 1);
            a.status = xherd_core::Status::Running;
            store.put_agent(&a).await.unwrap();
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
        ("XHERD_AGENT_ID", id.as_str()),
        ("XHERD_URL", url.as_str()),
        ("XHERD_AGENT_TOKEN", token),
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
        ("XHERD_AGENT_ID", id.as_str()),
        ("XHERD_URL", url.as_str()),
        ("XHERD_AGENT_TOKEN", "nope"),
    ];
    xherd(home.path(), wt.path(), &["hook", "stop"], &bad, "{}");
    assert!(!home.path().join("hook-queue.jsonl").exists());

    // XAP: accepted (0), refused (2), refused with --json on stdout.
    let o = xherd(home.path(), wt.path(), &["ask", "which db?"], &env, "");
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    assert!(ls().contains("question: which db?"), "{}", ls());
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
        ("XHERD_AGENT_ID", "A1"),
        ("XHERD_URL", down.as_str()),
        ("XHERD_AGENT_TOKEN", "t"),
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
    assert_eq!(q[0]["agentId"], "A1");
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
