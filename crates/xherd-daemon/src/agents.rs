//! Saved agents (issue #141, spec R10…R22): named, versioned setups that jobs
//! start from, stored in the `agents` tables. The pure record and its checks
//! are in `xherd_core::agent_def`; this module adds what needs plugins and
//! packs (`check_agent`, `agent_broken`), the writes under one mutex, and the
//! export and import of R22.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xherd_core::Job;
use xherd_core::agent_def::{self, AgentDef, AgentStatus};
use xherd_core::permission::PermissionMode;
use xherd_core::resolve::{self, Assignment};

use crate::{ActionError, Change, Daemon};

fn err(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

pub(crate) fn store(e: xherd_store::StoreError) -> ActionError {
    err("store", e.to_string())
}

/// An agent as `GET /agents` answers it: the record plus what is derived
/// (R17, R19).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    #[serde(flatten)]
    pub def: AgentDef,
    pub status: AgentStatus,
    /// Why jobs cannot start from it now (R15), when they cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broken: Option<String>,
    pub live_jobs: usize,
    /// Name of the newest job started from it.
    pub last_job: Option<String>,
    /// Today's tokens, recorded under scope `agent:<id>` (shown, not enforced).
    pub tokens_today: u64,
}

/// `POST /agents/import` and the shape `GET /agents/export` returns (R22):
/// no ids, no secrets (an agent holds none).
#[derive(Debug, Default, Deserialize)]
#[serde(try_from = "RawImport")]
pub struct Import {
    pub agents: Vec<AgentDef>,
    /// Per agent, the keys of an older export that agents no longer have
    /// (a workflow, repository, base branch, stage overrides): dropped, and
    /// said in the dry run.
    pub ignored: Vec<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawImport {
    agents: Vec<serde_json::Value>,
}

/// The keys the agent record lost with stage agents (D26).
const DROPPED_KEYS: [&str; 4] = ["workflow", "repo", "baseBranch", "stageOverrides"];

impl TryFrom<RawImport> for Import {
    type Error = String;

    fn try_from(raw: RawImport) -> Result<Self, String> {
        let mut i = Import::default();
        for mut v in raw.agents {
            let mut ignored = Vec::new();
            if let Some(o) = v.as_object_mut() {
                for k in DROPPED_KEYS {
                    if o.remove(k).is_some() {
                        ignored.push(k.to_string());
                    }
                }
            }
            i.agents
                .push(serde_json::from_value(v).map_err(|e| e.to_string())?);
            i.ignored.push(ignored);
        }
        Ok(i)
    }
}

/// The saved agent a job started from.
/// One stage of a flow as the Flows page and `xherd flow show` read it
/// (R35, R37): the agent it resolves to with nothing overridden, whether that
/// is its own assignment, and the `human` flag.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageView {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub role: xherd_core::domain::Role,
    pub class: xherd_core::domain::Class,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Set when the stage has an assignment row.
    pub assigned: bool,
    pub human: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Why the resolved agent cannot run now (archived, runtime off, gone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broken: Option<String>,
    /// The fresh-context check is on for this stage (new jobs copy it).
    pub fresh_check: bool,
    /// The stage can have the check (a spec or design stage with a document gate).
    pub fresh_check_allowed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowView {
    pub pack: String,
    pub flow: String,
    pub stages: Vec<StageView>,
}

/// The agent a memory proposal is about: the one named in it, else the job's
/// current one (proposals made before stage agents).
fn proposal_agent<'a>(
    job: &'a Job,
    p: &xherd_core::job::MemoryProposal,
) -> Option<&'a xherd_core::agent_def::JobAgent> {
    match p.agent.as_deref() {
        Some(id) => job.agents.get(id),
        None => job.agent(),
    }
}

/// Live jobs and the newest job's name, from the jobs of one agent (pure).
/// Live jobs of agent `id` (those whose current stage it runs, like the
/// supervisor's limit) and the newest job that used it.
fn summarize(id: &str, jobs: &[&Job]) -> (usize, Option<String>) {
    let live = jobs
        .iter()
        .filter(|j| j.status.is_live() && j.current_agent.as_deref() == Some(id))
        .count();
    let last = jobs
        .iter()
        .max_by_key(|j| (j.created_at, j.id.as_str()))
        .map(|j| j.name.clone());
    (live, last)
}

