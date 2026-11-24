//! `Daemon::install` (design §6.7): writes, keeps user edits, upgrades.

use std::sync::Arc;

use xherd_core::hash::content_sha256;
use xherd_core::paths::Paths;
use xherd_daemon::install::{Choice, FileState, InstallAction, PackChoice};
use xherd_daemon::*;

/// A first run (no config.json) starts with every plugin off; these tests want
/// the runtimes on, as a configured install has them.
fn home_with_config(home: &tempfile::TempDir) -> Paths {
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    xherd_core::config::Config::default()
        .save(&paths.config())
        .unwrap();
    paths
}

#[tokio::test]
async fn installs_keeps_edits_and_upgrades() {
    let home = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(home_with_config(&home), o).await.unwrap();
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
        pack: "xherd".into(),
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
    let d = Daemon::start(home_with_config(&home), o).await.unwrap();
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
    let d = Daemon::start(home_with_config(&home), o).await.unwrap();
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

#[tokio::test]
async fn start_installs_and_upgrades_when_given_an_env() {
    let home = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let opts = || {
        let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
        let h = h.clone();
        o.install_env = Some(Arc::new(move |k: &str| (k == "HOME").then(|| h.clone())));
        o
    };
    let status = user.path().join(".claude/commands/xherd/status.md");

    // Not sandboxed, yet the first start writes the commands.
    let d = Daemon::start(home_with_config(&home), opts())
        .await
        .unwrap();
    assert!(status.is_file());
    let current = std::fs::read_to_string(&status).unwrap();

    // An older text the daemon recorded is upgraded at the next start.
    let old = "old status\n";
    std::fs::write(&status, old).unwrap();
    let path = status.to_string_lossy().into_owned();
    d.store
        .put_install(
            "claude-code",
            "xherd",
            &path,
            &content_sha256(old.as_bytes()),
        )
        .await
        .unwrap();
    drop(d);
    let d = Daemon::start(home_with_config(&home), opts())
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&status).unwrap(), current);

    // A third start writes nothing.
    let before = std::fs::metadata(&status).unwrap().modified().unwrap();
    drop(d);
    Daemon::start(home_with_config(&home), opts())
        .await
        .unwrap();
    assert_eq!(
        std::fs::metadata(&status).unwrap().modified().unwrap(),
        before
    );
}

