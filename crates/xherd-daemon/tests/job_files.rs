//! `Daemon::refresh_job_files`: a live job's hook file and push guard follow
//! the daemon's `xherd` binary (R-008, R-009).

use std::path::Path;
use std::sync::Arc;

use xherd_core::Job;
use xherd_core::new::NewJob;
use xherd_core::paths::Paths;
use xherd_daemon::*;

fn git(dir: &Path, args: &[&str]) {
    let o = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}");
}

fn options(home: &Path, ws: &Path, bin: &str) -> Options {
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.xherd_bin = bin.into();
    // Never the real Claude Code config: a fixture that trusts the workspace.
    let trust = home.join("claude.json");
    std::fs::write(
        &trust,
        serde_json::json!({"projects": {ws.to_string_lossy(): {"hasTrustDialogAccepted": true}}})
            .to_string(),
    )
    .unwrap();
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    o
}

fn req(repo: &Path, name: &str) -> NewJob {
    NewJob {
        name: name.into(),
        ticket: None,
        ticket_key: None,
        slug: None,
        repo: repo.to_string_lossy().into_owned(),
        agent: None,
        agents: Default::default(),
        base: None,
        flow: None,
        pack: None,
        prompt: Some("add login".into()),
        autonomy: None,
        permission_mode: None,
        runtime: None,
        model: None,
        new_project: None,
    }
}

fn workdir(j: &Job) -> std::path::PathBuf {
    j.workdir.as_deref().unwrap().into()
}

#[tokio::test]
async fn a_new_binary_reaches_the_hook_file_and_the_guard_of_live_jobs() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "base"],
    );
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(ws.path().to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();

    let a = Daemon::start(
        paths.clone(),
        options(home.path(), ws.path(), "/opt/a/xherd"),
    )
    .await
    .unwrap();
    let live = a.new_job(req(repo.path(), "live")).await.unwrap();
    let finished = a.new_job(req(repo.path(), "finished")).await.unwrap();
    a.action(
        "finished",
        UserAction::Cancel {
            remove_worktree: false,
        },
    )
    .await
    .unwrap();

    let settings = |j: &Job| workdir(j).join(".claude/settings.local.json");
    let guard = |j: &Job| workdir(j).join(".xherd/hooks/pre-push");
    assert!(
        std::fs::read_to_string(settings(&live))
            .unwrap()
            .contains("/opt/a/xherd")
    );
    assert!(
        std::fs::read_to_string(guard(&live))
            .unwrap()
            .contains("/opt/a/xherd")
    );

    // The user adds a hook entry of their own to the job's hook file.
    let mut v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(settings(&live)).unwrap()).unwrap();
    v["hooks"]["PreToolUse"] =
        serde_json::json!([{"hooks": [{"type": "command", "command": "my-linter"}]}]);
    std::fs::write(settings(&live), serde_json::to_vec_pretty(&v).unwrap()).unwrap();
    let done_before = std::fs::read_to_string(settings(&finished)).unwrap();
    drop(a);

    // A daemon with another binary: the live job follows, the done one does not.
    let b = Daemon::start(
        paths.clone(),
        options(home.path(), ws.path(), "/opt/b/xherd"),
    )
    .await
    .unwrap();
    assert_eq!(b.refresh_job_files().await, 1);
    let text = std::fs::read_to_string(settings(&live)).unwrap();
    assert!(text.contains("/opt/b/xherd"), "{text}");
    assert!(!text.contains("/opt/a/xherd"), "{text}");
    assert!(text.contains("my-linter"), "user entry kept: {text}");
    assert!(
        std::fs::read_to_string(guard(&live))
            .unwrap()
            .contains("/opt/b/xherd")
    );
    assert_eq!(
        std::fs::read_to_string(settings(&finished)).unwrap(),
        done_before
    );

    // Nothing differs now: a second refresh writes nothing.
    assert_eq!(b.refresh_job_files().await, 0);
}
