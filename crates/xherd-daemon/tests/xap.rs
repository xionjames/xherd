//! XAP calls through the daemon with real files and git in a temp worktree.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::domain::{CommitStatus, Gate};
use xherd_core::hash::content_sha256;
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
    setup_with(true).await
}

async fn setup_with(fresh_sessions: bool) -> T {
    setup_config(json!({"freshSessions": fresh_sessions})).await
}

async fn setup_config(config: Value) -> T {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("config.json"), config.to_string()).unwrap();
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
    a.base = Some("main".into());
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

async fn stop(t: &T, seq: u64) {
    let r = Report {
        id: format!("stop-{seq}"),
        agent_id: t.a.id.clone(),
        event: ReportEvent::Stop,
        payload: Value::Null,
        seq,
        at: 0,
    };
    t.d.report(r).await;
}

async fn act(t: &T, a: UserAction) -> Result<Agent, ActionError> {
    t.d.action("SC-1", a).await
}

fn sends(t: &T) -> Vec<String> {
    t.drv
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("send "))
        .collect()
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

    stop(&t, 1).await;
    t.d.action(
        "SC-1",
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        get(&t).await.next_prompt.as_deref(),
        Some("/xhspec:commit C01")
    );
    assert_eq!(get(&t).await.commits[0].status, CommitStatus::Approved);

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

