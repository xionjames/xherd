//! A job's chat log and the per-job channel that feeds chat clients (R5).
//!
//! `chat/<job>/events.jsonl` holds one `ChatEvent` per line and `state.json`
//! the counters the follower resumes from. The log is blocking file I/O:
//! callers on the async side run it where a short block is fine (a tick) or
//! in `spawn_blocking`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use xherd_core::chat::{ChatBody, ChatEvent};
use xherd_core::paths::{Paths, write_atomic};

/// Size of the log past which it is folded.
pub const LOG_CAP: u64 = 4 * 1024 * 1024;
/// What a fold keeps at most (the newest lines).
pub const LOG_KEEP: u64 = 2 * 1024 * 1024;
/// Events a slow client may lag behind before it must re-subscribe.
const HUB_CAPACITY: usize = 256;

/// `state.json`: where the follower is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// Sequence number of the last event logged.
    pub seq: u64,
    /// Transcript lines the mapper did not understand.
    pub unknown: u64,
    /// Transcript offset read up to, per session id.
    pub positions: BTreeMap<String, u64>,
}

/// One job's log folder.
#[derive(Debug, Clone)]
pub struct ChatLog {
    dir: PathBuf,
}

impl ChatLog {
    pub fn new(paths: &Paths, job_id: &str) -> ChatLog {
        ChatLog {
            dir: paths.chat_dir(job_id),
        }
    }

    fn events(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    fn state_file(&self) -> PathBuf {
        self.dir.join("state.json")
    }

    /// Appends a batch with one write. Past `LOG_CAP` the log is folded; the
    /// ids whose events were dropped come back, for `Mapper::forget`.
    pub fn append(&self, events: &[ChatEvent]) -> std::io::Result<Vec<String>> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let mut buf = Vec::new();
        for e in events {
            serde_json::to_writer(&mut buf, e).map_err(std::io::Error::other)?;
            buf.push(b'\n');
        }
        std::fs::create_dir_all(&self.dir)?;
        let mut f = open_append(&self.events())?;
        f.write_all(&buf)?;
        if f.metadata()?.len() > LOG_CAP {
            return self.fold();
        }
        Ok(Vec::new())
    }

    /// The events a client gets after `after_seq`, oldest first. A torn or
    /// unreadable line is skipped; `Unknown` is never sent to clients.
    pub fn replay(&self, after_seq: u64) -> Vec<ChatEvent> {
        self.read_all()
            .into_iter()
            .map(|(_, e)| e)
            .filter(|e| e.seq > after_seq && !matches!(e.body, ChatBody::Unknown { .. }))
            .collect()
    }

    /// The transcript offset after the last logged line (`end` of the newest
    /// event that has one).
    pub fn last_end(&self) -> Option<u64> {
        self.read_all().iter().rev().find_map(|(_, e)| e.end)
    }

    /// A missing or unreadable `state.json` is the default state.
    pub fn load_state(&self) -> State {
        std::fs::read(self.state_file())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save_state(&self, state: &State) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(state).map_err(std::io::Error::other)?;
        write_atomic(&self.state_file(), &bytes, Some(0o600))
    }

    /// Complete lines of the log with their parsed event.
    fn read_all(&self) -> Vec<(Vec<u8>, ChatEvent)> {
        let Ok(bytes) = std::fs::read(self.events()) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // The last piece has no newline when the write was cut: drop it.
        let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
        for line in bytes[..complete].split(|b| *b == b'\n') {
            if let Ok(e) = serde_json::from_slice::<ChatEvent>(line) {
                out.push((line.to_vec(), e));
            }
        }
        out
    }

    /// Keeps the last event of each tool, permission and question id and the
    /// latest plan, then the newest `LOG_KEEP` bytes of what is left.
    fn fold(&self) -> std::io::Result<Vec<String>> {
        let all = self.read_all();
        let mut last: HashMap<String, usize> = HashMap::new();
        for (i, (_, e)) in all.iter().enumerate() {
            if let Some(k) = key(&e.body) {
                last.insert(k, i);
            }
        }
        let mut keep: Vec<usize> = (0..all.len())
            .filter(|i| key(&all[*i].1.body).is_none_or(|k| last[&k] == *i))
            .collect();
        let mut size: u64 = keep.iter().map(|i| all[*i].0.len() as u64 + 1).sum();
        let mut cut = 0;
        while size > LOG_KEEP && cut < keep.len() {
            size -= all[keep[cut]].0.len() as u64 + 1;
            cut += 1;
        }
        keep.drain(..cut);

        let mut out = Vec::with_capacity(size as usize);
        for i in &keep {
            out.extend_from_slice(&all[*i].0);
            out.push(b'\n');
        }
        write_atomic(&self.events(), &out, Some(0o600))?;

        let kept: BTreeSet<String> = keep.iter().filter_map(|i| id(&all[*i].1.body)).collect();
        let dropped: BTreeSet<String> = all
            .iter()
            .filter_map(|(_, e)| id(&e.body))
            .filter(|i| !kept.contains(i))
            .collect();
        Ok(dropped.into_iter().collect())
    }
}

