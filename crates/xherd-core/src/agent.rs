//! The agent record owned by its actor.

use serde::{Deserialize, Serialize};

use crate::autonomy::Autonomy;
use crate::domain::{Commit, Gate, Task};
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

/// `xherd memory propose` (spec §16.6): the proposed `AGENTS.md` is the
/// worktree file `file` (`./…`), read again when shown and when accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryProposal {
    pub file: String,
    pub reason: String,
    pub at: Millis,
}

/// An agent's ticket (#114): what the user gave, a key (`PAY-412`, `#12`)
/// or an issue URL, with the key found from the URL by a tracker plugin.
/// A URL no tracker reads keeps no key and is only shown as a link.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ticket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// An https URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: String,
    /// A lowercase slug (#114): the tmux session, the worktree folder and
    /// what the UI shows. Unique.
    pub name: String,
    /// The ticket the agent works on, if any: a key, a URL, or both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<Ticket>,
    /// The git branch: `<KEY>-<name>` with a ticket key, else the name
    /// (`new::branch_name`). Agents from before the field use their name.
    #[serde(default)]
    pub branch: String,
    /// Before #114 the ticket key was a field of its own; `upgrade` moves it.
    #[serde(default, rename = "ticketKey", skip_serializing)]
    legacy_ticket_key: Option<String>,
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
    /// Repository root the worktree was created from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// Base the branch was created from (resolved ref, e.g. `origin/main`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Repo profile matched at creation (R-002); unset or deleted = the
    /// global settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Runtime plugin name (`claude-code` when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Command to start the next session with (approval of a parked agent, R-028).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_prompt: Option<String>,
    /// Flow pack and flow (`None` = flow `none`, R-033).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    /// Current stage id, from `stage enter` / `gate open` (XAP).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// Stages the agent has been in, in order; earlier stages missing here
    /// were skipped (R-213: stage status `skipped`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visited_stages: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<Gate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tasks: Vec<Task>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<Commit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_url: Option<String>,
    /// When the agent last confirmed a handoff note (`handoff written`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_at: Option<Millis>,
    /// A pending update of the repo's shared memory, `AGENTS.md` (spec
    /// §16.6), until the user accepts or dismisses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_proposal: Option<MemoryProposal>,
    /// A pause is waiting for the agent's handoff note (R-026); cleared when
    /// the session ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_requested_at: Option<Millis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    /// The current `blocked` status came from a screen rule, not a real report.
    #[serde(default)]
    pub screen_blocked: bool,
    /// Why the agent is blocked when the daemon, not the agent, decided so
    /// (R-688: `git access`). Cleared when the agent leaves `blocked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
    #[serde(default)]
    pub idle_alerted: bool,
    /// When the current runtime session started (R-1206).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_since: Option<Millis>,
    /// Tokens of earlier sessions, and of the current one (`tokens_session_id`),
    /// read from the runtime's transcript (R-1205, `usage`).
    #[serde(default)]
    pub tokens_base: u64,
    #[serde(default)]
    pub tokens_session: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_session_id: Option<String>,
    /// What the last poll of the pull request found (R-303): its state, the
    /// rolled-up checks, whether failing checks and a merge conflict were
    /// already announced, and when it was polled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_state: Option<crate::pr::PrState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_ci: Option<crate::pr::Ci>,
    #[serde(default)]
    pub pr_ci_failed: bool,
    #[serde(default)]
    pub pr_conflict: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_polled_at: Option<Millis>,
    /// The last poll's check runs, reviewers and merge state, for the UI.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pr_checks: Vec<crate::pr::CheckRun>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pr_reviewers: Vec<crate::pr::Reviewer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_mergeable: Option<crate::pr::Mergeable>,
    /// Ids of the PR comments already handled or ignored (R-304).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pr_comments_seen: Vec<String>,
    /// Times the agent came back to review after fixing (R-120).
    #[serde(default)]
    pub review_rounds: u32,
    /// The 80 % budget warning was sent.
    #[serde(default)]
    pub budget_warned: bool,
    /// Which gates xherd may approve by itself (R-109).
    #[serde(default)]
    pub autonomy: Autonomy,
    /// Tool-permission mode (R-002a); `None` for agents saved before it
    /// existed, which keep their stored `launch`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<crate::permission::PermissionMode>,
    /// Model the runtime is launched with (issue #112); `None` = the
    /// runtime's own default. `launch` already carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Gates approved automatically since a human last approved one (R-120).
    #[serde(default)]
    pub auto_in_row: u32,
}

