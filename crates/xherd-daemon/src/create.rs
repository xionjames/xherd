//! `xherd new` / `POST /jobs` (spec §5.1–5.2): validate everything first,
//! then create the worktree and `.xherd/`, then register the agent. A failure
//! after the worktree exists removes it again, so nothing is left behind.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use xherd_core::agent_def::{AgentDef, SkillCopy, StageAgent};
use xherd_core::new::{self, FLOW_NONE, NewJob};
use xherd_core::paths::write_atomic;
use xherd_core::permission::PermissionMode;
use xherd_core::resolve::{Assignment, Effective};
use xherd_core::xap::Refusal;
use xherd_core::{Job, Status};
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

/// The runtime's launch argv for a tool-permission mode (R-002a) and a
/// model (issue #112, already checked): the runtime must offer both, and
/// `all` only runs in the container (T4).
pub(crate) fn mode_launch(
    rt: &Terminal,
    mode: PermissionMode,
    model: Option<&str>,
    sandbox: bool,
) -> Result<Vec<String>, ActionError> {
    if !mode.allowed(sandbox) {
        return Err(refuse(
            "mode_unavailable",
            "permission mode all asks nothing, so it is only offered in the container",
        ));
    }
    let mut argv = rt
        .argv(mode.as_str())
        .map_err(|e| refuse("mode_unavailable", e))?;
    if let Some(m) = model {
        argv.extend(
            rt.model_argv(m)
                .map_err(|e| refuse("model_unavailable", e))?,
        );
    }
    Ok(argv)
}

/// A model name from a request, checked (`None` = empty).
pub(crate) fn clean_model(m: Option<&str>) -> Result<Option<String>, ActionError> {
    xherd_core::model::clean(m.unwrap_or_default()).map_err(|e| refuse("bad_model", e))
}

/// Last path component, for `worktrees/<repo>/<agent>`.
fn repo_dir_name(repo: &Path) -> String {
    repo.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into())
}

/// The first branch of a new project.
const PROJECT_BRANCH: &str = "main";

/// Make the repository of a new project (R3): `git init` and one empty commit
/// in a hidden folder beside it, then a rename, so a failure never leaves a
/// half-made project at `repo`. The daemon makes `projects/`; git makes the
/// folder itself, as the agents' user who owns what is inside (R-1203).
async fn create_project(git: &Git, repo: &Path) -> Result<(), ActionError> {
    let parent = repo.parent().expect("project_path has a parent");
    std::fs::create_dir_all(parent)
        .map_err(|e| refuse("io", format!("{}: {e}", parent.display())))?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let name = repo_dir_name(repo);
    let tmp_name = format!(".{name}.creating-{nanos}");
    let tmp = parent.join(&tmp_name);
    let made = async {
        git.run(
            parent,
            &["init", "--quiet", "-b", PROJECT_BRANCH, &tmp_name],
        )
        .await?;
        // The managed git config has the identity; a bare one gets a stand-in
        // so the first commit never fails for lack of it.
        if git.run(&tmp, &["config", "user.email"]).await.is_err() {
            git.run(&tmp, &["config", "user.name", "xherd"]).await?;
            git.run(&tmp, &["config", "user.email", "xherd@localhost"])
                .await?;
        }
        git.commit_empty(&tmp, "Initial commit").await?;
        Ok::<(), xherd_git::GitError>(())
    }
    .await;
    if let Err(e) = made {
        crate::repos::remove_tree(&tmp).await;
        return Err(refuse("git", e.to_string()));
    }
    // Another creation of the same project may have won meanwhile: a
    // rename never replaces its folder (it holds `.git`), and the loser
    // gets `path_exists` (R3).
    let renamed = if repo.exists() {
        Err(None)
    } else {
        std::fs::rename(&tmp, repo).map_err(Some)
    };
    if let Err(e) = renamed {
        crate::repos::remove_tree(&tmp).await;
        return Err(match e {
            Some(e) if !repo.exists() => refuse("io", format!("{}: {e}", repo.display())),
            _ => refuse("path_exists", format!("{} already exists", repo.display())),
        });
    }
    Ok(())
}

