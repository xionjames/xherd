//! `Daemon::new_agent` against a real temp git repo (R-001..R-005c).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use xherd_core::new::NewAgent;
use xherd_core::paths::Paths;
use xherd_core::{Agent, Status};
use xherd_daemon::*;

struct T {
    _home: tempfile::TempDir,
    ws: tempfile::TempDir,
    repo: tempfile::TempDir,
    d: Daemon,
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
}

async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    assert!(
        git(repo.path(), &["init", "-q", "-b", "main"])
            .status
            .success()
    );
    assert!(
        git(
            repo.path(),
            &["commit", "-q", "--allow-empty", "-m", "base"]
        )
        .status
        .success()
    );
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    let cfg = xherd_core::config::Config {
        workspace: Some(ws.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    cfg.save(&paths.config()).unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(paths, o).await.unwrap();
    T {
        _home: home,
        ws,
        repo,
        d,
    }
}

fn req(t: &T, key: &str) -> NewAgent {
    NewAgent {
        ticket_key: key.into(),
        slug: None,
        repo: t.repo.path().to_string_lossy().into_owned(),
        base: None,
        flow: None,
        prompt: Some("add login".into()),
    }
}

fn repo_name(t: &T) -> String {
    t.repo
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[tokio::test]
async fn creates_worktree_and_private_folder() {
    let t = setup().await;
    let mut r = req(&t, "sc-1");
    r.slug = Some("login".into());
    let a: Agent = t.d.new_agent(r).await.unwrap();
    assert_eq!(
        (a.name.as_str(), a.ticket_key.as_str()),
        ("SC-1-login", "SC-1")
    );
    assert_eq!(
        (a.pack.as_deref(), a.flow.as_deref()),
        (Some("xhspec"), Some("feature"))
    );
    assert_eq!(
        a.next_prompt.as_deref(),
        Some("/xhspec:feature SC-1 add login")
    );
    assert_eq!(a.base.as_deref(), Some("main"));

    let wt =
        t.ws.path()
            .join("worktrees")
            .join(repo_name(&t))
            .join("SC-1-login");
    assert_eq!(a.workdir.as_deref(), Some(wt.to_string_lossy().as_ref()));
    let head = git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "SC-1-login");
    assert!(
        !git(&wt, &["rev-parse", "--abbrev-ref", "@{u}"])
            .status
            .success(),
        "no upstream (R-004)"
    );

    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(wt.join(".xherd/agent.json")).unwrap()).unwrap();
    assert_eq!(
        (j["id"].as_str(), j["ticketKey"].as_str()),
        (Some(a.id.as_str()), Some("SC-1"))
    );
    assert!(
        std::fs::read_to_string(wt.join(".xherd/instructions.md"))
            .unwrap()
            .contains("SC-1")
    );
    assert!(wt.join(".xherd/spec").is_dir());
    let status = git(&wt, &["status", "--porcelain", "--untracked-files=all"]);
    assert!(
        status.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[tokio::test]
async fn refusals_leave_nothing_behind() {
    let t = setup().await;
    let code = |r: Result<Agent, ActionError>| r.unwrap_err().code;

    let mut r = req(&t, "SC-2");
    r.base = Some("nope".into());
    assert_eq!(code(t.d.new_agent(r).await), "base_unresolved");
    assert!(
        !t.ws
            .path()
            .join("worktrees")
            .join(repo_name(&t))
            .join("SC-2")
            .exists()
    );
    assert!(
        !git(
            t.repo.path(),
            &["show-ref", "--verify", "--quiet", "refs/heads/SC-2"]
        )
        .status
        .success()
    );

    let mut r = req(&t, "SC-2");
    r.flow = Some("bug".into());
    assert_eq!(code(t.d.new_agent(r).await), "unknown_flow");
    let mut r = req(&t, "SC-2");
    r.repo = t.ws.path().to_string_lossy().into_owned();
    assert_eq!(code(t.d.new_agent(r).await), "not_a_repo");

    t.d.new_agent(req(&t, "SC-2")).await.unwrap();
    assert_eq!(code(t.d.new_agent(req(&t, "SC-2")).await), "name_taken");

    assert!(git(t.repo.path(), &["branch", "SC-3"]).status.success());
    assert_eq!(code(t.d.new_agent(req(&t, "SC-3")).await), "branch_exists");
}

#[tokio::test]
async fn flow_none_and_origin_base() {
    let t = setup().await;
    // A clone whose origin/HEAD points at main.
    let clone = tempfile::tempdir().unwrap();
    let src = t.repo.path().to_string_lossy().into_owned();
    assert!(
        git(clone.path(), &["clone", "-q", &src, "c"])
            .status
            .success()
    );
    let c = clone.path().join("c");
    assert!(
        git(&c, &["checkout", "-q", "-b", "local-work"])
            .status
            .success()
    );
    let mut r = req(&t, "SC-4");
    r.repo = c.to_string_lossy().into_owned();
    r.flow = Some("none".into());
    let a = t.d.new_agent(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("origin/main"));
    assert_eq!((a.pack, a.flow), (None, None));
    assert_eq!(a.next_prompt.as_deref(), Some("add login"));
}

#[tokio::test]
async fn without_a_runtime_command_the_agent_errors_clearly() {
    let t = setup().await;
    t.d.new_agent(req(&t, "SC-5")).await.unwrap();
    for _ in 0..200 {
        let a = t.d.action("SC-5", UserAction::Get).await.unwrap();
        if a.status == Status::Error {
            let ev = t.d.store.events(&a.id, 10).await.unwrap();
            assert!(ev.iter().any(|e| e.kind == "start-failed"));
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("agent never reached error");
}
