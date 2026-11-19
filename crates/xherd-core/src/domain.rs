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

/// The subject xherd gives its shared-memory commits (R6).
pub const MEMORY_COMMIT_SUBJECT: &str = "Update AGENTS.md (shared memory)";

/// A branch commit no recorded sha matches, with the files it touched (R4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchCommit {
    pub sha: String,
    pub subject: String,
    pub files: Vec<String>,
}

/// What `settle_branch_commits` did with one unrecorded branch commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settled {
    /// Made outside xherd: a new held `Xnn` row.
    Outside { id: String, sha: String },
    /// xherd's own shared-memory commit, accepted before `Mnn` rows were
    /// kept: a new `Mnn` row.
    Memory { id: String, sha: String },
    /// A recorded commit rewritten by a rebase or an amend: its row now
    /// carries the new sha.
    Rewritten {
        id: String,
        from: String,
        sha: String,
    },
}

fn sha_key(s: &str) -> Option<String> {
    let s = s.trim().to_ascii_lowercase();
    (s.len() >= 4).then_some(s)
}

/// Whether two shas name the same commit (one may be short).
fn same_sha(a: &str, b: &str) -> bool {
    match (sha_key(a), sha_key(b)) {
        (Some(a), Some(b)) => a.starts_with(&b) || b.starts_with(&a),
        _ => false,
    }
}

fn recorded_sha(recorded: &[Commit], sha: &str) -> bool {
    recorded
        .iter()
        .filter_map(|c| c.sha.as_deref())
        .any(|k| sha_key(k).is_some_and(|k| sha.to_ascii_lowercase().starts_with(&k)))
}

/// The subject git shows for a recorded commit (`%s`): the first paragraph
/// of its message on one line, else its subject.
fn git_subject(c: &Commit) -> String {
    let message = c.message.as_deref().unwrap_or(&c.subject);
    let first = message.trim().split("\n\n").next().unwrap_or_default();
    let line = first.lines().map(str::trim).collect::<Vec<_>>().join(" ");
    if line.is_empty() {
        c.subject.trim().to_string()
    } else {
        line
    }
}

fn sorted(files: &[String]) -> Vec<&str> {
    let mut v: Vec<&str> = files.iter().map(String::as_str).collect();
    v.sort_unstable();
    v
}