#[tokio::test]
async fn approve_docs_gate_hands_on_next_command() {
    // Without fresh sessions the command is typed into the current one.
    let t = setup_with(false).await;
    let spec = t.wt.path().join(".xherd/spec/spec.md");
    std::fs::write(&spec, "---\nticket: SC-1\n---\n# Spec\n").unwrap();

    // Nothing to approve yet; then not stopped yet.
    assert_eq!(
        act(
            &t,
            UserAction::Approve {
                reason: None,
                expect: Default::default(),
                message: None,
            }
        )
        .await
        .unwrap_err()
        .code,
        "no_gate"
    );
    xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    assert_eq!(
        act(
            &t,
            UserAction::Approve {
                reason: None,
                expect: Default::default(),
                message: None,
            }
        )
        .await
        .unwrap_err()
        .code,
        "not_waiting"
    );
    stop(&t, 1).await;
    assert_eq!(get(&t).await.status, Status::WaitingInput);

    let a = act(
        &t,
        UserAction::Approve {
            reason: Some("lgtm".into()),
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();
    assert_eq!((a.status, a.gate.is_none()), (Status::Running, true));
    assert_eq!(sends(&t), ["send SC-1 \"/xhspec:design\" 1"]);

    let sha = content_sha256(&std::fs::read(&spec).unwrap());
    let ledger =
        std::fs::read_to_string(t.wt.path().join(".xherd/ledger/approvals.jsonl")).unwrap();
    let line: Value = serde_json::from_str(ledger.lines().next().unwrap()).unwrap();
    assert_eq!(
        (line["gate"].as_str(), line["file"].as_str()),
        (Some("spec"), Some("spec.md"))
    );
    assert_eq!(
        (line["sha256"].as_str(), line["reason"].as_str()),
        (Some(sha.as_str()), Some("lgtm"))
    );
    assert!(line["commit"].as_str().is_some_and(|c| c.len() == 40));
    assert!(t.d.store.approved(&t.a.id, "spec.md", &sha).await.unwrap());
    assert!(
        t.d.store
            .reviewed(&t.a.id, "spec.md")
            .await
            .unwrap()
            .unwrap()
            .contains("# Spec")
    );

    // Same content again: already approved.
    let e = xap(&t, "c2", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "already_approved");
}

#[tokio::test]
async fn design_skip_goes_to_tasks() {
    let t = setup().await;
    std::fs::write(
        t.wt.path().join(".xherd/spec/spec.md"),
        "---\ndesign: skip\n---\n",
    )
    .unwrap();
    xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    stop(&t, 1).await;
    act(
        &t,
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(get(&t).await.next_prompt.as_deref(), Some("/xhspec:tasks"));
}

#[tokio::test]
async fn request_changes_keeps_the_gate() {
    let t = setup().await;
    std::fs::write(t.wt.path().join(".xherd/spec/spec.md"), "# v1\n").unwrap();
    xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    stop(&t, 1).await;
    let a = act(
        &t,
        UserAction::RequestChanges {
            comment: "split R3".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(a.status, Status::Running);
    assert!(a.gate.is_some());
    let s = sends(&t);
    assert!(s[0].contains("spec gate (spec.md): split R3"), "{s:?}");

    // Questions are answered with send, not approved.
    let t2 = setup().await;
    xap(&t2, "q1", "ask", json!({"text": "which db?"}))
        .await
        .unwrap();
    stop(&t2, 1).await;
    assert_eq!(
        act(
            &t2,
            UserAction::Approve {
                reason: None,
                expect: Default::default(),
                message: None,
            }
        )
        .await
        .unwrap_err()
        .code,
        "no_gate"
    );
    let a = act(
        &t2,
        UserAction::Send {
            text: "postgres".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(a.gate, None);
    assert_eq!(a.status, Status::Running);
    assert!(sends(&t2)[0].contains("postgres"));
}

#[tokio::test]
async fn edited_or_undeclared_documents_open_a_gate_on_stop() {
    let t = setup().await;
    let spec = t.wt.path().join(".xherd/spec/spec.md");
    std::fs::write(&spec, "# v1\n").unwrap();
    // Declared nothing: the spec is inferred.
    stop(&t, 1).await;
    let a = get(&t).await;
    assert_eq!(a.status, Status::WaitingInput);
    assert!(matches!(&a.gate, Some(Gate::Docs { stage, .. }) if stage == "spec"));
    act(
        &t,
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();

    // Approved and unchanged: idle.
    stop(&t, 2).await;
    assert_eq!(get(&t).await.status, Status::Idle);

    // Edited after approval: re-opened.
    std::fs::write(&spec, "# v2\n").unwrap();
    t.d.action("SC-1", UserAction::Send { text: "go".into() })
        .await
        .unwrap();
    stop(&t, 3).await;
    let a = get(&t).await;
    assert_eq!(a.status, Status::WaitingInput);
    let events = t.d.store.events(&t.a.id, 50).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "gate-inferred"));
    assert!(events.iter().any(|e| e.kind == "gate-reopened"));
}

#[tokio::test]
async fn task_progress_does_not_reopen_the_tasks_gate() {
    let t = setup().await;
    let tasks = t.wt.path().join(".xherd/spec/tasks.md");
    let v1 = "# Tasks\n\n## T01: Store\n- status: todo\n- files: `a.rs`\n- evidence:\n";
    std::fs::write(&tasks, v1).unwrap();
    stop(&t, 1).await;
    assert!(matches!(&get(&t).await.gate, Some(Gate::Docs { stage, .. }) if stage == "tasks"));
    act(
        &t,
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();

    // Progress recorded: still approved, nothing re-opened.
    let done = v1
        .replace("status: todo", "status: done")
        .replace("- evidence:", "- evidence: cargo test passes");
    std::fs::write(&tasks, &done).unwrap();
    let e = xap(&t, "c1", "gate.open", json!({"stage": "tasks"}))
        .await
        .unwrap_err();
    assert_eq!(e.code, "already_approved");
    stop(&t, 2).await;
    let a = get(&t).await;
    assert_eq!((a.status, a.gate.is_none()), (Status::Idle, true));

    // A changed task is content: re-opened.
    std::fs::write(&tasks, done.replace("`a.rs`", "`a.rs`, `b.rs`")).unwrap();
    t.d.action("SC-1", UserAction::Send { text: "go".into() })
        .await
        .unwrap();
    stop(&t, 3).await;
    let a = get(&t).await;
    assert_eq!(a.status, Status::WaitingInput);
    assert!(matches!(&a.gate, Some(Gate::Docs { stage, .. }) if stage == "tasks"));
}

#[tokio::test]
async fn ledger_file_wins_after_restart() {
    let t = setup().await;
    std::fs::write(t.wt.path().join(".xherd/spec/spec.md"), "# v1\n").unwrap();
    xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    stop(&t, 1).await;
    act(
        &t,
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();
    t.d.store
        .write(|c| {
            c.execute("DELETE FROM approvals", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let o = Options::new(t.drv.clone(), Arc::new(SystemClock));
    let d = Daemon::start(t.d.paths.clone(), o).await.unwrap();
    assert!(
        d.store
            .approved(&t.a.id, "spec.md", &content_sha256(b"# v1\n"))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn approving_a_parked_agent_queues_it_with_the_next_command() {
    let t = setup().await;
    std::fs::write(t.wt.path().join(".xherd/spec/spec.md"), "# v1\n").unwrap();
    xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    stop(&t, 1).await;
    let mut a = get(&t).await;
    a.status = Status::Parked;
    t.d.store.put_agent(&a).await.unwrap();
    let o = Options::new(t.drv.clone(), Arc::new(SystemClock));
    let d = Daemon::start(t.d.paths.clone(), o).await.unwrap();
    let a = d
        .action(
            "SC-1",
            UserAction::Approve {
                reason: None,
                expect: Default::default(),
                message: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(a.next_prompt.as_deref(), Some("/xhspec:design"));
    assert_ne!(a.status, Status::Parked);
}

async fn session_start(t: &T, seq: u64) {
    let r = Report {
        id: format!("start-{seq}"),
        agent_id: t.a.id.clone(),
        event: ReportEvent::SessionStart,
        payload: json!({"session_id": format!("s-{seq}")}),
        seq,
        at: 0,
    };
    t.d.report(r).await;
}

#[tokio::test]
async fn a_restart_parks_an_agent_waiting_at_a_gate_and_approval_still_works() {
    let t = setup().await;
    session_start(&t, 1).await;
    std::fs::write(t.wt.path().join(".xherd/spec/spec.md"), "# v1\n").unwrap();
    xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    stop(&t, 2).await;
    assert_eq!(get(&t).await.status, Status::WaitingInput);

    // The container restarted: the terminal server and the session are gone.
    t.drv.alive.lock().unwrap().clear();
    t.d.reconcile_with(true).await;
    for _ in 0..50 {
        if get(&t).await.status == Status::Parked {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let a = get(&t).await;
    assert_eq!((a.status, a.gate.is_some()), (Status::Parked, true));

    // The gate can still be approved; the next session starts with the command.
    let a = act(
        &t,
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(a.next_prompt.as_deref(), Some("/xhspec:design"));
}

#[tokio::test]
async fn approval_starts_a_fresh_session_with_the_next_command() {
    let t = setup().await;
    session_start(&t, 1).await;
    std::fs::write(t.wt.path().join(".xherd/spec/spec.md"), "# v1\n").unwrap();
    xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    stop(&t, 2).await;
    let generation = get(&t).await.generation;

    let a = act(
        &t,
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();
    // The window's process is replaced; nothing is typed into the old session.
    assert!(t.drv.calls().contains(&"respawn SC-1 rt".to_string()));
    assert!(sends(&t).is_empty());
    assert_eq!(a.status, Status::Running);
    assert_eq!(a.next_prompt.as_deref(), Some("/xhspec:design"));
    assert_eq!(a.session, None);
    assert!(a.generation > generation);
    let events = t.d.store.events(&t.a.id, 50).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "fresh-session"));

    // The new session is ready: the command is typed once.
    session_start(&t, 3).await;
    assert_eq!(sends(&t), ["send SC-1 \"/xhspec:design\" 1"]);
    let a = get(&t).await;
    // Kept until a prompt is accepted: a session start that loses the typed
    // text types it again.
    assert_eq!(
        (a.next_prompt.as_deref(), a.session.as_deref()),
        (Some("/xhspec:design"), Some("s-3"))
    );
    session_start(&t, 4).await;
    assert_eq!(sends(&t).len(), 2);
    let submit = Report {
        id: "submit-5".into(),
        agent_id: t.a.id.clone(),
        event: ReportEvent::UserPromptSubmit,
        payload: json!({"prompt": "/xhspec:design"}),
        seq: 5,
        at: 0,
    };
    t.d.report(submit).await;
    assert_eq!(get(&t).await.next_prompt, None);
    session_start(&t, 6).await;
    assert_eq!(sends(&t).len(), 2);
}

#[tokio::test]
async fn resume_starts_with_the_resume_command_unless_a_prompt_is_pending() {
    let t = setup().await;
    act(&t, UserAction::Pause).await.unwrap();
    let a = act(&t, UserAction::Resume).await.unwrap();
    assert_eq!(a.next_prompt.as_deref(), Some("/xhspec:resume"));

    let mut a = get(&t).await;
    a.status = Status::Paused;
    a.next_prompt = Some("/xhspec:pr-review".into());
    t.d.store.put_agent(&a).await.unwrap();
    let o = Options::new(t.drv.clone(), Arc::new(SystemClock));
    let d = Daemon::start(t.d.paths.clone(), o).await.unwrap();
    let a = d.action("SC-1", UserAction::Resume).await.unwrap();
    assert_eq!(a.next_prompt.as_deref(), Some("/xhspec:pr-review"));
}

#[tokio::test]
async fn diff_shows_the_proposed_commit_or_every_change() {
    let t = setup().await;
    let wt = t.wt.path();
    std::fs::write(wt.join(".git/info/exclude"), ".xherd/\n").unwrap();
    std::fs::write(wt.join("gone.txt"), "old\n").unwrap();
    git(wt, &["add", "gone.txt"]);
    git(wt, &["commit", "-q", "-m", "gone"]);
    std::fs::remove_file(wt.join("gone.txt")).unwrap();
    std::fs::write(wt.join("a.txt"), "new a\n").unwrap();
    std::fs::write(wt.join("other.txt"), "left out\n").unwrap();

    let v = t.d.diff("SC-1").await.unwrap();
    assert!(v["commit"].is_null());
    assert_eq!(v["files"], json!(["a.txt", "gone.txt", "other.txt"]));
    assert!(v["diff"].as_str().unwrap().contains("+left out"));

    xap(
        &t,
        "c1",
        "commit.propose",
        json!({"id": "C01", "subject": "a", "files": ["a.txt", "gone.txt"]}),
    )
    .await
    .unwrap();
    let v = t.d.diff("SC-1").await.unwrap();
    assert_eq!(v["commit"]["id"], "C01");
    assert_eq!(v["files"], json!(["a.txt", "gone.txt"]));
    assert_eq!(v["others"], json!(["other.txt"]));
    assert_eq!(v["head"], git(wt, &["rev-parse", "HEAD"]));
    let diff = v["diff"].as_str().unwrap();
    assert!(diff.contains("+new a") && diff.contains("-old"), "{diff}");
    assert!(!diff.contains("left out"), "{diff}");
    // Viewing changes nothing the agent will commit from.
    assert_eq!(git(wt, &["diff", "--cached", "--name-only"]), "");

    assert_eq!(t.d.diff("nobody").await.unwrap_err().code, "unknown_agent");
}

#[tokio::test]
async fn ship_gate_shows_the_branch_then_the_agent_watches_its_pr() {
    let t = setup().await;
    let wt = t.wt.path();
    std::fs::write(wt.join(".git/info/exclude"), ".xherd/\n").unwrap();
    git(wt, &["checkout", "-q", "-b", "SC-1"]);
    std::fs::write(wt.join("feature.rs"), "feature\n").unwrap();
    git(wt, &["add", "feature.rs"]);
    git(wt, &["commit", "-q", "-m", "feature"]);
    std::fs::write(wt.join("stray.txt"), "stray\n").unwrap();
    std::fs::write(wt.join(".xherd/spec/pr.md"), "# Add the feature\n").unwrap();
    xap(&t, "c1", "gate.open", json!({"stage": "ship"}))
        .await
        .unwrap();
    stop(&t, 1).await;
    assert_eq!(get(&t).await.status, Status::WaitingInput);

    // At the ship gate the user reviews what will be pushed.
    let v = t.d.diff("SC-1").await.unwrap();
    assert_eq!(v["branch"], "main...HEAD");
    assert_eq!(v["files"], json!(["feature.rs"]));
    assert_eq!(v["others"], json!(["stray.txt"]));
    assert!(v["diff"].as_str().unwrap().contains("+feature"));

    // Approval hands on the ship command, which pushes and opens the PR.
    act(
        &t,
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();
    session_start(&t, 2).await;
    assert_eq!(sends(&t), ["send SC-1 \"/xhspec:ship\" 1"]);
    let url = "https://github.com/acme/app/pull/42";
    xap(&t, "c2", "pr.opened", json!({"url": url}))
        .await
        .unwrap();
    // Stopped with a question: still live, the user answers first.
    let ask = Report {
        id: "ask".into(),
        agent_id: t.a.id.clone(),
        event: ReportEvent::Stop,
        payload: json!({"message": "Update the ticket too?"}),
        seq: 3,
        at: 0,
    };
    t.d.report(ask).await;
    assert_eq!(get(&t).await.status, Status::WaitingInput);
    act(&t, UserAction::Send { text: "no".into() })
        .await
        .unwrap();
    let typed = Report {
        id: "typed".into(),
        agent_id: t.a.id.clone(),
        event: ReportEvent::UserPromptSubmit,
        payload: json!({"prompt": "no"}),
        seq: 4,
        at: 0,
    };
    t.d.report(typed).await;

    // Nothing left to do: the session ends and the agent watches its PR (R-301).
    let done = Report {
        id: "done".into(),
        agent_id: t.a.id.clone(),
        event: ReportEvent::Stop,
        payload: json!({"message": "Draft PR opened."}),
        seq: 5,
        at: 0,
    };
    t.d.report(done).await;
    let a = get(&t).await;
    assert_eq!(
        (a.status, a.pr_url.as_deref()),
        (Status::Watching, Some(url))
    );
    assert!(t.drv.calls().contains(&"stop SC-1".to_string()));
    let events = t.d.store.events(&t.a.id, 50).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "watching"));
}

/// A fake `gh` that records its argv and working directory and answers with
/// `answer` (exit 1 when it is `fail`).
fn fake_gh(dir: &Path, answer: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("gh");
    let log = dir.join("gh.log");
    std::fs::write(
        &bin,
        format!(
            "#!/bin/sh\necho \"$(pwd) $*\" >> '{}'\n[ '{answer}' = fail ] && {{ echo 'not logged in' >&2; exit 1; }}\necho '{answer}'\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

async fn until(t: &T, what: impl Fn(&Agent) -> bool) -> Agent {
    for _ in 0..400 {
        let a = get(t).await;
        if what(&a) {
            return a;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    get(t).await
}

#[tokio::test]
async fn without_a_declared_pr_the_host_plugin_finds_it_after_ship() {
    let bin_dir = tempfile::tempdir().unwrap();
    let url = "https://github.com/acme/app/pull/7";
    let gh = fake_gh(bin_dir.path(), &format!(r#"[{{"url":"{url}"}}]"#));
    let t = setup_config(json!({"pluginSettings": {"github": {"gh": gh}}})).await;
    let log = bin_dir.path().join("gh.log");

    // Outside the ship stage nothing is looked up.
    xap(&t, "c1", "stage.enter", json!({"stage": "implement"}))
        .await
        .unwrap();
    stop(&t, 1).await;
    assert_eq!(get(&t).await.status, Status::Idle);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!log.exists());

    // The agent shipped but did not report the PR: the stop finds it (R-302).
    xap(&t, "c2", "stage.enter", json!({"stage": "ship"}))
        .await
        .unwrap();
    stop(&t, 2).await;
    let a = until(&t, |a| a.status == Status::Watching).await;
    assert_eq!(
        (a.status, a.pr_url.as_deref()),
        (Status::Watching, Some(url))
    );
    let called = std::fs::read_to_string(&log).unwrap();
    let wd = t.wt.path().canonicalize().unwrap();
    assert_eq!(
        called.trim(),
        format!(
            "{} pr list --head SC-1 --state all --json url --limit 1",
            wd.display()
        )
    );
    let events = t.d.store.events(&t.a.id, 50).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "pr-found"));
}

#[tokio::test]
async fn a_failed_or_empty_pr_lookup_leaves_the_agent_idle() {
    let bin_dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(bin_dir.path(), "fail");
    let t = setup_config(json!({"pluginSettings": {"github": {"gh": gh}}})).await;
    xap(&t, "c1", "stage.enter", json!({"stage": "ship"}))
        .await
        .unwrap();
    stop(&t, 1).await;
    let mut failed = false;
    for _ in 0..400 {
        let events = t.d.store.events(&t.a.id, 50).await.unwrap();
        if events.iter().any(|e| e.kind == "pr-lookup-failed") {
            failed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(failed);
    let a = get(&t).await;
    assert_eq!((a.status, a.pr_url), (Status::Idle, None));

    let bin_dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(bin_dir.path(), "[]");
    let t = setup_config(json!({"pluginSettings": {"github": {"gh": gh}}})).await;
    xap(&t, "c1", "stage.enter", json!({"stage": "ship"}))
        .await
        .unwrap();
    stop(&t, 1).await;
    for _ in 0..400 {
        if bin_dir.path().join("gh.log").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    let a = get(&t).await;
    assert_eq!((a.status, a.pr_url), (Status::Idle, None));
}

#[tokio::test]
async fn ui_reads_a_document_and_approves_what_it_read() {
    let t = setup_with(false).await;
    let spec = t.wt.path().join(".xherd/spec/spec.md");
    std::fs::write(&spec, "---\nticket: SC-1\n---\n# Spec\n").unwrap();
    std::fs::write(t.wt.path().join(".xherd/spec/notes.md"), "n").unwrap();
    xap(&t, "c1", "gate.open", json!({"stage": "spec"}))
        .await
        .unwrap();
    stop(&t, 1).await;

    assert_eq!(t.d.docs("SC-1").await.unwrap(), ["spec.md", "notes.md"]);
    let doc = t.d.doc("SC-1", "spec.md").await.unwrap();
    assert_eq!(doc["path"], "spec.md");
    assert_eq!(doc["reviewed"], Value::Null);
    let read = doc["sha256"].as_str().unwrap().to_string();
    assert_eq!(
        t.d.doc("SC-1", "../../etc/passwd").await.unwrap_err().code,
        "no_such_doc"
    );

    // Edited after the user read it: refused, the gate stays.
    std::fs::write(&spec, "---\nticket: SC-1\n---\n# Spec v2\n").unwrap();
    let expect = [("spec.md".to_string(), read)].into();
    let e = act(
        &t,
        UserAction::Approve {
            reason: None,
            expect,
            message: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, "changed_since_read");
    assert!(get(&t).await.gate.is_some());

    let doc = t.d.doc("SC-1", "spec.md").await.unwrap();
    let expect = [(
        "spec.md".to_string(),
        doc["sha256"].as_str().unwrap().into(),
    )]
    .into();
    act(
        &t,
        UserAction::Approve {
            reason: None,
            expect,
            message: None,
        },
    )
    .await
    .unwrap();
    let doc = t.d.doc("SC-1", "spec.md").await.unwrap();
    assert_eq!(doc["reviewed"], doc["text"]);
}

#[tokio::test]
async fn visited_stages_show_skipped_ones_and_are_rebuilt_from_events() {
    let t = setup().await;
    xap(&t, "c1", "stage.enter", json!({"stage": "spec"}))
        .await
        .unwrap();
    // Design skipped: straight to tasks.
    xap(&t, "c2", "stage.enter", json!({"stage": "tasks"}))
        .await
        .unwrap();
    assert_eq!(get(&t).await.visited_stages, ["spec", "tasks"]);

    // An agent saved before the field existed: rebuilt at the next start.
    let mut a = get(&t).await;
    a.visited_stages.clear();
    t.d.store.put_agent(&a).await.unwrap();
    let o = Options::new(t.drv.clone(), Arc::new(SystemClock));
    let d = Daemon::start(t.d.paths.clone(), o).await.unwrap();
    let mut seen = Vec::new();
    for _ in 0..100 {
        seen = d
            .store
            .get_agent(&t.a.id)
            .await
            .unwrap()
            .unwrap()
            .visited_stages;
        if !seen.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(seen, ["spec", "tasks"]);
}

/// Open a commit gate for `a.txt` (message proposed by the agent), with an
/// unrelated untracked `b.txt` lying around, and stop at it.
async fn commit_gate(t: &T) {
    git(t.wt.path(), &["config", "user.email", "t@t"]);
    git(t.wt.path(), &["config", "user.name", "t"]);
    std::fs::write(
        t.wt.path().join(".xherd/spec/tasks.md"),
        "## T01: add a\n- files: a.txt\n",
    )
    .unwrap();
    xap(t, "g1", "tasks.sync", json!({})).await.unwrap();
    xap(t, "g2", "task.start", json!({"id": "T01"}))
        .await
        .unwrap();
    std::fs::write(t.wt.path().join("a.txt"), "a\n").unwrap();
    std::fs::write(t.wt.path().join("b.txt"), "b\n").unwrap();
    xap(
        t,
        "g3",
        "commit.propose",
        json!({"id": "C01", "subject": "Add a", "message": "Add a\n\nBecause T01 needs it.",
               "files": ["a.txt"], "tasks": ["T01"]}),
    )
    .await
    .unwrap();
    stop(t, 1).await;
}

fn approve(message: Option<&str>) -> UserAction {
    UserAction::Approve {
        reason: None,
        expect: Default::default(),
        message: message.map(str::to_string),
    }
}

#[tokio::test]
async fn approving_a_commit_makes_the_commit_with_the_edited_message() {
    let t = setup().await;
    commit_gate(&t).await;
    assert_eq!(
        get(&t).await.commits[0].message.as_deref(),
        Some("Add a\n\nBecause T01 needs it.")
    );

    t.d.action("SC-1", approve(Some("Add a, as edited by the user")))
        .await
        .unwrap();
    let a = get(&t).await;
    let sha = git(t.wt.path(), &["rev-parse", "HEAD"]);
    assert_eq!(a.commits[0].status, CommitStatus::Done);
    assert_eq!(a.commits[0].sha.as_deref(), Some(sha.as_str()));
    assert_eq!(
        git(t.wt.path(), &["log", "-1", "--format=%B"]),
        "Add a, as edited by the user"
    );
    // Exactly the proposed file; the other one stays untouched.
    assert_eq!(
        git(t.wt.path(), &["show", "--name-only", "--format=", "HEAD"]),
        "a.txt"
    );
    assert!(git(t.wt.path(), &["status", "--porcelain"]).contains("?? b.txt"));
    // The agent only records it, in a fresh session.
    assert_eq!(a.next_prompt.as_deref(), Some("/xhspec:commit C01"));
    assert!(!t.wt.path().join(".xherd/commit-error.txt").exists());
    // Reporting the same sha later is harmless.
    xap(
        &t,
        "g4",
        "commit.done",
        json!({"id": "C01", "sha": &sha[..8]}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn a_refused_commit_goes_to_the_agent_with_gits_output() {
    let t = setup().await;
    commit_gate(&t).await;
    let hook = t.wt.path().join(".git/hooks/pre-commit");
    // A worktree keeps hooks in the common dir.
    let hook = if hook.parent().unwrap().exists() {
        hook
    } else {
        let common = git(t.wt.path(), &["rev-parse", "--git-common-dir"]);
        let dir = std::path::Path::new(&common).join("hooks");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("pre-commit")
    };
    std::fs::write(&hook, "#!/bin/sh\necho 'lint failed: a.txt' >&2\nexit 1\n").unwrap();
    std::process::Command::new("chmod")
        .args(["+x"])
        .arg(&hook)
        .status()
        .unwrap();

    t.d.action("SC-1", approve(None)).await.unwrap();
    let a = get(&t).await;
    // The approval stands, nothing was committed, the agent gets the error.
    assert_eq!(a.commits[0].status, CommitStatus::Approved);
    assert_eq!(a.commits[0].sha, None);
    let err = std::fs::read_to_string(t.wt.path().join(".xherd/commit-error.txt")).unwrap();
    assert!(err.contains("lint failed"), "{err}");
    assert_eq!(a.next_prompt.as_deref(), Some("/xhspec:commit C01"));
    let events = t.d.store.events(&t.a.id, 50).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "commit-failed"));
}
