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
    let mut a = Agent::new(name, created);
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
async fn pause_all_stops_everything_and_starts_nothing() {
    let t = daemon(1).await;
    t.d.create_agent(agent("SC-1", 1)).await.unwrap();
    t.d.create_agent(agent("SC-2", 2)).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    assert_eq!(status(&t.d, "SC-2").await, Status::Queued);

    let mut paused = t.d.pause_all().await.unwrap();
    paused.sort();
    assert_eq!(paused, ["SC-1", "SC-2"]);
    assert_eq!(status(&t.d, "SC-1").await, Status::Paused);
    assert_eq!(status(&t.d, "SC-2").await, Status::Paused);
    // The slot SC-1 freed did not start SC-2, and nothing was asked of the agent.
    assert_eq!(
        t.drv.calls(),
        vec!["start SC-1 fake-runtime --go", "stop SC-1"]
    );
    assert!(t.d.pause_all().await.unwrap().is_empty());
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
    a.pack = Some("xherd".into());
    a.flow = Some("feature".into());
    a.next_prompt = Some("/xherd:feature SC-1 add login".into());
    let a = t.d.create_agent(a).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
    assert_eq!(t.drv.calls(), vec!["start SC-1 fake-runtime --go"]);

    let start = rep(&a, "s1", ReportEvent::SessionStart, 1, Value::Null);
    assert_eq!(t.d.report(start).await, Some(ReportResult::Applied));
    let typed = r#"send SC-1 "/xherd:feature SC-1 add login" 1"#.to_string();
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

    let ghost = Agent::new("NOPE-1", 0);
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
    let starts = |t: &T| {
        t.drv
            .calls()
            .iter()
            .filter(|c| c.starts_with("start "))
            .count()
    };
    // A vanished session is resumed by itself twice per hour...
    for n in 2..=3 {
        t.drv.alive.lock().unwrap().clear();
        t.d.reconcile().await;
        for _ in 0..50 {
            if starts(&t) == n {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(starts(&t), n);
        assert_ne!(status(&t.d, "SC-1").await, Status::Error);
    }
    // ...then it stays in error until the user presses Resume.
    t.drv.alive.lock().unwrap().clear();
    t.d.reconcile().await;
    wait_status(&t.d, "SC-1", Status::Error).await;
    let ev = t.d.store.events(&a.id, 50).await.unwrap();
    assert!(ev.iter().any(|e| e.kind == "session-lost"));
    assert_eq!(ev.iter().filter(|e| e.kind == "auto-resume").count(), 2);
    t.d.action("SC-1", UserAction::Resume).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Running).await;
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
    let t = daemon_with(dir, 4, |o| o.delivery_ack = Duration::from_millis(250)).await;
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
    tokio::time::sleep(Duration::from_millis(800)).await;
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
    tokio::time::sleep(Duration::from_millis(800)).await;
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
async fn git_auth_failure_on_screen_blocks_with_reason() {
    let t = daemon(4).await;
    let a = t.d.create_agent(agent("GA-1", 1)).await.unwrap();
    wait_status(&t.d, "GA-1", Status::Running).await;

    *t.drv.screen.lock().unwrap() =
        "$ git push\nfatal: Authentication failed for 'https://github.com/o/r.git/'".into();
    t.d.sup.tick(TickKind::ScreenScan);
    wait_status(&t.d, "GA-1", Status::Blocked).await;
    let shown = t.d.action("GA-1", UserAction::Get).await.unwrap();
    assert_eq!(shown.block_reason.as_deref(), Some("git access"));

    // The screen shows the same failure: no flapping, and the block is not
    // a screen-rule block, so a clean screen does not clear it by itself.
    t.d.sup.tick(TickKind::ScreenScan);
    *t.drv.screen.lock().unwrap() = "working…".into();
    t.d.sup.tick(TickKind::ScreenScan);
    tokio::time::sleep(Duration::from_millis(200)).await;
    wait_status(&t.d, "GA-1", Status::Blocked).await;

    // The agent carrying on clears it.
    let r = rep(&a, "w1", ReportEvent::Working, 1, Value::Null);
    assert_eq!(t.d.report(r).await, Some(ReportResult::Applied));
    wait_status(&t.d, "GA-1", Status::Running).await;
}

#[tokio::test]
async fn a_stopped_agents_screen_is_not_scanned_for_git_failures() {
    let t = daemon(4).await;
    let a = t.d.create_agent(agent("GA-2", 1)).await.unwrap();
    wait_status(&t.d, "GA-2", Status::Running).await;
    let r = rep(&a, "s1", ReportEvent::Stop, 1, Value::Null);
    assert_eq!(t.d.report(r).await, Some(ReportResult::Applied));
    wait_status(&t.d, "GA-2", Status::Idle).await;

    // The agent's own summary, after it stopped, quoting an auth failure.
    *t.drv.screen.lock().unwrap() =
        "● Point 4: git said \"fatal: Authentication failed\" before the fix.".into();
    t.d.sup.tick(TickKind::ScreenScan);
    tokio::time::sleep(Duration::from_millis(300)).await;
    wait_status(&t.d, "GA-2", Status::Idle).await;
}

#[tokio::test]
async fn worklog_books_running_time_and_user_time() {
    let t = daemon(4).await;
    t.d.create_agent(agent("WL-1", 1)).await.unwrap();
    wait_status(&t.d, "WL-1", Status::Running).await;

    // The first flush only starts the clock; the next books the span.
    t.d.flush_worklog().await;
    t.d.worklog_flushed.store(
        t.d.clock.now() - 60_000,
        std::sync::atomic::Ordering::Relaxed,
    );
    t.d.flush_worklog().await;
    t.d.add_user_time("WL-1", 30_000).await.unwrap();
    t.d.add_user_time("WL-1", 10_000_000).await.unwrap(); // clamped

    let v = t.d.worklog(1).await.unwrap();
    let row = &v["days"][0]["tickets"][0];
    assert!(row["agentMs"].as_u64().unwrap() >= 60_000);
    assert_eq!(row["userMs"], 30_000 + 120_000);
    assert_eq!(v["tickets"][0]["agent"], "WL-1");
    assert_eq!(v["tickets"][0]["cycleMs"], Value::Null);
}

#[tokio::test]
async fn todos_come_due_notify_once_and_repeat() {
    use xherd_core::todo::TodoStatus;
    use xherd_daemon::todos::TodoInput;
    let t = daemon(4).await;
    let mut rx = t.d.hub.subscribe();
    let now = t.d.clock.now();
    let input = |title: &str, due, repeat| TodoInput {
        title: title.into(),
        note: None,
        due,
        repeat_minutes: repeat,
        condition: None,
        model: None,
    };
    let later =
        t.d.create_todo(input("later", now + 3_600_000, None))
            .await
            .unwrap();
    let once =
        t.d.create_todo(input("renew cert", now - 1, None))
            .await
            .unwrap();
    let again =
        t.d.create_todo(input("water plants", now - 1, Some(60)))
            .await
            .unwrap();
    assert!(t.d.create_todo(input(" ", now, None)).await.is_err());

    t.d.todo_tick().await;
    t.d.todo_tick().await; // a second tick must not notify again
    let mut texts = Vec::new();
    while let Ok(c) = rx.try_recv() {
        if let xherd_daemon::support::Change::Notify { kind, text, .. } = c {
            assert_eq!(kind, "todo");
            texts.push(text);
        }
    }
    texts.sort();
    assert_eq!(texts, ["TODO: renew cert", "TODO: water plants"]);

    let status = |list: &[xherd_core::todo::Todo], id: &str| {
        list.iter().find(|x| x.id == id).unwrap().status
    };
    let list = t.d.todos().await.unwrap();
    assert_eq!(status(&list, &later.id), TodoStatus::Open);
    assert_eq!(status(&list, &once.id), TodoStatus::Due);

    t.d.complete_todo(&once.id).await.unwrap();
    t.d.complete_todo(&again.id).await.unwrap();
    let list = t.d.todos().await.unwrap();
    assert_eq!(status(&list, &once.id), TodoStatus::Done);
    assert_eq!(status(&list, &again.id), TodoStatus::Open, "repeats");

    t.d.reschedule_todo(&once.id, now + 1_000).await.unwrap();
    assert_eq!(
        status(&t.d.todos().await.unwrap(), &once.id),
        TodoStatus::Open
    );
    t.d.delete_todo(&later.id).await.unwrap();
    assert!(t.d.delete_todo(&later.id).await.is_err());
}

/// A fake runtime check: the condition text decides the answer.
fn fake_check(o: &mut Options) {
    let script = r#"case "$1" in
  *MET*) echo 'thinking'; echo 'XHERD_RESULT {"met": true, "summary": "it is"}';;
  *NOT*) echo 'XHERD_RESULT {"met": false, "summary": "not yet"}';;
  *FAIL*) echo 'login needed' >&2; exit 3;;
  *) echo 'no idea';;
