//! XAP calls through the daemon with real files and git in a temp worktree.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::domain::{CommitStatus, Gate};
use xherd_core::paths::Paths;
use xherd_core::report::{Report, ReportEvent};
use xherd_core::xap::{Refusal, XapCall};
use xherd_core::{Agent, Status, queue};
use xherd_daemon::*;

struct T {
    _home: tempfile::TempDir,
    wt: tempfile::TempDir,
    d: Daemon,
    drv: Arc<FakeDriver>,
    a: Agent,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    git(wt.path(), &["init", "-q", "-b", "main"]);
    git(wt.path(), &["commit", "-q", "--allow-empty", "-m", "base"]);
    std::fs::create_dir_all(wt.path().join(".xherd/spec")).unwrap();
    let drv = Arc::new(FakeDriver::default());
    let o = Options::new(drv.clone(), Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let mut a = Agent::new("SC-1", "SC-1", 1);
    a.launch = Some(vec!["rt".into()]);
    a.workdir = Some(wt.path().to_string_lossy().into_owned());
    a.pack = Some("xhspec".into());
    a.flow = Some("feature".into());
    let a = d.create_agent(a).await.unwrap();
    for _ in 0..200 {
        if d.action("SC-1", UserAction::Get).await.unwrap().status == Status::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    T {
        _home: home,
        wt,
        d,
        drv,
        a,
    }
}

fn call(a: &Agent, id: &str, cmd: &str, args: Value) -> XapCall {
    serde_json::from_value(json!({"id": id, "agentId": a.id, "cmd": cmd, "args": args, "at": 1}))
        .unwrap()
}

async fn xap(t: &T, id: &str, cmd: &str, args: Value) -> Result<Value, Refusal> {
    t.d.xap(call(&t.a, id, cmd, args)).await
}

async fn get(t: &T) -> Agent {
    t.d.action("SC-1", UserAction::Get).await.unwrap()
}

#[tokio::test]
async fn gate_open_then_stop_waits_for_input() {
    let t = setup().await;
    let spec = t.wt.path().join(".xherd/spec/spec.md");

    let e = xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "doc_missing");

    std::fs::write(&spec, "---\nticket: SC-1\n---\n# Spec\r\n").unwrap();
    let r = xap(&t, "c2", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    assert_eq!(r["changed"], true);
    let a = get(&t).await;
    let Some(Gate::Docs { files, .. }) = &a.gate else {
        panic!("{:?}", a.gate)
    };
    assert_eq!(files[0].path, "spec.md");
    assert_eq!(
        files[0].sha256,
        xherd_core::hash::content_sha256(b"---\nticket: SC-1\n---\n# Spec\n")
    );
    // Replay of the same call id: no-op.
    assert_eq!(
        xap(&t, "c2", "gate.open", json!({"stage": "spec"}))
            .await
            .unwrap()["duplicate"],
        true
    );

    let stop = Report {
        id: "s1".into(),
        agent_id: t.a.id.clone(),
        event: ReportEvent::Stop,
        payload: Value::Null,
        seq: 1,
        at: 0,
    };
    t.d.report(stop).await;
    assert_eq!(get(&t).await.status, Status::WaitingInput);

    let events = t.d.store.events(&t.a.id, 50).await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.id == "c2" && e.kind == "xap.gate.open")
    );
}

#[tokio::test]
async fn already_approved_and_bad_frontmatter() {
    let t = setup().await;
    let spec = t.wt.path().join(".xherd/spec/spec.md");
    std::fs::write(&spec, "---\nticket: [\n---\n").unwrap();
    let e = xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "doc_schema");

    std::fs::write(&spec, "# ok\n").unwrap();
    let sha = xherd_core::hash::content_sha256(b"# ok\n");
    let id = t.a.id.clone();
    t.d.store
        .write(move |c| {
            c.execute(
                "INSERT INTO approvals (agent_id, file, sha256, at, actor, auto) VALUES (?1, 'spec.md', ?2, 0, 'user', 0)",
                [&id, &sha],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let e = xap(&t, "c2", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "already_approved");
}

#[tokio::test]
async fn symlink_escape_is_refused() {
    let t = setup().await;
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("x.md"), "x").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("x.md"),
        t.wt.path().join(".xherd/spec/spec.md"),
    )
    .unwrap();
    let e = xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "path_outside_workspace");
}