/// Remove the `.<project>.creating-<nanos>` folders a crash left in
/// `projects/`; called at start, before any creation can run.
pub(crate) async fn remove_project_leftovers(workspace: &Path) {
    let dir = workspace.join(new::PROJECTS_DIR);
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && name.contains(".creating-") {
            tracing::info!(folder = %name, "removing a half-made project");
            crate::repos::remove_tree(&e.path()).await;
        }
    }
}

impl Daemon {
    /// A saved agent a job may start from: it exists, is not archived and
    /// its runtime (or its fallback) can run now (R15, R36).
    async fn usable_agent(&self, key: &str) -> Result<AgentDef, ActionError> {
        let def = self.agent_def(key).await?;
        if def.archived_at.is_some() {
            return Err(refuse(
                "agent_archived",
                format!("agent {} is archived: unarchive it first", def.name),
            ));
        }
        if let Some(why) = self.agent_broken(&def) {
            return Err(refuse("agent_broken", why));
        }
        Ok(def)
    }

    /// Create an agent (R-001..R-005c). It is saved as `queued` (R-003).
    pub async fn new_job(&self, mut req: NewJob) -> Result<Job, ActionError> {
        // The agent for stages with no assignment (`--agent A`, #141).
        let default_def = match req
            .agent
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            Some(key) => Some(self.usable_agent(key).await?),
            None => None,
        };
        // No repository yet: the one a ticket URL names (#148). An explicit
        // repository always wins; without any, `check` refuses.
        if req.repo.trim().is_empty()
            && req.new_project.is_none()
            && let Some(Ok(Some(input))) = req.ticket.as_deref().map(new::parse_ticket)
            && let Some(url) = self.ticket_repo(&input)
        {
            req.repo = url;
        }
        let c = new::check(&req)?;
        if req.autonomy.is_some_and(|a| !a.selectable()) {
            return Err(refuse(
                "autonomy_unavailable",
                "autonomy never is not available yet (v1.x, R-1209)",
            ));
        }
        let pack_name = xherd_flow::BUILTIN;
        let pack = (c.flow != FLOW_NONE)
            .then(|| self.packs.get(pack_name))
            .flatten();
        if c.flow != FLOW_NONE && pack.and_then(|p| p.flow(&c.flow)).is_none() {
            return Err(refuse(
                "unknown_flow",
                format!("pack {pack_name} has no flow {:?}", c.flow),
            ));
        }

