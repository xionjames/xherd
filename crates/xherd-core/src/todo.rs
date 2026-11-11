//! TODOs (spec §16.2): reminders due at a time, once or repeating, and
//! condition checks. The lifecycle is pure:
//!
//! - a reminder: `open` (waiting) → `due` (needs you) → `done`, or back to
//!   `open` when it repeats or is rescheduled;
//! - a condition check: at each due time `open` → `checking` (a short read-only
//!   runtime run, outside this module) → met: `due`; not met: `open` again
//!   after `repeat_minutes`; unclear (no answer in the format asked): `due`,
//!   the user looks; failed: `error`.

use serde::{Deserialize, Serialize};

use crate::agent::Millis;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TodoStatus {
    /// Waiting for its due time.
    Open,
    /// A check is running.
    Checking,
    /// The time has come, or the condition is met: it needs you.
    Due,
    Done,
    /// The check could not run or failed.
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckOutcome {
    Met,
    NotMet,
    /// The run ended without the answer line, or timed out.
    Unclear,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastCheck {
    pub at: Millis,
    pub outcome: CheckOutcome,
    /// What the check said (model output: shown, never put in notifications).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub due: Millis,
    /// Minutes between repeats, or between checks of a condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_minutes: Option<u64>,
    /// What must be true, in words; the check run decides whether it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Model for the check run; the runtime's default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub status: TodoStatus,
    pub created_at: Millis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<Millis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checking_since: Option<Millis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<LastCheck>,
}

pub const MIN_REPEAT_MINUTES: u64 = 5;
pub const MAX_TITLE: usize = 200;
pub const MAX_NOTE: usize = 2000;
pub const MAX_CONDITION: usize = 1000;
/// A check that has run this long is taken as lost (the daemon restarted).
pub const CHECK_STALE_MS: Millis = 10 * 60_000;

/// What the user may set; checked before a TODO is stored.
pub struct Input<'a> {
    pub title: &'a str,
    pub note: Option<&'a str>,
    pub due: Millis,
    pub repeat_minutes: Option<u64>,
    pub condition: Option<&'a str>,
    pub model: Option<&'a str>,
}

pub fn check(i: &Input) -> Result<(), String> {
    let t = i.title.trim();
    if t.is_empty() || t.chars().count() > MAX_TITLE || t.chars().any(char::is_control) {
        return Err(format!(
            "the title is one line of 1 to {MAX_TITLE} characters"
        ));
    }
    if i.note.is_some_and(|n| n.chars().count() > MAX_NOTE) {
        return Err(format!("the note is at most {MAX_NOTE} characters"));
    }
    if i.due <= 0 {
        return Err("say when it is due".into());
    }
    if i.repeat_minutes.is_some_and(|r| r < MIN_REPEAT_MINUTES) {
        return Err(format!("repeat at most every {MIN_REPEAT_MINUTES} minutes"));
    }
    let cond = i.condition.map(str::trim).filter(|c| !c.is_empty());
    if let Some(c) = cond {
        if c.chars().count() > MAX_CONDITION {
            return Err(format!(
                "the condition is at most {MAX_CONDITION} characters"
            ));
        }
        if i.repeat_minutes.is_none() {
            return Err("a condition needs how often to check it".into());
        }
    }
    if let Some(m) = i.model.map(str::trim).filter(|m| !m.is_empty()) {
        // Goes into a command line: plain model names only.
        let plain = m.len() <= 100
            && !m.starts_with('-')
            && m.chars()
                .all(|c| c.is_ascii_alphanumeric() || "._:/-[]".contains(c));
        if !plain {
            return Err("the model name has letters, digits and . _ : / - [ ] only".into());
        }
        if cond.is_none() {
            return Err("a model is for checking a condition".into());
        }
    }
    Ok(())
}

/// Result of `fire`: what the daemon does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fire {
    Nothing,
    /// Notify: it needs you.
    Due,
    /// Start a check run.
    Check,
}