#[tokio::test]
async fn tasks_and_commit_with_real_git() {
    let t = setup().await;
    std::fs::write(
        t.wt.path().join(".xherd/spec/tasks.md"),
        "## T01: add a\n- files: a.txt\n",
    )
    .unwrap();
    xap(&t, "c1", "tasks.sync", json!({})).await.unwrap();
    xap(&t, "c2", "task.start", json!({"id": "T01"}))
        .await
        .unwrap();
    let e = xap(
        &t,
        "c3",
        "commit.propose",
        json!({"id": "C01", "subject": "a", "files": ["b.txt"], "tasks": ["T01"]}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "files_outside_scope");
    xap(
        &t,
        "c4",
        "commit.propose",
        json!({"id": "C01", "subject": "a", "files": ["a.txt"], "tasks": ["T01"]}),
    )
    .await
    .unwrap();
    assert!(matches!(get(&t).await.gate, Some(Gate::Commit(_))));

    std::fs::write(t.wt.path().join("a.txt"), "a").unwrap();
    std::fs::write(t.wt.path().join("b.txt"), "b").unwrap();
    git(t.wt.path(), &["add", "a.txt", "b.txt"]);
    git(t.wt.path(), &["commit", "-q", "-m", "a and b"]);
    let wrong = git(t.wt.path(), &["rev-parse", "HEAD"]);

    let e = xap(&t, "c5", "commit.done", json!({"id": "C01", "sha": wrong}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "commit_not_approved");

    // Approving comes with hash approvals; mark it approved in the store and
    // restart on the same data dir (the fake session stays alive).
    let mut a = get(&t).await;
    a.commits[0].status = CommitStatus::Approved;
    t.d.store.put_agent(&a).await.unwrap();
    let o = Options::new(t.drv.clone(), Arc::new(SystemClock));
    let d = Daemon::start(t.d.paths.clone(), o).await.unwrap();
    let t = T { d, ..t };

    let e = xap(
        &t,
        "c6",
        "commit.done",
        json!({"id": "C01", "sha": &wrong[..10]}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "sha_files_mismatch");
    let e = xap(
        &t,
        "c7",
        "commit.done",
        json!({"id": "C01", "sha": "deadbeef"}),
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "sha_unknown");

    git(t.wt.path(), &["reset", "-q", "--soft", "HEAD~1"]);
    git(t.wt.path(), &["reset", "-q", "b.txt"]);
    git(t.wt.path(), &["commit", "-q", "-m", "a"]);
    let good = git(t.wt.path(), &["rev-parse", "HEAD"]);
    xap(
        &t,
        "c8",
        "commit.done",
        json!({"id": "C01", "sha": &good[..8]}),
    )
    .await
    .unwrap();
    let a = get(&t).await;
    assert_eq!(a.commits[0].sha.as_deref(), Some(good.as_str()));
    assert!(a.gate.is_none());
}

#[tokio::test]
async fn issue_ledger_and_queued_calls() {
    let t = setup().await;
    xap(
        &t,
        "i1",
        "issue.add",
        json!({"title": "flaky test", "severity": "minor"}),
    )
    .await
    .unwrap();
    let ledger = std::fs::read_to_string(t.wt.path().join(".xherd/ledger/issues.jsonl")).unwrap();
    assert!(ledger.contains("flaky test"));

    std::fs::write(t.wt.path().join(".xherd/spec/tasks.md"), "## T01: a\n").unwrap();
    xap(&t, "c1", "tasks.sync", json!({})).await.unwrap();
    queue::append(
        &t.d.paths.hook_queue(),
        &call(&t.a, "q1", "task.start", json!({"id": "T01"})).into(),
    )
    .unwrap();
    assert_eq!(t.d.replay_queue().await.unwrap(), 1);
    assert_eq!(
        get(&t).await.tasks[0].status,
        xherd_core::domain::TaskStatus::Doing
    );
}

#[tokio::test]
async fn refused_when_not_live_or_without_flow() {
    let t = setup().await;
    t.d.action("SC-1", UserAction::Pause).await.unwrap();
    let e = xap(&t, "c1", "stage.enter", json!({"stage": "spec"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_live");

    let mut b = Agent::new("SC-2", "SC-2", 2);
    b.launch = Some(vec!["rt".into()]);
    let b = t.d.create_agent(b).await.unwrap();
    for _ in 0..200 {
        if t.d.action("SC-2", UserAction::Get).await.unwrap().status == Status::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let e =
        t.d.xap(call(&b, "c2", "stage.enter", json!({"stage": "spec"})))
            .await
            .unwrap_err();
    assert_eq!(e.code, "no_flow");
    let e =
        t.d.xap(call(
            &Agent::new("x", "x", 0),
            "c3",
            "ask",
            json!({"text": "?"}),
        ))
        .await
        .unwrap_err();
    assert_eq!(e.code, "unknown_agent");
}
