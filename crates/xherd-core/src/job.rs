//! The agent record owned by its actor.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::autonomy::Autonomy;
use crate::domain::{Commit, Gate, Task};
use crate::status::{Status, Trigger, next_status};

/// Milliseconds since the Unix epoch.
pub type Millis = i64;

fn is_false(b: &bool) -> bool {
    !*b
}

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
    /// The saved agent whose memory this proposes to change (D26: a job may
    /// have several). Empty in proposals made before stage agents: the job's
    /// current agent then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

/// Why and when xherd offers to save a job's setup as an agent (R30). Set
/// once; `dismissed` is remembered for the job (R32).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    /// `done`, `marked-done` or `repeated` (the third job with this setup).
    pub reason: String,
    pub at: Millis,
    #[serde(default)]
    pub dismissed: bool,
}

/// What a job without an agent proposes to save as one (R31): written by
/// `xherd agent propose`, read when the user accepts the suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraft {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub instructions: String,
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

/// A session the user started with Ask (#121): held open while the
/// agent only answers, until flow work, an approval, a change request,
/// Finish, or the end of the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskHold {
    pub at: Millis,
    /// The agent was `watching` its PR before the Ask: Finish returns it there.
    #[serde(default)]
    pub was_watching: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
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
    /// A pending update of the saved agent's memory (R29), until the user
    /// accepts or dismisses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_memory_proposal: Option<MemoryProposal>,
    /// Offer to save this job's setup as an agent (R30); never for a job that
    /// has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<Suggestion>,
    /// What the job proposes for that agent (R31).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_draft: Option<AgentDraft>,
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
    /// The branch the pull request merges into, as the host last said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr_base: Option<String>,
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
    /// The saved agent this job started from, as it was then (#141): editing
    /// the agent never changes a job that exists.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, crate::agent_def::JobAgent>,
    /// The agent each stage of the workflow runs with, resolved at creation
    /// (D26, R37); a stage with none keeps the one before.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stage_agents: BTreeMap<String, crate::agent_def::StageAgent>,
    /// The workflow stages whose documents get a fresh-context check before
    /// their gate opens, copied at creation like `stage_agents`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub fresh_checks: BTreeSet<String>,
    /// The agent for stages with no assignment (`--agent A`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    /// The agent the running session uses (D26): a key of `agents`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_agent: Option<String>,
    /// An agent attached to a job that was already running (R32): stage
    /// assignments never switch it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub pinned: bool,
    /// Jobs stored before stage agents had one `agent`; `upgrade` moves it.
    #[serde(default, rename = "agent", skip_serializing)]
    legacy_agent: Option<crate::agent_def::JobAgent>,
    /// Values a profile's ceiling lowered at creation or since (R12a).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lowered: Vec<crate::resolve::Lowered>,
    /// Gates approved automatically since a human last approved one (R-120).
    #[serde(default)]
    pub auto_in_row: u32,
    /// An Ask session held open (#121).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask_hold: Option<AskHold>,
    /// When the user removed the worktree (#123); the branch and the record
    /// stay, nothing reads the folder any more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_removed_at: Option<Millis>,
    /// The open gate will be approved by xherd at the next stop (autonomy,
    /// #127). Derived by the actor on every persist; stored only because the
    /// agent JSON is what the UI reads.
    #[serde(default)]
    pub auto_approve_pending: bool,
    /// The job's actions (R1): registered by the job or copied from the
    /// repository profile (R19).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<crate::action::JobAction>,
    /// The "save these actions to the profile" card was dismissed (R23).
    #[serde(default, skip_serializing_if = "is_false")]
    pub actions_card_dismissed: bool,
    /// First port of the job's block (`XHERD_PORT_BASE`), kept so a running
    /// action keeps its ports across a daemon restart (R12a).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port_base: Option<u16>,
}

impl Job {
    /// A new agent with no ticket; its branch is its name.
    pub fn new(name: &str, now: Millis) -> Job {
        Job {
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
            agent_memory_proposal: None,
            suggestion: None,
            agent_draft: None,
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
            agents: BTreeMap::new(),
            stage_agents: BTreeMap::new(),
            fresh_checks: BTreeSet::new(),
            default_agent: None,
            current_agent: None,
            pinned: false,
            legacy_agent: None,
            lowered: Vec::new(),
            auto_in_row: 0,
            ask_hold: None,
            worktree_removed_at: None,
            auto_approve_pending: false,
            actions: Vec::new(),
            actions_card_dismissed: false,
            port_base: None,
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
            pr_base: None,
        }
    }

    /// An agent as stored before #114: the ticket key moves into `ticket`
    /// and the branch was the name. Run on every agent read from the store.
    pub fn upgrade(mut self) -> Job {
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
        if let Some(a) = self.legacy_agent.take()
            && self.agents.is_empty()
        {
            self.set_agent(a);
        }
        self
    }

    /// The saved agent the running session uses (D26), if any.
    pub fn agent(&self) -> Option<&crate::agent_def::JobAgent> {
        self.agents.get(self.current_agent.as_deref()?)
    }