/// The fields of an imported agent a person should look at before saying yes
/// (R22): they widen what a job may do, or add text the job reads.
pub(crate) fn risky_fields(d: &AgentDef) -> Vec<&'static str> {
    let mut v = Vec::new();
    if matches!(
        d.permission_mode,
        Some(PermissionMode::Auto | PermissionMode::All)
    ) {
        v.push("permissionMode");
    }
    if d.autonomy
        .is_some_and(|a| a != xherd_core::autonomy::Autonomy::Ask)
    {
        v.push("autonomy");
    }
    if d.instructions.is_some() {
        v.push("instructions");
    }
    if !d.skills.is_empty() {
        v.push("skills");
    }
    v
}

impl Daemon {
    async fn jobs_of(&self, id: &str) -> Result<Vec<Job>, ActionError> {
        let mut jobs: Vec<Job> = self
            .store
            .list_jobs()
            .await
            .map_err(store)?
            .into_iter()
            .filter(|j| j.uses_agent(id))
            .collect();
        jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at));
        Ok(jobs)
    }

    async fn view(&self, def: AgentDef, jobs: &[&Job]) -> AgentView {
        let (live_jobs, last_job) = summarize(&def.id, jobs);
        AgentView {
            status: agent_def::derived_status(&def, jobs),
            broken: self.agent_broken(&def),
            live_jobs,
            last_job,
            tokens_today: self
                .store
                .usage(
                    &format!("agent:{}", def.id),
                    &xherd_core::usage::day(self.clock.now()),
                )
                .await
                .unwrap_or(0),
            def,
        }
    }

    pub async fn agents(&self) -> Result<Vec<AgentView>, ActionError> {
        let defs = self.store.list_agent_defs().await.map_err(store)?;
        let jobs = self.store.list_jobs().await.map_err(store)?;
        let mut out = Vec::new();
        for d in defs {
            let mine: Vec<&Job> = jobs.iter().filter(|j| j.uses_agent(&d.id)).collect();
            out.push(self.view(d, &mine).await);
        }
        Ok(out)
    }

    /// An agent by id or by name.
    pub async fn agent_def(&self, key: &str) -> Result<AgentDef, ActionError> {
        let all = self.store.list_agent_defs().await.map_err(store)?;
        all.into_iter()
            .find(|a| a.id == key || a.name == key)
            .ok_or_else(|| err("unknown_agent", format!("no agent {key:?}")))
    }

    pub async fn agent(&self, key: &str) -> Result<AgentView, ActionError> {
        let def = self.agent_def(key).await?;
        let jobs = self.jobs_of(&def.id).await?;
        let refs: Vec<&Job> = jobs.iter().collect();
        Ok(self.view(def, &refs).await)
    }

    /// Whether `key` names a saved agent (the API resolves agents before the
    /// old job routes).
    pub async fn is_agent(&self, key: &str) -> bool {
        self.agent_def(key).await.is_ok()
    }

    /// The checks that need plugins and packs (R10): the pack has the flow, the
    /// runtime plugin exists and the pack has commands for it, models belong to
    /// runtimes that take one, mode `all` only in the container, and every
    /// stage override names a stage of the flow. With `running`, the runtime
    /// must also be switched on (that is `broken`, R15).
    fn validate(&self, def: &AgentDef, running: bool) -> Result<(), ActionError> {
        let off: Vec<String> = if running {
            self.live_config().disabled_plugins
        } else {
            Vec::new()
        };
        // A runtime that cannot run now is fine when the fallback can (R36).
        if let Err(m) = self.check_runtime(&def.runtime, &off) {
            let fallback_runs = running
                && def
                    .fallback_runtime
                    .as_deref()
                    .is_some_and(|f| self.check_runtime(f, &off).is_ok());
            if !fallback_runs {
                return Err(err("unknown_runtime", m));
            }
        }
        // The fallback only has to exist; it may be off, that is what it is for.
        if let Some(f) = &def.fallback_runtime {
            self.check_runtime(f, &[])
                .map_err(|m| err("bad_fallback", format!("fallbackRuntime: {m}")))?;
        }
        for rt in def.models.keys() {
            self.check_model_runtime(rt)
                .map_err(|m| err("bad_agent", format!("models: {m}")))?;
        }
        if def
            .permission_mode
            .is_some_and(|m| !m.allowed(self.sandbox))
        {
            return Err(err(
                "mode_unavailable",
                "permission mode all asks nothing, so it is only offered in the container",
            ));
        }
        for s in &def.skills {
            if !self.has_skill(s) {
                return Err(err(
                    "unknown_skill",
                    format!("skills: no skill named {s:?} in the library"),
                ));
            }
        }
        Ok(())
    }

    /// Clean and check an agent before it is saved (R10).
    pub fn check_agent(&self, def: &mut AgentDef) -> Result<(), ActionError> {
        agent_def::check(def)?;
        self.validate(def, false)
    }

    /// Why jobs cannot start from this agent now: its pack, flow or runtime is
    /// gone or switched off (R15). `None` = fine.
    pub fn agent_broken(&self, def: &AgentDef) -> Option<String> {
        self.validate(def, true).err().map(|e| e.message)
    }

    pub(crate) fn agents_changed(&self) {
        let _ = self.hub.send(Change::Agents);
    }

    pub async fn create_agent(&self, mut def: AgentDef) -> Result<AgentView, ActionError> {
        let _w = self.agents_write.lock().await;
        self.check_agent(&mut def)?;
        let have = self.store.list_agent_defs().await.map_err(store)?;
        if have.iter().any(|a| a.name == def.name) {
            return Err(err(
                "name_taken",
                format!("an agent named {} exists", def.name),
            ));
        }
        let now = self.clock.now();
        def.id = xherd_core::new_id();
        def.version = 1;
        def.created_at = now;
        def.updated_at = now;
        def.archived_at = None;
        // Only a flow plugin's install sets `source` (R7).
        def.source = None;
        self.store.put_agent_def(&def).await.map_err(store)?;
        self.sup.agent_limit(&def.id, Some(def.max_live_jobs));
        self.agents_changed();
        Ok(self.view(def, &[]).await)
    }

    /// Replace an agent's settings: the version goes up by one and the old one
    /// stays in `agent_versions` (R11). The name is fixed (`name_fixed`).
    pub async fn update_agent(
        &self,
        key: &str,
        mut def: AgentDef,
    ) -> Result<AgentView, ActionError> {
        let _w = self.agents_write.lock().await;
        let old = self.agent_def(key).await?;
        def.name = def.name.trim().to_string();
        if def.name != old.name {
            return Err(err(
                "name_fixed",
                format!(
                    "an agent's name cannot change ({} → {})",
                    old.name, def.name
                ),
            ));
        }
        self.check_agent(&mut def)?;
        if def.runtime != old.runtime {
            self.refuse_mixed_by(&old.id, &def.runtime).await?;
        }
        def.id = old.id.clone();
        def.version = old.version + 1;
        def.created_at = old.created_at;
        def.updated_at = self.clock.now();
        def.archived_at = old.archived_at;
        def.source = old.source.clone();
        self.store.put_agent_def(&def).await.map_err(store)?;
        self.sup.agent_limit(&def.id, Some(def.max_live_jobs));
        self.agents_changed();
        drop(_w);
        self.agent(&def.id).await
    }

    /// Hide an agent from pickers, or bring it back (R18). Not a new version:
    /// the settings are the same (the current version row only learns the new
    /// `archivedAt`).
    pub async fn archive_agent(&self, key: &str, archive: bool) -> Result<AgentView, ActionError> {
        let _w = self.agents_write.lock().await;
        let mut def = self.agent_def(key).await?;
        if archive {
            self.refuse_if_assigned(&def, "archive").await?;
        }
        let now = self.clock.now();
        def.archived_at = archive.then_some(now);
        self.store.put_agent_def(&def).await.map_err(store)?;
        self.agents_changed();
        drop(_w);
        self.agent(&def.id).await
    }

    /// Refused while the agent has a live job (R18); afterwards its jobs keep
    /// their snapshot.
    pub async fn delete_agent(&self, key: &str) -> Result<(), ActionError> {
        let _w = self.agents_write.lock().await;
        let def = self.agent_def(key).await?;
        self.refuse_if_assigned(&def, "delete").await?;
        let jobs = self.jobs_of(&def.id).await?;
        if let Some(j) = jobs.iter().find(|j| j.status.is_live()) {
            return Err(err(
                "agent_busy",
                format!("job {} is still {}", j.name, j.status.as_str()),
            ));
        }
        self.store.delete_agent_def(&def.id).await.map_err(store)?;
        self.sup.agent_limit(&def.id, None);
        self.agents_changed();
        Ok(())
    }

    /// Every version, newest first.
    pub async fn agent_versions(&self, key: &str) -> Result<Vec<AgentDef>, ActionError> {
        let def = self.agent_def(key).await?;
        self.store.agent_versions(&def.id).await.map_err(store)
    }

    /// The agent's memory and its version (R28); empty and 0 before the first save.
    pub async fn agent_memory(&self, key: &str) -> Result<Value, ActionError> {
        let def = self.agent_def(key).await?;
        let (text, version) = self
            .store
            .agent_memory(&def.id)
            .await
            .map_err(store)?
            .unwrap_or_default();
        Ok(json!({"text": text, "version": version}))
    }

    /// Replace the memory (R28). Jobs already started keep the one they got.
    pub async fn set_agent_memory(&self, key: &str, text: &str) -> Result<Value, ActionError> {
        if text.chars().count() > agent_def::MAX_MEMORY || text.contains('\0') {
            return Err(err(
                "bad_memory",
                format!("memory: up to {} characters", agent_def::MAX_MEMORY),
            ));
        }
        let _w = self.agents_write.lock().await;
        let def = self.agent_def(key).await?;
        let now = self.clock.now();
        self.store
            .put_agent_memory(&def.id, text, now)
            .await
            .map_err(store)?;
        self.agents_changed();
        drop(_w);
        self.agent_memory(&def.id).await
    }

    /// `GET /jobs/{id}/agent-memory` (R29): the job's pending agent-memory
    /// proposal, the proposed text read from the worktree, and the agent's
    /// memory now. Read-only, outside the actor.
    pub async fn agent_memory_proposal(&self, who: &str) -> Result<Value, ActionError> {
        let a = self.action(who, crate::actor::UserAction::Get).await?;
        let Some(p) = a.agent_memory_proposal.clone() else {
            return Ok(json!({"proposal": null}));
        };
        let wd = crate::workdir_of(&a)?.to_path_buf();
        let proposed = xherd_flow::xap::doc_path(&p.file, &wd)
            .ok()
            .and_then(|d| crate::actor::inside(&wd, &d))
            .and_then(|f| std::fs::read_to_string(f).ok());
        // The agent the proposal is for: the one that wrote it (D26), else
        // the job's current one.
        let agent = proposal_agent(&a, &p);
        let current = match &agent {
            Some(g) => self.agent_memory(&g.id).await.ok(),
            None => None,
        };
        Ok(json!({
            "proposal": p,
            "proposed": proposed,
            "agent": agent.as_ref().map(|j| json!({"id": j.id, "name": j.name})),
            "current": current.as_ref().map(|c| c["text"].clone()),
            "version": current.as_ref().map(|c| c["version"].clone()),
        }))
    }

    /// `POST /jobs/{id}/agent-memory-accept {text?}` (R29): the user's edit,
    /// else the job's file, becomes the agent's memory (version + 1); then
    /// the actor drops the proposal. The job's own snapshot is not touched:
    /// only jobs created later get the new text.
    pub async fn agent_memory_accept(
        &self,
        who: &str,
        edited: Option<String>,
    ) -> Result<Value, ActionError> {
        let a = self.action(who, crate::actor::UserAction::Get).await?;
        let Some(p) = a.agent_memory_proposal.clone() else {
            return Err(err(
                "no_proposal",
                format!("{} has no agent-memory proposal", a.name),
            ));
        };
        let Some(agent) = proposal_agent(&a, &p) else {
            return Err(err("no_agent", format!("{} has no saved agent", a.name)));
        };
        let text = match edited.filter(|t| !t.trim().is_empty()) {
            Some(t) => t,
            None => {
                let wd = crate::workdir_of(&a)?.to_path_buf();
                xherd_flow::xap::doc_path(&p.file, &wd)
                    .ok()
                    .and_then(|d| crate::actor::inside(&wd, &d))
                    .and_then(|f| std::fs::read_to_string(f).ok())
                    .ok_or_else(|| err("doc_missing", format!("{} is gone", p.file)))?
            }
        };
        let done = self.set_agent_memory(&agent.id, text.trim_end()).await?;
        self.action(who, crate::actor::UserAction::AgentMemoryClear)
            .await?;
        Ok(done)
    }

    /// Whether `new` (not stored yet) is the third job without an agent with
    /// the same repository, runtime, model, flow, autonomy and mode created
    /// in the last 30 days (R30). Off with `suggestAgents`. Only the third:
    /// a fourth would nag after the third was dismissed.
    pub(crate) async fn repeated_setup(&self, new: &Job) -> bool {
        const DAYS_30: i64 = 30 * 24 * 3600 * 1000;
        if !new.agents.is_empty() || !self.live_config().suggest_agents {
            return false;
        }
        let Ok(jobs) = self.store.list_jobs().await else {
            return false;
        };
        let same = jobs
            .iter()
            .filter(|j| {
                j.agents.is_empty()
                    && new.created_at - j.created_at <= DAYS_30
                    && j.repo == new.repo
                    && j.runtime == new.runtime
                    && j.model == new.model
                    && j.flow == new.flow
                    && j.autonomy == new.autonomy
                    && j.permission_mode == new.permission_mode
            })
            .count();
        same == 2
    }

    /// The notice for a suggestion made at creation (R30); the actor sends
    /// its own when a job ends.
    pub(crate) fn suggestion_notify(&self, job: &Job) {
        if self
            .live_config()
            .muted_notifications
            .iter()
            .any(|k| k == "suggestion")
        {
            return;
        }
        let _ = self.hub.send(Change::Notify {
            job_id: job.id.clone(),
            kind: "suggestion".into(),
            text: format!("{}: save this setup as an agent?", job.name),
        });
    }

    /// `POST /jobs/{id}/suggestion/accept {agent}` and
    /// `POST /jobs/{id}/save-as-agent {agent}` (R32): create the agent
    /// (version 1) from the form the user edited, then attach the job to it.
    /// A suggestion must exist for `accept`; `save-as-agent` works for any
    /// job without an agent. The job keeps its own settings and snapshot.
    pub async fn save_as_agent(
        &self,
        who: &str,
        def: AgentDef,
        accept: bool,
    ) -> Result<Value, ActionError> {
        let a = self.action(who, crate::actor::UserAction::Get).await?;
        if !a.agents.is_empty() {
            return Err(err(
                "has_agent",
                format!("{} already belongs to an agent", a.name),
            ));
        }
        if accept && a.suggestion.is_none() {
            return Err(err(
                "no_suggestion",
                format!("{} has no suggestion to accept", a.name),
            ));
        }
        let view = self.create_agent(def).await?;
        let snapshot = Box::new(view.def.snapshot());
        let job = self
            .action(who, crate::actor::UserAction::AttachAgent { snapshot })
            .await?;
        Ok(json!({"agent": view, "job": job}))
    }

    /// `POST /jobs/{id}/suggestion/dismiss` (R32): remembered for the job.
    pub async fn dismiss_suggestion(&self, who: &str) -> Result<Job, ActionError> {
        self.action(who, crate::actor::UserAction::SuggestionDismiss)
            .await
    }

    /// The jobs started from an agent, newest first.
    pub async fn agent_jobs(&self, key: &str) -> Result<Vec<Job>, ActionError> {
        let def = self.agent_def(key).await?;
        self.jobs_of(&def.id).await
    }

    /// All agents, or the one named; without ids, versions or dates.
    pub async fn export_agents(&self, name: Option<&str>) -> Result<Value, ActionError> {
        let mut all = self.store.list_agent_defs().await.map_err(store)?;
        if let Some(n) = name {
            all.retain(|a| a.name == n || a.id == n);
            if all.is_empty() {
                return Err(err("unknown_agent", format!("no agent {n:?}")));
            }
        }
        for a in &mut all {
            a.id.clear();
            a.version = 1;
            a.created_at = 0;
            a.updated_at = 0;
            a.archived_at = None;
        }
        Ok(json!({"agents": all}))
    }

    /// All or nothing (R22): one invalid agent imports none, and a name that
    /// exists is refused (`name_taken`), never replaced: an import must not
    /// silently change an agent that jobs start from. With `dry_run`, nothing
    /// is written; the answer lists each agent as it would be saved, with the
    /// fields to look at (`risky`) and whether its name is taken.
    pub async fn import_agents(&self, i: Import, dry_run: bool) -> Result<Value, ActionError> {
        let _w = self.agents_write.lock().await;
        let mut names: Vec<String> = self
            .store
            .list_agent_defs()
            .await
            .map_err(store)?
            .into_iter()
            .map(|a| a.name)
            .collect();
        let now = self.clock.now();
        let mut out = Vec::new();
        let mut taken = false;
        let mut ignored = i.ignored.into_iter();
        for mut def in i.agents {
            let dropped = ignored.next().unwrap_or_default();
            self.check_agent(&mut def)
                .map_err(|e| err(e.code, format!("{}: {}", def.name, e.message)))?;
            let name_taken = names.contains(&def.name);
            if name_taken && !dry_run {
                return Err(err(
                    "name_taken",
                    format!("an agent named {} exists", def.name),
                ));
            }
            taken |= name_taken;
            names.push(def.name.clone());
            def.id = xherd_core::new_id();
            def.version = 1;
            def.created_at = now;
            def.updated_at = now;
            def.archived_at = None;
            out.push((def, name_taken, dropped));
        }
        if dry_run {
            let agents: Vec<Value> = out
                .iter()
                .map(|(d, t, ig)| {
                    json!({"agent": d, "risky": risky_fields(d), "nameTaken": t, "ignored": ig})
                })
                .collect();
            return Ok(json!({"agents": agents, "nameTaken": taken}));
        }
        for (d, _, _) in &out {
            self.store.put_agent_def(d).await.map_err(store)?;
            self.sup.agent_limit(&d.id, Some(d.max_live_jobs));
        }
        self.agents_changed();
        Ok(json!({"added": out.len()}))
    }
}

