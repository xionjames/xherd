//! `xherd new` / `POST /agents` (spec §5.1–5.2): validate everything first,
//! then create the worktree and `.xherd/`, then register the agent. A failure
//! after the worktree exists removes it again, so nothing is left behind.

use std::path::{Path, PathBuf};

use xherd_core::new::{self, FLOW_NONE, NewAgent};
use xherd_core::paths::write_atomic;
use xherd_core::xap::Refusal;
use xherd_core::{Agent, Status};
use xherd_git::Git;
use xherd_plugin::runtime::{self, Terminal};

use crate::{ActionError, Daemon};

pub(crate) fn refuse(code: &'static str, message: impl Into<String>) -> ActionError {
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

        let rt_name = xherd_flow::approve::DEFAULT_RUNTIME;
        let (label, rt) = self
            .plugins
            .get(rt_name)
            .filter(|_| self.plugin_enabled(rt_name))
            .ok_or_else(|| format!("runtime plugin {rt_name} is not installed or is switched off"))
            .and_then(|p| Ok((p.manifest.label.clone(), Terminal::of(&p.manifest)?)))
            .map_err(|e| refuse("runtime_unusable", e))?;
        let launch = rt
            .argv(runtime::DEFAULT_PERMISSION_MODE)
            .map_err(|e| refuse("runtime_unusable", e))?;

        let git = Git::new("git");
        // A URL is cloned (or fetched) here, before the other checks: the
        // clone is kept for the next agent even if a later check fails.
        let repo = self.locate_repo(&req.repo).await?.path;
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
        a.runtime = Some(rt_name.into());
        a.launch = Some(launch);
        a.next_prompt = Some(match pack {
            Some(p) => {
                let start = &p.flow(&c.flow).expect("checked above").start;
                p.command(
                    rt_name,
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
            if let Some(t) = &rt.trust {
                let w = TrustWriter {
                    trust: t.clone(),
                    root: self.config.workspace_root().join("worktrees"),
                };
                w.ensure(&workdir)
                    .map_err(|e| refuse("runtime_trust", format!("{label}: {e}")))?;
            }
            prepare_xherd_dir(&git, &workdir, &a).await?;
            if let Some(h) = &rt.hooks {
                install_hooks(&git, &workdir, h, &self.xherd_bin).await?;
            }
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

/// Marks agent worktrees as trusted in the runtime's trust file (declared by
/// its plugin), so no agent stops at the runtime's trust dialog before any
/// hook runs. Only folders under the worktrees root are ever written.
#[derive(Debug, Clone)]
pub struct TrustWriter {
    pub trust: runtime::Trust,
    /// `<workspace>/worktrees`.
    pub root: PathBuf,
}

impl TrustWriter {
    pub fn ensure(&self, dir: &Path) -> Result<(), String> {
        if !dir.starts_with(&self.root) || dir == self.root {
            return Err(format!(
                "{} is not an agent worktree under {}",
                dir.display(),
                self.root.display()
            ));
        }
        let file = xherd_flow::install::expand(&self.trust.file, &|v| std::env::var(v).ok())?;
        let text = std::fs::read_to_string(&file).ok();
        match self.trust.with_trusted(text.as_deref(), dir) {
            Ok(Some(new)) => write_atomic(Path::new(&file), new.as_bytes(), Some(0o600))
                .map_err(|e| format!("{file}: {e}")),
            Ok(None) => Ok(()),
            Err(e) => Err(format!("{file}: {e}")),
        }
    }
}

/// In the container only the workspace and `XHERD_EXTRA_PATHS` are mounted
/// at the same path; a repository elsewhere is refused with the mount to add
/// (R-605).
pub(crate) fn outside_mounts(
    repo: &Path,
    workspace: &Path,
    extra: &str,
) -> Result<(), ActionError> {
    let roots: Vec<PathBuf> = std::iter::once(workspace.to_path_buf())
        .chain(std::env::split_paths(extra).filter(|p| p.is_absolute()))
        .collect();
    if roots.iter().any(|r| repo.starts_with(r)) {
        return Ok(());
    }
    Err(refuse(
        "repo_outside_workspace",
        format!(
            "{} is not inside the workspace {}: clone it there, or mount its folder at the \
             same path with `xherd docker init --force --extra-path <folder>` and restart the container",
            repo.display(),
            workspace.display()
        ),
    ))
}

/// Given base, else the remote's default branch, else the current branch.
/// A plain name that only exists on `origin` resolves to `origin/<name>` (R-002).
pub(crate) async fn resolve_base(
    git: &Git,
    repo: &Path,
    given: Option<&str>,
) -> Result<String, ActionError> {
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

/// Make the runtime report to xherd (R-005b): merge xherd's tagged hook
/// entries into the runtime's settings file and keep that file out of git.
async fn install_hooks(
    git: &Git,
    workdir: &Path,
    h: &runtime::HookFile,
    xherd_bin: &str,
) -> Result<(), ActionError> {
    let f = workdir.join(&h.file);
    let existing = std::fs::read_to_string(&f).ok();
    let text = runtime::merge_hooks(existing.as_deref(), h, xherd_bin)
        .map_err(|e| refuse("runtime_hooks", e))?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| refuse("io", format!("{}: {e}", dir.display())))?;
    }
    write_atomic(&f, text.as_bytes(), None)
        .map_err(|e| refuse("io", format!("{}: {e}", f.display())))?;
    git.add_excludes(workdir, &[&h.file])
        .await
        .map_err(|e| refuse("git", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repos_must_be_inside_a_mount() {
        let ws = Path::new("/Users/me/xherd");
        assert!(outside_mounts(Path::new("/Users/me/xherd/repos/app"), ws, "").is_ok());
        assert!(outside_mounts(Path::new("/Volumes/code/app"), ws, "/Volumes/code").is_ok());
        let e = outside_mounts(Path::new("/Users/me/src/app"), ws, "/Volumes/code").unwrap_err();
        assert_eq!(e.code, "repo_outside_workspace");
        assert!(e.message.contains("--extra-path"));
        // A sibling with a common prefix is not inside.
        assert!(outside_mounts(Path::new("/Users/me/xherd2/app"), ws, "").is_err());
    }
}
