//! Flow domain types shared by the flow engine, actors and the API (design §6.3).

use serde::{Deserialize, Serialize};

/// Canonical step roles shared by all packs (spec §8.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    Explore,
    Spec,
    Design,
    Tasks,
    Implement,
    Review,
    Ship,
    Pr,
    Custom,
}

/// Autonomy class of a stage (spec §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    Critical,
    Routine,
    Code,
    HumanOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StageStatus {
    Pending,
    Active,
    Gated,
    Approved,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stage {
    pub id: String,
    pub role: Role,
    pub status: StageStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HashedFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskStatus {
    Todo,
    Doing,
    Done,
    Blocked,
}

/// Where a task came from (`- source:` in `tasks.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskSource {
    #[default]
    Spec,
    /// A review finding that must be fixed (the review→fix loop, R-120).
    Review,
    Pr,
}

impl TaskSource {
    pub fn is_spec(&self) -> bool {
        *self == TaskSource::Spec
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub depends: Vec<String>,
    /// Binding file list (scope fence, R-116).
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "TaskSource::is_spec")]
    pub source: TaskSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommitStatus {
    /// Waiting at the commit gate.
    #[default]
    Proposed,
    /// Approved; the agent may commit it.
    Approved,
    Done,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub id: String,
    pub subject: String,
    /// The full commit message the agent proposes (`git commit -m`); the
    /// user may edit it at approval. Empty: the subject is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub files: Vec<String>,
    #[serde(default)]
    pub status: CommitStatus,
    #[serde(default)]
    pub tasks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_human: Option<String>,
}

/// An open gate. Priority when several apply: commit > PR > docs > question (R-103).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum Gate {
    Docs {
        stage: String,
        role: Role,
        files: Vec<HashedFile>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
        /// Tripwire set by the agent (`--needs-human`, R-112).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        needs_human: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        questions: Vec<String>,
    },
    Commit(Commit),
    PrTriage {
        count: u32,
    },
    PrReplies {
        count: u32,
    },
    Question {
        text: String,
    },
}

impl Gate {
    /// Lower is more urgent (R-103).
    pub fn priority(&self) -> u8 {
        match self {
            Gate::Commit(_) => 0,
            Gate::PrTriage { .. } | Gate::PrReplies { .. } => 1,
            Gate::Docs { .. } => 2,
            Gate::Question { .. } => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_json_shape() {
        let g = Gate::Question {
            text: "which db?".into(),
        };
        assert_eq!(
            serde_json::to_value(&g).unwrap(),
            serde_json::json!({"kind": "question", "text": "which db?"})
        );
        let c = Gate::Commit(Commit {
            id: "C1".into(),
            subject: "add store".into(),
            message: None,
            files: vec!["a.rs".into()],
            status: CommitStatus::Proposed,
            tasks: vec![],
            sha: None,
            needs_human: None,
        });
        assert!(c.priority() < g.priority());
    }
}
