//! XAP validation and effects (design §6.5). Pure: the actor observes the
//! files and git first (`Observed`), then calls [`apply`]; on success it
//! persists the agent and logs the call.

use std::path::Path;

use xherd_core::domain::{Commit, CommitStatus, Gate, HashedFile, TaskStatus};
use xherd_core::xap::{Refusal, XapCmd, refuse};
use xherd_core::{Agent, Millis};

use crate::{Flow, Pack};

/// The private spec folder inside a worktree (spec §7.5).
pub const SPEC_DIR: &str = ".xherd/spec";

/// Canonical form of a document path named in a call (xap.md §1.5): relative
/// to the spec folder (`spec.md`), or to the worktree root with `./`
/// (`./docs/x.md`). Absolute paths must be inside the worktree.
pub fn doc_path(given: &str, workdir: &Path) -> Result<String, Refusal> {
    let outside = || {
        refuse(
            "path_outside_workspace",
            format!("{given} is outside the worktree"),
        )
    };
    let joined = if Path::new(given).is_absolute() {
        Path::new(given)
            .strip_prefix(workdir)
            .map_err(|_| outside())?
            .to_string_lossy()
            .into_owned()
    } else if let Some(rest) = given.strip_prefix("./") {
        rest.to_string()
    } else {
        format!("{SPEC_DIR}/{given}")
    };
    let mut parts: Vec<&str> = Vec::new();
    for c in joined.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or_else(outside)?;
            }
            c => parts.push(c),
        }
    }
    if parts.is_empty() {
        return Err(refuse("doc_missing", format!("{given:?} is not a file")));
    }
    let rel = parts.join("/");
    Ok(match rel.strip_prefix(&format!("{SPEC_DIR}/")) {
        Some(in_spec) => in_spec.to_string(),
        None => format!("./{rel}"),
    })
}