impl Daemon {
    fn flow_of(&self, pack: &str, flow: &str) -> Result<&xherd_flow::Flow, ActionError> {
        self.packs
            .get(pack)
            .ok_or_else(|| err("unknown_pack", format!("no flow pack named {pack:?}")))?
            .flow(flow)
            .ok_or_else(|| err("unknown_flow", format!("pack {pack} has no flow {flow:?}")))
    }

    async fn flow_view(
        &self,
        pack: &str,
        flow: &str,
        agents: &[AgentDef],
    ) -> Result<FlowView, ActionError> {
        let f = self.flow_of(pack, flow)?;
        let rows = self
            .store
            .flow_assignments(pack, flow)
            .await
            .map_err(store)?;
        let fresh = self
            .store
            .flow_fresh_checks(pack, flow)
            .await
            .map_err(store)?;
        let ids: Vec<&str> = f.stages.iter().map(|s| s.id.as_str()).collect();
        let resolved = resolve::stage_agents(&ids, &rows, &Default::default(), None);
        let stages = f
            .stages
            .iter()
            .zip(resolved)
            .map(|(s, id)| {
                let row = rows.iter().find(|a| a.stage == s.id);
                let def = id
                    .as_ref()
                    .and_then(|id| agents.iter().find(|a| &a.id == id));
                let broken = match (&id, def) {
                    (Some(id), None) => Some(format!("agent {id} no longer exists")),
                    (_, Some(d)) if d.archived_at.is_some() => Some("the agent is archived".into()),
                    (_, Some(d)) => self.agent_broken(d),
                    _ => None,
                };
                StageView {
                    id: s.id.clone(),
                    label: s.label.clone(),
                    role: s.role,
                    class: s.class,
                    command: s.command.clone(),
                    assigned: row.is_some(),
                    human: row.is_some_and(|a| a.human),
                    agent_id: id,
                    agent: def.map(|d| d.name.clone()),
                    broken,
                    fresh_check: fresh.contains(&s.id),
                    fresh_check_allowed: s.fresh_check_eligible(),
                }
            })
            .collect();
        Ok(FlowView {
            pack: pack.to_string(),
            flow: flow.to_string(),
            stages,
        })
    }

