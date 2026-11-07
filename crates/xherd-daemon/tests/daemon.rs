//! Supervisor and actor tests with a fake driver and a manual clock.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::paths::Paths;
use xherd_core::report::{Report, ReportEvent};
use xherd_core::{Agent, Status, queue};
use xherd_daemon::*;

struct T {
    _dir: tempfile::TempDir,
    d: Daemon,
    drv: Arc<FakeDriver>,
    clock: Arc<ManualClock>,
}

async fn daemon_with(dir: tempfile::TempDir, max: usize, tweak: impl FnOnce(&mut Options)) -> T {
    let paths = Paths::at(dir.path());
    let cfg = xherd_core::config::Config {
        max_concurrent_agents: max,
        ..Default::default()
    };
    paths.ensure().unwrap();
    cfg.save(&paths.config()).unwrap();
    let drv = Arc::new(FakeDriver::default());
    let clock = Arc::new(ManualClock::new(1_000_000));
    let mut o = Options::new(drv.clone(), clock.clone());
    tweak(&mut o);
    let d = Daemon::start(paths, o).await.unwrap();
    T {
        _dir: dir,
        d,
        drv,
        clock,
    }
}

async fn daemon(max: usize) -> T {
    daemon_with(tempfile::tempdir().unwrap(), max, |_| {}).await
}

fn agent(name: &str, created: i64) -> Agent {
    let mut a = Agent::new(name, name, created);
    a.workdir = Some("/tmp".into());
    a.launch = Some(vec!["fake-runtime".into(), "--go".into()]);
    a
}

fn rep(a: &Agent, id: &str, event: ReportEvent, seq: u64, payload: Value) -> Report {
    Report {
        id: id.into(),
        agent_id: a.id.clone(),
        event,
        payload,
        seq,
        at: 0,
    }
}

async fn status(d: &Daemon, who: &str) -> Status {
    d.action(who, UserAction::Get).await.unwrap().status
}

async fn wait_status(d: &Daemon, who: &str, want: Status) {
    for _ in 0..200 {
        if status(d, who).await == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("{who} never reached {want}; is {}", status(d, who).await);
}

#[tokio::test]
async fn slots_queue_and_start_order() {
    let t = daemon(1).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    let b = t.d.create_agent(agent("SC-2", 2)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    assert_eq!(status(&t.d, "SC-2").await, Status::Queued);
    assert!(
        t.d.create_agent(agent("SC-1", 3)).await.is_err(),
        "names are unique"
    );

    let started = t.drv.calls();
    assert_eq!(started, vec!["start SC-1 fake-runtime --go"]);
    // The agent token was minted and only its hash is stored.
    let events = t.d.store.events(&a.id, 10).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "session-started"));

    t.d.action("SC-1", UserAction::Pause).await.unwrap();
    wait_status(&t.d, "SC-2", Status::Running).await;
    assert_eq!(status(&t.d, &a.id).await, Status::Paused);
    assert!(t.drv.calls().contains(&"stop SC-1".to_string()));

    // Resume puts SC-1 back in the queue; it waits for SC-2's slot.
    t.d.action("SC-1", UserAction::Resume).await.unwrap();
    assert_eq!(status(&t.d, "SC-1").await, Status::Queued);
    t.d.action(&b.id, UserAction::Cancel).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
}

#[tokio::test]
async fn failed_start_is_an_error_and_frees_the_slot() {
    let t = daemon(1).await;
    t.drv.fail_start.store(true, Ordering::SeqCst);
    t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Error).await;
    t.drv.fail_start.store(false, Ordering::SeqCst);
    let mut no_launch = agent("SC-2", 2);
    no_launch.launch = None;
    t.d.create_agent(no_launch).await.unwrap();
    wait_status(&t.d, "SC-2", Status::Error).await;
    t.d.create_agent(agent("SC-3", 3)).await.unwrap();
    wait_status(&t.d, "SC-3", Status::Running).await;
}

