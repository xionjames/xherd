//! Pull-request follow-up (R-303, R-305, R-307, pure): what a poll of the host
//! found, and what it means for the agent. The host plugin answers with a
//! `Snapshot`; `Agent::apply_pr` keeps the last state and says what is new.

use serde::{Deserialize, Serialize};

use crate::Agent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mergeable {
    Yes,
    No,
    Unknown,
}

/// Rolled-up CI result of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Ci {
    Pending,
    Failing,
    Passing,
}

/// One check run. `bucket` is the host's roll-up of its result: `pass`,
/// `fail`, `pending`, `skipping` or `cancel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRun {
    pub name: String,
    pub bucket: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub state: PrState,
    pub mergeable: Mergeable,
    pub checks: Vec<CheckRun>,
}

/// Something new in a snapshot, to tell the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Merged or closed (R-307).
    Finished(PrState),
    /// Checks started failing: the names of the failing ones (R-305).
    CiFailing(Vec<String>),
    /// All checks pass after having failed (R-305).
    CiPassingAgain,
    /// The PR cannot merge as it is; said once until it can again (R-305).
    Conflict,
}

impl Snapshot {
    /// `None` when the PR has no checks.
    pub fn ci(&self) -> Option<Ci> {
        if self.checks.is_empty() {
            None
        } else if self.checks.iter().any(|c| c.bucket == "fail") {
            Some(Ci::Failing)
        } else if self.checks.iter().any(|c| c.bucket == "pending") {
            Some(Ci::Pending)
        } else {
            Some(Ci::Passing)
        }
    }
}

impl Agent {
    /// Take a poll's result in and return what is new.
    pub fn apply_pr(&mut self, s: &Snapshot) -> Vec<Event> {
        let mut out = Vec::new();
        self.pr_state = Some(s.state);
        if s.state != PrState::Open {
            out.push(Event::Finished(s.state));
            return out;
        }
        let ci = s.ci();
        match ci {
            Some(Ci::Failing) if self.pr_ci != Some(Ci::Failing) => {
                self.pr_ci_failed = true;
                let names = s
                    .checks
                    .iter()
                    .filter(|c| c.bucket == "fail")
                    .map(|c| c.name.clone())
                    .collect();
                out.push(Event::CiFailing(names));
            }
            Some(Ci::Passing) if self.pr_ci_failed => {
                self.pr_ci_failed = false;
                out.push(Event::CiPassingAgain);
            }
            _ => {}
        }
        self.pr_ci = ci;
        match s.mergeable {
            Mergeable::No if !self.pr_conflict => {
                self.pr_conflict = true;
                out.push(Event::Conflict);
            }
            Mergeable::Yes => self.pr_conflict = false,
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: PrState, mergeable: Mergeable, checks: &[(&str, &str)]) -> Snapshot {
        Snapshot {
            state,
            mergeable,
            checks: checks
                .iter()
                .map(|(n, b)| CheckRun {
                    name: n.to_string(),
                    bucket: b.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn ci_rolls_up_checks() {
        let ci = |c: &[(&str, &str)]| snap(PrState::Open, Mergeable::Yes, c).ci();
        assert_eq!(ci(&[]), None);
        assert_eq!(ci(&[("a", "pass"), ("b", "skipping")]), Some(Ci::Passing));
        assert_eq!(ci(&[("a", "pass"), ("b", "pending")]), Some(Ci::Pending));
        assert_eq!(ci(&[("a", "fail"), ("b", "pending")]), Some(Ci::Failing));
    }

    #[test]
    fn failing_then_passing_notifies_once_each() {
        let mut a = Agent::new("A", "A", 1);
        let pass = snap(PrState::Open, Mergeable::Yes, &[("t", "pass")]);
        assert_eq!(a.apply_pr(&pass), [], "passing from the start is not news");
        let fail = snap(
            PrState::Open,
            Mergeable::Yes,
            &[("t", "fail"), ("l", "pass")],
        );
        assert_eq!(a.apply_pr(&fail), [Event::CiFailing(vec!["t".into()])]);
        assert_eq!(a.apply_pr(&fail), [], "still failing: said once");
        let pending = snap(PrState::Open, Mergeable::Yes, &[("t", "pending")]);
        assert_eq!(a.apply_pr(&pending), []);
        assert_eq!(a.apply_pr(&pass), [Event::CiPassingAgain]);
        assert_eq!(a.apply_pr(&pass), []);
        // Fails again after a pending run: news again.
        assert_eq!(a.apply_pr(&fail).len(), 1);
    }

    #[test]
    fn conflict_is_said_once_until_it_clears() {
        let mut a = Agent::new("A", "A", 1);
        let bad = snap(PrState::Open, Mergeable::No, &[]);
        assert_eq!(a.apply_pr(&bad), [Event::Conflict]);
        assert_eq!(a.apply_pr(&bad), []);
        let unknown = snap(PrState::Open, Mergeable::Unknown, &[]);
        assert_eq!(a.apply_pr(&unknown), [], "unknown keeps the flag");
        assert_eq!(a.apply_pr(&bad), []);
        a.apply_pr(&snap(PrState::Open, Mergeable::Yes, &[]));
        assert_eq!(a.apply_pr(&bad), [Event::Conflict]);
    }

    #[test]
    fn merged_or_closed_finishes() {
        let mut a = Agent::new("A", "A", 1);
        let s = snap(PrState::Merged, Mergeable::Unknown, &[("t", "fail")]);
        assert_eq!(a.apply_pr(&s), [Event::Finished(PrState::Merged)]);
        assert_eq!(a.pr_state, Some(PrState::Merged));
    }
}
