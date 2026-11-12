//! `xherd new` / `POST /agents` (spec §5.1–5.2): validate everything first,
//! then create the worktree and `.xherd/`, then register the agent. A failure
//! after the worktree exists removes it again, so nothing is left behind.

use std::path::{Path, PathBuf};

use xherd_core::new::{self, FLOW_NONE, NewAgent};
use xherd_core::paths::write_atomic;
use xherd_core::permission::PermissionMode;
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

/// The runtime's launch argv for a tool-permission mode (R-002a): the
/// runtime must offer it, and `all` only runs in the container (T4).
pub(crate) fn mode_launch(
    rt: &Terminal,
    mode: PermissionMode,
    sandbox: bool,
) -> Result<Vec<String>, ActionError> {
    if !mode.allowed(sandbox) {
        return Err(refuse(
            "mode_unavailable",
            "permission mode all asks nothing, so it is only offered in the container",
        ));
    }
    rt.argv(mode.as_str())
        .map_err(|e| refuse("mode_unavailable", e))
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
        if req.autonomy.is_some_and(|a| !a.selectable()) {
            return Err(refuse(
                "autonomy_unavailable",
                "autonomy never is not available yet (v1.x, R-1209)",
            ));
        }
        let pack = (c.flow != FLOW_NONE)
            .then(|| self.packs.get(xherd_flow::BUILTIN))
            .flatten();
        if c.flow != FLOW_NONE && pack.and_then(|p| p.flow(&c.flow)).is_none() {
            return Err(refuse(
                "unknown_flow",
                format!("pack {} has no flow {:?}", xherd_flow::BUILTIN, c.flow),
            ));
        }

        let git = Git::new("git");
        // A URL is cloned (or fetched) here, before the other checks: the
        // clone is kept for the next agent even if a later check fails.
        let repo = self.locate_repo(&req.repo).await?.path;
        // Request, then profile, then Settings (R-002, R-109, R-135).
        let profile = self.profile_for_repo(&repo).await;
        let rt_name = req
            .runtime
            .clone()
            .or(profile.as_ref().and_then(|p| p.runtime.clone()))
            .unwrap_or(self.live_config().default_runtime);
        let rt_name = rt_name.as_str();
        let rt = self.runtime_terminal(rt_name)?;
        if let Some(p) = pack
            && !p.install.contains_key(rt_name)
        {
            return Err(refuse(
                "runtime_unusable",
                format!("the {} pack has no commands for {rt_name}", p.name),
            ));
        }
        let autonomy = req
            .autonomy
            .or(profile.as_ref().and_then(|p| p.autonomy))
            .unwrap_or(self.live_config().autonomy);
        let mode = req
            .permission_mode
            .or(profile.as_ref().and_then(|p| p.permission_mode))
            .unwrap_or(self.live_config().permission_mode);
        let launch = mode_launch(&rt, mode, self.sandbox)?;
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
        let base = resolve_base(
            &git,
            &repo,
            req.base
                .as_deref()
                .or(profile.as_ref().and_then(|p| p.base_branch.as_deref())),
        )
        .await?;

        // Everything checked; create.
        if let Some(parent) = workdir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| refuse("io", format!("{}: {e}", parent.display())))?;
        }
        git.worktree_add(&repo, &workdir, &c.name, &base)
            .await
            .map_err(|e| refuse("git", e.to_string()))?;

        let mut a = Agent::new(&c.name, &c.ticket_key, self.clock.now());
        a.autonomy = autonomy;
        a.permission_mode = Some(mode);
        a.profile = profile.as_ref().map(|p| p.id.clone());
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
            a.pack = Some(xherd_flow::BUILTIN.into());
            a.flow = Some(c.flow.clone());
        }

        let res = async {
            self.prepare_worktree(&a).await?;
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

impl Daemon {
    /// What every agent worktree needs besides the checkout: the runtime's
    /// trust entry, `.xherd/` (R-005c), the pre-push guard (R-1204) and the
    /// runtime's hook file (R-005b). Idempotent; also finishes a worktree that
    /// a restore created (`repair_restored`).
    pub(crate) async fn prepare_worktree(&self, a: &Agent) -> Result<(), ActionError> {
        let workdir = PathBuf::from(
            a.workdir
                .as_deref()
                .ok_or_else(|| refuse("io", format!("agent {} has no worktree", a.name)))?,
        );
        let rt_name = a
            .runtime
            .as_deref()
            .unwrap_or(xherd_flow::approve::DEFAULT_RUNTIME);
        let (label, rt) = self
            .plugins
            .get(rt_name)
            .filter(|_| self.plugin_enabled(rt_name))
            .ok_or_else(|| format!("runtime plugin {rt_name} is not installed or is switched off"))
            .and_then(|p| Ok((p.manifest.label.clone(), Terminal::of(&p.manifest)?)))
            .map_err(|e| refuse("runtime_unusable", e))?;
        let pack = a
            .pack
            .is_some()
            .then(|| self.packs.get(xherd_flow::BUILTIN))
            .flatten();
        let git = Git::new("git");
        if let Some(t) = &rt.trust {
            let w = TrustWriter {
                trust: t.clone(),
                root: self.config.workspace_root().join("worktrees"),
            };
            w.ensure(&workdir)
                .await
                .map_err(|e| refuse("runtime_trust", format!("{label}: {e}")))?;
        }
        let repo_instructions = self.profile_of(a).await.and_then(|p| p.instructions);
        prepare_xherd_dir(&git, &workdir, a, repo_instructions.as_deref()).await?;
        git.install_push_guard(
            &workdir,
            &workdir.join(".xherd/hooks"),
            &self.xherd_bin,
            &a.name,
        )
        .await
        .map_err(|e| refuse("push_guard", e.to_string()))?;
        if let Some(h) = &rt.hooks {
            let allow = permission_rules(pack, rt_name);
            install_hooks(&git, &workdir, h, &self.xherd_bin, &allow).await?;
        }
        Ok(())
    }

    /// A runtime plugin agents can be started with: installed, of kind
    /// runtime, switched on, with a usable terminal driver.
    pub(crate) fn runtime_terminal(&self, name: &str) -> Result<Terminal, ActionError> {
        self.plugins
            .get(name)
            .filter(|p| p.manifest.kind == xherd_plugin::Kind::Runtime)
            .filter(|_| self.plugin_enabled(name))
            .ok_or_else(|| format!("runtime plugin {name} is not installed or is switched off"))
            .and_then(|p| Terminal::of(&p.manifest))
            .map_err(|e| refuse("runtime_unusable", e))
    }

    /// A worktree a backup restore recreated has no `.xherd/agent.json` (it is
    /// not in the backup): finish it like a new agent's, once, at daemon start.
    pub(crate) async fn repair_restored(&self) {
        let Ok(agents) = self.store.list_agents().await else {
            return;
        };
        for a in agents {
            let Some(w) = a.workdir.as_deref().map(PathBuf::from) else {
                continue;
            };
            let live = !matches!(a.status, Status::Done | Status::Cancelled);
            if live && w.join(".xherd").is_dir() && !w.join(".xherd/agent.json").exists() {
                match self.prepare_worktree(&a).await {
                    Ok(()) => tracing::info!(agent = %a.name, "restored worktree prepared"),
                    Err(e) => {
                        tracing::warn!(agent = %a.name, error = %e.message, "restored worktree not prepared")
                    }
                }
            }
        }
    }
}

/// What the daemon uses of one runtime plugin besides launching it: built
/// once at start for every runtime plugin, chosen by the agent's runtime.
#[derive(Debug, Clone, Default)]
pub struct RuntimeKit {
    pub trust: Option<TrustWriter>,
    pub resume: Option<Resumer>,
    pub usage: Option<runtime::Usage>,
    /// `promptArgs`: the first prompt goes on the launch line (empty: typed).
    pub prompt_args: Vec<String>,
}

/// Every runtime plugin's [`RuntimeKit`], by plugin name.
pub fn runtime_kits(
    plugins: &xherd_plugin::Registry,
    worktrees: &Path,
) -> std::collections::BTreeMap<String, RuntimeKit> {
    plugins
        .of_kind(xherd_plugin::Kind::Runtime)
        .filter_map(|p| {
            let t = Terminal::of(&p.manifest).ok()?;
            let kit = RuntimeKit {
                trust: t.trust.map(|trust| TrustWriter {
                    trust,
                    root: worktrees.to_path_buf(),
                }),
                resume: t.resume.map(Resumer),
                usage: t.usage,
                prompt_args: t.prompt_args,
            };
            Some((p.manifest.name.clone(), kit))
        })
        .collect()
}

/// Longest export xherd reads: a long session with large tool outputs.
const EXPORT_MAX: usize = 64 * 1024 * 1024;

/// Continue the runtime's last conversation (R-027): the launch arguments for
/// a session id, only while the runtime still has that session's transcript.
#[derive(Debug, Clone)]
pub struct Resumer(pub runtime::Resume);

impl Resumer {
    pub async fn argv(
        &self,
        session: &str,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Option<Vec<String>> {
        let args = self.0.argv(session)?;
        let found = match self.0.export.is_empty() {
            true => self.transcript_file(session, env).await.is_some(),
            // Only whether the runtime still has it: the text is not needed.
            false => self.export(session, "/dev/null").await.is_some(),
        };
        found.then_some(args)
    }

    /// The transcript of `session` as text, when the runtime has it: its
    /// file, or the export's output. An export to `{file}` writes into
    /// `scratch` (the worktree's `.xherd/`, shared by the agent user and the
    /// daemon, R-1203); without one it cannot be read.
    pub async fn read(
        &self,
        session: &str,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
        scratch: Option<&Path>,
    ) -> Option<String> {
        if self.0.exports_to_file() {
            let file = scratch?.join(format!("runtime-export-{}.json", xherd_core::new_id()));
            let f = file.to_string_lossy();
            let ran = self.export(session, &f).await;
            let text = match ran {
                Some(_) => xherd_core::runner::read_file(&file).await.ok().flatten(),
                None => None,
            };
            let _ = std::fs::remove_file(&file);
            return text.map(|b| String::from_utf8_lossy(&b).into_owned());
        }
        if !self.0.export.is_empty() {
            return self.export(session, "").await;
        }
        let path = self.transcript_file(session, env).await?;
        match xherd_core::runner::read_file(&path).await {
            Ok(Some(b)) => Some(String::from_utf8_lossy(&b).into_owned()),
            _ => None,
        }
    }

    /// The transcript file of `session`.
    ///
    /// Looked up as the agent user: the runtime keeps it in the agent's home
    /// (R-1203).
    async fn transcript_file(
        &self,
        session: &str,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Option<PathBuf> {
        runtime::plain_id(session).then_some(())?;
        let (Some(dir), Some(file)) = (&self.0.transcripts, &self.0.file) else {
            return None;
        };
        let dir = xherd_flow::install::expand(dir, env).ok()?;
        let file = file.replace("{session}", session);
        xherd_core::runner::find_in_subdirs_as_agent(Path::new(&dir), &file, self.0.depth).await
    }

    /// The export command's output, run as the agent user (its sessions are
    /// in the agent's home); `None` when it fails, i.e. no such session.
    async fn export(&self, session: &str, file: &str) -> Option<String> {
        let argv = self.0.export_argv(session, file)?;
        let mut cmd = xherd_core::exec::Cmd::new(argv)
            .timeout(std::time::Duration::from_secs(60))
            .as_agent();
        cmd.max_output = EXPORT_MAX;
        match xherd_core::exec::run(cmd).await {
            Ok(o) if o.code == Some(0) => Some(String::from_utf8_lossy(&o.stdout).into_owned()),
            Ok(o) => {
                tracing::debug!(session, code = ?o.code, stderr = %o.stderr, "runtime export failed");
                None
            }
            Err(e) => {
                tracing::debug!(session, error = %e, "runtime export failed");
                None
            }
        }
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
    pub async fn ensure(&self, dir: &Path) -> Result<(), String> {
        if !dir.starts_with(&self.root) || dir == self.root {
            return Err(format!(
                "{} is not an agent worktree under {}",
                dir.display(),
                self.root.display()
            ));
        }
        // The runtime's file is in the agent user's home: read and written as
        // that user (R-1203).
        let file = xherd_flow::install::expand(&self.trust.file, &xherd_core::runner::agent_env)?;
        let text = xherd_core::runner::read_file(Path::new(&file))
            .await
            .ok()
            .flatten()
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        match self.trust.with_trusted(text.as_deref(), dir) {
            Ok(Some(new)) => {
                xherd_core::runner::write_file(Path::new(&file), new.as_bytes(), Some(0o600))
                    .await
                    .map_err(|e| format!("{file}: {e}"))
            }
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

/// setgid, group-writable: the folders the daemon and the agent share.
const SHARED_DIR_MODE: u32 = 0o2775;

/// `.xherd/` with `agent.json`, `instructions.md` and `spec/`, excluded from
/// git (R-005b exclude part, R-005c).
async fn prepare_xherd_dir(
    git: &Git,
    workdir: &Path,
    a: &Agent,
    repo_instructions: Option<&str>,
) -> Result<(), ActionError> {
    let io = |p: &Path, e: std::io::Error| refuse("io", format!("{}: {e}", p.display()));
    let dir: PathBuf = workdir.join(".xherd");
    let spec = workdir.join(xherd_flow::xap::SPEC_DIR);
    // Made as the agent user and shared with the daemon's group: both write
    // in here (R-1203); the daemon's own files stay out of agents' reach.
    for d in [&dir, &spec] {
        xherd_core::runner::mkdir(d, SHARED_DIR_MODE)
            .await
            .map_err(|e| io(d, e))?;
    }
    let json = serde_json::to_vec_pretty(&new::agent_json(a)).expect("json");
    let f = dir.join("agent.json");
    write_atomic(&f, &json, None).map_err(|e| io(&f, e))?;
    let f = dir.join("instructions.md");
    write_atomic(
        &f,
        new::instructions_md(a, repo_instructions).as_bytes(),
        None,
    )
    .map_err(|e| io(&f, e))?;
    git.add_excludes(workdir, &[".xherd/"])
        .await
        .map_err(|e| refuse("git", e.to_string()))
}

/// The XAP calls an agent makes, as shell command patterns. The prompts call
/// plain `xherd` (on the session's PATH), which the runtime can match; a
/// `$XHERD_BIN` variable cannot be checked and asks every time.
const XAP_ALLOW: &[&str] = &[
    "xherd stage *",
    "xherd gate *",
    "xherd ask *",
    "xherd tasks *",
    "xherd task *",
    "xherd commit *",
    "xherd pr *",
    "xherd handoff *",
    "xherd issue *",
    "xherd memory *",
];

/// What the runtime allows without asking in a new worktree: reading the
/// pack's installed commands and the XAP calls. This is where profile and
/// toolchain commands will be added.
fn permission_rules(pack: Option<&xherd_flow::Pack>, runtime: &str) -> runtime::Allow {
    runtime::Allow {
        commands: XAP_ALLOW.iter().map(|s| s.to_string()).collect(),
        read_dirs: pack.map(|p| command_dirs(p, runtime)).unwrap_or_default(),
    }
}

/// Folders the runtime's pack commands are installed in (outside the
/// worktree, read by every command: `rules.md`, `next.md`…). Empty when the
/// plan cannot be made (no HOME).
fn command_dirs(pack: &xherd_flow::Pack, runtime: &str) -> Vec<String> {
    let env = xherd_core::runner::agent_env;
    let plan = pack
        .plan_install(runtime, xherd_flow::install::BUILTIN_FILES, &env)
        .unwrap_or_default();
    let mut dirs: Vec<String> = plan
        .iter()
        .filter_map(|f| f.path.parent())
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    dirs.sort();
    dirs.dedup();
    dirs
}

/// Make the runtime report to xherd (R-005b): merge xherd's tagged hook
/// entries into the runtime's settings file and keep that file out of git.
async fn install_hooks(
    git: &Git,
    workdir: &Path,
    h: &runtime::HookFile,
    xherd_bin: &str,
    allow: &runtime::Allow,
) -> Result<(), ActionError> {
    let f = workdir.join(&h.file);
    let existing = xherd_core::runner::read_file(&f)
        .await
        .ok()
        .flatten()
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    let text = runtime::hook_file_text(existing.as_deref(), h, xherd_bin, allow)
        .map_err(|e| refuse("runtime_hooks", e))?;
    xherd_core::runner::write_file(&f, text.as_bytes(), None)
        .await
        .map_err(|e| refuse("io", format!("{}: {e}", f.display())))?;
    git.add_excludes(workdir, &[&h.file])
        .await
        .map_err(|e| refuse("git", e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_rules_are_the_command_reads_and_the_xap_calls() {
        let xap: Vec<String> = XAP_ALLOW.iter().map(|s| s.to_string()).collect();
        let none = permission_rules(None, "claude-code");
        assert_eq!((none.commands, none.read_dirs.len()), (xap.clone(), 0));
        let pack = xherd_flow::builtin();
        for rt in ["claude-code", "opencode", "codex"] {
            let rules = permission_rules(Some(&pack), rt);
            assert_eq!(rules.commands, xap);
            assert_eq!(rules.read_dirs, command_dirs(&pack, rt));
        }
        let dirs = |rt| command_dirs(&pack, rt);
        assert_ne!(
            dirs("claude-code"),
            dirs("opencode"),
            "each runtime's own folder"
        );
    }

    #[tokio::test]
    async fn resume_needs_the_transcript_and_a_plain_id() {
        let home = tempfile::tempdir().unwrap();
        let proj = home.path().join("projects/-work-TST-1");
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join("abc-123.jsonl"), "{}").unwrap();
        let r = Resumer(runtime::Resume {
            args: vec!["--resume".into(), "{session}".into()],
            transcripts: Some(format!("{}/projects", home.path().display())),
            file: Some("{session}.jsonl".into()),
            depth: 1,
            export: Vec::new(),
            prompt_env: Default::default(),
            prompt_args: Vec::new(),
        });
        let env = |_: &str| None;
        assert_eq!(
            r.argv("abc-123", &env).await,
            Some(vec!["--resume".to_string(), "abc-123".to_string()])
        );
        assert_eq!(r.argv("missing", &env).await, None);
        assert_eq!(r.argv("../abc-123", &env).await, None);
        assert_eq!(r.argv("", &env).await, None);
    }

    #[tokio::test]
    async fn resume_through_an_export_command() {
        // Prints the session's transcript for `s-1` only, like `opencode export`.
        let r = Resumer(runtime::Resume {
            args: vec!["--session".into(), "{session}".into()],
            transcripts: None,
            file: None,
            depth: 1,
            prompt_args: Vec::new(),
            export: vec![
                "sh".into(),
                "-c".into(),
                r#"test "$0" = s-1 && echo '{"info": {"tokens": {"input": 3}}}'"#.into(),
                "{session}".into(),
            ],
            prompt_env: Default::default(),
        });
        let env = |_: &str| None;
        assert_eq!(
            r.argv("s-1", &env).await,
            Some(vec!["--session".to_string(), "s-1".to_string()])
        );
        assert_eq!(r.argv("s-2", &env).await, None, "export failed: gone");
        assert!(
            r.read("s-1", &env, None)
                .await
                .unwrap()
                .contains("\"input\": 3")
        );
        assert_eq!(r.read("s 1", &env, None).await, None, "odd id never runs");

        // An export to a file, in the scratch folder, removed after reading.
        let dir = tempfile::tempdir().unwrap();
        let r = Resumer(runtime::Resume {
            export: vec![
                "sh".into(),
                "-c".into(),
                r#"test "$0" = s-1 && echo '{"ok": 1}' > "$1""#.into(),
                "{session}".into(),
                "{file}".into(),
            ],
            ..r.0
        });
        assert!(
            r.argv("s-1", &env).await.is_some(),
            "checked against /dev/null"
        );
        assert_eq!(r.argv("s-2", &env).await, None);
        let text = r.read("s-1", &env, Some(dir.path())).await.unwrap();
        assert_eq!(text.trim(), r#"{"ok": 1}"#);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "removed");
        assert_eq!(r.read("s-1", &env, None).await, None, "no scratch folder");
    }

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
