//! Permission requests from the runtime's hook: a card, a block, the user's answer (R19, R20).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::chat::{AnsweredFrom, ChatBody, PermOption, Resolution};
use xherd_core::paths::Paths;
use xherd_core::report::{Report, ReportEvent};
use xherd_core::{Job, Status};
use xherd_daemon::chat::ChatLog;
use xherd_daemon::*;

#[allow(dead_code)]
struct T {
    _dir: tempfile::TempDir,
    d: Daemon,
    drv: Arc<FakeDriver>,
    job: Job,
    transcript: PathBuf,
}

fn options(drv: &Arc<FakeDriver>, projects: &std::path::Path) -> Options {
    let mut o = Options::new(drv.clone(), Arc::new(ManualClock::new(1_000_000)));
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data["terminal"]["resume"]["transcripts"] = projects.to_string_lossy().into();
    o
}

fn report(job: &Job, id: &str, event: ReportEvent, seq: u64, payload: Value) -> Report {
    Report {
        id: id.into(),
        job_id: job.id.clone(),
        event,
        payload,
        seq,
        at: 0,
    }
}

async fn wait_status(d: &Daemon, who: &str, want: Status) {
    for _ in 0..200 {
        if d.action(who, UserAction::Get).await.unwrap().status == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("{who} never reached {want}");
}

/// A running job with session `s1`, whose transcript is in a temp folder.
async fn running() -> T {
    running_waiting(900).await
}

/// The same, with `permissionWaitSec` set (below the setting's own minimum).
async fn running_waiting(wait_sec: u64) -> T {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path());
    paths.ensure().unwrap();
    let config = xherd_core::config::Config {
        permission_wait_sec: wait_sec,
        ..Default::default()
    };
    config.save(&paths.config()).unwrap();
    let projects = dir.path().join("projects");
    std::fs::create_dir_all(projects.join("p")).unwrap();
    let drv = Arc::new(FakeDriver::default());
    let d = Daemon::start(paths, options(&drv, &projects))
        .await
        .unwrap();
    let mut j = Job::new("SC-1", 1);
    j.workdir = Some("/tmp".into());
    j.launch = Some(vec!["fake-runtime".into()]);
    let job = d.create_job(j).await.unwrap();
    wait_status(&d, "SC-1", Status::Running).await;
    d.report(report(
        &job,
        "r1",
        ReportEvent::SessionStart,
        1,
        json!({"session_id": "s1"}),
    ))
    .await;
    T {
        transcript: projects.join("p/s1.jsonl"),
        _dir: dir,
        d,
        drv,
        job,
    }
}

fn request(t: &T, tool: &str, suggestions: bool) -> tokio::task::JoinHandle<Option<PermOption>> {
    let sup = t.d.sup.clone();
    let id = t.job.id.clone();
    let tool = tool.to_string();
    tokio::spawn(async move {
        sup.permission(
            &id,
            PermRequest {
                tool,
                input: json!({"command": "rm -rf target"}),
                suggestions,
            },
        )
        .await
    })
}

async fn job(t: &T) -> Job {
    t.d.action("SC-1", UserAction::Get).await.unwrap()
}

