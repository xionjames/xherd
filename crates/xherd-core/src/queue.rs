//! `hook-queue.jsonl`: reports and report-only XAP calls written by the CLI
//! while the daemon is down, replayed in order on daemon start (R-014,
//! xap.md §1.2). Synchronous (the CLI hot path has
//! no async runtime). Appends and the daemon's take-over hold an exclusive lock.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::report::Report;
use crate::xap::XapCall;

/// One queued line. Older queues hold reports only; both shapes parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Queued {
    Report(Report),
    Xap(XapCall),
}

impl From<Report> for Queued {
    fn from(r: Report) -> Queued {
        Queued::Report(r)
    }
}

impl From<XapCall> for Queued {
    fn from(c: XapCall) -> Queued {
        Queued::Xap(c)
    }
}

fn open_locked_append(path: &Path) -> std::io::Result<fd_lock::RwLock<File>> {
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    Ok(fd_lock::RwLock::new(opts.open(path)?))
}

#[cfg(unix)]
fn same_file(f: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (f.metadata(), std::fs::metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(_f: &File, path: &Path) -> bool {
    path.exists()
}

/// Append one report or XAP call as a JSON line.
pub fn append(path: &Path, item: &Queued) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_vec(item).map_err(std::io::Error::other)?;
    line.push(b'\n');
    // The daemon may rename the file between our open and our lock; retry on the new file.
    for _ in 0..5 {
        let mut lock = open_locked_append(path)?;
        let mut g = lock.write()?;
        if !same_file(&g, path) {
            continue;
        }
        g.write_all(&line)?;
        g.flush()?;
        return Ok(());
    }
    Err(std::io::Error::other("hook queue kept moving"))
}

fn replaying(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".replaying");
    PathBuf::from(p)
}

/// Take every queued item, oldest first. A take-over interrupted by a crash
/// is picked up again next time; replays are de-duplicated by report id.
/// Lines that do not parse are logged and skipped.
pub fn take(path: &Path) -> std::io::Result<Vec<Queued>> {
    let pending = replaying(path);
    if path.exists() {
        let mut lock = open_locked_append(path)?;
        let _g = lock.write()?;
        if pending.exists() {
            // Previous take-over was interrupted: keep its lines first.
            let mut old = std::fs::read(&pending)?;
            old.extend(std::fs::read(path)?);
            crate::paths::write_atomic(&pending, &old, Some(0o600))?;
            std::fs::remove_file(path)?;
        } else {
            std::fs::rename(path, &pending)?;
        }
    }
    let file = match File::open(&pending) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Queued>(&line) {
            Ok(r) => out.push(r),
            Err(e) => tracing::warn!(line = i + 1, error = %e, "skipping bad hook-queue line"),
        }
    }
    Ok(out)
}

/// Call after the items returned by [`take`] were applied.
pub fn done(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(replaying(path)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::ReportEvent;

    fn seq(q: &Queued) -> u64 {
        match q {
            Queued::Report(r) => r.seq,
            Queued::Xap(c) => c.at as u64,
        }
    }

    fn rep(n: u64) -> Queued {
        Queued::Report(Report {
            id: format!("r{n}"),
            agent_id: "a".into(),
            event: ReportEvent::Stop,
            payload: serde_json::Value::Null,
            seq: n,
            at: 0,
        })
    }

    #[test]
    fn append_take_done() {
        let d = tempfile::tempdir().unwrap();
        let q = d.path().join("hook-queue.jsonl");
        assert!(take(&q).unwrap().is_empty());
        append(&q, &rep(1)).unwrap();
        append(&q, &rep(2)).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&q)
            .unwrap()
            .write_all(b"garbage\n")
            .unwrap();
        let got = take(&q).unwrap();
        assert_eq!(got.iter().map(seq).collect::<Vec<_>>(), vec![1, 2]);
        // New appends go to a fresh file while the old batch is being replayed.
        append(&q, &rep(3)).unwrap();
        done(&q).unwrap();
        assert_eq!(take(&q).unwrap().len(), 1);
        done(&q).unwrap();
        assert!(take(&q).unwrap().is_empty());
    }

    #[test]
    fn reports_and_xap_calls_mix() {
        let d = tempfile::tempdir().unwrap();
        let q = d.path().join("hook-queue.jsonl");
        append(&q, &rep(1)).unwrap();
        let call: XapCall = serde_json::from_value(serde_json::json!({
            "id": "x", "agentId": "a", "cmd": "task.start", "args": {"id": "T01"}, "at": 2
        }))
        .unwrap();
        append(&q, &call.into()).unwrap();
        let got = take(&q).unwrap();
        assert!(matches!(got[0], Queued::Report(_)));
        assert!(matches!(&got[1], Queued::Xap(c) if c.cmd.name() == "task.start"));
    }

    #[test]
    fn interrupted_take_is_replayed_first() {
        let d = tempfile::tempdir().unwrap();
        let q = d.path().join("hook-queue.jsonl");
        append(&q, &rep(1)).unwrap();
        take(&q).unwrap(); // crash: no done()
        append(&q, &rep(2)).unwrap();
        let got = take(&q).unwrap();
        assert_eq!(got.iter().map(seq).collect::<Vec<_>>(), vec![1, 2]);
    }

    #[test]
    fn concurrent_appends_are_not_lost() {
        let d = tempfile::tempdir().unwrap();
        let q = d.path().join("hook-queue.jsonl");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let q = q.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        append(&q, &rep(t * 100 + i)).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(take(&q).unwrap().len(), 200);
    }
}
