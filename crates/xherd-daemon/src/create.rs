//! `xherd new` / `POST /agents` (spec §5.1–5.2): validate everything first,
//! then create the worktree and `.xherd/`, then register the agent. A failure
//! after the worktree exists removes it again, so nothing is left behind.

use std::path::{Path, PathBuf};

use xherd_core::new::{self, FLOW_NONE, NewAgent};
use xherd_core::paths::write_atomic;
use xherd_core::xap::Refusal;
use xherd_core::{Agent, Status};
use xherd_git::Git;

use crate::{ActionError, Daemon};

fn refuse(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

impl From<Refusal> for ActionError {
    fn from(r: Refusal) -> ActionError {
        refuse(r.code, r.message)
    }
}

/// Last path component, for `worktrees/<repo>/<agent>`.
fn repo_dir_name(repo: &Path) -> String {
    repo.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into())
}

impl Daemon {
    /// Create an agent (R-001..R-005c). It is saved as `queued` (R-003).
    pub async fn new_agent(&self, req: NewAgent) -> Result<Agent, ActionError> {
        let c = new::check(&req)?;
        let pack = (c.flow != FLOW_NONE)
            .then(|| self.packs.get(xherd_flow::XHSPEC))
            .flatten();
        if c.flow != FLOW_NONE && pack.and_then(|p| p.flow(&c.flow)).is_none() {
            return Err(refuse(
                "unknown_flow",
                format!("pack {} has no flow {:?}", xherd_flow::XHSPEC, c.flow),
            ));
        }

        let git = Git::new("git");
        let given = Path::new(req.repo.trim());
        let repo = match git.toplevel(given).await {
            Ok(r) if given.is_dir() => r,
            _ => {
                return Err(refuse(
                    "not_a_repo",
                    format!("{} is not a git repository", given.display()),
                ));
            }
        };
        let agents = self
            .store
            .list_agents()
            .await
            .map_err(|e| refuse("store", e.to_string()))?;
        if agents.iter().any(|a| a.name == c.name) {
            return Err(refuse("name_taken", format!("agent {} exists", c.name)));
        }
        if git.branch_exists(&repo, &c.name).await {
            return Err(refuse(
                "branch_exists",
                format!("branch {} already exists in {}", c.name, repo.display()),
            ));
        }
        let workdir = self
            .config
            .workspace_root()
            .join("worktrees")
            .join(repo_dir_name(&repo))
            .join(&c.name);
        if workdir.exists() {
            return Err(refuse(
                "path_exists",
                format!("{} already exists", workdir.display()),
            ));
        }
        let base = resolve_base(&git, &repo, req.base.as_deref()).await?;

        // Everything checked; create.
        if let Some(parent) = workdir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| refuse("io", format!("{}: {e}", parent.display())))?;
        }
        git.worktree_add(&repo, &workdir, &c.name, &base)
            .await
            .map_err(|e| refuse("git", e.to_string()))?;

        let mut a = Agent::new(&c.name, &c.ticket_key, self.clock.now());
        a.workdir = Some(workdir.to_string_lossy().into_owned());
        a.repo = Some(repo.to_string_lossy().into_owned());
        a.base = Some(base);
        a.runtime = Some(xherd_flow::approve::DEFAULT_RUNTIME.into());
        a.next_prompt = Some(match pack {
            Some(p) => {
                let start = &p.flow(&c.flow).expect("checked above").start;
                p.command(
                    xherd_flow::approve::DEFAULT_RUNTIME,
                    start,
                    &[
                        ("key", &c.ticket_key),
                        ("prompt", &c.prompt),
                        ("commit", ""),
                    ],
                )
                .map_err(|e| refuse("flow", e))?
                .trim_end()
                .to_string()
            }
            None => c.prompt.clone(),
        });
        if pack.is_some() {
            a.pack = Some(xherd_flow::XHSPEC.into());
            a.flow = Some(c.flow.clone());
        }

        let res = async {
            prepare_xherd_dir(&git, &workdir, &a).await?;
            self.create_agent(a).await
        }
        .await;
        if res.is_err() {
            // Leave nothing behind (R-002).
            let _ = git.worktree_remove(&repo, &workdir, true).await;
            let _ = git.run(&repo, &["branch", "-D", &c.name]).await;
        }
        let a = res?;
        debug_assert_eq!(a.status, Status::Queued);
        Ok(a)
    }
}

/// Given base, else the remote's default branch, else the current branch.
/// A plain name that only exists on `origin` resolves to `origin/<name>` (R-002).
async fn resolve_base(git: &Git, repo: &Path, given: Option<&str>) -> Result<String, ActionError> {
    let given = given.map(str::trim).filter(|b| !b.is_empty());
    let candidates: Vec<String> = match given {
        Some(b) => vec![b.to_string(), format!("origin/{b}")],
        None => git
            .remote_default_branch(repo, "origin")
            .await
            .into_iter()
            .chain(git.current_branch(repo).await)
            .collect(),
    };
    for c in &candidates {
        if git.rev_parse(repo, c).await.is_ok() {
            return Ok(c.clone());
        }
    }
    Err(refuse(
        "base_unresolved",
        match given {
            Some(b) => format!("base branch {b:?} not found in {}", repo.display()),
            None => format!(
                "cannot tell the default branch of {}; pass --base",
                repo.display()
            ),
        },
    ))
}

/// `.xherd/` with `agent.json`, `instructions.md` and `spec/`, excluded from
/// git (R-005b exclude part, R-005c).
async fn prepare_xherd_dir(git: &Git, workdir: &Path, a: &Agent) -> Result<(), ActionError> {
    let io = |p: &Path, e: std::io::Error| refuse("io", format!("{}: {e}", p.display()));
    let dir: PathBuf = workdir.join(".xherd");
    let spec = workdir.join(xherd_flow::xap::SPEC_DIR);
    std::fs::create_dir_all(&spec).map_err(|e| io(&spec, e))?;
    let json = serde_json::to_vec_pretty(&new::agent_json(a)).expect("json");
    let f = dir.join("agent.json");
    write_atomic(&f, &json, None).map_err(|e| io(&f, e))?;
    let f = dir.join("instructions.md");
    write_atomic(&f, new::instructions_md(a).as_bytes(), None).map_err(|e| io(&f, e))?;
    git.add_excludes(workdir, &[".xherd/"])
        .await
        .map_err(|e| refuse("git", e.to_string()))
}
