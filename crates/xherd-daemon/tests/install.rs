//! `Daemon::install` (design §6.7): writes, keeps user edits, upgrades.

use std::sync::Arc;

use xherd_core::hash::content_sha256;
use xherd_core::paths::Paths;
use xherd_daemon::install::InstallAction;
use xherd_daemon::*;

#[tokio::test]
async fn installs_keeps_edits_and_upgrades() {
    let home = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let dir = user.path().join(".claude/commands/xherd");

    let first = d.install(None, None, &env, false).await.unwrap();
    assert!(first.len() >= 10);
    assert!(first.iter().all(|f| f.action == InstallAction::Written));
    assert!(dir.join("feature.md").is_file());
    let again = d
        .install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    assert!(again.iter().all(|f| f.action == InstallAction::Unchanged));

    // Edited by the user: kept, new version beside it.
    std::fs::write(dir.join("next.md"), "my own next\n").unwrap();
    // Untouched since an older install: upgraded in place.
    let old = "old status\n";
    std::fs::write(dir.join("status.md"), old).unwrap();
    let status = dir.join("status.md").to_string_lossy().into_owned();
    d.store
        .put_install(
            "claude-code",
            "xherd",
            &status,
            &content_sha256(old.as_bytes()),
        )
        .await
        .unwrap();
    let third = d.install(None, None, &env, false).await.unwrap();
    let action = |name: &str| {
        third
            .iter()
            .find(|f| f.path.ends_with(name))
            .unwrap()
            .action
    };
    assert_eq!(action("next.md"), InstallAction::Kept);
    assert_eq!(
        std::fs::read_to_string(dir.join("next.md")).unwrap(),
        "my own next\n"
    );
    assert!(dir.join("next.md.xherd-new").is_file());
    assert_eq!(action("status.md"), InstallAction::Written);
    assert_ne!(std::fs::read_to_string(dir.join("status.md")).unwrap(), old);

    let e = d
        .install(Some("aider"), None, &env, false)
        .await
        .unwrap_err();
    assert_eq!(e.code, "unknown_runtime");
    let e = d
        .install(None, Some("other"), &env, false)
        .await
        .unwrap_err();
    assert_eq!(e.code, "unknown_pack");

    // Nothing changed since: a run writes nothing.
    let fourth = d.install(None, None, &env, false).await.unwrap();
    assert!(
        fourth
            .iter()
            .filter(|f| !f.path.ends_with("next.md"))
            .all(|f| f.action == InstallAction::Unchanged)
    );

    // A file the pack no longer ships stays unless removal is asked for.
    let gone = user.path().join(".claude/commands/xherd/old/gone.md");
    std::fs::create_dir_all(gone.parent().unwrap()).unwrap();
    std::fs::write(&gone, "gone\n").unwrap();
    let gone_s = gone.to_string_lossy().into_owned();
    d.store
        .put_install("claude-code", "xherd", &gone_s, &content_sha256(b"gone\n"))
        .await
        .unwrap();
    let kept = d
        .install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    let o = kept.iter().find(|f| f.path == gone_s).unwrap();
    assert_eq!(o.action, InstallAction::Obsolete);
    assert!(gone.is_file());
    let removed = d
        .install(Some("claude-code"), None, &env, true)
        .await
        .unwrap();
    let o = removed.iter().find(|f| f.path == gone_s).unwrap();
    assert_eq!(o.action, InstallAction::Removed);
    assert!(!gone.exists());
    assert!(!gone.parent().unwrap().exists(), "emptied folder is pruned");
    assert!(dir.is_dir(), "the install root stays");
    assert!(
        d.store
            .install_sha("claude-code", "xherd", &gone_s)
            .await
            .unwrap()
            .is_none()
    );
}
