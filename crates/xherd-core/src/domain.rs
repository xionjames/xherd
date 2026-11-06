//! Flow domain types shared by the flow engine, actors and the API (design §6.3).
//! M0 only defines them; the flow engine fills them from M1 on.

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub id: String,
    pub subject: String,
    pub files: Vec<String>,
    #[serde(default)]
    pub tasks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_human: Option<String>,
}

/// An open gate. Priority when several apply: commit > PR > docs > question (R-103).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Gate {
    Docs {
        stage: String,
        role: Role,
        files: Vec<HashedFile>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next: Option<String>,
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
            files: vec!["a.rs".into()],
            tasks: vec![],
            sha: None,
            needs_human: None,
        });
        assert!(c.priority() < g.priority());
    }
}