const ACME_PACK: &str = r#"
name: acme-sdd
version: 1.2.0
protocol: 1
specDir: private
memoryNote: required
private: [HANDOFF.md]
install:
  claude-code:
    prefix: "/acme-sdd:"
    commands: { from: commands/*.md, to: "${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme-sdd/{name}.md", format: command }
    args: "$ARGUMENTS"
commands: { resume: "{prefix}resume", handoff: "{prefix}handoff", status: "{prefix}status", ask: "{prefix}ask {prompt}" }
flows:
  feature:
    start: "{prefix}start {key} {prompt}"
    stages:
      - { id: brief, role: spec, class: critical, gate: { docs: [spec.md] }, next: [{ run: "{prefix}design" }] }
"#;

const ACME_PROMPTS: [&str; 12] = [
    "rules", "start", "design", "plan", "next", "commit", "review", "ship", "ask", "resume",
    "handoff", "status",
];

/// A home with the `acme-sdd` flow plugin in its plugins folder, with `to`
/// as the claude-code destination of its commands.
fn home_with_acme(to: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    home_with_config(&home);
    let dir = home.path().join("plugins/acme-sdd");
    std::fs::create_dir_all(dir.join("pack/commands")).unwrap();
    std::fs::write(
        dir.join("xherd-plugin.json"),
        serde_json::json!({
            "name": "acme-sdd", "kind": "flow", "version": "1.2.3",
            "data": {"flow": {"pack": "pack"}}
        })
        .to_string(),
    )
    .unwrap();
    let pack = ACME_PACK.replace(
        "${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme-sdd/{name}.md",
        to,
    );
    std::fs::write(dir.join("pack/xherd-flow.yaml"), pack).unwrap();
    for p in ACME_PROMPTS {
        std::fs::write(
            dir.join(format!("pack/commands/{p}.md")),
            format!("# {p}\n"),
        )
        .unwrap();
    }
    home
}

#[tokio::test]
async fn a_plugin_pack_installs_under_its_own_folder() {
    let home = home_with_acme("${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme-sdd/{name}.md");
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let dir = user.path().join(".claude/commands/acme-sdd");

    let done = d
        .install(Some("claude-code"), Some("acme-sdd"), &env, false)
        .await
        .unwrap();
    assert_eq!(done.len(), ACME_PROMPTS.len());
    assert!(done.iter().all(|f| f.action == InstallAction::Written));
    assert!(dir.join("start.md").is_file());
    assert!(!user.path().join(".claude/commands/xherd").exists());
    // Recorded under the plugin's pack, not the built-in one.
    let rows = d.store.installs("claude-code", "acme-sdd").await.unwrap();
    assert_eq!(rows.len(), done.len());
    assert!(
        d.store
            .installs("claude-code", "xherd")
            .await
            .unwrap()
            .is_empty()
    );
    let files = d.pack_files("acme-sdd", "claude-code", &env).await.unwrap();
    assert!(
        files
            .iter()
            .all(|f| f.pack == "acme-sdd" && f.state == FileState::Current)
    );
}

#[tokio::test]
async fn a_plugin_pack_that_escapes_its_folder_writes_nothing() {
    let home = home_with_acme("${HOME}/.ssh/{name}.md");
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());

    // The start-up load already refused the pack.
    let e = d
        .install(Some("claude-code"), Some("acme-sdd"), &env, false)
        .await
        .unwrap_err();
    assert_eq!(e.code, "unknown_pack");
    assert!(!user.path().join(".ssh").exists());
}

#[tokio::test]
async fn a_path_recorded_under_another_pack_is_refused() {
    let home = home_with_acme("${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme-sdd/{name}.md");
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let taken = user.path().join(".claude/commands/acme-sdd/start.md");
    d.store
        .put_install("claude-code", "xherd", &taken.to_string_lossy(), "abc")
        .await
        .unwrap();

    let e = d
        .install(Some("claude-code"), Some("acme-sdd"), &env, false)
        .await
        .unwrap_err();
    assert_eq!(e.code, "path_taken");
    assert!(!taken.exists());
}

#[tokio::test]
async fn remove_pack_files_deletes_unedited_and_prunes_the_folder() {
    let home = home_with_acme("${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme-sdd/{name}.md");
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let dir = user.path().join(".claude/commands/acme-sdd");
    d.install(Some("claude-code"), Some("acme-sdd"), &env, false)
        .await
        .unwrap();
    let start = dir.join("start.md");
    std::fs::write(&start, "my start\n").unwrap();
    let gone = dir.join("plan.md");
    std::fs::remove_file(&gone).unwrap();

    // The edited file stays with its row and is listed as obsolete once the
    // pack is no longer loaded; the others are deleted with their rows.
    let done = d.remove_pack_files("acme-sdd", &env).await.unwrap();
    assert_eq!(done.len(), ACME_PROMPTS.len());
    let kept: Vec<_> = done
        .iter()
        .filter(|f| f.action == InstallAction::Obsolete)
        .collect();
    assert_eq!(kept.len(), 1);
    assert!(kept[0].path.ends_with("/start.md"));
    assert_eq!(std::fs::read_to_string(&start).unwrap(), "my start\n");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    let rows = d.store.installs("claude-code", "acme-sdd").await.unwrap();
    assert_eq!(rows.len(), 1);

    // Nothing edited: the folder goes too, but never the runtime's folder.
    std::fs::remove_file(&start).unwrap();
    d.remove_pack_files("acme-sdd", &env).await.unwrap();
    assert!(!dir.exists());
    assert!(user.path().join(".claude/commands").is_dir());
    assert!(
        d.store
            .installs("claude-code", "acme-sdd")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn files_of_a_pack_that_is_not_loaded_are_reviewed_as_obsolete() {
    let home = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(home_with_config(&home), o).await.unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let dir = user.path().join(".claude/commands/gone-pack");
    std::fs::create_dir_all(&dir).unwrap();
    let (kept, edited) = (dir.join("a.md"), dir.join("b.md"));
    std::fs::write(&kept, "a\n").unwrap();
    std::fs::write(&edited, "mine\n").unwrap();
    for (p, text) in [(&kept, "a\n"), (&edited, "b\n")] {
        d.store
            .put_install(
                "claude-code",
                "gone-pack",
                &p.to_string_lossy(),
                &content_sha256(text.as_bytes()),
            )
            .await
            .unwrap();
    }

    let review = d.pack_review(&env).await.unwrap();
    assert_eq!(review.len(), 2, "{review:?}");
    assert!(review.iter().all(|f| f.pack == "gone-pack"));
    assert!(review.iter().all(|f| f.state == FileState::Obsolete));
    let flag = |p: &std::path::Path| {
        review
            .iter()
            .find(|f| f.path == p.to_string_lossy())
            .unwrap()
            .edited
    };
    assert!(!flag(&kept) && flag(&edited));

    // The user can resolve them under their own pack.
    let pick = |p: &std::path::Path, c: Choice| PackChoice {
        pack: "gone-pack".into(),
        runtime: "claude-code".into(),
        path: p.to_string_lossy().into_owned(),
        choice: c,
    };
    d.resolve_pack_files(
        &[pick(&kept, Choice::Remove), pick(&edited, Choice::Keep)],
        &env,
    )
    .await
    .unwrap();
    assert!(!kept.exists());
    assert_eq!(std::fs::read_to_string(&edited).unwrap(), "mine\n");
    assert!(d.pack_review(&env).await.unwrap().is_empty());
}
