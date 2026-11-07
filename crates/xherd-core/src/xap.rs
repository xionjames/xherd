//! XAP wire format (xap.md §8): `POST /api/v1/xap` bodies and queued calls.
//! Validation and effects live in `xherd-flow` (the flow engine).

use serde::{Deserialize, Serialize};

use crate::agent::Millis;
use crate::domain::Role;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XapCall {
    /// Client-generated id; replays are de-duplicated by it.
    pub id: String,
    pub agent_id: String,
    #[serde(flatten)]
    pub cmd: XapCmd,
    pub at: Millis,
}

/// One XAP command with its arguments (`{"cmd": "gate.open", "args": {...}}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "cmd",
    content = "args",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum XapCmd {
    #[serde(rename = "stage.enter")]
    StageEnter {
        stage: String,
        #[serde(default)]
        role: Option<Role>,
    },
    #[serde(rename = "gate.open")]
    GateOpen {
        stage: String,
        #[serde(default)]
        role: Option<Role>,
        #[serde(default)]
        docs: Vec<String>,
        #[serde(default)]
        artifacts: Vec<String>,
        #[serde(default)]
        next: Option<String>,
        #[serde(default)]
        needs_human: Option<String>,
        #[serde(default)]
        questions: Vec<String>,
    },
    #[serde(rename = "ask")]
    Ask { text: String },
    #[serde(rename = "tasks.sync")]
    TasksSync {
        #[serde(default)]
        file: Option<String>,
    },
    #[serde(rename = "task.start")]
    TaskStart { id: String },
    #[serde(rename = "task.done")]
    TaskDone { id: String, evidence: String },
    #[serde(rename = "task.block")]
    TaskBlock { id: String, reason: String },
    #[serde(rename = "commit.propose")]
    CommitPropose {
        id: String,
        subject: String,
        /// Full commit message; xherd commits with it once approved.
        #[serde(default)]
        message: Option<String>,
        files: Vec<String>,
        #[serde(default)]
        tasks: Vec<String>,
        #[serde(default)]
        needs_human: Option<String>,
    },
    #[serde(rename = "commit.done")]
    CommitDone { id: String, sha: String },
    #[serde(rename = "commit.drop")]
    CommitDrop {
        id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    #[serde(rename = "pr.opened")]
    PrOpened { url: String },
    #[serde(rename = "handoff.written")]
    HandoffWritten {
        #[serde(default)]
        file: Option<String>,
    },
    #[serde(rename = "issue.add")]
    IssueAdd {
        title: String,
        severity: Severity,
        #[serde(default)]
        about: Vec<String>,
        #[serde(default)]
        body: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Minor,
    Major,
    Blocker,
}

impl XapCmd {
    /// Wire name, also the event kind suffix (`xap.gate.open`).
    pub fn name(&self) -> &'static str {
        match self {
            XapCmd::StageEnter { .. } => "stage.enter",
            XapCmd::GateOpen { .. } => "gate.open",
            XapCmd::Ask { .. } => "ask",
            XapCmd::TasksSync { .. } => "tasks.sync",
            XapCmd::TaskStart { .. } => "task.start",
            XapCmd::TaskDone { .. } => "task.done",
            XapCmd::TaskBlock { .. } => "task.block",
            XapCmd::CommitPropose { .. } => "commit.propose",
            XapCmd::CommitDone { .. } => "commit.done",
            XapCmd::CommitDrop { .. } => "commit.drop",
            XapCmd::PrOpened { .. } => "pr.opened",
            XapCmd::HandoffWritten { .. } => "handoff.written",
            XapCmd::IssueAdd { .. } => "issue.add",
        }
    }

    /// Queued while the daemon is down instead of failing (xap.md §1.2).
    pub fn is_report_only(&self) -> bool {
        matches!(
            self,
            XapCmd::StageEnter { .. }
                | XapCmd::TaskStart { .. }
                | XapCmd::TaskDone { .. }
                | XapCmd::TaskBlock { .. }
                | XapCmd::IssueAdd { .. }
        )
    }
}

/// A refused call: a stable code and a one-line message (xap.md §1.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

pub fn refuse(code: &'static str, message: impl Into<String>) -> Refusal {
    Refusal {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_format() {
        let v = json!({
            "id": "01J", "agentId": "a1", "cmd": "gate.open",
            "args": {"stage": "spec", "role": "spec", "docs": ["spec.md"], "needsHuman": "auth"},
            "at": 5
        });
        let c: XapCall = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(c.cmd.name(), "gate.open");
        let XapCmd::GateOpen {
            docs, needs_human, ..
        } = &c.cmd
        else {
            panic!()
        };
        assert_eq!(docs, &["spec.md"]);
        assert_eq!(needs_human.as_deref(), Some("auth"));
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["cmd"], "gate.open");
        assert_eq!(back["args"]["stage"], "spec");

        let t: XapCall = serde_json::from_value(json!({
            "id": "1", "agentId": "a", "cmd": "task.done",
            "args": {"id": "T01", "evidence": "tests pass"}, "at": 0
        }))
        .unwrap();
        assert!(t.cmd.is_report_only());
    }

    #[test]
    fn rejects_unknown_commands_and_args() {
        let bad = [
            json!({"id": "1", "agentId": "a", "cmd": "gate.close", "args": {}, "at": 0}),
            json!({"id": "1", "agentId": "a", "cmd": "ask", "args": {"text": "x", "extra": 1}, "at": 0}),
            json!({"id": "1", "agentId": "a", "cmd": "issue.add", "args": {"title": "x", "severity": "huge"}, "at": 0}),
        ];
        for b in bad {
            assert!(serde_json::from_value::<XapCall>(b.clone()).is_err(), "{b}");
        }
    }
}