    /// Every flow of every pack with its stages and what runs each (R35).
    pub async fn flows(&self) -> Result<Vec<FlowView>, ActionError> {
        let agents = self.store.list_agent_defs().await.map_err(store)?;
        let mut out = Vec::new();
        for (pack, p) in self.packs.iter() {
            for flow in p.flows.keys() {
                out.push(self.flow_view(pack, flow, &agents).await?);
            }
        }
        Ok(out)
    }

    pub async fn flow(&self, pack: &str, flow: &str) -> Result<FlowView, ActionError> {
        let agents = self.store.list_agent_defs().await.map_err(store)?;
        self.flow_view(pack, flow, &agents).await
    }

    /// The stored assignments of one flow.
    pub async fn assignments(
        &self,
        pack: &str,
        flow: &str,
    ) -> Result<Vec<Assignment>, ActionError> {
        self.flow_of(pack, flow)?;
        self.store.flow_assignments(pack, flow).await.map_err(store)
    }

    /// Replace a flow's assignments (R35, R38). Every stage must belong to the
    /// flow, every agent exist and not be archived, and the agents of one flow
    /// share a runtime until a job can change runtime in the middle (R-025).
    /// Agents are named by id or name; the rows keep the id.
    pub async fn set_assignments(
        &self,
        pack: &str,
        flow: &str,
        rows: Vec<Assignment>,
    ) -> Result<FlowView, ActionError> {
        self.set_flow_settings(pack, flow, rows, None).await
    }

