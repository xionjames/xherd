//! What happens around an approval (spec §6.1–6.2): the command handed on
//! after it, the message for a change request, and gates inferred from the
//! documents when the agent stopped without declaring one (R-102, R-107).

use std::path::{Path, PathBuf};

use xherd_core::Agent;
use xherd_core::domain::{Gate, HashedFile, Role, Task, TaskStatus};

use crate::{Facts, Flow, Pack, frontmatter};

/// Runtime used when an agent does not name one (v1 ships Claude Code only).
pub const DEFAULT_RUNTIME: &str = "claude-code";

/// [`Facts`] read from an agent's spec folder.
pub struct DirFacts<'a> {
    pub spec_dir: PathBuf,
    pub tasks: &'a [Task],
}

impl DirFacts<'_> {
    fn read(&self, file: &str) -> Option<String> {
        crate::manifest::relative_path_ok(file)
            .then(|| std::fs::read_to_string(self.spec_dir.join(file)).ok())
            .flatten()
    }
}

impl Facts for DirFacts<'_> {
    fn frontmatter(&self, file: &str, key: &str) -> Option<String> {
        let fm = frontmatter::parse(&self.read(file)?).ok()??;
        fm.get(key).map(frontmatter::as_text)
    }
    fn file_exists(&self, file: &str) -> bool {
        crate::manifest::relative_path_ok(file) && self.spec_dir.join(file).is_file()
    }
    fn all_tasks_done(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|t| t.status == TaskStatus::Done)
    }
    fn setting(&self, _key: &str) -> Option<String> {
        None
    }
}

pub fn spec_dir(workdir: &Path) -> PathBuf {
    workdir.join(crate::xap::SPEC_DIR)
}

/// The command to send once `gate` is approved, rendered for the agent's
/// runtime, with the Enters it needs. `None`: nothing to hand on.
pub fn next_command(
    pack: &Pack,
    a: &Agent,
    gate: &Gate,
    facts: &dyn Facts,
    auto: bool,
) -> Result<Option<(String, u32)>, String> {
    let runtime = a.runtime.as_deref().unwrap_or(DEFAULT_RUNTIME);
    let enters = pack.install.get(runtime).map_or(1, |i| i.slash_enters);
    let flow = pack
        .flow(a.flow.as_deref().unwrap_or_default())
        .ok_or_else(|| format!("pack {} has no flow {:?}", pack.name, a.flow))?;
    let (tpl, commit) = match gate {
        Gate::Docs { next: Some(n), .. } => return Ok(Some((n.clone(), enters))),
        Gate::Docs { stage, .. } => {
            let def = flow
                .stage(stage)
                .ok_or_else(|| format!("no stage {stage}"))?;
            (def.next_template(facts, auto), "")
        }
        Gate::Commit(c) => {
            let def = flow.stages.iter().find(|s| s.is_commit_gate());
            (
                def.and_then(|d| d.next_template(facts, auto)),
                c.id.as_str(),
            )
        }
        _ => (None, ""),
    };
    let Some(tpl) = tpl else { return Ok(None) };
    let cmd = pack.command(
        runtime,
        tpl,
        &[("key", &a.ticket_key), ("prompt", ""), ("commit", commit)],
    )?;
    Ok(Some((cmd, enters)))
}

/// Text typed to the agent for a change request (R-105).
pub fn changes_message(gate: &Gate, comment: &str) -> String {
    match gate {
        Gate::Commit(c) => format!(
            "Changes requested for commit {} ({}): {comment}\n\
             Revise the change, then propose {} again with `xherd commit propose` and stop.",
            c.id,
            c.files.join(", "),
            c.id
        ),
        Gate::Docs { stage, files, .. } => {
            let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
            format!(
                "Changes requested at the {stage} gate ({}): {comment}\n\
                 Revise the documents, then call `xherd gate open {stage}` again and stop.",
                names.join(", ")
            )
        }
        _ => comment.to_string(),
    }
}

/// Hash of a gate document for approvals (R-104, R-107): the content hash,
/// except that in the document gated by the flow's tasks stage the task
/// progress lines do not count ([`crate::tasks::without_progress`]).
/// `path` is canonical ([`crate::xap::doc_path`]).
pub fn doc_sha256(flow: Option<&Flow>, workdir: &Path, path: &str, bytes: &[u8]) -> String {
    let tasks_doc = flow.is_some_and(|f| {
        f.stages
            .iter()
            .filter(|s| s.role == Role::Tasks)
            .flat_map(|s| s.docs())
            .any(|d| crate::xap::doc_path(d, workdir).is_ok_and(|p| p == path))
    });
    if !tasks_doc {
        return xherd_core::hash::content_sha256(bytes);
    }
    let text = crate::tasks::without_progress(&String::from_utf8_lossy(bytes));
    xherd_core::hash::content_sha256(text.as_bytes())
}

/// One of a flow's gate documents as found on disk at a stop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSeen {
    pub stage: String,
    /// Canonical path (spec-relative).
    pub path: String,
    /// Current content hash; `None` when the file does not exist.
    pub sha256: Option<String>,
    /// The ledger approves this exact content.
    pub approved: bool,
    /// Some content of this file was approved before.
    pub ever_approved: bool,
}