esac"#;
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data["terminal"]["check"]["argv"] =
        json!(["sh", "-c", script, "sh", "{prompt}", "{system}"]);
}

#[tokio::test]
async fn todo_conditions_are_checked_and_decide_what_happens() {
    use xherd_core::todo::{CheckOutcome, TodoStatus};
    use xherd_daemon::todos::TodoInput;
    let t = daemon_with(tempfile::tempdir().unwrap(), 4, fake_check).await;
    let mut rx = t.d.hub.subscribe();
    let now = t.d.clock.now();
    let mk = |title: &str, cond: &str| TodoInput {
        title: title.into(),
        note: None,
        due: now,
        repeat_minutes: Some(10),
        condition: Some(cond.into()),
        model: Some("m1".into()),
    };
    let met = t.d.create_todo(mk("met", "MET please")).await.unwrap();
    let not = t.d.create_todo(mk("not", "NOT yet")).await.unwrap();
    let bad = t.d.create_todo(mk("bad", "FAIL it")).await.unwrap();
    let odd = t.d.create_todo(mk("odd", "something")).await.unwrap();
    // A condition without an interval is refused.
    let mut no_interval = mk("x", "MET");
    no_interval.repeat_minutes = None;
    assert!(t.d.create_todo(no_interval).await.is_err());

    t.d.todo_tick().await;
    for _ in 0..200 {
        let l = t.d.todos().await.unwrap();
        if l.iter().all(|x| x.status != TodoStatus::Checking) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let list = t.d.todos().await.unwrap();
    let get = |id: &str| list.iter().find(|x| x.id == id).unwrap().clone();
    let (m, n, b, o) = (get(&met.id), get(&not.id), get(&bad.id), get(&odd.id));
    assert_eq!(m.status, TodoStatus::Due);
    assert_eq!(m.last_check.as_ref().unwrap().outcome, CheckOutcome::Met);
    assert_eq!(m.last_check.unwrap().summary.as_deref(), Some("it is"));
    assert_eq!(n.status, TodoStatus::Open, "not met: rescheduled");
    assert_eq!(n.due, now + 10 * 60_000);
    assert_eq!(b.status, TodoStatus::Error);
    assert!(
        b.last_check
            .unwrap()
            .summary
            .unwrap()
            .contains("login needed")
    );
    assert_eq!(o.status, TodoStatus::Due, "no answer line: you look");
    assert_eq!(o.last_check.unwrap().outcome, CheckOutcome::Unclear);

    // Notifications carry titles only, never what the check said.
    let mut texts = Vec::new();
    while let Ok(c) = rx.try_recv() {
        if let xherd_daemon::support::Change::Notify { text, .. } = c {
            texts.push(text);
        }
    }
    texts.sort();
    assert_eq!(texts, ["TODO check failed: bad", "TODO: met", "TODO: odd"]);

    // Check now: allowed from error, refused for a reminder.
    assert!(t.d.check_todo(&bad.id).await.is_ok());
    let plain =
        t.d.create_todo(TodoInput {
            condition: None,
            model: None,
            ..mk("p", "x")
        })
        .await
        .unwrap();
    assert_eq!(
        t.d.check_todo(&plain.id).await.unwrap_err().code,
        "not_checkable"
    );
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

#[tokio::test]
async fn a_session_over_the_limit_is_paused() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path());
    let cfg = xherd_core::config::Config {
        max_session_minutes: 30,
        ..Default::default()
    };
    paths.ensure().unwrap();
    cfg.save(&paths.config()).unwrap();
    let clock = Arc::new(ManualClock::new(1_000_000));
    let o = Options::new(Arc::new(FakeDriver::default()), clock.clone());
    let d = Daemon::start(paths, o).await.unwrap();
    d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&d, "SC-1", Status::Running).await;

    clock.advance(29 * 60_000);
    d.sup.tick(TickKind::SessionLimit);
    assert_eq!(status(&d, "SC-1").await, Status::Running);

    clock.advance(2 * 60_000);
    d.sup.tick(TickKind::SessionLimit);
    wait_status(&d, "SC-1", Status::Paused).await;
}