    pub fn agent_mut(&mut self) -> Option<&mut crate::agent_def::JobAgent> {
        let id = self.current_agent.clone()?;
        self.agents.get_mut(&id)
    }

    /// Make `a` the job's only agent, for every stage.
    pub fn set_agent(&mut self, a: crate::agent_def::JobAgent) {
        let id = a.id.clone();
        self.agents = BTreeMap::from([(id.clone(), a)]);
        self.stage_agents.clear();
        self.default_agent = Some(id.clone());
        self.current_agent = Some(id);
    }

    /// Whether the job was started with, or attached to, agent `id`.
    pub fn uses_agent(&self, id: &str) -> bool {
        self.agents.contains_key(id)
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
    fn a_job_without_actions_omits_them_and_reads_back() {
        let j = Job::new("j", 1);
        let v = serde_json::to_value(&j).unwrap();
        for k in ["actions", "actionsCardDismissed", "portBase"] {
            assert!(v.get(k).is_none(), "{k}");
        }
        let mut j2 = j.clone();
        j2.port_base = Some(41010);
        j2.actions_card_dismissed = true;
        j2.actions.push(crate::action::JobAction {
            def: crate::action::ActionDef {
                id: "dev".into(),
                run: "pnpm dev".into(),
                ..Default::default()
            },
            source: Default::default(),
            approved: None,
            last: None,
            succeeded: false,
        });
        let back: Job = serde_json::from_value(serde_json::to_value(&j2).unwrap()).unwrap();
        assert_eq!(back.port_base, Some(41010));
        assert!(back.actions_card_dismissed);
        assert_eq!(back.actions[0].def.id, "dev");
        let old: Job = serde_json::from_value(v).unwrap();
        assert!(old.actions.is_empty() && old.port_base.is_none());
    }

    #[test]
    fn a_job_stored_with_one_agent_reads_as_that_agent_for_every_stage() {
        let def = crate::agent_def::JobAgent {
            id: "a1".into(),
            name: "rev".into(),
            version: 3,
            runtime: String::new(),
            fell_back: false,
            autonomy: None,
            permission_mode: None,
            model: None,
            launch: vec![],
            instructions: None,
            sensitive_paths: vec![],
            max_diff_lines: None,
            job_token_budget: None,
            skills: vec![],
            memory: None,
            source: None,
        };
        let mut v = serde_json::to_value(Job::new("j", 1)).unwrap();
        v.as_object_mut()
            .unwrap()
            .insert("agent".into(), serde_json::to_value(&def).unwrap());
        let j: Job = serde_json::from_value(v).unwrap();
        let j = j.upgrade();
        assert_eq!(j.agent().map(|a| a.version), Some(3));
        assert_eq!(j.default_agent.as_deref(), Some("a1"));
        assert!(j.stage_agents.is_empty() && j.uses_agent("a1"));
        // written back in the new shape only
        let out = serde_json::to_value(&j).unwrap();
        assert!(out.get("agent").is_none());
        assert!(out["agents"]["a1"].is_object() && out["currentAgent"] == "a1");
    }

    #[test]
    fn an_agent_stored_before_names_keeps_its_ticket_and_branch() {
        let mut v = serde_json::to_value(Job::new("SC-1-login", 1)).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("branch");
        o.insert("ticketKey".into(), "SC-1".into());
        let a: Job = serde_json::from_value(v).unwrap();
        let a = a.upgrade();
        assert_eq!(a.branch, "SC-1-login");
        assert_eq!(a.ticket_key(), Some("SC-1"));
        assert_eq!((a.ticket_ref(), a.work_item()), ("SC-1", "SC-1"));
        let out = serde_json::to_value(&a).unwrap();
        assert!(out.get("ticketKey").is_none());
        assert_eq!(out["ticket"], serde_json::json!({"key": "SC-1"}));

        let plain = Job::new("refund-retry", 1).upgrade();
        assert_eq!(
            (plain.ticket_ref(), plain.work_item()),
            ("", "refund-retry")
        );
    }

    #[test]
    fn ask_hold_and_worktree_fields_are_camel_case_and_optional() {
        let mut v = serde_json::to_value(Job::new("x", 1)).unwrap();
        let o = v.as_object_mut().unwrap();
        assert_eq!(o.get("autoApprovePending"), Some(&false.into()));
        assert!(o.get("askHold").is_none() && o.get("worktreeRemovedAt").is_none());
        o.remove("autoApprovePending");
        let old: Job = serde_json::from_value(v).unwrap();
        assert!(old.ask_hold.is_none() && !old.auto_approve_pending);

        let mut a = Job::new("x", 1);
        a.ask_hold = Some(AskHold {
            at: 7,
            was_watching: true,
        });
        a.worktree_removed_at = Some(9);
        let out = serde_json::to_value(&a).unwrap();
        assert_eq!(
            out["askHold"],
            serde_json::json!({"at": 7, "wasWatching": true})
        );
        assert_eq!(out["worktreeRemovedAt"], 9);
        assert_eq!(serde_json::from_value::<Job>(out).unwrap(), a);
    }
}
