//! Real tmux on a private socket; skipped when tmux is not installed.

use std::time::Duration;

use xherd_term::Tmux;

fn tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success())
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