/// Worktree-relative location of a canonical document path.
pub fn worktree_rel(doc: &str) -> String {
    match doc.strip_prefix("./") {
        Some(rel) => rel.to_string(),
        None => format!("{SPEC_DIR}/{doc}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocState {
    Missing,
    /// Resolves (through a symlink) outside the worktree.
    Outside,
    Present {
        sha256: String,
        /// Frontmatter that does not parse (`doc_schema`).
        frontmatter_error: Option<String>,
        /// The ledger already approves this exact content.
        approved: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocFact {
    pub path: String,
    pub state: DocState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFact {
    /// Full sha when the commit exists.
    pub sha: Option<String>,
    /// The commit is on the agent's branch (an ancestor of its HEAD).
    pub on_branch: bool,
    pub files: Vec<String>,
}

/// What the actor looked at before calling [`apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    Nothing,
    /// `gate.open`: one fact per path from [`gate_docs`], same order.
    Docs(Vec<DocFact>),
    /// `tasks.sync`: the file's text, `None` when missing.
    Text(Option<String>),
    /// `commit.done`.
    Commit(CommitFact),
    /// `handoff.written`: whether the file exists.
    Exists(bool),
}

pub fn flow_of<'a>(a: &Agent, pack: Option<&'a Pack>) -> Result<&'a Flow, Refusal> {
    let no = || {
        refuse(
            "no_flow",
            format!(
                "{} runs without a flow; XAP gates and tasks do not apply",
                a.name
            ),
        )
    };
    pack.zip(a.flow.as_deref())
        .and_then(|(p, f)| p.flow(f))
        .ok_or_else(no)
}

/// The documents a `gate.open` covers, canonicalised: the given `--doc`s and
/// `--artifact`s, else the stage's declared documents.
pub fn gate_docs(
    a: &Agent,
    pack: Option<&Pack>,
    cmd: &XapCmd,
    workdir: &Path,
) -> Result<Vec<String>, Refusal> {
    let XapCmd::GateOpen {
        stage,
        docs,
        artifacts,
        ..
    } = cmd
    else {
        return Ok(Vec::new());
    };
    let flow = flow_of(a, pack)?;
    let def = flow.stage(stage).ok_or_else(|| {
        refuse(
            "unknown_stage",
            format!("flow {} has no stage {stage}", flow_name(a)),
        )
    })?;
    if def.is_commit_gate() {
        return Err(refuse(
            "no_docs_gate",
            format!("stage {stage} gates commits; use `xherd commit propose`"),
        ));
    }
    let given: Vec<&String> = docs.iter().chain(artifacts).collect();
    let list: Vec<&String> = if given.is_empty() {
        def.docs().iter().collect()
    } else {
        given
    };
    if list.is_empty() {
        return Err(refuse(
            "no_docs_gate",
            format!("stage {stage} has no documents to approve"),
        ));
    }
    let mut out: Vec<String> = Vec::new();
    for d in list {
        let p = doc_path(d, workdir)?;
        if !out.contains(&p) {
            out.push(p);
        }
    }
    Ok(out)
}

fn flow_name(a: &Agent) -> &str {
    a.flow.as_deref().unwrap_or("none")
}

fn open_commit(a: &Agent) -> Option<&Commit> {
    a.commits
        .iter()
        .find(|c| matches!(c.status, CommitStatus::Proposed | CommitStatus::Approved))
}

fn wrong(what: &str) -> Refusal {
    refuse(
        "internal",
        format!("{what}: observation does not match the call"),
    )
}

/// Validate and apply one call. Returns whether the agent changed; `Ok(false)`
/// means "already in that state" (still exit 0, xap.md §1.4).
pub fn apply(
    a: &mut Agent,
    pack: Option<&Pack>,
    cmd: &XapCmd,
    obs: Observed,
    now: Millis,
) -> Result<bool, Refusal> {
    // Flow `none` keeps questions, PRs, handoffs and issues (R-033).
    if !matches!(
        cmd,
        XapCmd::Ask { .. }
            | XapCmd::PrOpened { .. }
            | XapCmd::HandoffWritten { .. }
            | XapCmd::IssueAdd { .. }
    ) {
        flow_of(a, pack)?;
    }
    match cmd {
        XapCmd::StageEnter { stage, .. } => {
            let flow = flow_of(a, pack)?;
            if flow.stage(stage).is_none() {
                return Err(refuse(
                    "unknown_stage",
                    format!("flow {} has no stage {stage}", flow_name(a)),
                ));
            }
            Ok(set(&mut a.stage, Some(stage.clone())))
        }

        XapCmd::GateOpen {
            stage,
            next,
            needs_human,
            questions,
            ..
        } => {
            let role = flow_of(a, pack)?
                .stage(stage)
                .ok_or_else(|| refuse("unknown_stage", format!("no stage {stage}")))?
                .role;
            if next.is_some() {
                return Err(refuse(
                    "next_not_allowed",
                    "this flow's pack decides the next command; drop --next",
                ));
            }
            if let Some(c) = open_commit(a) {
                return Err(refuse(
                    "gate_open_commit",
                    format!(
                        "commit proposal {} is still open; finish or drop it first",
                        c.id
                    ),
                ));
            }
            let Observed::Docs(facts) = obs else {
                return Err(wrong("gate.open"));
            };
            let mut files = Vec::new();
            let mut all_approved = true;
            for f in facts {
                match f.state {
                    DocState::Missing => {
                        return Err(refuse("doc_missing", format!("{} does not exist", f.path)));
                    }
                    DocState::Outside => {
                        return Err(refuse(
                            "path_outside_workspace",
                            format!("{} resolves outside the worktree", f.path),
                        ));
                    }
                    DocState::Present {
                        sha256,
                        frontmatter_error,
                        approved,
                    } => {
                        if let Some(e) = frontmatter_error {
                            return Err(refuse("doc_schema", format!("{}: {e}", f.path)));
                        }
                        all_approved &= approved;
                        files.push(HashedFile {
                            path: f.path,
                            sha256,
                        });
                    }
                }
            }
            if all_approved {
                return Err(refuse(
                    "already_approved",
                    "these documents are already approved as they are; continue with the next step",
                ));
            }
            let gate = Gate::Docs {
                stage: stage.clone(),
                role,
                files,
                next: None,
                needs_human: needs_human.clone(),
                questions: questions.clone(),
            };
            let changed = set(&mut a.stage, Some(stage.clone())) | set(&mut a.gate, Some(gate));
            if changed {
                a.generation += 1;
            }
            Ok(changed)
        }

        XapCmd::Ask { text } => {
            if let Some(c) = open_commit(a) {
                return Err(refuse(
                    "gate_open_commit",
                    format!("commit proposal {} is still open", c.id),
                ));
            }
            let changed = match &mut a.gate {
                Some(Gate::Docs { questions, .. }) => {
                    if questions.contains(text) {
                        false
                    } else {
                        questions.push(text.clone());
                        true
                    }
                }
                g => set(g, Some(Gate::Question { text: text.clone() })),
            };
            if changed {
                a.generation += 1;
            }
            Ok(changed)
        }

        XapCmd::TasksSync { file } => {
            flow_of(a, pack)?;
            let Observed::Text(text) = obs else {
                return Err(wrong("tasks.sync"));
            };
            let name = file.as_deref().unwrap_or("tasks.md");
            let text =
                text.ok_or_else(|| refuse("doc_missing", format!("{name} does not exist")))?;
            let mut tasks = crate::tasks::parse(&text).map_err(|e| refuse("doc_schema", e))?;
            // The file is the source; progress already reported through XAP is
            // kept when the file still says `todo`.
            for t in &mut tasks {
                if t.status == TaskStatus::Todo
                    && let Some(old) = a.tasks.iter().find(|o| o.id == t.id)
                {
                    t.status = old.status;
                    t.evidence = t.evidence.take().or_else(|| old.evidence.clone());
                    t.blocked_reason = old.blocked_reason.clone();
                }
            }
            Ok(set(&mut a.tasks, tasks))
        }

        XapCmd::TaskStart { id } => {
            let i = task_index(a, id)?;
            if a.tasks[i].status == TaskStatus::Doing {
                return Ok(false);
            }
            if let Some(other) = a.tasks.iter().find(|t| t.status == TaskStatus::Doing) {
                return Err(refuse(
                    "task_already_doing",
                    format!("{} is in progress; finish or block it first", other.id),
                ));
            }
            let pending: Vec<&str> = a.tasks[i]
                .depends
                .iter()
                .filter(|d| {
                    a.tasks
                        .iter()
                        .any(|t| &t.id == *d && t.status != TaskStatus::Done)
                })
                .map(String::as_str)
                .collect();
            if !pending.is_empty() {
                return Err(refuse(
                    "task_not_ready",
                    format!("{id} waits for {}", pending.join(", ")),
                ));
            }
            let t = &mut a.tasks[i];
            t.status = TaskStatus::Doing;
            t.blocked_reason = None;
            Ok(true)
        }

        XapCmd::TaskDone { id, evidence } => {
            let i = task_index(a, id)?;
            if evidence.trim().is_empty() {
                return Err(refuse(
                    "evidence_missing",
                    "say how the task was verified (--evidence)",
                ));
            }
            let t = &mut a.tasks[i];
            if t.status == TaskStatus::Done && t.evidence.as_deref() == Some(evidence) {
                return Ok(false);
            }
            t.status = TaskStatus::Done;
            t.evidence = Some(evidence.clone());
            t.blocked_reason = None;
            Ok(true)
        }

        XapCmd::TaskBlock { id, reason } => {
            let i = task_index(a, id)?;
            let t = &mut a.tasks[i];
            if t.status == TaskStatus::Blocked && t.blocked_reason.as_deref() == Some(reason) {
                return Ok(false);
            }
            t.status = TaskStatus::Blocked;
            t.blocked_reason = Some(reason.clone());
            Ok(true)
        }

        XapCmd::CommitPropose {
            id,
            subject,
            message,
            files,
            tasks,
            needs_human,
        } => {
            let flow = flow_of(a, pack)?;
            let mut files = files.clone();
            files.sort();
            files.dedup();
            if files.is_empty() {
                return Err(refuse(
                    "files_empty",
                    "list the files of this commit (--files)",
                ));
            }
            let proposal = Commit {
                id: id.clone(),
                subject: subject.clone(),
                message: message
                    .as_deref()
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map(str::to_string),
                files,
                status: CommitStatus::Proposed,
                tasks: tasks.clone(),
                sha: None,
                needs_human: needs_human.clone(),
            };
            if let Some(open) = open_commit(a) {
                if open.id == *id && open.status == CommitStatus::Proposed {
                    if *open == proposal {
                        return Ok(false);
                    }
                    // Revised after a change request: replace the proposal.
                    a.commits.retain(|c| c.id != *id);
                } else {
                    return Err(refuse(
                        "commit_open",
                        format!(
                            "commit proposal {} is still open; finish or drop it first",
                            open.id
                        ),
                    ));
                }
            }
            if a.commits.iter().any(|c| c.id == *id) {
                return Err(refuse(
                    "commit_id_used",
                    format!("{id} was already used; pick the next id"),
                ));
            }
            if let Some(Gate::Docs { stage, .. }) = &a.gate {
                return Err(refuse(
                    "docs_gate_open",
                    format!("the {stage} gate is still open; wait for its approval"),
                ));
            }
            let mut allowed: Vec<&str> = Vec::new();
            let mut fenced = !tasks.is_empty();
            for t in tasks {
                let task = a.tasks.iter().find(|x| &x.id == t).ok_or_else(|| {
                    refuse(
                        "task_unknown",
                        format!("no task {t}; run `xherd tasks sync`"),
                    )
                })?;
                fenced &= !task.files.is_empty();
                allowed.extend(task.files.iter().map(String::as_str));
            }
            if fenced {
                let outside: Vec<&str> = proposal
                    .files
                    .iter()
                    .map(String::as_str)
                    .filter(|f| !allowed.contains(f))
                    .collect();
                if !outside.is_empty() {
                    return Err(refuse(
                        "files_outside_scope",
                        format!(
                            "{} not in the files: of {}; update tasks.md and sync, or split the commit",
                            outside.join(", "),
                            tasks.join(", ")
                        ),
                    ));
                }
            }
            if let Some(s) = flow.stages.iter().find(|s| s.is_commit_gate()) {
                a.stage = Some(s.id.clone());
            }
            a.gate = Some(Gate::Commit(proposal.clone()));
            a.commits.push(proposal);
            a.generation += 1;
            Ok(true)
        }

        XapCmd::CommitDone { id, sha } => {
            let i = commit_index(a, id)?;
            let c = &a.commits[i];
            match c.status {
                CommitStatus::Done
                    if c.sha
                        .as_deref()
                        .is_some_and(|s| s.starts_with(sha.as_str())) =>
                {
                    return Ok(false);
                }
                CommitStatus::Done => {
                    return Err(refuse(
                        "commit_done",
                        format!(
                            "{id} is already done as {}",
                            c.sha.as_deref().unwrap_or("?")
                        ),
                    ));
                }
                CommitStatus::Dropped => {
                    return Err(refuse("commit_unknown", format!("{id} was dropped")));
                }
                CommitStatus::Proposed => {
                    return Err(refuse(
                        "commit_not_approved",
                        format!("{id} is waiting for approval; do not commit yet"),
                    ));
                }
                CommitStatus::Approved => {}
            }
            let Observed::Commit(fact) = obs else {
                return Err(wrong("commit.done"));
            };
            let full = fact.sha.ok_or_else(|| {
                refuse("sha_unknown", format!("no commit {sha} in this repository"))
            })?;
            if !fact.on_branch {
                return Err(refuse(
                    "sha_unknown",
                    format!("{sha} is not on this agent's branch"),
                ));
            }
            let mut got = fact.files;
            got.sort();
            got.dedup();
            if got != c.files {
                return Err(refuse(
                    "sha_files_mismatch",
                    format!(
                        "{sha} touches [{}] but {id} proposed [{}]",
                        got.join(", "),
                        c.files.join(", ")
                    ),
                ));
            }
            let c = &mut a.commits[i];
            c.status = CommitStatus::Done;
            c.sha = Some(full);
            if matches!(&a.gate, Some(Gate::Commit(g)) if g.id == *id) {
                a.gate = None;
            }
            a.generation += 1;
            Ok(true)
        }

        XapCmd::CommitDrop { id, .. } => {
            let i = commit_index(a, id)?;
            match a.commits[i].status {
                CommitStatus::Dropped => return Ok(false),
                CommitStatus::Done => {
                    return Err(refuse("commit_done", format!("{id} is already committed")));
                }
                _ => {}
            }
            a.commits[i].status = CommitStatus::Dropped;
            if matches!(&a.gate, Some(Gate::Commit(g)) if g.id == *id) {
                a.gate = None;
            }
            a.generation += 1;
            Ok(true)
        }

        XapCmd::PrOpened { url } => {
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(refuse("bad_url", format!("{url:?} is not an http(s) URL")));
            }
            Ok(set(&mut a.pr_url, Some(url.clone())))
        }

        XapCmd::HandoffWritten { file } => {
            let Observed::Exists(exists) = obs else {
                return Err(wrong("handoff.written"));
            };
            if !exists {
                let f = file.as_deref().unwrap_or(HANDOFF);
                return Err(refuse("doc_missing", format!("{f} does not exist")));
            }
            a.handoff_at = Some(now);
            Ok(true)
        }

        XapCmd::IssueAdd { title, .. } => {
            if title.trim().is_empty() {
                return Err(refuse("usage", "an issue needs a title"));
            }
            // The actor appends it to the issue ledger.
            Ok(true)
        }
    }
}

/// Default handoff note, at the root of `.xherd/` (spec §7.5).
pub const HANDOFF: &str = "./.xherd/HANDOFF.md";

fn set<T: PartialEq>(slot: &mut T, v: T) -> bool {
    if *slot == v {
        false
    } else {
        *slot = v;
        true
    }
}

fn task_index(a: &Agent, id: &str) -> Result<usize, Refusal> {
    a.tasks.iter().position(|t| t.id == id).ok_or_else(|| {
        refuse(
            "task_unknown",
            format!("no task {id}; run `xherd tasks sync`"),
        )
    })
}

fn commit_index(a: &Agent, id: &str) -> Result<usize, Refusal> {
    a.commits
        .iter()
        .position(|c| c.id == id)
        .ok_or_else(|| refuse("commit_unknown", format!("no commit proposal {id}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xherd_core::domain::{Role, Task};

    fn agent() -> Agent {
        let mut a = Agent::new("SC-1", "SC-1", 0);
        a.pack = Some("xhspec".into());
        a.flow = Some("feature".into());
        a
    }

    fn pack() -> Pack {
        crate::xhspec()
    }

    fn present(path: &str, sha: &str, approved: bool) -> DocFact {
        DocFact {
            path: path.into(),
            state: DocState::Present {
                sha256: sha.into(),
                frontmatter_error: None,
                approved,
            },
        }
    }

    fn gate_open(stage: &str) -> XapCmd {
        XapCmd::GateOpen {
            stage: stage.into(),
            role: None,
            docs: vec![],
            artifacts: vec![],
            next: None,
            needs_human: None,
            questions: vec![],
        }
    }

    fn task(id: &str, status: TaskStatus, depends: &[&str], files: &[&str]) -> Task {
        Task {
            id: id.into(),
            title: id.into(),
            status,
            depends: depends.iter().map(|s| s.to_string()).collect(),
            files: files.iter().map(|s| s.to_string()).collect(),
            evidence: None,
            verify: None,
            blocked_reason: None,
        }
    }

    fn code(r: Result<bool, Refusal>) -> &'static str {
        r.unwrap_err().code
    }

    #[test]
    fn doc_paths() {
        let wt = Path::new("/w/SC-1");
        assert_eq!(doc_path("spec.md", wt).unwrap(), "spec.md");
        assert_eq!(doc_path("./docs/a.md", wt).unwrap(), "./docs/a.md");
        assert_eq!(
            doc_path("/w/SC-1/.xherd/spec/x/../spec.md", wt).unwrap(),
            "spec.md"
        );
        assert_eq!(doc_path("../../README.md", wt).unwrap(), "./README.md");
        assert_eq!(
            doc_path("/w/other/a.md", wt).unwrap_err().code,
            "path_outside_workspace"
        );
        assert_eq!(
            doc_path("./../a.md", wt).unwrap_err().code,
            "path_outside_workspace"
        );
        assert_eq!(worktree_rel("spec.md"), ".xherd/spec/spec.md");
        assert_eq!(worktree_rel("./docs/a.md"), "docs/a.md");
    }

    #[test]
    fn gate_docs_defaults_and_refusals() {
        let a = agent();
        let p = pack();
        let wt = Path::new("/w");
        assert_eq!(
            gate_docs(&a, Some(&p), &gate_open("spec"), wt).unwrap(),
            ["spec.md"]
        );
        let mut c = gate_open("spec");
        if let XapCmd::GateOpen {
            docs, artifacts, ..
        } = &mut c
        {
            *docs = vec!["spec.md".into(), "./spec.md".into(), "spec.md".into()];
            *artifacts = vec!["mock.png".into()];
        }
        assert_eq!(
            gate_docs(&a, Some(&p), &c, wt).unwrap(),
            ["spec.md", "./spec.md", "mock.png"]
        );
        assert_eq!(
            gate_docs(&a, Some(&p), &gate_open("nope"), wt)
                .unwrap_err()
                .code,
            "unknown_stage"
        );
        assert_eq!(
            gate_docs(&a, Some(&p), &gate_open("implement"), wt)
                .unwrap_err()
                .code,
            "no_docs_gate"
        );
        assert_eq!(
            gate_docs(&Agent::new("x", "x", 0), None, &gate_open("spec"), wt)
                .unwrap_err()
                .code,
            "no_flow"
        );
    }

    #[test]
    fn gate_open_sets_gate_once() {
        let mut a = agent();
        let p = pack();
        let obs = || Observed::Docs(vec![present("spec.md", "h1", false)]);
        assert!(apply(&mut a, Some(&p), &gate_open("spec"), obs(), 0).unwrap());
        let Some(Gate::Docs {
            stage, role, files, ..
        }) = &a.gate
        else {
            panic!("{:?}", a.gate)
        };
        assert_eq!(
            (stage.as_str(), *role, files[0].sha256.as_str()),
            ("spec", Role::Spec, "h1")
        );
        assert_eq!(a.stage.as_deref(), Some("spec"));
        let g = a.generation;
        assert!(!apply(&mut a, Some(&p), &gate_open("spec"), obs(), 0).unwrap());
        assert_eq!(a.generation, g);
    }

    #[test]
    fn gate_open_refusals() {
        let p = pack();
        let mut a = agent();
        let cases = [
            (
                DocFact {
                    path: "spec.md".into(),
                    state: DocState::Missing,
                },
                "doc_missing",
            ),
            (
                DocFact {
                    path: "spec.md".into(),
                    state: DocState::Outside,
                },
                "path_outside_workspace",
            ),
            (
                DocFact {
                    path: "spec.md".into(),
                    state: DocState::Present {
                        sha256: "h".into(),
                        frontmatter_error: Some("frontmatter: bad".into()),
                        approved: false,
                    },
                },
                "doc_schema",
            ),
            (present("spec.md", "h", true), "already_approved"),
        ];
        for (fact, want) in cases {
            assert_eq!(
                code(apply(
                    &mut a,
                    Some(&p),
                    &gate_open("spec"),
                    Observed::Docs(vec![fact]),
                    0
                )),
                want
            );
        }
        let mut c = gate_open("spec");
        if let XapCmd::GateOpen { next, .. } = &mut c {
            *next = Some("/x".into());
        }
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &c,
                Observed::Docs(vec![present("spec.md", "h", false)]),
                0
            )),
            "next_not_allowed"
        );
        assert!(a.gate.is_none());
    }

    #[test]
    fn ask_adds_question() {
        let p = pack();
        let mut a = agent();
        let ask = XapCmd::Ask {
            text: "which db?".into(),
        };
        assert!(apply(&mut a, Some(&p), &ask, Observed::Nothing, 0).unwrap());
        assert_eq!(
            a.gate,
            Some(Gate::Question {
                text: "which db?".into()
            })
        );
        let mut a = agent();
        apply(
            &mut a,
            Some(&p),
            &gate_open("spec"),
            Observed::Docs(vec![present("spec.md", "h", false)]),
            0,
        )
        .unwrap();
        apply(&mut a, Some(&p), &ask, Observed::Nothing, 0).unwrap();
        assert!(!apply(&mut a, Some(&p), &ask, Observed::Nothing, 0).unwrap());
        assert!(
            matches!(&a.gate, Some(Gate::Docs { questions, .. }) if questions == &["which db?"])
        );
    }

    #[test]
    fn tasks_sync_keeps_reported_progress() {
        let p = pack();
        let mut a = agent();
        let text = "## T01: a\n- files: a.rs\n## T02: b\n- depends: T01\n";
        let sync = XapCmd::TasksSync { file: None };
        assert!(
            apply(
                &mut a,
                Some(&p),
                &sync,
                Observed::Text(Some(text.into())),
                0
            )
            .unwrap()
        );
        assert_eq!(a.tasks.len(), 2);
        let done = XapCmd::TaskDone {
            id: "T01".into(),
            evidence: "tests".into(),
        };
        assert!(apply(&mut a, Some(&p), &done, Observed::Nothing, 0).unwrap());
        assert!(
            !apply(
                &mut a,
                Some(&p),
                &sync,
                Observed::Text(Some(text.into())),
                0
            )
            .unwrap()
        );
        assert_eq!(a.tasks[0].status, TaskStatus::Done);
        assert_eq!(
            code(apply(&mut a, Some(&p), &sync, Observed::Text(None), 0)),
            "doc_missing"
        );
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &sync,
                Observed::Text(Some("# none".into())),
                0
            )),
            "doc_schema"
        );
    }

    #[test]
    fn task_rules() {
        let p = pack();
        let mut a = agent();
        a.tasks = vec![
            task("T01", TaskStatus::Todo, &[], &[]),
            task("T02", TaskStatus::Todo, &["T01"], &[]),
            task("T03", TaskStatus::Todo, &[], &[]),
        ];
        let start = |id: &str| XapCmd::TaskStart { id: id.into() };
        assert_eq!(
            code(apply(&mut a, Some(&p), &start("T09"), Observed::Nothing, 0)),
            "task_unknown"
        );
        assert_eq!(
            code(apply(&mut a, Some(&p), &start("T02"), Observed::Nothing, 0)),
            "task_not_ready"
        );
        assert!(apply(&mut a, Some(&p), &start("T01"), Observed::Nothing, 0).unwrap());
        assert!(!apply(&mut a, Some(&p), &start("T01"), Observed::Nothing, 0).unwrap());
        assert_eq!(
            code(apply(&mut a, Some(&p), &start("T03"), Observed::Nothing, 0)),
            "task_already_doing"
        );
        let done = XapCmd::TaskDone {
            id: "T01".into(),
            evidence: " ".into(),
        };
        assert_eq!(
            code(apply(&mut a, Some(&p), &done, Observed::Nothing, 0)),
            "evidence_missing"
        );
        let block = XapCmd::TaskBlock {
            id: "T01".into(),
            reason: "needs a key".into(),
        };
        assert!(apply(&mut a, Some(&p), &block, Observed::Nothing, 0).unwrap());
        assert_eq!(a.tasks[0].status, TaskStatus::Blocked);
        assert!(apply(&mut a, Some(&p), &start("T03"), Observed::Nothing, 0).unwrap());
    }

    fn propose(id: &str, files: &[&str], tasks: &[&str]) -> XapCmd {
        XapCmd::CommitPropose {
            id: id.into(),
            subject: "s".into(),
            message: None,
            files: files.iter().map(|s| s.to_string()).collect(),
            tasks: tasks.iter().map(|s| s.to_string()).collect(),
            needs_human: None,
        }
    }

    #[test]
    fn commit_propose_rules() {
        let p = pack();
        let mut a = agent();
        a.tasks = vec![
            task("T01", TaskStatus::Doing, &[], &["a.rs", "b.rs"]),
            task("T02", TaskStatus::Todo, &[], &[]),
        ];
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &propose("C01", &[], &[]),
                Observed::Nothing,
                0
            )),
            "files_empty"
        );
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &propose("C01", &["a.rs"], &["T09"]),
                Observed::Nothing,
                0
            )),
            "task_unknown"
        );
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &propose("C01", &["a.rs", "c.rs"], &["T01"]),
                Observed::Nothing,
                0
            )),
            "files_outside_scope"
        );
        // A task without files: no fence.
        assert!(
            apply(
                &mut a,
                Some(&p),
                &propose("C01", &["c.rs", "a.rs"], &["T01", "T02"]),
                Observed::Nothing,
                0
            )
            .unwrap()
        );
        assert!(matches!(&a.gate, Some(Gate::Commit(c)) if c.files == ["a.rs", "c.rs"]));
        assert_eq!(a.stage.as_deref(), Some("implement"));
        assert!(
            !apply(
                &mut a,
                Some(&p),
                &propose("C01", &["a.rs", "c.rs"], &["T01", "T02"]),
                Observed::Nothing,
                0
            )
            .unwrap()
        );
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &propose("C02", &["a.rs"], &[]),
                Observed::Nothing,
                0
            )),
            "commit_open"
        );
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &gate_open("review"),
                Observed::Docs(vec![present("review.md", "h", false)]),
                0
            )),
            "gate_open_commit"
        );
        // Drop it; the id cannot be reused.
        assert!(
            apply(
                &mut a,
                Some(&p),
                &XapCmd::CommitDrop {
                    id: "C01".into(),
                    reason: None
                },
                Observed::Nothing,
                0
            )
            .unwrap()
        );
        assert!(a.gate.is_none());
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &propose("C01", &["a.rs"], &[]),
                Observed::Nothing,
                0
            )),
            "commit_id_used"
        );
    }

    #[test]
    fn commit_done_rules() {
        let p = pack();
        let mut a = agent();
        apply(
            &mut a,
            Some(&p),
            &propose("C01", &["b.rs", "a.rs"], &[]),
            Observed::Nothing,
            0,
        )
        .unwrap();
        let done = |sha: &str| XapCmd::CommitDone {
            id: "C01".into(),
            sha: sha.into(),
        };
        let fact = |sha: Option<&str>, on_branch: bool, files: &[&str]| {
            Observed::Commit(CommitFact {
                sha: sha.map(str::to_string),
                on_branch,
                files: files.iter().map(|s| s.to_string()).collect(),
            })
        };
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &done("abc"),
                fact(Some("abc1"), true, &["a.rs", "b.rs"]),
                0
            )),
            "commit_not_approved"
        );
        a.commits[0].status = CommitStatus::Approved;
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &done("abc"),
                fact(None, false, &[]),
                0
            )),
            "sha_unknown"
        );
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &done("abc"),
                fact(Some("abc1"), false, &["a.rs", "b.rs"]),
                0
            )),
            "sha_unknown"
        );
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &done("abc"),
                fact(Some("abc1"), true, &["a.rs"]),
                0
            )),
            "sha_files_mismatch"
        );
        assert!(
            apply(
                &mut a,
                Some(&p),
                &done("abc"),
                fact(Some("abc1"), true, &["b.rs", "a.rs"]),
                0
            )
            .unwrap()
        );
        assert_eq!(a.commits[0].sha.as_deref(), Some("abc1"));
        assert!(a.gate.is_none());
        assert!(!apply(&mut a, Some(&p), &done("abc"), Observed::Nothing, 0).unwrap());
        assert_eq!(
            code(apply(&mut a, Some(&p), &done("fff"), Observed::Nothing, 0)),
            "commit_done"
        );
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &XapCmd::CommitDone {
                    id: "C9".into(),
                    sha: "a".into()
                },
                Observed::Nothing,
                0
            )),
            "commit_unknown"
        );
    }

    #[test]
    fn pr_handoff_issue() {
        let p = pack();
        let mut a = agent();
        let pr = XapCmd::PrOpened {
            url: "https://github.com/o/r/pull/1".into(),
        };
        assert!(apply(&mut a, Some(&p), &pr, Observed::Nothing, 0).unwrap());
        assert!(!apply(&mut a, Some(&p), &pr, Observed::Nothing, 0).unwrap());
        assert_eq!(
            code(apply(
                &mut a,
                Some(&p),
                &XapCmd::PrOpened {
                    url: "ftp://x".into()
                },
                Observed::Nothing,
                0
            )),
            "bad_url"
        );
        let h = XapCmd::HandoffWritten { file: None };
        assert_eq!(
            code(apply(&mut a, Some(&p), &h, Observed::Exists(false), 0)),
            "doc_missing"
        );
        assert!(apply(&mut a, Some(&p), &h, Observed::Exists(true), 7).unwrap());
        assert_eq!(a.handoff_at, Some(7));
    }
}