#[tokio::test]
async fn token_budget_warns_then_pauses_at_a_stop() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path());
    let cfg = xherd_core::config::Config {
        agent_token_budget: 1000,
        ..Default::default()
    };
    paths.ensure().unwrap();
    cfg.save(&paths.config()).unwrap();
    let projects = dir.path().join("projects");
    std::fs::create_dir_all(projects.join("p")).unwrap();
    let transcript = projects.join("p/s1.jsonl");
    let mut o = Options::new(
        Arc::new(FakeDriver::default()),
        Arc::new(ManualClock::new(1_000_000)),
    );
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data["terminal"]["resume"]["transcripts"] = projects.to_string_lossy().into();
    let d = Daemon::start(paths, o).await.unwrap();
    let a = d.create_agent(agent("SC-1", 1)).await.unwrap();
    wait_status(&d, "SC-1", Status::Running).await;
    let line =
        |n: u64| format!("{{\"message\":{{\"id\":\"m\",\"usage\":{{\"output_tokens\":{n}}}}}}}\n");
    let stop = |id: &str, seq: u64| {
        rep(
            &a,
            id,
            ReportEvent::Stop,
            seq,
            json!({"session_id": "s1", "message": "ok"}),
        )
    };

    std::fs::write(&transcript, line(850)).unwrap();
    d.report(stop("b1", 1)).await;
    for _ in 0..200 {
        if d.action("SC-1", UserAction::Get)
            .await
            .unwrap()
            .tokens_session
            == 850
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let ag = d.action("SC-1", UserAction::Get).await.unwrap();
    assert_eq!((ag.tokens_total(), ag.budget_warned), (850, true));
    assert_ne!(ag.status, Status::Paused);

    std::fs::write(&transcript, line(1200)).unwrap();
    d.report(stop("b2", 2)).await;
    wait_status(&d, "SC-1", Status::Paused).await;
    let events = d.store.events(&a.id, 50).await.unwrap();
    assert!(events.iter().any(|e| e.kind == "budget-exhausted"));
}

/// Git-access tokens and plugin secrets share `secrets.json`: saves made at
/// the same time keep both changes.
#[tokio::test]
async fn concurrent_secret_saves_keep_each_other() {
    let t = daemon(1).await;
    let (a, b) = (t.d.clone(), t.d.clone());
    let plugin = tokio::task::spawn_blocking(move || {
        for i in 0..40 {
            let v =
                std::collections::BTreeMap::from([("token".to_string(), Some(format!("p{i}")))]);
            a.set_plugin_secrets("github", v).unwrap();
        }
    });
    let hosts = tokio::task::spawn_blocking(move || {
        for i in 0..40 {
            let u: xherd_daemon::git_access::HostsUpdate = serde_json::from_value(json!({
                "hosts": [],
                "tokens": {"git.example.com": format!("g{i}")}
            }))
            .unwrap();
            b.set_git_hosts(u).unwrap();
        }
    });
    plugin.await.unwrap();
    hosts.await.unwrap();
    let s = xherd_core::config::Secrets::load(&t.d.paths.secrets()).unwrap();
    assert_eq!(s.for_plugin("github")["token"], "p39");
    assert_eq!(s.git_token("git.example.com").as_deref(), Some("g39"));
}