fn next_id(commits: &[Commit], prefix: char) -> String {
    let n = commits
        .iter()
        .filter_map(|c| c.id.strip_prefix(prefix)?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    format!("{prefix}{:02}", n + 1)
}

/// The branch's commits (`(sha, subject)`, oldest first) that no recorded
/// commit matches. A recorded sha matches by prefix, since `commit done`
/// takes short shas (R4).
pub fn unrecorded_commits(
    recorded: &[Commit],
    branch: &[(String, String)],
) -> Vec<(String, String)> {
    branch
        .iter()
        .filter(|(sha, _)| !recorded_sha(recorded, sha))
        .cloned()
        .collect()
}

/// Record the branch's unrecorded commits in `commits`, oldest first (R4,
/// R6). `on_branch` is every sha on the branch, `memory_shas` the shas of
/// the job's accepted shared-memory commits. Each commit still unrecorded is:
/// xherd's shared-memory commit (its sha is in `memory_shas`) → a `Mnn` row;
/// else a rewritten one (a `done` row whose sha left the branch has the same
/// git subject and the same files) → that row takes the new sha; else a
/// held `Xnn` row. Running it again changes nothing.
pub fn settle_branch_commits(
    commits: &mut Vec<Commit>,
    on_branch: &[String],
    unknown: &[BranchCommit],
    memory_shas: &[String],
) -> Vec<Settled> {
    let mut out = Vec::new();
    for b in unknown {
        if recorded_sha(commits, &b.sha) {
            continue;
        }
        if memory_shas.iter().any(|m| same_sha(m, &b.sha)) {
            let id = next_id(commits, 'M');
            commits.push(Commit {
                id: id.clone(),
                subject: MEMORY_COMMIT_SUBJECT.into(),
                message: None,
                files: b.files.clone(),
                status: CommitStatus::Done,
                tasks: vec![],
                sha: Some(b.sha.clone()),
                needs_human: None,
            });
            out.push(Settled::Memory {
                id,
                sha: b.sha.clone(),
            });
            continue;
        }
        let gone = |c: &Commit| {
            c.sha
                .as_deref()
                .is_some_and(|s| !on_branch.iter().any(|o| same_sha(o, s)))
        };
        let rewritten = commits.iter_mut().find(|c| {
            c.status == CommitStatus::Done
                && gone(c)
                && git_subject(c) == b.subject.trim()
                && sorted(&c.files) == sorted(&b.files)
        });
        if let Some(c) = rewritten {
            let from = c.sha.replace(b.sha.clone()).unwrap_or_default();
            out.push(Settled::Rewritten {
                id: c.id.clone(),
                from,
                sha: b.sha.clone(),
            });
            continue;
        }
        let id = next_id(commits, 'X');
        commits.push(Commit {
            id: id.clone(),
            subject: b.subject.clone(),
            message: None,
            files: b.files.clone(),
            status: CommitStatus::Done,
            tasks: vec![],
            sha: Some(b.sha.clone()),
            needs_human: Some(OUTSIDE_COMMIT_HOLD.into()),
        });
        out.push(Settled::Outside {
            id,
            sha: b.sha.clone(),
        });
    }
    out
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

    /// Settle a branch of `shas` (subject `subject <sha>`, files `f.rs`).
    fn settle(commits: &mut Vec<Commit>, shas: &[&str], memory: &[&str]) -> Vec<Settled> {
        let all = branch(shas);
        let on_branch: Vec<String> = all.iter().map(|(s, _)| s.clone()).collect();
        let unknown: Vec<BranchCommit> = unrecorded_commits(commits, &all)
            .into_iter()
            .map(|(sha, subject)| BranchCommit {
                sha,
                subject,
                files: vec!["f.rs".into()],
            })
            .collect();
        let memory: Vec<String> = memory.iter().map(|s| s.to_string()).collect();
        settle_branch_commits(commits, &on_branch, &unknown, &memory)
    }

    fn outside(id: &str, sha: &str) -> Settled {
        Settled::Outside {
            id: id.into(),
            sha: sha.into(),
        }
    }

    #[test]
    fn unknown_commits_are_held_x_rows_and_recorded_ones_match_by_prefix() {
        let mut commits = vec![done("C01", "aaaa111"), done("M01", "BBBB2222")];
        let got = settle(
            &mut commits,
            &["aaaa1112222", "bbbb22223333", "cccc3333"],
            &[],
        );
        assert_eq!(got, [outside("X01", "cccc3333")]);
        let c = commits.last().unwrap();
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
        assert!(c.tasks.is_empty());
        assert_eq!(c.files, ["f.rs"]);
        assert_eq!(c.needs_human.as_deref(), Some(OUTSIDE_COMMIT_HOLD));
        // Again: nothing new.
        assert!(
            settle(
                &mut commits,
                &["aaaa1112222", "bbbb22223333", "cccc3333"],
                &[]
            )
            .is_empty()
        );
        assert_eq!(commits.len(), 3);
    }

    #[test]
    fn x_rows_number_after_the_last_x() {
        let mut commits = vec![done("X01", "aaaa1111"), done("X03", "bbbb2222")];
        let got = settle(&mut commits, &["aaaa1111", "bbbb2222", "cccc", "dddd"], &[]);
        assert_eq!(got, [outside("X04", "cccc"), outside("X05", "dddd")]);
    }

    #[test]
    fn proposals_without_a_sha_match_nothing() {
        let mut p = done("C02", "");
        p.sha = None;
        p.status = CommitStatus::Proposed;
        let mut commits = vec![p, done("C01", "")];
        assert_eq!(
            settle(&mut commits, &["eeee"], &[]),
            [outside("X01", "eeee")]
        );
    }

    #[test]
    fn an_earlier_shared_memory_commit_becomes_an_m_row() {
        let mut commits = vec![done("C01", "aaaa1111"), done("M01", "bbbb2222")];
        let got = settle(
            &mut commits,
            &["aaaa1111", "bbbb2222", "cccc3333", "dddd4444"],
            &["cccc3333ffff"],
        );
        assert_eq!(
            got,
            [
                Settled::Memory {
                    id: "M02".into(),
                    sha: "cccc3333".into()
                },
                outside("X01", "dddd4444")
            ]
        );
        let m = commits.iter().find(|c| c.id == "M02").unwrap();
        assert_eq!(
            (
                m.subject.as_str(),
                m.needs_human.as_deref(),
                m.files.clone()
            ),
            (MEMORY_COMMIT_SUBJECT, None, vec!["f.rs".to_string()])
        );
    }

    #[test]
    fn a_rewritten_commit_keeps_its_row_with_the_new_sha() {
        // Recorded with a multi-line first paragraph: git shows it on one line.
        let mut c = done("C01", "aaaa1111");
        c.subject = "the proposal's short subject".into();
        c.message = Some("subject\neeee5555\n\nWhy.".into());
        c.files = vec!["f.rs".into()];
        let mut commits = vec![c, done("C02", "bbbb2222")];
        // `aaaa1111` was amended into `eeee5555`; C02 is still on the branch.
        let got = settle(&mut commits, &["eeee5555", "bbbb2222"], &[]);
        assert_eq!(
            got,
            [Settled::Rewritten {
                id: "C01".into(),
                from: "aaaa1111".into(),
                sha: "eeee5555".into()
            }]
        );
        assert_eq!(commits[0].sha.as_deref(), Some("eeee5555"));
        assert_eq!(commits.len(), 2);
    }

    #[test]
    fn a_rewrite_with_other_files_or_of_a_commit_still_there_is_outside() {
        let mut c = done("C01", "aaaa1111");
        c.subject = "subject eeee5555".into();
        c.files = vec!["f.rs".into(), "g.rs".into()];
        let mut commits = vec![c];
        // Different files: an X row, C01 keeps its sha.
        assert_eq!(
            settle(&mut commits, &["eeee5555"], &[]),
            [outside("X01", "eeee5555")]
        );
        assert_eq!(commits[0].sha.as_deref(), Some("aaaa1111"));
        // Same subject and files, but C01 is still on the branch: an X row.
        let mut c = done("C01", "aaaa1111");
        c.subject = "subject ffff6666".into();
        c.files = vec!["f.rs".into()];
        let mut commits = vec![c];
        assert_eq!(
            settle(&mut commits, &["aaaa1111", "ffff6666"], &[]),
            [outside("X01", "ffff6666")]
        );
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
