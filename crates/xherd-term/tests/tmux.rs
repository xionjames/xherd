//! Real tmux on a private socket; skipped when tmux is not installed.

use std::time::Duration;

use xherd_term::Tmux;

fn tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// True when tmux is 3.5 or newer, which reports a dead pane's exit status.
/// Parses `tmux -V` as `checks::tmux_version` does: `tmux 3.5a`, `tmux next-3.6`.
fn tmux_reports_exit() -> bool {
    let Ok(out) = std::process::Command::new("tmux").arg("-V").output() else {
        return false;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(num) = text.trim().rsplit([' ', '-']).next() else {
        return false;
    };
    let mut parts = num.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts
        .next()
        .map(|s| s.trim_end_matches(|c: char| !c.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (major, minor) >= (3, 5)
}

/// Kills the private server even when an assertion fails.
struct Server(Tmux);
impl Drop for Server {
    fn drop(&mut self) {
        let t = self.0.clone();
        let _ = std::thread::spawn(move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(t.kill_server())
        })
        .join();
    }
}

async fn eventually(mut f: impl AsyncFnMut() -> bool) -> bool {
    for _ in 0..50 {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

#[tokio::test]
async fn session_lifecycle_send_capture_respawn() {
    if !tmux_available() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let sock = format!("xherd-test-{}", std::process::id());
    let srv = Server(Tmux::new(Some(sock)));
    let t = &srv.0;
    let dir = tempfile::tempdir().unwrap();

    assert!(t.list_sessions().await.unwrap().is_empty(), "no server yet");
    let env = vec![("XHERD_AGENT_ID".to_string(), "a1".to_string())];
    t.new_session("SC-1", dir.path(), &env, &["sh".into()])
        .await
        .unwrap();
    assert!(t.has_session("SC-1").await);
    assert!(!t.has_session("SC").await, "exact match only");
    assert_eq!(t.list_sessions().await.unwrap(), vec!["SC-1"]);

    t.send_text("SC-1", Some("agent"), "echo id=$XHERD_AGENT_ID; pwd", 1)
        .await
        .unwrap();
    let want = "id=a1";
    assert!(
        eventually(async || t.capture("SC-1", None, 50).await.unwrap().contains(want)).await,
        "typed command output not seen"
    );

    t.new_window("SC-1", "dev", dir.path(), &["sh".into()])
        .await
        .unwrap();
    t.respawn_window(
        "SC-1",
        Some("agent"),
        dir.path(),
        &["sh".into(), "-c".into(), "echo fresh; sleep 30".into()],
    )
    .await
    .unwrap();
    assert!(
        eventually(async || t
            .capture("SC-1", Some("agent"), 50)
            .await
            .unwrap()
            .contains("fresh"))
        .await
    );
    // The other window survived the respawn.
    assert!(t.capture("SC-1", Some("dev"), 5).await.is_ok());

    t.kill_session("SC-1").await.unwrap();
    assert!(!t.has_session("SC-1").await);
}

#[tokio::test]
async fn held_session_keeps_exit_status_and_history() {
    if !tmux_available() {
        eprintln!("tmux not installed; skipping");
        return;
    }
    let sock = format!("xherd-test-held-{}", std::process::id());
    let srv = Server(Tmux::new(Some(sock)));
    let t = &srv.0;
    let dir = tempfile::tempdir().unwrap();
    let sh = |cmd: &str| vec!["sh".to_string(), "-lc".to_string(), cmd.to_string()];

    assert_eq!(t.pane_status("act-none").await.unwrap(), None);

    // A command that exits at once still leaves its pane, status and output.
    let env = vec![("XHERD_PORT_BASE".to_string(), "4801".to_string())];
    t.start_held(
        "act-a",
        dir.path(),
        &env,
        &sh("echo base=$XHERD_PORT_BASE; echo token=${XHERD_JOB_TOKEN:-none}; pwd; exit 3"),
        1000,
    )
    .await
    .unwrap();
    let mut status = None;
    assert!(
        eventually(async || {
            status = t.pane_status("act-a").await.unwrap();
            status.is_some_and(|s| s.dead)
        })
        .await
    );
    // tmux 3.4 (CI, Ubuntu 24.04) reports a dead pane with no exit status.
    if tmux_reports_exit() {
        assert_eq!(status.unwrap().exit, Some(3));
    }
    let out = t.history("act-a", 1 << 20).await.unwrap();
    assert!(out.contains("base=4801"), "{out}");
    assert!(out.contains("token=none"), "{out}");
    assert!(
        out.contains(dir.path().file_name().unwrap().to_str().unwrap()),
        "{out}"
    );
    let tail = t.history("act-a", 8).await.unwrap();
    assert!(tail.len() <= 8 && out.ends_with(&tail));

    // A running command is alive with no exit code.
    t.start_held("act-b", dir.path(), &[], &sh("sleep 30"), 1000)
        .await
        .unwrap();
    assert_eq!(
        t.pane_status("act-b").await.unwrap(),
        Some(xherd_term::PaneStatus {
            dead: false,
            exit: None
        })
    );
    // A name already taken fails and leaves the first session alone.
    assert!(
        t.start_held("act-b", dir.path(), &[], &sh("true"), 1000)
            .await
            .is_err()
    );
    assert!(t.has_session("act-b").await);
}