    /// `set_assignments` plus the fresh-context check stages: `Some` replaces
    /// the set in the same transaction (unknown stages and stages that cannot
    /// have the check are refused, duplicates dropped), `None` keeps it.
    pub async fn set_flow_settings(
        &self,
        pack: &str,
        flow: &str,
        rows: Vec<Assignment>,
        fresh: Option<Vec<String>>,
    ) -> Result<FlowView, ActionError> {
        let _w = self.agents_write.lock().await;
        let f = self.flow_of(pack, flow)?;
        let fresh = match fresh {
            None => None,
            Some(stages) => {
                let mut out: Vec<String> = Vec::new();
                for id in stages {
                    let s = f.stage(&id).ok_or_else(|| {
                        err("unknown_stage", format!("flow {flow} has no stage {id:?}"))
                    })?;
                    if !s.fresh_check_eligible() {
                        return Err(err(
                            "fresh_check_not_allowed",
                            format!("stage {id} cannot have the fresh-context check"),
                        ));
                    }
                    if !out.contains(&id) {
                        out.push(id);
                    }
                }
                Some(out)
            }
        };
        let agents = self.store.list_agent_defs().await.map_err(store)?;
        let mut out: Vec<Assignment> = Vec::new();
        for r in rows {
            if f.stage(&r.stage).is_none() {
                return Err(err(
                    "unknown_stage",
                    format!("flow {flow} has no stage {:?}", r.stage),
                ));
            }
            if out.iter().any(|o| o.stage == r.stage) {
                return Err(err(
                    "bad_assignment",
                    format!("stage {} is given twice", r.stage),
                ));
            }
            let a = agents
                .iter()
                .find(|a| a.id == r.agent_id || a.name == r.agent_id)
                .ok_or_else(|| err("unknown_agent", format!("no agent {:?}", r.agent_id)))?;
            if a.archived_at.is_some() {
                return Err(err(
                    "agent_archived",
                    format!("agent {} is archived", a.name),
                ));
            }
            out.push(Assignment {
                agent_id: a.id.clone(),
                ..r
            });
        }
        let mut runtimes = out
            .iter()
            .filter_map(|r| agents.iter().find(|a| a.id == r.agent_id))
            .map(|a| a.runtime.as_str());
        if let Some(first) = runtimes.next()
            && let Some(other) = runtimes.find(|r| *r != first)
        {
            return Err(err(
                "mixed_runtime",
                format!(
                    "the agents of one flow must share a runtime ({first} and {other}); \
                     a job cannot change runtime between stages yet"
                ),
            ));
        }
        out.sort_by(|a, b| a.stage.cmp(&b.stage));
        self.store
            .set_flow_assignments(pack, flow, &out, fresh.as_deref())
            .await
            .map_err(store)?;
        self.agents_changed();
        self.flow_view(pack, flow, &agents).await
    }

