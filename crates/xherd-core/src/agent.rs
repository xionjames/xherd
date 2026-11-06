//! The agent record owned by its actor.

use serde::{Deserialize, Serialize};

use crate::domain::Gate;
use crate::status::{Status, Trigger, next_status};

/// Milliseconds since the Unix epoch.
pub type Millis = i64;

pub fn now_ms() -> Millis {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as Millis)
        .unwrap_or(0)
}

pub fn new_id() -> String {
    ulid::Ulid::generate().to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    /// `<KEY>` or `<KEY>-<slug>`; also the tmux session and branch name.
    pub name: String,
    pub ticket_key: String,
    pub status: Status,
    pub created_at: Millis,
    pub status_since: Millis,
    pub last_activity: Millis,
    /// Highest sequence number seen from the agent's own reports (R-009).
    #[serde(default)]
    pub last_seq: u64,
    /// Highest sequence number of daemon-synthesised reports (screen rules).
    #[serde(default)]
    pub last_daemon_seq: u64,
    /// Bumped by every change relevant to a gate; stale async results are dropped.
    #[serde(default)]
    pub generation: u64,
    /// Runtime session id, as reported by hooks or `report --session`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_cmd: Option<String>,
    /// Working directory of the session (worktree or folder).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    /// argv that starts the runtime; filled from the runtime plugin (M1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<Gate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    /// The current `blocked` status came from a screen rule, not a real report.
    #[serde(default)]
    pub screen_blocked: bool,
    #[serde(default)]
    pub idle_alerted: bool,
}

impl Agent {
    pub fn new(name: &str, ticket_key: &str, now: Millis) -> Agent {
        Agent {
            id: new_id(),
            name: name.to_string(),
            ticket_key: ticket_key.to_string(),
            status: Status::Queued,
            created_at: now,
            status_since: now,
            last_activity: now,
            last_seq: 0,
            last_daemon_seq: 0,
            generation: 0,
            session: None,
            resume_cmd: None,
            workdir: None,
            launch: None,
            gate: None,
            last_message: None,
            screen_blocked: false,
            idle_alerted: false,
        }
    }

    /// Needs you = waiting-input, blocked, or an open gate (R-007).
    pub fn needs_you(&self) -> bool {
        matches!(self.status, Status::WaitingInput | Status::Blocked) || self.gate.is_some()
    }

    /// Apply a lifecycle trigger. Returns the previous status when it changed.
    pub fn fire(&mut self, trigger: Trigger, now: Millis) -> Option<Status> {
        let to = next_status(self.status, trigger)?;
        let from = self.status;
        if to != from {
            self.status = to;
            self.status_since = now;
            self.idle_alerted = false;
            self.generation += 1;
            if to != Status::Blocked {
                self.screen_blocked = false;
            }
        }
        Some(from)
    }
}
