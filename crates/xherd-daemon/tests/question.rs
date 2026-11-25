//! Questions from the transcript: a card, answered with keys once the screen shows it (R21, R22).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::chat::{AnsweredFrom, ChatBody, Resolution};
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

fn append(path: &std::path::Path, text: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn assistant(blocks: Value) -> String {
    format!(
        "{}\n",
        json!({"type": "assistant", "message": {"role": "assistant", "content": blocks}})
    )
}

fn ask(id: &str, questions: Value) -> String {
    assistant(
        json!([{"type": "tool_use", "id": id, "name": "AskUserQuestion",
        "input": {"questions": questions}}]),
    )
}

fn store_question() -> Value {
    json!([{"question": "Which store should the cache use?", "header": "Store",
    "multiSelect": false,
    "options": [
        {"label": "SQLite", "description": "one file"},
        {"label": "Redis", "description": "a server"}
    ]}])
}

fn result(id: &str) -> String {
    format!(
        "{}\n",
        json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": id, "content": "User has answered"}
        ]}})
    )
}

async fn job(t: &T) -> Job {
    t.d.action("SC-1", UserAction::Get).await.unwrap()
}

/// Ticks until `n` cards wait (the tick is what reads the transcript).
async fn cards(t: &T, n: usize) -> Vec<String> {
    for _ in 0..400 {
        t.d.sup.tick(TickKind::ScreenScan);
        let j = job(t).await;
        if j.waiting.len() == n {
            return j.waiting.iter().map(|w| w.id.clone()).collect();
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never {n} cards");
}

async fn answer(t: &T, qid: &str, option: usize) -> Result<Job, ActionError> {
    t.d.action(
        "SC-1",
        UserAction::Question {
            qid: qid.into(),
            option,
            from: AnsweredFrom::Chat,
        },
    )
    .await
}

fn states(t: &T, qid: &str) -> Vec<Resolution> {
    ChatLog::new(&t.d.paths, &t.job.id)
        .replay(0)
        .into_iter()
        .filter_map(|e| match e.body {
            ChatBody::Question { id, state, .. } if id == qid => Some(state),
            _ => None,
        })
        .collect()
}

async fn states_len(t: &T, qid: &str, n: usize) -> Vec<Resolution> {
    for _ in 0..400 {
        t.d.sup.tick(TickKind::ScreenScan);
        let s = states(t, qid);
        if s.len() >= n {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("never {n} states: {:?}", states(t, qid));
}

const SCREEN: &str = "Which store should the cache use?\n1. SQLite\n2. Redis";

#[tokio::test]
async fn a_question_in_the_transcript_becomes_a_card() {
    let t = running().await;
    append(&t.transcript, &ask("q1", store_question()));
    let ids = cards(&t, 1).await;
    assert_eq!(ids, ["q1"]);
    let w = &job(&t).await.waiting[0];
    assert_eq!(w.kind, xherd_core::job::WaitKind::Question);
    assert_eq!(w.summary, "Which store should the cache use?");
    assert!(w.answerable);
    assert_eq!(w.questions[0].options.len(), 2);
    assert_eq!(states(&t, "q1"), vec![Resolution::Waiting]);
}

#[tokio::test]
async fn the_answer_is_typed_only_when_the_screen_shows_the_question() {
    let t = running().await;
    append(&t.transcript, &ask("q1", store_question()));
    cards(&t, 1).await;

    *t.drv.screen.lock().unwrap() = "working…".into();
    assert_eq!(
        answer(&t, "q1", 1).await.unwrap_err().code,
        "question_not_shown"
    );
    assert_eq!(job(&t).await.waiting.len(), 1);
    assert!(!t.drv.calls().iter().any(|c| c.starts_with("send-keys")));

    assert_eq!(answer(&t, "q1", 7).await.unwrap_err().code, "not_offered");

    *t.drv.screen.lock().unwrap() = SCREEN.into();
    let j = answer(&t, "q1", 1).await.unwrap();
    assert!(j.waiting.is_empty());
    assert!(t.drv.calls().contains(&"send-keys SC-1 2".to_string()));
    assert_eq!(
        answer(&t, "q1", 1).await.unwrap_err().code,
        "already_answered"
    );

    // The tool result ends the card, with the answer and where it came from.
    append(&t.transcript, &result("q1"));
    let s = states_len(&t, "q1", 2).await;
    assert_eq!(
        s[1],
        Resolution::Answered {
            answer: "Redis".into(),
            from: AnsweredFrom::Chat
        }
    );
}

#[tokio::test]
async fn a_question_answered_in_the_terminal_leaves_the_card() {
    let t = running().await;
    append(&t.transcript, &ask("q1", store_question()));
    cards(&t, 1).await;
    append(&t.transcript, &result("q1"));
    cards(&t, 0).await;
    let s = states_len(&t, "q1", 2).await;
    assert_eq!(s[1], Resolution::InTerminal);
}

#[tokio::test]
async fn a_multi_select_question_is_not_answerable_here() {
    let t = running().await;
    let mut qs = store_question();
    qs[0]["multiSelect"] = json!(true);
    append(&t.transcript, &ask("q1", qs));
    cards(&t, 1).await;
    assert!(!job(&t).await.waiting[0].answerable);
    *t.drv.screen.lock().unwrap() = SCREEN.into();
    assert_eq!(
        answer(&t, "q1", 0).await.unwrap_err().code,
        "question_not_answerable"
    );
}

#[tokio::test]
async fn a_blocked_job_still_takes_the_answer() {
    let t = running().await;
    append(&t.transcript, &ask("q1", store_question()));
    cards(&t, 1).await;
    // A permission request blocks the job; the question's dialog may be what
    // blocks the agent, so keys must still go to it.
    let sup = t.d.sup.clone();
    let id = t.job.id.clone();
    let _hook = tokio::spawn(async move {
        sup.permission(
            &id,
            PermRequest {
                tool: "Bash".into(),
                input: json!({"command": "ls"}),
                suggestions: false,
            },
        )
        .await
    });
    cards(&t, 2).await;
    assert_eq!(job(&t).await.status, Status::Blocked);
    *t.drv.screen.lock().unwrap() = SCREEN.into();
    answer(&t, "q1", 0).await.unwrap();
    assert!(t.drv.calls().contains(&"send-keys SC-1 1".to_string()));
}
