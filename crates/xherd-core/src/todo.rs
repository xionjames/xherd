//! TODOs (spec §16.2): reminders due at a time, once or repeating. The
//! lifecycle is pure: `open` (waiting for its time) → `due` (needs you) →
//! `done`, or back to `open` when it repeats or is rescheduled.
//! Condition checks (a read-only runtime session) are not built.

use serde::{Deserialize, Serialize};

use crate::agent::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TodoStatus {
    /// Waiting for its due time.
    Open,
    /// The time has come: it needs you.
    Due,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub due: Millis,
    /// Minutes between repeats; `None` = once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_minutes: Option<u64>,
    pub status: TodoStatus,
    pub created_at: Millis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<Millis>,
}

pub const MIN_REPEAT_MINUTES: u64 = 5;
pub const MAX_TITLE: usize = 200;
pub const MAX_NOTE: usize = 2000;

/// What the user may set; checked before a TODO is stored.
pub fn check(
    title: &str,
    note: Option<&str>,
    due: Millis,
    repeat: Option<u64>,
) -> Result<(), String> {
    let t = title.trim();
    if t.is_empty() || t.chars().count() > MAX_TITLE || t.chars().any(char::is_control) {
        return Err(format!(
            "the title is one line of 1 to {MAX_TITLE} characters"
        ));
    }
    if note.is_some_and(|n| n.chars().count() > MAX_NOTE) {
        return Err(format!("the note is at most {MAX_NOTE} characters"));
    }
    if due <= 0 {
        return Err("say when it is due".into());
    }
    if repeat.is_some_and(|r| r < MIN_REPEAT_MINUTES) {
        return Err(format!("repeat at most every {MIN_REPEAT_MINUTES} minutes"));
    }
    Ok(())
}

impl Todo {
    pub fn new(
        id: String,
        title: &str,
        note: Option<&str>,
        due: Millis,
        repeat_minutes: Option<u64>,
        now: Millis,
    ) -> Todo {
        Todo {
            id,
            title: title.trim().to_string(),
            note: note
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_string),
            due,
            repeat_minutes,
            status: TodoStatus::Open,
            created_at: now,
            completed_at: None,
        }
    }

    /// Its time has come. Returns whether it changed (and should notify).
    pub fn fire(&mut self, now: Millis) -> bool {
        if self.status == TodoStatus::Open && self.due <= now {
            self.status = TodoStatus::Due;
            return true;
        }
        false
    }

    /// Complete: a repeating TODO waits for its next time after `now`, a
    /// one-off is done.
    pub fn complete(&mut self, now: Millis) {
        match self.repeat_minutes {
            Some(m) => {
                let step = m as i64 * 60_000;
                let missed = ((now - self.due).max(0) / step) + 1;
                self.due += missed * step;
                self.status = TodoStatus::Open;
            }
            None => {
                self.status = TodoStatus::Done;
                self.completed_at = Some(now);
            }
        }
    }

    /// Wait until `due` instead (snooze or a new date); a done TODO comes back.
    pub fn reschedule(&mut self, due: Millis) {
        self.due = due;
        self.status = TodoStatus::Open;
        self.completed_at = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(due: Millis, repeat: Option<u64>) -> Todo {
        Todo::new("t1".into(), " Renew the cert ", None, due, repeat, 0)
    }

    #[test]
    fn fires_once_when_due() {
        let mut t = todo(1_000, None);
        assert!(!t.fire(999));
        assert!(t.fire(1_000));
        assert_eq!(t.status, TodoStatus::Due);
        assert!(!t.fire(2_000), "already due: no second notification");
        assert_eq!(t.title, "Renew the cert");
    }

    #[test]
    fn completing_a_one_off_finishes_it() {
        let mut t = todo(1_000, None);
        t.fire(1_000);
        t.complete(5_000);
        assert_eq!((t.status, t.completed_at), (TodoStatus::Done, Some(5_000)));
    }

    #[test]
    fn a_repeating_one_skips_the_times_already_missed() {
        let mut t = todo(0, Some(10));
        t.fire(1);
        // 35 minutes later: the next time is the 40-minute mark, not 10.
        t.complete(35 * 60_000);
        assert_eq!((t.status, t.due), (TodoStatus::Open, 40 * 60_000));
        assert_eq!(t.completed_at, None);
    }

    #[test]
    fn reschedule_reopens() {
        let mut t = todo(0, None);
        t.fire(1);
        t.complete(2);
        t.reschedule(9_000);
        assert_eq!(
            (t.status, t.due, t.completed_at),
            (TodoStatus::Open, 9_000, None)
        );
    }

    #[test]
    fn input_checks() {
        assert!(check("ok", None, 1, None).is_ok());
        assert!(check("  ", None, 1, None).is_err());
        assert!(check("a\nb", None, 1, None).is_err());
        assert!(check("ok", None, 0, None).is_err());
        assert!(check("ok", None, 1, Some(1)).is_err());
        assert!(check("ok", Some(&"x".repeat(2001)), 1, None).is_err());
    }
}