    /// A runtime change that would leave a flow with agents on two runtimes.
    async fn refuse_mixed_by(&self, id: &str, runtime: &str) -> Result<(), ActionError> {
        let agents = self.store.list_agent_defs().await.map_err(store)?;
        let all = self.store.all_flow_assignments().await.map_err(store)?;
        for (pack, flow, a) in all.iter().filter(|(_, _, a)| a.agent_id == id) {
            let other = all
                .iter()
                .filter(|(p, f, o)| p == pack && f == flow && o.agent_id != id)
                .filter_map(|(_, _, o)| agents.iter().find(|d| d.id == o.agent_id))
                .find(|d| d.runtime != runtime);
            if let Some(d) = other {
                return Err(err(
                    "mixed_runtime",
                    format!(
                        "stage {} of {pack}/{flow} would run on {runtime} while {} runs on {}",
                        a.stage, d.name, d.runtime
                    ),
                ));
            }
        }
        Ok(())
    }

    /// `agent_assigned` when a flow gives a stage to this agent (R38).
    async fn refuse_if_assigned(&self, def: &AgentDef, what: &str) -> Result<(), ActionError> {
        let all = self.store.all_flow_assignments().await.map_err(store)?;
        let Some((pack, flow, a)) = all.iter().find(|(_, _, a)| a.agent_id == def.id) else {
            return Ok(());
        };
        Err(err(
            "agent_assigned",
            format!(
                "{} runs stage {} of {pack}/{flow}: give that stage to another agent before you {what} it",
                def.name, a.stage
            ),
        ))
    }