/// The gate to open when the agent stopped without declaring one: the first
/// stage (in flow order) with documents that exist and are not approved as
/// they are. All its such documents form one gate (R-108). The flag says
/// whether a previously approved document changed (re-opened, R-107) rather
/// than a new one appearing (inferred, R-102).
pub fn infer_gate(flow: &Flow, seen: &[DocSeen]) -> Option<(Gate, bool)> {
    for s in &flow.stages {
        let pending: Vec<&DocSeen> = seen
            .iter()
            .filter(|d| d.stage == s.id && d.sha256.is_some() && !d.approved)
            .collect();
        if pending.is_empty() {
            continue;
        }
        let reopened = pending.iter().any(|d| d.ever_approved);
        let files = pending
            .iter()
            .map(|d| HashedFile {
                path: d.path.clone(),
                sha256: d.sha256.clone().unwrap_or_default(),
            })
            .collect();
        let gate = Gate::Docs {
            stage: s.id.clone(),
            role: s.role,
            files,
            next: None,
            needs_human: None,
            questions: Vec::new(),
        };
        return Some((gate, reopened));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use xherd_core::domain::{Commit, CommitStatus};

    struct F(bool);
    impl Facts for F {
        fn frontmatter(&self, _: &str, key: &str) -> Option<String> {
            (self.0 && key == "design").then(|| "skip".into())
        }
        fn file_exists(&self, _: &str) -> bool {
            false
        }
        fn all_tasks_done(&self) -> bool {
            false
        }
        fn setting(&self, _: &str) -> Option<String> {
            None
        }
    }

    fn agent() -> Agent {
        let mut a = Agent::new("SC-1", "SC-1", 0);
        a.pack = Some("xhspec".into());
        a.flow = Some("feature".into());
        a
    }

    fn docs(stage: &str) -> Gate {
        Gate::Docs {
            stage: stage.into(),
            role: xherd_core::domain::Role::Spec,
            files: vec![HashedFile {
                path: format!("{stage}.md"),
                sha256: "h".into(),
            }],
            next: None,
            needs_human: None,
            questions: vec![],
        }
    }

    fn commit() -> Gate {
        Gate::Commit(Commit {
            id: "C02".into(),
            subject: "s".into(),
            message: None,
            files: vec!["a.rs".into()],
            status: CommitStatus::Proposed,
            tasks: vec![],
            sha: None,
            needs_human: None,
        })
    }

    #[test]
    fn next_commands() {
        let p = crate::xhspec();
        let a = agent();
        let n = |g: &Gate, f: &F, auto| next_command(&p, &a, g, f, auto).unwrap();
        assert_eq!(
            n(&docs("spec"), &F(false), false),
            Some(("/xhspec:design".into(), 1))
        );
        assert_eq!(
            n(&docs("spec"), &F(true), false),
            Some(("/xhspec:tasks".into(), 1))
        );
        assert_eq!(
            n(&docs("tasks"), &F(false), false),
            Some(("/xhspec:next".into(), 1))
        );
        assert_eq!(
            n(&commit(), &F(false), false),
            Some(("/xhspec:commit C02".into(), 1))
        );
        assert_eq!(
            n(&docs("ship"), &F(false), true),
            Some(("/xhspec:ship --no-ticket-update".into(), 1))
        );
        assert_eq!(
            n(&Gate::Question { text: "?".into() }, &F(false), false),
            None
        );
        let mut g = docs("spec");
        if let Gate::Docs { next, .. } = &mut g {
            *next = Some("/sdd-task".into());
        }
        assert_eq!(n(&g, &F(false), false), Some(("/sdd-task".into(), 1)));
    }

    #[test]
    fn messages_name_the_gate() {
        let m = changes_message(&docs("spec"), "split R3");
        assert!(m.contains("spec gate (spec.md): split R3") && m.contains("xherd gate open spec"));
        let m = changes_message(&commit(), "no unwrap");
        assert!(m.contains("commit C02 (a.rs): no unwrap"));
    }

    #[test]
    fn task_progress_is_not_content() {
        let f = crate::xhspec().flow("feature").unwrap().clone();
        let wd = Path::new("/w");
        let a = b"## T01: a\n- status: todo\n- files: a.rs\n";
        let b = b"## T01: a\n- status: done\n- files: a.rs\n- evidence: ok\n";
        let h = |p: &str, x: &[u8]| doc_sha256(Some(&f), wd, p, x);
        assert_eq!(h("tasks.md", a), h("tasks.md", b));
        assert_ne!(h("spec.md", a), h("spec.md", b));
        assert_ne!(
            h("tasks.md", a),
            h("tasks.md", b"## T01: b\n- status: todo\n- files: a.rs\n")
        );
        assert_eq!(
            doc_sha256(None, wd, "tasks.md", a),
            xherd_core::hash::content_sha256(a)
        );
    }

    fn seen(stage: &str, sha: Option<&str>, approved: bool, ever: bool) -> DocSeen {
        DocSeen {
            stage: stage.into(),
            path: format!("{stage}.md"),
            sha256: sha.map(str::to_string),
            approved,
            ever_approved: ever,
        }
    }

    #[test]
    fn inference() {
        let f = crate::xhspec().flow("feature").unwrap().clone();
        assert_eq!(
            infer_gate(
                &f,
                &[
                    seen("spec", Some("a"), true, true),
                    seen("design", None, false, false)
                ]
            ),
            None
        );
        let (g, reopened) = infer_gate(
            &f,
            &[
                seen("spec", Some("a"), true, true),
                seen("design", Some("d"), false, false),
            ],
        )
        .unwrap();
        assert!(!reopened);
        assert!(matches!(&g, Gate::Docs { stage, .. } if stage == "design"));
        // An edited, earlier-approved spec comes first.
        let (g, reopened) = infer_gate(
            &f,
            &[
                seen("design", Some("d"), false, false),
                seen("spec", Some("b"), false, true),
            ],
        )
        .unwrap();
        assert!(reopened);
        assert!(
            matches!(&g, Gate::Docs { stage, files, .. } if stage == "spec" && files[0].sha256 == "b")
        );
    }
}
