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

/// How serious a review finding is (`- severity:` in `review.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Minor,
    Major,
    Blocker,
}

impl Severity {
    /// Major and blocker findings hold the review gate (R-114).
    pub fn holds(self) -> bool {
        self >= Severity::Major
    }

    pub fn label(self) -> &'static str {
        match self {
            Severity::Minor => "minor",
            Severity::Major => "major",
            Severity::Blocker => "blocker",
        }
    }
}

/// One finding of `review.md`: a `### F01: title` heading with a field list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub id: String,
    pub title: String,
    pub severity: Severity,
    /// `src/store.rs:42`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// Tasks that fix it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixed: Vec<String>,
    /// Why it is left as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
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

/// Why a commit made outside xherd is held (shown on its row).
pub const OUTSIDE_COMMIT_HOLD: &str = "made outside xherd: no proposal or approval";

/// The branch's commits (`(sha, subject)`, oldest first) that no recorded
/// commit matches, as new `done` rows `X01`, `X02`… held for a human; ids
/// continue after the highest `Xnn` in `recorded`. A recorded sha matches by
/// prefix, since `commit done` takes short shas. Files are left empty for the
/// caller to fill (R4).
pub fn outside_commits(recorded: &[Commit], branch: &[(String, String)]) -> Vec<Commit> {
    let known: Vec<String> = recorded
        .iter()
        .filter_map(|c| c.sha.as_deref())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s.len() >= 4)
        .collect();
    let mut next = recorded
        .iter()
        .filter_map(|c| c.id.strip_prefix('X')?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    branch
        .iter()
        .filter(|(sha, _)| {
            let sha = sha.to_ascii_lowercase();
            !known.iter().any(|k| sha.starts_with(k.as_str()))
        })
        .map(|(sha, subject)| {
            next += 1;
            Commit {
                id: format!("X{next:02}"),
                subject: subject.clone(),
                message: None,
                files: vec![],
                status: CommitStatus::Done,
                tasks: vec![],
                sha: Some(sha.clone()),
                needs_human: Some(OUTSIDE_COMMIT_HOLD.into()),
            }
        })
        .collect()
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
        /// Open questions read from the documents' `## Open questions`
        /// sections (R-105a), in document order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        questions: Vec<String>,
        /// Questions that are in no document: `--question` and `xherd ask`
        /// while the gate is open.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        asked: Vec<String>,
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

    /// Short name of the gate for notices: the stage id, `commit C01`,
    /// `a question` or `PR comments`. Never carries question or document text.
    pub fn label(&self) -> String {
        match self {
            Gate::Docs { stage, .. } => stage.clone(),
            Gate::Commit(c) => format!("commit {}", c.id),
            Gate::PrTriage { .. } | Gate::PrReplies { .. } => "PR comments".into(),
            Gate::Question { .. } => "a question".into(),
        }
    }

    /// Why the gate is held for a human: its `needs_human`, else for a docs
    /// gate with open questions a fixed text (never the questions themselves).
    pub fn hold(&self) -> Option<String> {
        match self {
            Gate::Docs {
                needs_human,
                questions,
                asked,
                ..
            } => needs_human.clone().or_else(|| {
                (!questions.is_empty() || !asked.is_empty())
                    .then(|| "the document has open questions".into())
            }),
            Gate::Commit(c) => c.needs_human.clone(),
            Gate::PrTriage { .. } | Gate::PrReplies { .. } | Gate::Question { .. } => None,
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

    fn docs(needs_human: Option<&str>, questions: &[&str], asked: &[&str]) -> Gate {
        Gate::Docs {
            stage: "spec".into(),
            role: Role::Spec,
            files: vec![],
            next: None,
            needs_human: needs_human.map(Into::into),
            questions: questions.iter().map(|s| s.to_string()).collect(),
            asked: asked.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn commit(needs_human: Option<&str>) -> Gate {
        Gate::Commit(Commit {
            id: "C01".into(),
            subject: "s".into(),
            message: None,
            files: vec![],
            status: CommitStatus::Proposed,
            tasks: vec![],
            sha: None,
            needs_human: needs_human.map(Into::into),
        })
    }

    fn done(id: &str, sha: &str) -> Commit {
        Commit {
            id: id.into(),
            subject: "s".into(),
            message: None,
            files: vec![],
            status: CommitStatus::Done,
            tasks: vec![],
            sha: Some(sha.into()),
            needs_human: None,
        }
    }

    fn branch(shas: &[&str]) -> Vec<(String, String)> {
        shas.iter()
            .map(|s| (s.to_string(), format!("subject {s}")))
            .collect()
    }

    #[test]
    fn outside_commits_skips_recorded_ones_by_prefix() {
        let recorded = [done("C01", "aaaa111"), done("M01", "BBBB2222")];
        let got = outside_commits(
            &recorded,
            &branch(&["aaaa1112222", "bbbb22223333", "cccc3333"]),
        );
        assert_eq!(got.len(), 1);
        let c = &got[0];
        assert_eq!(
            (
                c.id.as_str(),
                c.sha.as_deref(),
                c.subject.as_str(),
                c.status
            ),
            (
                "X01",
                Some("cccc3333"),
                "subject cccc3333",
                CommitStatus::Done
            )
        );
        assert!(c.tasks.is_empty() && c.files.is_empty());
        assert_eq!(c.needs_human.as_deref(), Some(OUTSIDE_COMMIT_HOLD));
    }

    #[test]
    fn outside_commits_numbers_after_the_last_x() {
        let recorded = [done("X01", "aaaa1111"), done("X03", "bbbb2222")];
        let got = outside_commits(&recorded, &branch(&["aaaa1111", "cccc", "dddd"]));
        let ids: Vec<&str> = got.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["X04", "X05"]);
        // Everything recorded: nothing new.
        assert!(outside_commits(&recorded, &branch(&["aaaa1111", "bbbb2222"])).is_empty());
    }

    #[test]
    fn outside_commits_ignores_proposals_without_a_sha() {
        let mut p = done("C02", "");
        p.sha = None;
        p.status = CommitStatus::Proposed;
        let got = outside_commits(&[p, done("C01", "")], &branch(&["eeee"]));
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn gate_label() {
        assert_eq!(docs(None, &[], &[]).label(), "spec");
        assert_eq!(commit(None).label(), "commit C01");
        assert_eq!(Gate::PrTriage { count: 2 }.label(), "PR comments");
        assert_eq!(Gate::PrReplies { count: 1 }.label(), "PR comments");
        let q = Gate::Question {
            text: "secret question?".into(),
        };
        assert_eq!(q.label(), "a question");
    }

    #[test]
    fn gate_hold() {
        assert_eq!(docs(None, &[], &[]).hold(), None);
        assert_eq!(
            docs(Some("touches auth"), &["which db?"], &[])
                .hold()
                .as_deref(),
            Some("touches auth")
        );
        let open = Some("the document has open questions");
        assert_eq!(docs(None, &["which db?"], &[]).hold().as_deref(), open);
        assert_eq!(docs(None, &[], &["which db?"]).hold().as_deref(), open);
        assert!(
            !docs(None, &["which db?"], &[])
                .hold()
                .unwrap()
                .contains("db")
        );
        assert_eq!(commit(None).hold(), None);
        assert_eq!(commit(Some("secret")).hold().as_deref(), Some("secret"));
        assert_eq!(Gate::PrTriage { count: 1 }.hold(), None);
        assert_eq!(Gate::Question { text: "q".into() }.hold(), None);
    }
}
