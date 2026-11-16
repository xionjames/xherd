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

use crate::{ActionError, Change, Daemon};

fn err(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

fn store(e: xherd_store::StoreError) -> ActionError {
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
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub agents: Vec<AgentDef>,
}

/// The saved agent a job started from.
fn job_agent(job: &Job) -> Option<&str> {
    job.agent.as_ref().map(|a| a.id.as_str())
}

/// Live jobs and the newest job's name, from the jobs of one agent (pure).
fn summarize(jobs: &[&Job]) -> (usize, Option<String>) {
    let live = jobs.iter().filter(|j| j.status.is_live()).count();
    let last = jobs
        .iter()
        .max_by_key(|j| (j.created_at, j.id.as_str()))
        .map(|j| j.name.clone());
    (live, last)
}

/// The fields of an imported agent a person should look at before saying yes
/// (R22): they widen what a job may do, or add text the job reads.
fn risky_fields(d: &AgentDef) -> Vec<&'static str> {
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
            .filter(|j| job_agent(j) == Some(id))
            .collect();
        jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at));
        Ok(jobs)
    }

    async fn view(&self, def: AgentDef, jobs: &[&Job]) -> AgentView {
        let (live_jobs, last_job) = summarize(jobs);
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
            let mine: Vec<&Job> = jobs
                .iter()
                .filter(|j| job_agent(j) == Some(d.id.as_str()))
                .collect();
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
        let w = &def.workflow;
        let flow_none = w.flow == xherd_core::new::FLOW_NONE;
        let pack = self.packs.get(&w.pack);
        if !flow_none && pack.is_none() {
            return Err(err(
                "unknown_pack",
                format!("no flow pack named {}", w.pack),
            ));
        }
        let flow = match (flow_none, pack) {
            (false, Some(p)) => Some(p.flow(&w.flow).ok_or_else(|| {
                err(
                    "unknown_flow",
                    format!("pack {} has no flow {:?}", w.pack, w.flow),
                )
            })?),
            _ => None,
        };
        let off: Vec<String> = if running {
            self.live_config().disabled_plugins
        } else {
            Vec::new()
        };
        self.check_runtime(&def.runtime, &off)
            .map_err(|m| err("unknown_runtime", m))?;
        if let Some(p) = pack
            && !flow_none
            && !p.install.contains_key(&def.runtime)
        {
            return Err(err(
                "runtime_unusable",
                format!("the {} pack has no commands for {}", p.name, def.runtime),
            ));
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
        for stage in def.stage_overrides.keys() {
            let known = flow.is_some_and(|f| f.stages.iter().any(|s| &s.id == stage));
            if !known {
                return Err(err(
                    "unknown_stage",
                    format!(
                        "stageOverrides: the flow {:?} has no stage {stage:?}",
                        w.flow
                    ),
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

    fn agents_changed(&self) {
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
        def.id = old.id.clone();
        def.version = old.version + 1;
        def.created_at = old.created_at;
        def.updated_at = self.clock.now();
        def.archived_at = old.archived_at;
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
        for mut def in i.agents {
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
            out.push((def, name_taken));
        }
        if dry_run {
            let agents: Vec<Value> = out
                .iter()
                .map(|(d, t)| json!({"agent": d, "risky": risky_fields(d), "nameTaken": t}))
                .collect();
            return Ok(json!({"agents": agents, "nameTaken": taken}));
        }
        for (d, _) in &out {
            self.store.put_agent_def(d).await.map_err(store)?;
            self.sup.agent_limit(&d.id, Some(d.max_live_jobs));
        }
        self.agents_changed();
        Ok(json!({"added": out.len()}))
    }
}

impl Daemon {
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
        let a = job("a", Status::Running, 1);
        let b = job("b", Status::Done, 3);
        let c = job("c", Status::Paused, 2);
        assert_eq!(summarize(&[]), (0, None));
        assert_eq!(summarize(&[&a, &b, &c]), (1, Some("b".into())));
    }

    #[test]
    fn risky_fields_are_the_ones_that_widen_or_instruct() {
        let d: AgentDef = serde_json::from_str(
            r#"{"name":"a","runtime":"r","workflow":{"pack":"p","flow":"f"}}"#,
        )
        .unwrap();
        assert!(risky_fields(&d).is_empty());
        let d: AgentDef = serde_json::from_str(
            r#"{"name":"a","runtime":"r","workflow":{"pack":"p","flow":"f"},
                "permissionMode":"all","autonomy":"important","instructions":"x","skills":["s"]}"#,
        )
        .unwrap();
        assert_eq!(
            risky_fields(&d),
            ["permissionMode", "autonomy", "instructions", "skills"]
        );
    }
}