        let git = Git::new("git");
        // A URL is cloned (or fetched) here, before the other checks: the
        // clone is kept for the next job even if a later check fails.
        // A new project has no repository yet: its folder is only computed
        // here and made right before the worktree (R1, R3).
        let repo = match &c.project {
            Some(p) => new::project_path(&self.config.workspace_root(), p),
            None => self.locate_repo(&req.repo).await?.path,
        };
        // Who runs each stage (D26, R37): the request's override, the
        // workflow's assignment, the default agent, the stage before.
        let stage_ids: Vec<&str> = pack
            .and_then(|p| p.flow(&c.flow))
            .map(|f| f.stages.iter().map(|s| s.id.as_str()).collect())
            .unwrap_or_default();
        let assignments: Vec<Assignment> = if pack.is_some() {
            self.store
                .flow_assignments(pack_name, &c.flow)
                .await
                .map_err(|e| refuse("store", e.to_string()))?
                .into_iter()
                .filter(|a| stage_ids.contains(&a.stage.as_str()))
                .collect()
        } else {
            Vec::new()
        };
        // The fresh-context check stages, copied now: later changes to the
        // workflow do not touch this job. Only stages of this flow that are
        // still eligible count.
        let fresh_checks: BTreeSet<String> = match pack.and_then(|p| p.flow(&c.flow)) {
            Some(f) => self
                .store
                .flow_fresh_checks(pack_name, &c.flow)
                .await
                .map_err(|e| refuse("store", e.to_string()))?
                .into_iter()
                .filter(|id| {
                    f.stages
                        .iter()
                        .any(|s| s.id == *id && s.fresh_check_eligible())
                })
                .collect(),
            None => BTreeSet::new(),
        };
        let mut overrides = BTreeMap::new();
        for (stage, key) in &req.agents {
            if !stage_ids.contains(&stage.as_str()) {
                return Err(refuse(
                    "unknown_stage",
                    format!("workflow {} has no stage {stage:?}", c.flow),
                ));
            }
            overrides.insert(stage.clone(), self.usable_agent(key).await?.id);
        }
        let default_id = default_def.as_ref().map(|d| d.id.clone());
        let per_stage = xherd_core::resolve::stage_agents(
            &stage_ids,
            &assignments,
            &overrides,
            default_id.as_deref(),
        );
        let mut stage_agents = BTreeMap::new();
        for (stage, id) in stage_ids.iter().zip(&per_stage) {
            if let Some(id) = id {
                let human = assignments
                    .iter()
                    .find(|a| a.stage == *stage)
                    .is_some_and(|a| a.human);
                stage_agents.insert(
                    stage.to_string(),
                    StageAgent {
                        agent: id.clone(),
                        human,
                    },
                );
            }
        }
        // The session starts on the first stage's agent; a plain job, or a
        // flow none job, on the default one.
        let current_id = if pack.is_some() {
            per_stage.first().cloned().flatten()
        } else {
            default_id.clone()
        };
        let mut defs: BTreeMap<String, AgentDef> = BTreeMap::new();
        for id in per_stage.iter().flatten().chain(default_id.iter()) {
            if !defs.contains_key(id) {
                defs.insert(id.clone(), self.usable_agent(id).await?);
            }
        }