impl Todo {
    pub fn new(id: String, i: &Input, now: Millis) -> Todo {
        let clean = |s: Option<&str>| {
            s.map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        Todo {
            id,
            title: i.title.trim().to_string(),
            note: clean(i.note),
            due: i.due,
            repeat_minutes: i.repeat_minutes,
            condition: clean(i.condition),
            model: clean(i.model),
            status: TodoStatus::Open,
            created_at: now,
            completed_at: None,
            checking_since: None,
            last_check: None,
        }
    }

    /// Replace what the user set; a new time re-opens it.
    pub fn edit(&mut self, i: &Input) {
        let clean = |s: Option<&str>| {
            s.map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        self.title = i.title.trim().to_string();
        self.note = clean(i.note);
        self.repeat_minutes = i.repeat_minutes;
        self.condition = clean(i.condition);
        self.model = clean(i.model);
        if i.due != self.due {
            self.reschedule(i.due);
        }
    }

    /// Its time has come.
    pub fn fire(&mut self, now: Millis) -> Fire {
        if self.status != TodoStatus::Open || self.due > now {
            return Fire::Nothing;
        }
        if self.condition.is_some() {
            self.status = TodoStatus::Checking;
            self.checking_since = Some(now);
            Fire::Check
        } else {
            self.status = TodoStatus::Due;
            Fire::Due
        }
    }

    /// "Check now": only a condition that is not being checked or done.
    pub fn start_check(&mut self, now: Millis) -> bool {
        if self.condition.is_none()
            || matches!(self.status, TodoStatus::Checking | TodoStatus::Done)
        {
            return false;
        }
        self.status = TodoStatus::Checking;
        self.checking_since = Some(now);
        true
    }

    /// The check run ended. Returns whether it needs the user now.
    pub fn finish_check(
        &mut self,
        now: Millis,
        outcome: CheckOutcome,
        summary: Option<String>,
    ) -> bool {
        if self.status != TodoStatus::Checking {
            return false;
        }
        self.checking_since = None;
        self.last_check = Some(LastCheck {
            at: now,
            outcome,
            summary,
        });
        match outcome {
            CheckOutcome::Met | CheckOutcome::Unclear => {
                self.status = TodoStatus::Due;
                true
            }
            CheckOutcome::NotMet => {
                self.due = self.next_time(now);
                self.status = TodoStatus::Open;
                false
            }
            CheckOutcome::Failed => {
                self.status = TodoStatus::Error;
                true
            }
        }
    }

    /// A check that never reported back (the daemon restarted): an error.
    pub fn expire_check(&mut self, now: Millis) -> bool {
        let lost = self.status == TodoStatus::Checking
            && self.checking_since.is_none_or(|s| now - s > CHECK_STALE_MS);
        if lost {
            self.finish_check(
                now,
                CheckOutcome::Failed,
                Some("the check did not finish".into()),
            );
        }
        lost
    }

    /// The first repeat time after `now`.
    fn next_time(&self, now: Millis) -> Millis {
        let step = self.repeat_minutes.unwrap_or(MIN_REPEAT_MINUTES) as i64 * 60_000;
        self.due + (((now - self.due).max(0) / step) + 1) * step
    }

    /// Complete: a repeating reminder waits for its next time, everything
    /// else (a one-off, a condition that was met) is done.
    pub fn complete(&mut self, now: Millis) {
        if self.repeat_minutes.is_some() && self.condition.is_none() {
            self.due = self.next_time(now);
            self.status = TodoStatus::Open;
        } else {
            self.status = TodoStatus::Done;
            self.completed_at = Some(now);
        }
    }

    /// Wait until `due` instead (snooze or a new date); a done TODO comes back.
    pub fn reschedule(&mut self, due: Millis) {
        self.due = due;
        self.status = TodoStatus::Open;
        self.completed_at = None;
        self.checking_since = None;
    }
}

/// The answer a check run must end with: a last line
/// `XHERD_RESULT {"met": true, "summary": "…"}`. `None` when there is none.
pub fn parse_result(output: &str) -> Option<(bool, Option<String>)> {
    let line = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("XHERD_RESULT"))?;
    let v: serde_json::Value =
        serde_json::from_str(line.trim_start_matches("XHERD_RESULT").trim()).ok()?;
    let met = v.get("met")?.as_bool()?;
    let summary = v
        .get("summary")
        .and_then(|s| s.as_str())
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control())
                .take(300)
                .collect::<String>()
        })
        .filter(|s| !s.trim().is_empty());
    Some((met, summary))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input<'a>(due: Millis, repeat: Option<u64>, condition: Option<&'a str>) -> Input<'a> {
        Input {
            title: " Renew the cert ",
            note: None,
            due,
            repeat_minutes: repeat,
            condition,
            model: None,
        }
    }
    fn todo(due: Millis, repeat: Option<u64>) -> Todo {
        Todo::new("t1".into(), &input(due, repeat, None), 0)
    }
    fn cond(due: Millis) -> Todo {
        Todo::new(
            "c1".into(),
            &input(due, Some(10), Some("the PR is merged")),
            0,
        )
    }

    #[test]
    fn fires_once_when_due() {
        let mut t = todo(1_000, None);
        assert_eq!(t.fire(999), Fire::Nothing);
        assert_eq!(t.fire(1_000), Fire::Due);
        assert_eq!(t.status, TodoStatus::Due);
        assert_eq!(
            t.fire(2_000),
            Fire::Nothing,
            "already due: no second notification"
        );
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
    fn a_condition_is_checked_at_its_time() {
        let mut t = cond(1_000);
        assert_eq!(t.fire(1_000), Fire::Check);
        assert_eq!(
            (t.status, t.checking_since),
            (TodoStatus::Checking, Some(1_000))
        );
        assert_eq!(t.fire(2_000), Fire::Nothing, "never two checks at once");
        // Not met: rescheduled to the next time after now.
        assert!(!t.finish_check(61_000, CheckOutcome::NotMet, Some("open".into())));
        assert_eq!((t.status, t.due), (TodoStatus::Open, 1_000 + 10 * 60_000));
        // Met: needs you; completing finishes it (it does not repeat).
        t.fire(t.due);
        assert!(t.finish_check(t.due + 5, CheckOutcome::Met, None));
        assert_eq!(t.status, TodoStatus::Due);
        t.complete(t.due + 10);
        assert_eq!(t.status, TodoStatus::Done);
    }

    #[test]
    fn unclear_needs_you_and_failure_is_an_error() {
        let mut t = cond(0);
        t.fire(1);
        assert!(t.finish_check(2, CheckOutcome::Unclear, None));
        assert_eq!(t.status, TodoStatus::Due);
        assert!(t.start_check(3), "check now works from due");
        assert!(t.finish_check(4, CheckOutcome::Failed, Some("no login".into())));
        assert_eq!(t.status, TodoStatus::Error);
        assert!(t.start_check(5), "and again from error");
        assert!(!t.start_check(6), "not while checking");
        assert!(
            !todo(0, None).start_check(1),
            "a reminder has nothing to check"
        );
    }

    #[test]
    fn a_lost_check_becomes_an_error() {
        let mut t = cond(0);
        t.fire(1);
        assert!(!t.expire_check(60_000));
        assert!(t.expire_check(1 + CHECK_STALE_MS + 1));
        assert_eq!(t.status, TodoStatus::Error);
    }

    #[test]
    fn input_checks() {
        let ok = |i: Input| check(&i).is_ok();
        assert!(ok(input(1, None, None)));
        assert!(ok(input(1, Some(10), Some("x"))));
        assert!(!ok(Input {
            title: "  ",
            ..input(1, None, None)
        }));
        assert!(!ok(Input {
            title: "a\nb",
            ..input(1, None, None)
        }));
        assert!(!ok(input(0, None, None)));
        assert!(!ok(input(1, Some(1), None)));
        assert!(
            !ok(input(1, None, Some("x"))),
            "a condition needs an interval"
        );
        assert!(!ok(Input {
            note: Some(&"x".repeat(2001)),
            ..input(1, None, None)
        }));
        assert!(ok(Input {
            model: Some("claude-sonnet-5-5"),
            ..input(1, Some(10), Some("x"))
        }));
        assert!(!ok(Input {
            model: Some("--dangerous"),
            ..input(1, Some(10), Some("x"))
        }));
        assert!(!ok(Input {
            model: Some("a b"),
            ..input(1, Some(10), Some("x"))
        }));
        assert!(!ok(Input {
            model: Some("m"),
            ..input(1, None, None)
        }));
    }

    #[test]
    fn the_answer_line_is_the_last_one() {
        let out = "thinking\nXHERD_RESULT {\"met\": false, \"summary\": \"old\"}\nmore\nXHERD_RESULT {\"met\": true, \"summary\": \"merged\"}\n";
        assert_eq!(parse_result(out), Some((true, Some("merged".into()))));
        assert_eq!(
            parse_result("XHERD_RESULT {\"met\": false}"),
            Some((false, None))
        );
        assert_eq!(parse_result("no answer here"), None);
        assert_eq!(parse_result("XHERD_RESULT not json"), None);
        assert_eq!(parse_result("XHERD_RESULT {\"met\": \"yes\"}"), None);
    }
}