impl Agent {
    /// A new agent with no ticket; its branch is its name.
    pub fn new(name: &str, now: Millis) -> Agent {
        Agent {
            id: new_id(),
            name: name.to_string(),
            ticket: None,
            branch: name.to_string(),
            legacy_ticket_key: None,
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
            repo: None,
            base: None,
            profile: None,
            runtime: None,
            next_prompt: None,
            pack: None,
            flow: None,
            stage: None,
            visited_stages: Vec::new(),
            gate: None,
            tasks: Vec::new(),
            commits: Vec::new(),
            pr_url: None,
            handoff_at: None,
            memory_proposal: None,
            pause_requested_at: None,
            last_message: None,
            screen_blocked: false,
            block_reason: None,
            idle_alerted: false,
            session_since: None,
            tokens_base: 0,
            tokens_session: 0,
            tokens_session_id: None,
            budget_warned: false,
            autonomy: Autonomy::Ask,
            permission_mode: None,
            model: None,
            auto_in_row: 0,
            review_rounds: 0,
            pr_state: None,
            pr_comments_seen: Vec::new(),
            pr_ci: None,
            pr_ci_failed: false,
            pr_conflict: false,
            pr_polled_at: None,
            pr_checks: Vec::new(),
            pr_reviewers: Vec::new(),
            pr_mergeable: None,
        }
    }

    /// An agent as stored before #114: the ticket key moves into `ticket`
    /// and the branch was the name. Run on every agent read from the store.
    pub fn upgrade(mut self) -> Agent {
        if let Some(k) = self.legacy_ticket_key.take()
            && self.ticket.is_none()
            && !k.is_empty()
        {
            self.ticket = Some(Ticket {
                key: Some(k),
                url: None,
            });
        }
        if self.branch.is_empty() {
            self.branch = self.name.clone();
        }
        self
    }

    /// The ticket's short id (`PAY-412`, `#12`), if it has one.
    pub fn ticket_key(&self) -> Option<&str> {
        self.ticket.as_ref()?.key.as_deref()
    }

    /// The ticket as handed to the agent's first command: its URL, else its
    /// key, else empty.
    pub fn ticket_ref(&self) -> &str {
        self.ticket
            .as_ref()
            .and_then(|t| t.url.as_deref().or(t.key.as_deref()))
            .unwrap_or("")
    }

    /// What time is booked to (spec §16.4): the ticket key, else the name.
    pub fn work_item(&self) -> &str {
        self.ticket_key().unwrap_or(&self.name)
    }

    /// Add the current stage to `visited_stages` if it is new.
    pub fn note_stage(&mut self) {
        if let Some(s) = &self.stage
            && !self.visited_stages.contains(s)
        {
            self.visited_stages.push(s.clone());
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
                self.block_reason = None;
            }
        }
        Some(from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_stored_before_names_keeps_its_ticket_and_branch() {
        let mut v = serde_json::to_value(Agent::new("SC-1-login", 1)).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("branch");
        o.insert("ticketKey".into(), "SC-1".into());
        let a: Agent = serde_json::from_value(v).unwrap();
        let a = a.upgrade();
        assert_eq!(a.branch, "SC-1-login");
        assert_eq!(a.ticket_key(), Some("SC-1"));
        assert_eq!((a.ticket_ref(), a.work_item()), ("SC-1", "SC-1"));
        let out = serde_json::to_value(&a).unwrap();
        assert!(out.get("ticketKey").is_none());
        assert_eq!(out["ticket"], serde_json::json!({"key": "SC-1"}));

        let plain = Agent::new("refund-retry", 1).upgrade();
        assert_eq!(
            (plain.ticket_ref(), plain.work_item()),
            ("", "refund-retry")
        );
    }
}