#[tokio::test]
async fn first_prompt_is_typed_once_the_session_is_ready() {
    let t = daemon(4).await;
    let mut a = agent("SC-1", 1);
    a.pack = Some("xhspec".into());
    a.flow = Some("feature".into());
    a.next_prompt = Some("/xhspec:feature SC-1 add login".into());
    let a = t.d.create_agent(a).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    assert_eq!(t.drv.calls(), vec!["start SC-1 fake-runtime --go"]);

    let start = rep(&a, "s1", ReportEvent::SessionStart, 1, Value::Null);
    assert_eq!(t.d.report(start).await, Some(ReportResult::Applied));
    let typed = r#"send SC-1 "/xhspec:feature SC-1 add login" 1"#.to_string();
    assert_eq!(t.drv.calls().last(), Some(&typed));
    // Not acknowledged yet: Claude Code may start a second session and lose
    // the typed text, so the command is typed again at the next start.
    let now = t.d.action("SC-1", UserAction::Get).await.unwrap();
    assert!(now.next_prompt.is_some());
    let again = rep(&a, "s2", ReportEvent::SessionStart, 2, Value::Null);
    t.d.report(again).await;
    assert_eq!(t.drv.calls().iter().filter(|c| **c == typed).count(), 2);

    // Once a prompt is submitted the command is done; a later session start
    // (e.g. after /clear) types nothing.
    let submit = rep(&a, "p1", ReportEvent::UserPromptSubmit, 3, Value::Null);
    t.d.report(submit).await;
    let now = t.d.action("SC-1", UserAction::Get).await.unwrap();
    assert_eq!(now.next_prompt, None);
    let clear = rep(&a, "s3", ReportEvent::SessionStart, 4, Value::Null);
    t.d.report(clear).await;
    assert_eq!(t.drv.calls().iter().filter(|c| **c == typed).count(), 2);
}

#[tokio::test]
async fn reports_sequence_dedupe_and_unknown_agents() {
    let t = daemon(4).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;

    let stop = rep(&a, "r2", ReportEvent::Stop, 2, json!({"message": "done"}));
    assert_eq!(t.d.report(stop.clone()).await, Some(ReportResult::Applied));
    assert_eq!(status(&t.d, "SC-1").await, Status::Idle);
    assert_eq!(t.d.report(stop).await, Some(ReportResult::Duplicate));
    let old = rep(&a, "r1", ReportEvent::Working, 1, Value::Null);
    assert_eq!(t.d.report(old).await, Some(ReportResult::Stale));
    assert_eq!(status(&t.d, "SC-1").await, Status::Idle);

    let ghost = Agent::new("NOPE-1", "NOPE-1", 0);
    assert_eq!(
        t.d.report(rep(&ghost, "x", ReportEvent::Stop, 1, Value::Null))
            .await,
        None
    );

    // Permission prompts come mid-turn: working first, then the notification.
    let working = rep(&a, "r3", ReportEvent::Working, 3, Value::Null);
    assert_eq!(t.d.report(working).await, Some(ReportResult::Applied));
    let mut notes = t.d.hub.subscribe();
    let blocked = rep(
        &a,
        "r4",
        ReportEvent::Notification,
        4,
        json!({"message": "Allow Bash?"}),
    );
    assert_eq!(t.d.report(blocked).await, Some(ReportResult::Applied));
    let mut saw_notify = false;
    while let Ok(c) = notes.try_recv() {
        saw_notify |= matches!(c, Change::Notify { ref text, .. } if text.contains("Allow Bash?"));
    }
    assert!(saw_notify);

    // R-021: never type into a blocked agent.
    let err =
        t.d.action("SC-1", UserAction::Send { text: "hi".into() })
            .await
            .unwrap_err();
    assert_eq!(err.code, "agent_blocked");

    // Persisted, and reloaded the same after a restart of the actor's view.
    let stored = t.d.store.get_agent(&a.id).await.unwrap().unwrap();
    assert_eq!(stored.last_seq, 4);
    assert_eq!(stored.status, Status::Blocked);
}

#[tokio::test]
async fn reports_after_pause_do_not_revive() {
    let t = daemon(4).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    t.d.action("SC-1", UserAction::Pause).await.unwrap();
    let r = rep(&a, "late", ReportEvent::SessionStart, 9, Value::Null);
    assert_eq!(t.d.report(r).await, Some(ReportResult::Dormant));
    assert_eq!(status(&t.d, "SC-1").await, Status::Paused);
}

#[tokio::test]
async fn reconcile_marks_lost_sessions() {
    let t = daemon(4).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    t.d.reconcile().await;
    assert_eq!(status(&t.d, "SC-1").await, Status::Running);
    t.drv.alive.lock().unwrap().clear();
    t.d.reconcile().await;
    wait_status(&t.d, "SC-1", Status::Error).await;
    let ev = t.d.store.events(&a.id, 20).await.unwrap();
    assert!(ev.iter().any(|e| e.kind == "session-lost"));
}