fn open_append(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

fn id(body: &ChatBody) -> Option<String> {
    match body {
        ChatBody::Tool { id, .. }
        | ChatBody::Permission { id, .. }
        | ChatBody::Question { id, .. } => Some(id.clone()),
        _ => None,
    }
}

/// What makes two events versions of one thing.
fn key(body: &ChatBody) -> Option<String> {
    match body {
        ChatBody::Plan { .. } => Some("plan".into()),
        other => id(other).map(|i| format!("id:{i}")),
    }
}

/// What a chat client's channel carries.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatMsg {
    Event(ChatEvent),
    /// The count of transcript lines not understood changed.
    Unknown(u64),
}

/// One broadcast channel per job, created on first use. Chat traffic goes
/// only to the clients of that job, not through the global `Change` hub.
#[derive(Debug, Default)]
pub struct ChatHubs {
    jobs: Mutex<HashMap<String, broadcast::Sender<ChatMsg>>>,
}

impl ChatHubs {
    pub fn subscribe(&self, job_id: &str) -> broadcast::Receiver<ChatMsg> {
        self.sender(job_id).subscribe()
    }

    /// Sends to the job's clients; nobody listening is fine.
    pub fn publish(&self, job_id: &str, msg: ChatMsg) {
        let _ = self.sender(job_id).send(msg);
    }

    fn sender(&self, job_id: &str) -> broadcast::Sender<ChatMsg> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.entry(job_id.to_string())
            .or_insert_with(|| broadcast::channel(HUB_CAPACITY).0)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xherd_core::chat::{PlanItem, PlanStatus, Source, ToolStatus};

    fn ev(seq: u64, body: ChatBody) -> ChatEvent {
        ChatEvent {
            seq,
            at: seq as i64,
            session: None,
            end: Some(seq * 10),
            body,
        }
    }

    fn user(seq: u64, text: &str) -> ChatEvent {
        ev(
            seq,
            ChatBody::User {
                text: text.into(),
                source: Source::User,
            },
        )
    }

    fn tool(seq: u64, id: &str, status: ToolStatus, result: &str) -> ChatEvent {
        ev(
            seq,
            ChatBody::Tool {
                id: id.into(),
                name: "Bash".into(),
                summary: "Bash: ls".into(),
                status,
                result: Some(result.into()),
                path: None,
            },
        )
    }

    fn log() -> (tempfile::TempDir, ChatLog) {
        let dir = tempfile::tempdir().unwrap();
        let log = ChatLog::new(&Paths::at(dir.path()), "j1");
        (dir, log)
    }

    #[test]
    fn chat_log_replays_after_a_seq_and_skips_unknown() {
        let (_d, log) = log();
        log.append(&[
            user(1, "a"),
            ev(2, ChatBody::Unknown { raw: "?".into() }),
            user(3, "b"),
        ])
        .unwrap();
        let seqs = |after| log.replay(after).iter().map(|e| e.seq).collect::<Vec<_>>();
        assert_eq!(seqs(0), vec![1, 3]);
        assert_eq!(seqs(1), vec![3]);
        assert!(seqs(3).is_empty());
        assert_eq!(log.last_end(), Some(30));
    }

    #[test]
    fn chat_log_without_a_file_is_empty() {
        let (_d, log) = log();
        assert!(log.replay(0).is_empty());
        assert_eq!(log.last_end(), None);
        assert_eq!(log.load_state(), State::default());
    }

