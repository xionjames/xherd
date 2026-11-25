//! The actor follows a live session's transcript into the chat log (R4, R5).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::chat::{ChatBody, ChatEvent, NoticeKind, Source, ToolStatus};
use xherd_core::paths::Paths;
use xherd_core::report::{Report, ReportEvent};
use xherd_core::{Job, Status};
use xherd_daemon::chat::ChatLog;
use xherd_daemon::*;

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
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::at(dir.path());
    paths.ensure().unwrap();
    xherd_core::config::Config::default()
        .save(&paths.config())
        .unwrap();
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

fn user(text: &str) -> String {
    format!(
        "{}\n",
        json!({"type": "user", "message": {"role": "user", "content": text}})
    )
}

fn assistant(blocks: Value) -> String {
    format!(
        "{}\n",
        json!({"type": "assistant", "message": {"role": "assistant", "content": blocks}})
    )
}

fn tool_use(id: &str) -> String {
    assistant(json!([{"type": "tool_use", "id": id, "name": "Bash", "input": {"command": "ls"}}]))
}

fn tool_result(id: &str, out: &str) -> String {
    format!(
        "{}\n",
        json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": id, "content": out}
        ]}})
    )
}

/// Ticks until the log holds `n` events (the tick is what reads).
async fn events(d: &Daemon, job: &Job, n: usize) -> Vec<ChatEvent> {
    let log = ChatLog::new(&d.paths, &job.id);
    for _ in 0..400 {
        d.sup.tick(TickKind::ScreenScan);
        let e = log.replay(0);
        if e.len() >= n {
            return e;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the log never reached {n} events: {:?}", log.replay(0));
}

#[tokio::test]
async fn a_transcript_becomes_logged_events_and_a_partial_line_waits() {
    let t = running().await;
    append(&t.transcript, &user("hello"));
    append(
        &t.transcript,
        &assistant(json!([{"type":"text","text":"hi"}])),
    );
    append(&t.transcript, &tool_use("t1"));
    let e = events(&t.d, &t.job, 3).await;
    assert_eq!(e.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(matches!(
        &e[0].body,
        ChatBody::User { text, source: Source::User } if text == "hello"
    ));
    assert!(matches!(&e[1].body, ChatBody::Assistant { text } if text == "hi"));
    assert!(matches!(
        &e[2].body,
        ChatBody::Tool {
            status: ToolStatus::Running,
            ..
        }
    ));
    assert_eq!(e[2].session.as_deref(), Some("s1"));

    // The result of a known tool updates it; a line still being written is
    // left until its newline arrives.
    let result = tool_result("t1", "a.txt");
    let (head, tail) = result.split_at(20);
    append(
        &t.transcript,
        &format!(
            "{}{head}",
            assistant(json!([{"type":"text","text":"done"}]))
        ),
    );
    let e = events(&t.d, &t.job, 4).await;
    assert!(matches!(&e[3].body, ChatBody::Assistant { text } if text == "done"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(ChatLog::new(&t.d.paths, &t.job.id).replay(0).len(), 4);
    append(&t.transcript, tail);
    let e = events(&t.d, &t.job, 5).await;
    assert!(matches!(
        &e[4].body,
        ChatBody::Tool { id, status: ToolStatus::Ok, result: Some(r), .. } if id == "t1" && r == "a.txt"
    ));

    let state = ChatLog::new(&t.d.paths, &t.job.id).load_state();
    let len = std::fs::metadata(&t.transcript).unwrap().len();
    assert_eq!((state.seq, state.positions["s1"]), (5, len));
}

#[tokio::test]
async fn a_restart_neither_repeats_nor_skips_lines() {
    let t = running().await;
    append(&t.transcript, &user("hello"));
    append(&t.transcript, &tool_use("t1"));
    events(&t.d, &t.job, 2).await;

    // The daemon restarts on the same data folder; the session is still there.
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
    wait_status(&d, "SC-1", Status::Running).await;
    append(&t.transcript, &tool_result("t1", "ok"));
    append(
        &t.transcript,
        &assistant(json!([{"type":"text","text":"next"}])),
    );
    let e = events(&d, &t.job, 4).await;
    assert_eq!(e.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3, 4]);
    // The tool logged before the restart is still known, so its result counts.
    assert!(matches!(
        &e[2].body,
        ChatBody::Tool { id, status: ToolStatus::Ok, .. } if id == "t1"
    ));
    assert!(matches!(&e[3].body, ChatBody::Assistant { text } if text == "next"));
}

#[tokio::test]
async fn a_runtime_without_a_chat_view_is_not_followed() {
    let t = running().await;
    // A job on another runtime has no `chat`: nothing is read or written.
    let mut j = Job::new("SC-2", 2);
    j.workdir = Some("/tmp".into());
    j.launch = Some(vec!["fake-runtime".into()]);
    j.runtime = Some("opencode".into());
    let other = t.d.create_job(j).await.unwrap();
    wait_status(&t.d, "SC-2", Status::Running).await;
    t.d.report(report(
        &other,
        "o1",
        ReportEvent::SessionStart,
        1,
        json!({"session_id": "s1"}),
    ))
    .await;
    append(&t.transcript, &user("hello"));
    events(&t.d, &t.job, 1).await;
    assert!(ChatLog::new(&t.d.paths, &other.id).replay(0).is_empty());
}

#[tokio::test]
async fn a_new_session_is_read_after_the_old_one_is_drained_with_a_notice() {
    let t = running().await;
    append(&t.transcript, &user("first"));
    events(&t.d, &t.job, 1).await;
    // A line written to the old session is not read before the switch.
    append(&t.transcript, &user("last words"));
    t.d.report(report(
        &t.job,
        "r2",
        ReportEvent::SessionStart,
        2,
        json!({"session_id": "s2"}),
    ))
    .await;
    let second = t.transcript.with_file_name("s2.jsonl");
    append(&second, &user("again"));
    let e = events(&t.d, &t.job, 4).await;
    assert!(matches!(&e[1].body, ChatBody::User { text, .. } if text == "last words"));
    assert_eq!(e[1].session.as_deref(), Some("s1"));
    assert!(matches!(
        &e[2].body,
        ChatBody::Notice {
            notice: NoticeKind::NewSession,
            ..
        }
    ));
    assert!(matches!(&e[3].body, ChatBody::User { text, .. } if text == "again"));
    assert_eq!(e[3].session.as_deref(), Some("s2"));
    assert_eq!(e.iter().map(|e| e.seq).collect::<Vec<_>>(), [1, 2, 3, 4]);
}

#[tokio::test]
async fn leaving_the_live_states_reads_what_is_left_and_stops() {
    let t = running().await;
    append(&t.transcript, &user("before"));
    events(&t.d, &t.job, 1).await;
    append(&t.transcript, &user("after"));
    t.d.action("SC-1", UserAction::PauseNow).await.unwrap();
    wait_status(&t.d, "SC-1", Status::Paused).await;
    // No tick comes for a paused job: the drain alone reads the line.
    let log = ChatLog::new(&t.d.paths, &t.job.id);
    for _ in 0..400 {
        if log.replay(0).len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let e = log.replay(0);
    assert!(matches!(&e[1].body, ChatBody::User { text, .. } if text == "after"));
    // Stopped: later writes are not followed.
    append(&t.transcript, &user("never"));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(log.replay(0).len(), 2);
}

#[tokio::test]
async fn a_log_past_its_cap_is_folded_and_the_newest_events_stay() {
    let t = running().await;
    let big = "x".repeat(200 * 1024);
    for i in 0..24 {
        append(&t.transcript, &user(&format!("{i} {big}")));
    }
    let log = ChatLog::new(&t.d.paths, &t.job.id);
    for _ in 0..800 {
        t.d.sup.tick(TickKind::ScreenScan);
        if log.load_state().seq >= 24 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(log.load_state().seq, 24);
    let e = log.replay(0);
    assert!(e.len() < 24, "the oldest events were dropped");
    assert!(matches!(
        &e.last().unwrap().body,
        ChatBody::User { text, .. } if text.starts_with("23 ")
    ));
    assert!(e[0].seq > 1);
}