    /// A job from a saved agent (`POST /agents/{id}/jobs`, R21): without a
    /// name it is `<agent>-<ticket key>`, else `<agent>-<n>`.
    pub async fn new_agent_job(
        &self,
        key: &str,
        mut req: xherd_core::new::NewJob,
    ) -> Result<Job, ActionError> {
        let def = self.agent_def(key).await?;
        req.agent = Some(def.id.clone());
        if req.name.trim().is_empty() && req.slug.is_none() && req.ticket_key.is_none() {
            let ticket = match req.ticket.as_deref() {
                Some(t) => xherd_core::new::parse_ticket(t)?,
                None => None,
            };
            let taken: std::collections::HashSet<String> = self
                .store
                .list_jobs()
                .await
                .map_err(store)?
                .into_iter()
                .map(|j| j.name)
                .collect();
            req.name = agent_def::default_job_name(
                &def.name,
                match &ticket {
                    Some(xherd_core::new::TicketInput::Key(k)) => Some(k.as_str()),
                    _ => None,
                },
                &|n| taken.contains(n),
            );
        }
        self.new_job(req).await
    }
}

#[cfg(test)]
mod tests {
    use xherd_core::Status;

    use super::*;

    fn job(name: &str, status: Status, created: i64) -> Job {
        let mut j = Job::new(name, created);
        j.status = status;
        j.created_at = created;
        j
    }

    #[test]
    fn summary_counts_live_jobs_and_names_the_newest() {
        let mut a = job("a", Status::Running, 1);
        let b = job("b", Status::Done, 3);
        let c = job("c", Status::Paused, 2);
        a.current_agent = Some("g1".into());
        assert_eq!(summarize("g1", &[]), (0, None));
        assert_eq!(summarize("g1", &[&a, &b, &c]), (1, Some("b".into())));
        // running, but another agent's stage now: not live for this one
        assert_eq!(summarize("g2", &[&a]), (0, Some("a".into())));
    }

    #[test]
    fn risky_fields_are_the_ones_that_widen_or_instruct() {
        let d: AgentDef = serde_json::from_str(r#"{"name":"a","runtime":"r"}"#).unwrap();
        assert!(risky_fields(&d).is_empty());
        let d: AgentDef = serde_json::from_str(
            r#"{"name":"a","runtime":"r",
                "permissionMode":"all","autonomy":"important","instructions":"x","skills":["s"]}"#,
        )
        .unwrap();
        assert_eq!(
            risky_fields(&d),
            ["permissionMode", "autonomy", "instructions", "skills"]
        );
    }
}