    #[test]
    fn chat_log_skips_a_torn_last_line() {
        let (_d, log) = log();
        log.append(&[user(1, "a")]).unwrap();
        let mut f = open_append(&log.events()).unwrap();
        f.write_all(b"{\"seq\":2,\"at\":2,\"kind\":\"us").unwrap();
        assert_eq!(log.replay(0).len(), 1);
        // The next batch starts a fresh line only when the cut one ended.
        f.write_all(b"\n").unwrap();
        log.append(&[user(3, "c")]).unwrap();
        assert_eq!(
            log.replay(0).iter().map(|e| e.seq).collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn chat_state_round_trips_and_a_broken_file_is_the_default() {
        let (_d, log) = log();
        let mut s = State {
            seq: 43,
            unknown: 2,
            ..State::default()
        };
        s.positions.insert("5c1".into(), 18234);
        log.save_state(&s).unwrap();
        assert_eq!(log.load_state(), s);
        std::fs::write(log.state_file(), b"{not json").unwrap();
        assert_eq!(log.load_state(), State::default());
    }

    #[test]
    fn chat_state_json_is_camel_case() {
        let mut s = State {
            seq: 1,
            unknown: 2,
            ..State::default()
        };
        s.positions.insert("a".into(), 3);
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"seq":1,"unknown":2,"positions":{"a":3}}"#
        );
    }

    #[test]
    fn chat_log_folds_past_the_cap_keeping_the_last_version_of_each_id() {
        let (_d, log) = log();
        let big = "x".repeat(64 * 1024);
        let mut seq = 0;
        let mut next = || {
            seq += 1;
            seq
        };
        // One tool, first running then done, and a plan, early on.
        log.append(&[
            tool(next(), "t1", ToolStatus::Running, ""),
            tool(next(), "t1", ToolStatus::Ok, "done"),
            ev(
                next(),
                ChatBody::Plan {
                    items: vec![PlanItem {
                        text: "old".into(),
                        status: PlanStatus::Pending,
                    }],
                },
            ),
            ev(
                next(),
                ChatBody::Plan {
                    items: vec![PlanItem {
                        text: "new".into(),
                        status: PlanStatus::Pending,
                    }],
                },
            ),
        ])
        .unwrap();
        let mut dropped = Vec::new();
        for i in 0..70 {
            let mut batch = vec![user(next(), &big)];
            batch.push(tool(next(), &format!("big{i}"), ToolStatus::Ok, "r"));
            dropped.extend(log.append(&batch).unwrap());
        }
        let size = std::fs::metadata(log.events()).unwrap().len();
        assert!(size <= LOG_CAP, "log is {size} bytes");
        assert!(!dropped.is_empty());
        assert!(dropped.iter().all(|i| i.starts_with("big") || i == "t1"));
        let events = log.replay(0);
        // No duplicate version of an id survives.
        let ids: Vec<_> = events.iter().filter_map(|e| id(&e.body)).collect();
        let unique: BTreeSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len());
        // The newest events are there, in order.
        assert!(events.windows(2).all(|w| w[0].seq < w[1].seq));
        assert_eq!(events.last().unwrap().seq, 2 + 2 + 140);
        // The old plan is gone for good.
        let plans = events
            .iter()
            .filter(|e| matches!(e.body, ChatBody::Plan { .. }))
            .count();
        assert!(plans <= 1);
    }

    #[test]
    fn chat_fold_keeps_the_newest_two_mib_at_most() {
        let (_d, log) = log();
        let big = "x".repeat(100 * 1024);
        let size = || std::fs::metadata(log.events()).unwrap().len();
        // The append that crosses the cap folds the log to LOG_KEEP at most.
        let mut seq = 0;
        let mut before = 0;
        loop {
            seq += 1;
            log.append(&[user(seq, &big)]).unwrap();
            if size() < before {
                break;
            }
            before = size();
            assert!(seq < 100, "the log never folded");
        }
        assert!(size() <= LOG_KEEP, "log is {} bytes", size());
        assert_eq!(log.replay(0).last().unwrap().seq, seq);
    }

    #[tokio::test]
    async fn chat_hub_delivers_to_one_job_only() {
        let hubs = ChatHubs::default();
        let mut a = hubs.subscribe("a");
        let mut b = hubs.subscribe("b");
        hubs.publish("a", ChatMsg::Unknown(3));
        hubs.publish("c", ChatMsg::Unknown(9)); // nobody there: not an error
        assert_eq!(a.recv().await.unwrap(), ChatMsg::Unknown(3));
        assert!(b.try_recv().is_err());
    }
}