        // Request, agent, profile, Settings; the profile's autonomy and mode
        // are ceilings (R-002, R-109, R-135, #141 R12/R12a).
        let profile = self.profile_for_repo(&repo).await;
        let cfg = self.live_config();
        let model_req = clean_model(req.model.as_deref())?;
        let defaults = xherd_core::resolve::Defaults {
            runtime: cfg.default_runtime.clone(),
            autonomy: cfg.autonomy,
            mode: cfg.permission_mode,
            models: cfg.runtime_models.clone(),
        };
        let requested = |runtime: Option<String>| xherd_core::resolve::Requested {
            runtime,
            model: model_req.clone(),
            autonomy: req.autonomy,
            mode: req.permission_mode,
            base: req.base.clone(),
            flow: Some(c.flow.clone()),
        };
        let plain = xherd_core::resolve::effective(
            &defaults,
            profile.as_ref(),
            None,
            &requested(req.runtime.clone()),
        );
        // Every agent of the job, resolved once and kept (R37, R40): the
        // runtime (its own, else its fallback), autonomy, mode, model and
        // the launch line.
        let mut baked: BTreeMap<String, (xherd_core::agent_def::JobAgent, Effective)> =
            BTreeMap::new();
        for (id, def) in &defs {
            let (runtime, fell_back) = match req.runtime.as_deref().map(str::trim) {
                Some(r) if !r.is_empty() => (r.to_string(), false),
                _ => xherd_core::resolve::usable_runtime(
                    &def.runtime,
                    def.fallback_runtime.as_deref(),
                    |r| self.runtime_terminal(r).is_ok(),
                )
                .ok_or_else(|| {
                    refuse(
                        "agent_broken",
                        format!(
                            "agent {}: its runtime and its fallback cannot run",
                            def.name
                        ),
                    )
                })?,
            };
            let eff = xherd_core::resolve::effective(
                &defaults,
                profile.as_ref(),
                Some(def),
                &requested(Some(runtime)),
            );
            if !eff.autonomy.selectable() {
                return Err(refuse(
                    "autonomy_unavailable",
                    "autonomy never is not available yet (v1.x, R-1209)",
                ));
            }
            let term = self.runtime_terminal(&eff.runtime)?;
            let mut snap = def.snapshot();
            snap.runtime = eff.runtime.clone();
            snap.fell_back = fell_back;
            snap.autonomy = Some(eff.autonomy);
            snap.permission_mode = Some(eff.mode);
            snap.model = eff.model.clone();
            snap.launch = mode_launch(&term, eff.mode, eff.model.as_deref(), self.sandbox)?;
            // The skills it lists, not copied yet: the first stage's agent gets
            // them below, the others when the job switches to them (R39).
            snap.skills = def
                .skills
                .iter()
                .map(|n| SkillCopy {
                    name: n.clone(),
                    sha256: String::new(),
                    installed: false,
                    skipped: None,
                })
                .collect();
            // The memory as it is now; later edits do not reach this job (R28).
            snap.memory = self
                .store
                .agent_memory(id)
                .await
                .map_err(|e| refuse("store", e.to_string()))?
                .map(|(t, _)| t)
                .filter(|t| !t.trim().is_empty());
            baked.insert(id.clone(), (snap, eff));
        }
        // One runtime per job until R-025: a stage without an agent counts
        // when the session starts on it.
        let mut runtimes: Vec<&str> = baked.values().map(|(_, e)| e.runtime.as_str()).collect();
        if current_id.is_none() && !baked.is_empty() {
            runtimes.push(plain.runtime.as_str());
        }
        runtimes.sort_unstable();
        runtimes.dedup();
        if runtimes.len() > 1 {
            return Err(refuse(
                "mixed_runtime",
                format!(
                    "the agents of this job run on different runtimes ({}): a job keeps one",
                    runtimes.join(", ")
                ),
            ));
        }
        let cur = current_id.as_ref().and_then(|id| baked.get(id));
        let eff = cur.map_or(&plain, |(_, e)| e);
        if !eff.autonomy.selectable() {
            return Err(refuse(
                "autonomy_unavailable",
                "autonomy never is not available yet (v1.x, R-1209)",
            ));
        }
        let rt_name = eff.runtime.as_str();
        let rt = self.runtime_terminal(rt_name)?;
        self.runtime_ready(rt_name).await?;
        if let Some(p) = pack
            && !p.install.contains_key(rt_name)
        {
            return Err(refuse(
                "runtime_unusable",
                format!("the {} pack has no commands for {rt_name}", p.name),
            ));
        }
        let (autonomy, mode, model) = (eff.autonomy, eff.mode, eff.model.clone());
        let launch = match cur {
            Some((snap, _)) => snap.launch.clone(),
            None => mode_launch(&rt, mode, model.as_deref(), self.sandbox)?,
        };
        let agents = self
            .store
            .list_jobs()
            .await
            .map_err(|e| refuse("store", e.to_string()))?;
        if agents.iter().any(|a| a.name == c.name) {
            return Err(refuse("name_taken", format!("agent {} exists", c.name)));
        }
        // A ticket URL gets its key from the tracker that reads it (#114).
        let ticket = match &c.ticket {
            Some(t) => {
                let remote = crate::actor::origin_host(&repo).await;
                Some(self.ticket_of(t, remote.as_deref()).0)
            }
            None => None,
        };
        let branch = new::branch_name(ticket.as_ref().and_then(|t| t.key.as_deref()), &c.name);
        if c.project.is_some() && repo.exists() {
            return Err(refuse(
                "path_exists",
                format!("{} already exists", repo.display()),
            ));
        }
        if c.project.is_none() && git.branch_exists(&repo, &branch).await {
            return Err(refuse(
                "branch_exists",
                format!("branch {branch} already exists in {}", repo.display()),
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
        let base = if c.project.is_some() {
            PROJECT_BRANCH.to_string()
        } else {
            let base = resolve_base(&git, &repo, eff.base.as_deref()).await?;
            // A plain local base branch follows its origin counterpart when
            // that is a fast-forward; anything else leaves it as it is.
            let upstream = format!("origin/{base}");
            if git.branch_exists(&repo, &base).await
                && git.rev_parse(&repo, &upstream).await.is_ok()
                && let Err(e) = git.fast_forward_branch(&repo, &base, &upstream).await
            {
                tracing::warn!(base = %base, error = %e, "base branch not fast-forwarded");
            }
            base
        };

        // Everything checked; create. A project made here goes again when
        // anything after it fails (R6).
        if c.project.is_some() {
            create_project(&git, &repo).await?;
        }
        let made = async {
            if let Some(parent) = workdir.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| refuse("io", format!("{}: {e}", parent.display())))?;
            }
            git.worktree_add(&repo, &workdir, &branch, &base)
                .await
                .map_err(|e| refuse("git", e.to_string()))
        }
        .await;
        if let Err(e) = made {
            if c.project.is_some() {
                crate::repos::remove_tree(&repo).await;
            }
            return Err(e);
        }

        let mut a = Job::new(&c.name, self.clock.now());
        a.ticket = ticket;
        a.branch = branch.clone();
        a.autonomy = autonomy;
        a.permission_mode = Some(mode);
        a.model = model;
        a.profile = profile.as_ref().map(|p| p.id.clone());
        // R19: a snapshot of the profile's actions, approved; later edits of
        // the profile do not change this job.
        if let Some(p) = profile.as_ref() {
            a.actions = p
                .actions
                .iter()
                .map(xherd_core::action::from_profile)
                .collect();
        }
        a.workdir = Some(workdir.to_string_lossy().into_owned());
        a.repo = Some(repo.to_string_lossy().into_owned());
        a.base = Some(base);
        a.runtime = Some(rt_name.into());
        a.agents = baked
            .iter()
            .map(|(id, (snap, _))| (id.clone(), snap.clone()))
            .collect();
        a.stage_agents = stage_agents;
        a.fresh_checks = fresh_checks;
        a.default_agent = default_id;
        a.current_agent = current_id.clone();
        a.lowered = eff.lowered.clone();
        a.launch = Some(launch);
        a.next_prompt = Some(match pack {
            Some(p) => {
                let start = &p.flow(&c.flow).expect("checked above").start;
                p.command(
                    rt_name,
                    start,
                    &[
                        ("key", a.ticket_ref()),
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
            a.pack = Some(pack_name.into());
            a.flow = Some(c.flow.clone());
        }
        // R30: the third job without an agent with this setup in 30 days.
        let suggested = self.repeated_setup(&a).await;
        if suggested {
            a.suggestion = Some(xherd_core::job::Suggestion {
                reason: "repeated".into(),
                at: a.created_at,
                dismissed: false,
            });
        }

        // The session starts on the first stage's agent: its skills go in now.
        let wanted: Vec<(String, Option<String>)> = current_id
            .as_ref()
            .and_then(|id| defs.get(id))
            .map(|d| d.skills.iter().map(|s| (s.clone(), None)).collect())
            .unwrap_or_default();
        let res = async {
            self.prepare_worktree(&a).await?;
            if let Some(snap) = a.agent_mut() {
                snap.skills = self.install_skills(&workdir, rt_name, &rt, &wanted).await?;
            }
            self.create_job(a).await
        }
        .await;
        if res.is_err() {
            // Leave nothing behind (R-002).
            let _ = git.worktree_remove(&repo, &workdir, true).await;
            let _ = git.run(&repo, &["branch", "-D", &branch]).await;
            if c.project.is_some() {
                crate::repos::remove_tree(&repo).await;
            }
        }
        let a = res?;
        debug_assert_eq!(a.status, Status::Queued);
        for g in a.agents.values().filter(|g| g.fell_back) {
            let e = xherd_store::Event {
                id: xherd_core::new_id(),
                job_id: a.id.clone(),
                kind: "agent-fallback".into(),
                payload: Some(serde_json::json!({"agent": g.id, "runtime": g.runtime})),
                at: self.clock.now(),
            };
            let _ = self.store.insert_event(e).await;
        }
        if suggested {
            self.suggestion_notify(&a);
        }
        Ok(a)
    }
}

impl Daemon {
    /// What every agent worktree needs besides the checkout: the runtime's
    /// trust entry, `.xherd/` (R-005c), the pre-push guard (R-1204) and the
    /// runtime's hook file (R-005b). Idempotent; also finishes a worktree that
    /// a restore created (`repair_restored`).
    pub(crate) async fn prepare_worktree(&self, a: &Job) -> Result<(), ActionError> {
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
        self.write_hook_files(&git, a, &workdir, &rt, pack, rt_name)
            .await?;
        Ok(())
    }

    /// The pre-push guard and the runtime's hook file, written only when
    /// their text differs. True when anything was written.
    async fn write_hook_files(
        &self,
        git: &Git,
        a: &Job,
        workdir: &Path,
        rt: &Terminal,
        pack: Option<&xherd_flow::Pack>,
        rt_name: &str,
    ) -> Result<bool, ActionError> {
        let mut wrote = git
            .install_push_guard(
                workdir,
                &workdir.join(".xherd/hooks"),
                &self.xherd_bin,
                &a.branch,
            )
            .await
            .map_err(|e| refuse("push_guard", e.to_string()))?;
        if let Some(h) = &rt.hooks {
            let allow = permission_rules(pack, rt_name);
            wrote |= install_hooks(git, workdir, h, &self.xherd_bin, &allow).await?;
        }
        Ok(wrote)
    }

    /// Point the hook file and the push guard of every live job at this
    /// daemon's `xherd` binary (R-008, R-009), as an upgrade or a moved binary
    /// leaves them naming the old one. Skips finished jobs, missing worktrees
    /// and unusable runtimes (a warning each). Returns the jobs changed.
    pub async fn refresh_job_files(&self) -> usize {
        let Ok(jobs) = self.store.list_jobs().await else {
            return 0;
        };
        let git = Git::new("git");
        let mut changed = 0;
        for a in jobs {
            if matches!(a.status, Status::Done | Status::Cancelled) {
                continue;
            }
            let Some(w) = a.workdir.as_deref().map(PathBuf::from) else {
                continue;
            };
            if !w.is_dir() {
                tracing::warn!(job = %a.name, "worktree missing, hook files not refreshed");
                continue;
            }
            let rt_name = a
                .runtime
                .as_deref()
                .unwrap_or(xherd_flow::approve::DEFAULT_RUNTIME);
            let rt = match self.runtime_terminal(rt_name) {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!(job = %a.name, error = %e.message, "hook files not refreshed");
                    continue;
                }
            };
            let pack = a
                .pack
                .is_some()
                .then(|| self.packs.get(xherd_flow::BUILTIN))
                .flatten();
            match self
                .write_hook_files(&git, &a, &w, &rt, pack, rt_name)
                .await
            {
                Ok(true) => changed += 1,
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(job = %a.name, error = %e.message, "hook files not refreshed")
                }
            }
        }
        changed
    }

    /// Copy the skills an agent lists into the worktree, where the job's
    /// runtime reads project skills (R26, R27), and say what became of each.
    /// `wanted` is a name and, for a restored worktree, the hash the job
    /// recorded. A skill that cannot be installed is skipped with the reason,
    /// never an error; only a failed write is (the creation then removes the
    /// worktree). Each copied folder holds a `.gitignore` with `*`, which
    /// hides its whole content, itself included: `git status` shows nothing
    /// and nothing outside the worktree changes. Skills are never put in a
    /// prompt.
    pub(crate) async fn install_skills(
        &self,
        workdir: &Path,
        rt_name: &str,
        rt: &Terminal,
        wanted: &[(String, Option<String>)],
    ) -> Result<Vec<SkillCopy>, ActionError> {
        crate::skills::install(
            &self.paths.skills(),
            workdir,
            rt_name,
            rt.skills.as_ref(),
            wanted,
        )
        .await
    }

    /// A restored worktree has none of the skills its job installed (they are
    /// not in the backup): copy again those the job recorded, from the
    /// library copy with the same hash. Failures are only logged.
    async fn reinstall_skills(&self, a: &Job, workdir: &Path) {
        let wanted: Vec<(String, Option<String>)> = a
            .agent()
            .into_iter()
            .flat_map(|g| &g.skills)
            .filter(|s| s.installed)
            .map(|s| (s.name.clone(), Some(s.sha256.clone())))
            .collect();
        if wanted.is_empty() {
            return;
        }
        let rt_name = a
            .runtime
            .as_deref()
            .unwrap_or(xherd_flow::approve::DEFAULT_RUNTIME);
        let Ok(rt) = self.runtime_terminal(rt_name) else {
            return;
        };
        match self.install_skills(workdir, rt_name, &rt, &wanted).await {
            Ok(done) => {
                for s in done.iter().filter(|s| !s.installed) {
                    tracing::warn!(job = %a.name, skill = %s.name, why = ?s.skipped, "skill not restored");
                }
            }
            Err(e) => {
                tracing::warn!(job = %a.name, error = %e.message, "skills not restored")
            }
        }
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

    /// A worktree a backup restore recreated has no `.xherd/job.json` (it is
    /// not in the backup): finish it like a new agent's, once, at daemon start.
    pub(crate) async fn repair_restored(&self) {
        let Ok(agents) = self.store.list_jobs().await else {
            return;
        };
        for a in agents {
            let Some(w) = a.workdir.as_deref().map(PathBuf::from) else {
                continue;
            };
            let live = !matches!(a.status, Status::Done | Status::Cancelled);
            if live
                && w.join(".xherd").is_dir()
                && !w.join(".xherd/job.json").exists()
                && !w.join(".xherd/agent.json").exists()
            {
                match self.prepare_worktree(&a).await {
                    Ok(()) => tracing::info!(agent = %a.name, "restored worktree prepared"),
                    Err(e) => {
                        tracing::warn!(agent = %a.name, error = %e.message, "restored worktree not prepared");
                        continue;
                    }
                }
                self.reinstall_skills(&a, &w).await;
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
    /// The runtime's terminal driver: where its skills go and how a mode
    /// becomes launch arguments (a switch of agent rebuilds `launch`).
    pub terminal: Option<Terminal>,
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
            let terminal = t.clone();
            let kit = RuntimeKit {
                trust: t.trust.map(|trust| TrustWriter {
                    trust,
                    root: worktrees.to_path_buf(),
                }),
                resume: t.resume.map(Resumer),
                usage: t.usage,
                prompt_args: t.prompt_args,
                terminal: Some(terminal),
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

/// `.xherd/` with `job.json`, `instructions.md` and `spec/`, excluded from
/// git (R-005b exclude part, R-005c).
async fn prepare_xherd_dir(
    git: &Git,
    workdir: &Path,
    a: &Job,
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
    let json = serde_json::to_vec_pretty(&new::job_json(a)).expect("json");
    let f = dir.join("job.json");
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
    "xherd agent memory *",
    "xherd agent propose *",
    "xherd action *",
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
) -> Result<bool, ActionError> {
    let f = workdir.join(&h.file);
    let existing = xherd_core::runner::read_file(&f)
        .await
        .ok()
        .flatten()
        .map(|b| String::from_utf8_lossy(&b).into_owned());
    let text = runtime::hook_file_text(existing.as_deref(), h, xherd_bin, allow)
        .map_err(|e| refuse("runtime_hooks", e))?;
    let changed = existing.as_deref() != Some(text.as_str());
    if changed {
        xherd_core::runner::write_file(&f, text.as_bytes(), None)
            .await
            .map_err(|e| refuse("io", format!("{}: {e}", f.display())))?;
    }
    git.add_excludes(workdir, &[&h.file])
        .await
        .map_err(|e| refuse("git", e.to_string()))?;
    Ok(changed)
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
        for rt in ["claude-code", "opencode", "codex", "pi"] {
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