#[tokio::test]
async fn a_restart_resumes_a_running_agent_instead_of_failing_it() {
    let t = daemon(4).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    // The container restarted: the session is gone.
    t.drv.alive.lock().unwrap().clear();
    t.d.reconcile_with(true).await;
    // Queued again and started in a new session, not left in error.
    for _ in 0..50 {
        if t.drv
            .calls()
            .iter()
            .filter(|c| c.starts_with("start "))
            .count()
            == 2
        {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        t.drv
            .calls()
            .iter()
            .filter(|c| c.starts_with("start "))
            .count(),
        2
    );
    assert_ne!(status(&t.d, "SC-1").await, Status::Error);
    let ev = t.d.store.events(&a.id, 20).await.unwrap();
    assert!(ev.iter().any(|e| e.kind == "session-lost"));
}

#[tokio::test]
async fn idle_alert_once_and_parking() {
    let t = daemon(4).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    let b = t.d.create_agent(agent("SC-2", 2)).await.unwrap();
    wait_status(&t.d, "SC-2", Status::Running).await;
    t.d.report(rep(
        &a,
        "a1",
        ReportEvent::Stop,
        1,
        json!({"message": "ok"}),
    ))
    .await;
    t.d.report(rep(
        &b,
        "b1",
        ReportEvent::Stop,
        1,
        json!({"message": "Which port?"}),
    ))
    .await;
    assert_eq!(status(&t.d, "SC-2").await, Status::WaitingInput);

    t.clock.advance(14 * 60_000);
    t.d.sup.tick(TickKind::IdleCheck);
    t.d.sup.tick(TickKind::ParkCheck);
    assert!(
        !t.d.action("SC-1", UserAction::Get)
            .await
            .unwrap()
            .idle_alerted
    );

    t.clock.advance(20 * 60_000);
    t.d.sup.tick(TickKind::IdleCheck);
    t.d.sup.tick(TickKind::IdleCheck);
    t.d.sup.tick(TickKind::ParkCheck);
    assert!(
        t.d.action("SC-1", UserAction::Get)
            .await
            .unwrap()
            .idle_alerted
    );
    let alerts =
        t.d.store
            .events(&a.id, 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "idle-alert")
            .count();
    assert_eq!(alerts, 1);
    wait_status(&t.d, "SC-2", Status::Parked).await;
    assert!(t.drv.calls().contains(&"stop SC-2".to_string()));
}

#[tokio::test]
async fn delivery_is_retried_once_when_not_acknowledged() {
    let dir = tempfile::tempdir().unwrap();
    let t = daemon_with(dir, 4, |o| o.delivery_ack = Duration::from_millis(50)).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    t.d.report(rep(&a, "a1", ReportEvent::Stop, 1, Value::Null))
        .await;

    t.d.action(
        "SC-1",
        UserAction::Send {
            text: "go on".into(),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let sends: Vec<String> = t
        .drv
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("send"))
        .collect();
    assert_eq!(sends, vec!["send SC-1 \"go on\" 1", "send SC-1 \"\" 1"]);

    // An acknowledged delivery is not retried.
    t.d.action(
        "SC-1",
        UserAction::Send {
            text: "again".into(),
        },
    )
    .await
    .unwrap();
    t.d.report(rep(
        &a,
        "a2",
        ReportEvent::UserPromptSubmit,
        2,
        json!({"prompt": "again"}),
    ))
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let sends = t
        .drv
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("send"))
        .count();
    assert_eq!(sends, 3);
}

#[tokio::test]
async fn screen_rules_block_and_unblock() {
    let dir = tempfile::tempdir().unwrap();
    let rules = xherd_term::ScreenRules::new(&["Do you want to proceed\\?"], 5).unwrap();
    let t = daemon_with(dir, 4, |o| o.screen_rules = Some(rules)).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;

    *t.drv.screen.lock().unwrap() = "Bash(rm -rf build)\nDo you want to proceed?\n1. Yes".into();
    t.d.sup.tick(TickKind::ScreenScan);
    wait_status(&t.d, "SC-1", Status::Blocked).await;

    *t.drv.screen.lock().unwrap() = "working…".into();
    t.d.sup.tick(TickKind::ScreenScan);
    wait_status(&t.d, "SC-1", Status::Running).await;

    // A real report still applies after daemon reports (separate sequences).
    let r = rep(&a, "r1", ReportEvent::Stop, 1, Value::Null);
    assert_eq!(t.d.report(r).await, Some(ReportResult::Applied));
}

#[tokio::test]
async fn queued_reports_are_replayed_on_start() {
    let t = daemon(4).await;
    let a = t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    // The daemon goes away; hooks queue their reports meanwhile.
    let q = t.d.paths.hook_queue();
    queue::append(
        &q,
        &rep(
            &a,
            "q1",
            ReportEvent::Stop,
            5,
            json!({"message": "all done"}),
        )
        .into(),
    )
    .unwrap();
    queue::append(
        &q,
        &rep(&a, "q0", ReportEvent::Working, 4, Value::Null).into(),
    )
    .unwrap();

    // Restart on the same data dir; the fake session is still alive.
    let o = Options::new(t.drv.clone(), Arc::new(ManualClock::new(2_000_000)));
    let d = Daemon::start(t.d.paths.clone(), o).await.unwrap();
    assert_eq!(
        status(&d, &a.id).await,
        Status::Idle,
        "stop applied, older working ignored"
    );
    assert!(!d.paths.hook_queue().exists());
    assert_eq!(d.replay_queue().await.unwrap(), 0);
}
