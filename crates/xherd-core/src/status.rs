//! Agent statuses and the lifecycle table (spec §5.3).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Queued,
    Running,
    WaitingInput,
    Blocked,
    Idle,
    Parked,
    Paused,
    Watching,
    Done,
    Cancelled,
    Error,
}

impl Status {
    pub const ALL: [Status; 11] = [
        Status::Queued,
        Status::Running,
        Status::WaitingInput,
        Status::Blocked,
        Status::Idle,
        Status::Parked,
        Status::Paused,
        Status::Watching,
        Status::Done,
        Status::Cancelled,
        Status::Error,
    ];

    /// Live statuses count against the concurrency cap (R-006) and have a session.
    pub fn is_live(self) -> bool {
        matches!(
            self,
            Status::Running | Status::WaitingInput | Status::Blocked | Status::Idle
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Queued => "queued",
            Status::Running => "running",
            Status::WaitingInput => "waiting-input",
            Status::Blocked => "blocked",
            Status::Idle => "idle",
            Status::Parked => "parked",
            Status::Paused => "paused",
            Status::Watching => "watching",
            Status::Done => "done",
            Status::Cancelled => "cancelled",
            Status::Error => "error",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|st| st.as_str() == s)
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a status changes. One variant per row group of the lifecycle table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Trigger {
    /// A concurrency slot became free and the session was started.
    SlotFree,
    /// The agent stopped and nothing waits on the user.
    StoppedIdle,
    /// The agent stopped at a gate or asked a question.
    StoppedAtGate,
    /// A dialog is on screen or the agent reported `blocked`.
    Blocked,
    /// Work resumed (dialog gone, new prompt, `working` report).
    Working,
    /// User message, approval or change request typed into the session.
    UserInput,
    /// Waited at a gate longer than `gateParkMinutes`.
    Park,
    /// Approval or message for a parked agent: it needs a slot again.
    Unpark,
    /// User paused (handoff written, session ended).
    Pause,
    /// User resumed a paused, cancelled or errored agent.
    Resume,
    /// PR open and nothing left to do.
    Watch,
    /// New PR review comments.
    PrComments,
    /// PR merged or closed.
    PrFinished,
    /// The terminal session disappeared.
    SessionLost,
    /// User cancelled.
    Cancel,
}

/// The lifecycle table. `None` means the trigger does not apply in that status.
pub fn next_status(from: Status, trigger: Trigger) -> Option<Status> {
    use Status as S;
    use Trigger as T;
    match (trigger, from) {
        (T::SlotFree, S::Queued) => Some(S::Running),
        (T::StoppedIdle, S::Running | S::Blocked) => Some(S::Idle),
        (T::StoppedAtGate, S::Running | S::Blocked | S::Idle) => Some(S::WaitingInput),
        (T::Blocked, S::Running | S::Idle | S::WaitingInput) => Some(S::Blocked),
        (T::Working, S::Blocked | S::Idle | S::WaitingInput) => Some(S::Running),
        (T::UserInput, S::Idle | S::WaitingInput) => Some(S::Running),
        (T::Park, S::WaitingInput) => Some(S::Parked),
        (T::Unpark, S::Parked) => Some(S::Queued),
        (T::Pause, S::Running | S::Blocked | S::Idle | S::WaitingInput | S::Parked) => {
            Some(S::Paused)
        }
        (T::Resume, S::Paused | S::Cancelled | S::Error) => Some(S::Queued),
        (T::Watch, S::Idle) => Some(S::Watching),
        (T::PrComments, S::Watching) => Some(S::Queued),
        (T::PrFinished, S::Watching) => Some(S::Done),
        (T::SessionLost, s) if s.is_live() => Some(S::Error),
        (T::Cancel, s) if s.is_live() || matches!(s, S::Queued | S::Parked | S::Watching) => {
            Some(S::Cancelled)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_names() {
        for s in Status::ALL {
            assert_eq!(Status::parse(s.as_str()), Some(s));
            let json = serde_json::to_string(&s).unwrap();
            assert_eq!(json, format!("\"{}\"", s.as_str()));
        }
    }

    #[test]
    fn table_rows() {
        use Status as S;
        use Trigger as T;
        let rows = [
            (S::Queued, T::SlotFree, S::Running),
            (S::Running, T::StoppedIdle, S::Idle),
            (S::Running, T::StoppedAtGate, S::WaitingInput),
            (S::Running, T::Blocked, S::Blocked),
            (S::Blocked, T::Working, S::Running),
            (S::Idle, T::UserInput, S::Running),
            (S::WaitingInput, T::UserInput, S::Running),
            (S::WaitingInput, T::Park, S::Parked),
            (S::Parked, T::Unpark, S::Queued),
            (S::Running, T::Pause, S::Paused),
            (S::Paused, T::Resume, S::Queued),
            (S::Cancelled, T::Resume, S::Queued),
            (S::Error, T::Resume, S::Queued),
            (S::Idle, T::Watch, S::Watching),
            (S::Watching, T::PrComments, S::Queued),
            (S::Watching, T::PrFinished, S::Done),
            (S::Running, T::SessionLost, S::Error),
            (S::Blocked, T::Cancel, S::Cancelled),
        ];
        for (from, t, to) in rows {
            assert_eq!(next_status(from, t), Some(to), "{from} --{t:?}-->");
        }
    }

    #[test]
    fn terminal_statuses_stay() {
        for t in [
            Trigger::Working,
            Trigger::StoppedIdle,
            Trigger::SessionLost,
            Trigger::Cancel,
        ] {
            assert_eq!(next_status(Status::Done, t), None);
        }
        assert_eq!(next_status(Status::Paused, Trigger::Working), None);
        assert_eq!(next_status(Status::Queued, Trigger::SessionLost), None);
    }
}
