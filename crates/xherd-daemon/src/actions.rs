//! Job actions, daemon side: the log of an action's last run (R5a, R13).

use std::path::Path;

use xherd_core::Job;
use xherd_core::action::approval_hash;
use xherd_core::paths::{Paths, write_atomic};
use xherd_core::profile;

use crate::{ActionError, Daemon, UserAction};

fn err(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

/// Most bytes of an action's log kept on disk: the end of the output (R13).
pub const MAX_LOG_BYTES: usize = 1024 * 1024;

/// Most bytes `xherd action log` returns (R5a).
pub const MAX_READ_BYTES: usize = 64 * 1024;

impl Daemon {
    /// Save the job's actions `ids` to its repo profile (R22, R23): each
    /// replaces the profile's action with the same id, and counts as the
    /// user's approval of its command. All or nothing for the profile;
    /// `no_profile` when the job has none that still exists.
    pub async fn save_actions_to_profile(
        &self,
        who: &str,
        ids: &[String],
    ) -> Result<Job, ActionError> {
        if ids.is_empty() {
            return Err(err("usage", "say which actions to save"));
        }
        let job = self.action(who, UserAction::Get).await?;
        let mut picked = Vec::new();
        for id in ids {
            let a = job
                .actions
                .iter()
                .find(|a| a.def.id == *id)
                .ok_or_else(|| err("action_unknown", format!("no action `{id}`")))?;
            picked.push(a.def.clone());
        }
        let _w = self.profiles_write.lock().await;
        let mut p = self.profile_of(&job).await.ok_or_else(|| {
            err(
                "no_profile",
                format!("{} has no repo profile to save to", job.name),
            )
        })?;
        for def in &picked {
            match p.actions.iter_mut().find(|x| x.id == def.id) {
                Some(x) => *x = def.clone(),
                None => p.actions.push(def.clone()),
            }
        }
        let others: Vec<String> = self
            .profiles()
            .await?
            .into_iter()
            .filter(|x| x.id != p.id)
            .map(|x| x.name)
            .collect();
        let names: Vec<&str> = others.iter().map(String::as_str).collect();
        profile::check(&mut p, &names).map_err(|m| err("bad_profile", m))?;
        self.store
            .put_profile(&p)
            .await
            .map_err(|e| err("store", e.to_string()))?;
        drop(_w);
        let mut job = job;
        for def in picked {
            job = self
                .action(
                    who,
                    UserAction::ActionApprove {
                        hash: approval_hash(&def),
                        id: def.id,
                        profile: p.name.clone(),
                    },
                )
                .await?;
        }
        Ok(job)
    }
}

/// Keep the last `max` bytes of `text`, starting at a character boundary.
fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Write the captured output of an action's last run (replacing the one
/// before), cut to its last [`MAX_LOG_BYTES`].
pub fn write_log(paths: &Paths, job_id: &str, action_id: &str, text: &str) -> std::io::Result<()> {
    let path = paths.action_log(job_id, action_id);
    write_atomic(&path, tail(text, MAX_LOG_BYTES).as_bytes(), Some(0o600))
}

/// The end of an action's log, at most `max` bytes; `None` when it never ran
/// or its log is gone.
pub fn read_log(paths: &Paths, job_id: &str, action_id: &str, max: usize) -> Option<String> {
    read_file_tail(&paths.action_log(job_id, action_id), max)
}

fn read_file_tail(path: &Path, max: usize) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let skip = len.saturating_sub(max as u64);
    f.seek(SeekFrom::Start(skip)).ok()?;
    let mut buf = Vec::new();
    f.take(max as u64).read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).into_owned();
    // A cut in the middle of a character leaves one replacement mark first.
    Some(if skip > 0 {
        text.strip_prefix('\u{fffd}').unwrap_or(&text).to_string()
    } else {
        text
    })
}

/// Delete the logs of a job (its worktree is gone or the job was removed).
pub fn remove_logs(paths: &Paths, job_id: &str) {
    if let Some(dir) = paths.action_log(job_id, "x").parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let d = tempfile::tempdir().unwrap();
        let p = Paths::at(d.path());
        (d, p)
    }

    #[test]
    fn a_log_round_trips_and_a_second_run_replaces_it() {
        let (_d, p) = paths();
        assert_eq!(read_log(&p, "j1", "dev", 100), None);
        write_log(&p, "j1", "dev", "first\n").unwrap();
        write_log(&p, "j1", "dev", "second\n").unwrap();
        assert_eq!(read_log(&p, "j1", "dev", 100).as_deref(), Some("second\n"));
    }

    #[test]
    fn the_file_is_capped_to_its_end_at_a_character_boundary() {
        let (_d, p) = paths();
        let text = format!("start{}", "é".repeat(MAX_LOG_BYTES));
        write_log(&p, "j1", "dev", &text).unwrap();
        let len = std::fs::metadata(p.action_log("j1", "dev")).unwrap().len();
        assert!(len as usize <= MAX_LOG_BYTES);
        let back = read_log(&p, "j1", "dev", MAX_LOG_BYTES).unwrap();
        assert!(!back.contains("start") && back.chars().all(|c| c == 'é'));
    }

    #[test]
    fn a_read_returns_only_the_end() {
        let (_d, p) = paths();
        write_log(&p, "j1", "dev", "0123456789\nend").unwrap();
        assert_eq!(read_log(&p, "j1", "dev", 5).as_deref(), Some("9\nend"));
    }

    #[test]
    fn a_read_cut_inside_a_character_drops_the_broken_part() {
        let (_d, p) = paths();
        write_log(&p, "j1", "dev", "aéé").unwrap();
        // 5 bytes: "aéé" is a(1) é(2) é(2); the last 3 start inside the first é.
        assert_eq!(read_log(&p, "j1", "dev", 3).as_deref(), Some("é"));
    }

    #[test]
    fn logs_are_removed_per_job() {
        let (_d, p) = paths();
        write_log(&p, "j1", "dev", "x").unwrap();
        write_log(&p, "j2", "dev", "y").unwrap();
        remove_logs(&p, "j1");
        assert_eq!(read_log(&p, "j1", "dev", 10), None);
        assert!(read_log(&p, "j2", "dev", 10).is_some());
    }
}
