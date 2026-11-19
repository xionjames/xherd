//! `Daemon::install` (design §6.7): writes, keeps user edits, upgrades.

use std::sync::Arc;

use xherd_core::hash::content_sha256;
use xherd_core::paths::Paths;
use xherd_daemon::install::{Choice, InstallAction, PackChoice};
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

fn choice(path: &std::path::Path, c: Choice) -> PackChoice {
    PackChoice {
        runtime: "claude-code".into(),
        path: path.to_string_lossy().into_owned(),
        choice: c,
    }
}

#[tokio::test]
async fn review_and_resolve_pack_files() {
    let home = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let dir = user.path().join(".claude/commands/xherd");
    d.install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    assert!(d.pack_review(&env).await.unwrap().is_empty());

    let next = dir.join("next.md");
    let status = dir.join("status.md");
    let side = dir.join("next.md.xherd-new");
    let pack_next = std::fs::read_to_string(&next).unwrap();
    std::fs::write(&next, "my next\n").unwrap();
    std::fs::write(&status, "my status\n").unwrap();
    let gone = dir.join("old/gone.md");
    std::fs::create_dir_all(gone.parent().unwrap()).unwrap();
    std::fs::write(&gone, "gone\n").unwrap();
    let gone_s = gone.to_string_lossy().into_owned();
    d.store
        .put_install("claude-code", "xherd", &gone_s, &content_sha256(b"gone\n"))
        .await
        .unwrap();
    d.install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    assert!(side.is_file());
    let review = d.pack_review(&env).await.unwrap();
    assert_eq!(review.len(), 3, "{review:?}");

    // A path the review does not list, or a choice that does not fit: nothing changes.
    let stranger = dir.join("feature.md");
    let e = d
        .resolve_pack_files(
            &[
                choice(&next, Choice::KeepMine),
                choice(&stranger, Choice::Remove),
            ],
            &env,
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_pending");
    let e = d
        .resolve_pack_files(&[choice(&gone, Choice::UseNew)], &env)
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_pending");
    assert!(side.is_file());
    assert_eq!(d.pack_review(&env).await.unwrap().len(), 3);

    // Keep mine: the side file goes and the next install leaves it away.
    d.resolve_pack_files(&[choice(&next, Choice::KeepMine)], &env)
        .await
        .unwrap();
    assert!(!side.exists());
    assert_eq!(std::fs::read_to_string(&next).unwrap(), "my next\n");
    let again = d
        .install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    let a = again.iter().find(|f| f.path.ends_with("/next.md")).unwrap();
    assert_eq!(a.action, InstallAction::Declined);
    assert!(!side.exists());
    assert_eq!(d.pack_review(&env).await.unwrap().len(), 2);

    // A different pack text asks again.
    d.store
        .decline_install("claude-code", "xherd", &next.to_string_lossy(), "older")
        .await
        .unwrap();
    d.install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    assert!(side.is_file());

    // Use xherd's: the pack text replaces the file and is recorded.
    d.resolve_pack_files(
        &[
            choice(&next, Choice::UseNew),
            choice(&status, Choice::KeepMine),
        ],
        &env,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read_to_string(&next).unwrap(), pack_next);
    assert!(!side.exists());
    let again = d
        .install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    let a = again.iter().find(|f| f.path.ends_with("/next.md")).unwrap();
    assert_eq!(a.action, InstallAction::Unchanged);

    // Obsolete: keep forgets the record, remove deletes the file and its folder.
    d.resolve_pack_files(&[choice(&gone, Choice::Keep)], &env)
        .await
        .unwrap();
    assert!(gone.is_file());
    assert!(
        d.store
            .install_sha("claude-code", "xherd", &gone_s)
            .await
            .unwrap()
            .is_none()
    );
    assert!(d.pack_review(&env).await.unwrap().is_empty());
    d.store
        .put_install("claude-code", "xherd", &gone_s, &content_sha256(b"gone\n"))
        .await
        .unwrap();
    d.resolve_pack_files(&[choice(&gone, Choice::Remove)], &env)
        .await
        .unwrap();
    assert!(!gone.exists());
    assert!(!gone.parent().unwrap().exists());
    assert!(dir.is_dir());
    assert!(d.pack_review(&env).await.unwrap().is_empty());
}

#[tokio::test]
async fn pack_checks_report_missing_outdated_and_review() {
    let home = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let dir = user.path().join(".claude/commands/xherd");
    let find = |cs: &[xherd_daemon::checks::Check], id: &str| {
        cs.iter().find(|c| c.id == id).cloned().unwrap()
    };

    // Nothing installed: the commands are missing, nothing to review.
    let cs = d.checks(&env).await;
    let pack = find(&cs, "pack");
    assert!(!pack.ok && pack.required);
    assert!(
        pack.detail.contains("command files missing"),
        "{}",
        pack.detail
    );
    assert!(find(&cs, "pack-files").ok);

    d.install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    let cs = d.checks(&env).await;
    assert!(find(&cs, "pack").ok && find(&cs, "pack-files").ok);

    // An old pack text nobody edited: out of date, fixed by install.
    let next = dir.join("next.md");
    std::fs::write(&next, "old pack text\n").unwrap();
    d.store
        .put_install(
            "claude-code",
            "xherd",
            &next.to_string_lossy(),
            &content_sha256(b"old pack text\n"),
        )
        .await
        .unwrap();
    let pack = find(&d.checks(&env).await, "pack");
    assert!(!pack.ok);
    assert!(pack.detail.contains("1 out of date"), "{}", pack.detail);

    // An edited file: the pack check passes, the review check fails.
    std::fs::write(&next, "my next\n").unwrap();
    let cs = d.checks(&env).await;
    assert!(find(&cs, "pack").ok);
    let files = find(&cs, "pack-files");
    assert!(!files.ok && files.required);
    assert!(matches!(
        files.fix,
        Some(xherd_daemon::checks::Fix::Review { .. })
    ));
    let e = d.fix_check("pack-files", &env).await.unwrap_err();
    assert_eq!(e.code, "not_fixable");

    // Declined: nothing left to decide.
    d.install(Some("claude-code"), None, &env, false)
        .await
        .unwrap();
    d.resolve_pack_files(&[choice(&next, Choice::KeepMine)], &env)
        .await
        .unwrap();
    assert!(find(&d.checks(&env).await, "pack-files").ok);
}