/// Waits until `n` cards wait; returns their ids.
async fn cards(t: &T, n: usize) -> Vec<String> {
    for _ in 0..400 {
        let j = job(t).await;
        if j.waiting.len() == n {
            return j.waiting.iter().map(|w| w.id.clone()).collect();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never {n} cards");
}

async fn answer(t: &T, pid: &str, answer: PermOption) -> Result<Job, ActionError> {
    t.d.action(
        "SC-1",
        UserAction::Permission {
            pid: pid.into(),
            answer,
            from: AnsweredFrom::Chat,
        },
    )
    .await
}

#[tokio::test]
async fn a_request_blocks_the_job_and_the_answer_reaches_the_hook() {
    let t = running().await;
    t.d.sup.tick(TickKind::ScreenScan);
    let hook = request(&t, "Bash", true);
    let ids = cards(&t, 1).await;
    let j = job(&t).await;
    assert_eq!(j.status, Status::Blocked);
    assert_eq!(j.block_reason.as_deref(), Some("permission"));
    assert!(!j.screen_blocked);
    assert_eq!(
        j.waiting[0].options,
        vec![PermOption::Allow, PermOption::AllowAlways, PermOption::Deny]
    );
    assert_eq!(j.waiting[0].summary, "Bash: rm -rf target");

    let j = answer(&t, &ids[0], PermOption::AllowAlways).await.unwrap();
    assert_eq!(hook.await.unwrap(), Some(PermOption::AllowAlways));
    assert_eq!(j.status, Status::Running);
    assert!(j.waiting.is_empty());
    assert!(j.block_reason.is_none());

    // The chat log holds the card waiting, then answered, under one id.
    let log = ChatLog::new(&t.d.paths, &t.job.id).replay(0);
    let states: Vec<&Resolution> = log
        .iter()
        .filter_map(|e| match &e.body {
            ChatBody::Permission { id, state, .. } if *id == ids[0] => Some(state),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        vec![
            &Resolution::Waiting,
            &Resolution::Answered {
                answer: "allow-always".into(),
                from: AnsweredFrom::Chat
            }
        ]
    );
}

#[tokio::test]
async fn deny_is_an_answer_and_no_suggestions_means_no_allow_always() {
    let t = running().await;
    let hook = request(&t, "Bash", false);
    let ids = cards(&t, 1).await;
    assert_eq!(job(&t).await.waiting[0].options.len(), 2);
    let e = answer(&t, &ids[0], PermOption::AllowAlways)
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_offered");
    answer(&t, &ids[0], PermOption::Deny).await.unwrap();
    assert_eq!(hook.await.unwrap(), Some(PermOption::Deny));
    let e = answer(&t, &ids[0], PermOption::Deny).await.unwrap_err();
    assert_eq!(e.code, "already_answered");
}

#[tokio::test]
async fn the_job_runs_again_only_after_the_last_of_two_requests() {
    let t = running().await;
    let first = request(&t, "Bash", false);
    cards(&t, 1).await;
    let second = request(&t, "Edit", false);
    let ids = cards(&t, 2).await;
    assert_eq!(job(&t).await.block_reason.as_deref(), Some("2 permissions"));

    // A tool that was allowed finishes: its post-tool-use must not unblock the other.
    t.d.report(report(&t.job, "r2", ReportEvent::PostToolUse, 2, json!({})))
        .await;
    assert_eq!(job(&t).await.status, Status::Blocked);

    let j = answer(&t, &ids[0], PermOption::Allow).await.unwrap();
    assert_eq!(j.status, Status::Blocked);
    assert_eq!(j.block_reason.as_deref(), Some("permission"));
    assert_eq!(first.await.unwrap(), Some(PermOption::Allow));

    let j = answer(&t, &ids[1], PermOption::Deny).await.unwrap();
    assert_eq!(j.status, Status::Running);
    assert_eq!(second.await.unwrap(), Some(PermOption::Deny));
}

#[tokio::test]
async fn xherd_does_not_decide_a_question_or_an_unknown_job() {
    let t = running().await;
    assert_eq!(request(&t, "AskUserQuestion", false).await.unwrap(), None);
    assert!(job(&t).await.waiting.is_empty());
    assert_eq!(
        t.d.permission_request(
            "nope",
            PermRequest {
                tool: "Bash".into(),
                input: json!({}),
                suggestions: false
            }
        )
        .await,
        None
    );
}

#[tokio::test]
async fn a_job_that_is_not_live_gets_no_card() {
    let t = running().await;
    t.d.action("SC-1", UserAction::PauseNow).await.unwrap();
    assert_eq!(request(&t, "Bash", false).await.unwrap(), None);
    assert!(job(&t).await.waiting.is_empty());
}

fn states(t: &T, pid: &str) -> Vec<Resolution> {
    ChatLog::new(&t.d.paths, &t.job.id)
        .replay(0)
        .into_iter()
        .filter_map(|e| match e.body {
            ChatBody::Permission { id, state, .. } if id == pid => Some(state),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_timed_out_request_gets_none_and_the_job_stays_blocked_until_a_hook() {
    let t = running_waiting(1).await;
    t.d.sup.tick(TickKind::ScreenScan);
    let hook = request(&t, "Bash", false);
    let ids = cards(&t, 1).await;
    assert_eq!(hook.await.unwrap(), None);
    let j = job(&t).await;
    assert!(j.waiting.is_empty());
    assert_eq!(j.status, Status::Blocked);
    assert_eq!(
        j.block_reason.as_deref(),
        Some("permission: answer in the terminal")
    );
    assert_eq!(
        states(&t, &ids[0]),
        vec![Resolution::Waiting, Resolution::TimedOut]
    );
    let e = answer(&t, &ids[0], PermOption::Allow).await.unwrap_err();
    assert_eq!(e.code, "already_answered");

    t.d.report(report(&t.job, "r2", ReportEvent::PostToolUse, 2, json!({})))
        .await;
    assert_eq!(job(&t).await.status, Status::Running);
}

#[tokio::test]
async fn a_dropped_request_leaves_waiting_and_is_resolved_in_the_terminal() {
    let t = running().await;
    t.d.sup.tick(TickKind::ScreenScan);
    let hook = request(&t, "Bash", false);
    let ids = cards(&t, 1).await;
    // The CLI's request broke: nobody waits for the answer any more.
    hook.abort();
    for _ in 0..200 {
        t.d.sup.tick(TickKind::ScreenScan);
        if job(&t).await.waiting.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let j = job(&t).await;
    assert!(j.waiting.is_empty());
    assert_eq!(j.status, Status::Blocked);
    assert_eq!(
        states(&t, &ids[0]),
        vec![Resolution::Waiting, Resolution::InTerminal]
    );
}

#[tokio::test]
async fn a_request_that_waits_at_a_restart_is_lost() {
    let t = running().await;
    t.d.sup.tick(TickKind::ScreenScan);
    let _hook = request(&t, "Bash", false);
    let ids = cards(&t, 1).await;

    let projects = t
        .transcript
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let d = Daemon::start(t.d.paths.clone(), options(&t.drv, &projects))
        .await
        .unwrap();
    let mut j = d.action("SC-1", UserAction::Get).await.unwrap();
    for _ in 0..200 {
        if j.waiting.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        j = d.action("SC-1", UserAction::Get).await.unwrap();
    }
    assert!(j.waiting.is_empty());
    assert_eq!(j.status, Status::Blocked);
    assert_eq!(
        j.block_reason.as_deref(),
        Some("permission: answer in the terminal")
    );
    assert_eq!(
        states(&t, &ids[0]),
        vec![Resolution::Waiting, Resolution::Lost]
    );
}
