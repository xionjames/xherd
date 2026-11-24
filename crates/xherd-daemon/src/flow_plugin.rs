//! Flow plugins: a pack, saved agents and default stage assignments shipped
//! as one plugin of kind `flow` (R1, R3, R4, R6). This part only reads and
//! checks the structure of a plugin folder; it needs no daemon.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;
use xherd_core::agent_def::{self, AgentDef};
use xherd_core::resolve::Assignment;
use xherd_flow::install::{missing_prompts, read_pack_dir};
use xherd_flow::manifest::Pack;
use xherd_plugin::manifest::{Manifest, flow_data};

use xherd_core::Status;

use crate::agents::Import;
use crate::{ActionError, Daemon};

/// What a flow plugin folder holds, checked for structure.
#[derive(Debug)]
pub struct FlowPlugin {
    pub pack: Pack,
    /// The pack's own files (`FORMAT.md`, the prompts), `plan_install`'s input.
    pub files: Vec<(String, String)>,
    /// Named `<plugin>-<name>`, `source` set, no id or dates.
    pub agents: Vec<AgentDef>,
    /// `(flow, assignment)`; `agent_id` holds the agent's saved name until
    /// the install maps it to an id.
    pub assignments: Vec<(String, Assignment)>,
}

/// Read the plugin in `dir` described by `m` (kind `flow`). Errors are the
/// text of a `bad_manifest` refusal.
pub fn read_flow_plugin(dir: &Path, m: &Manifest) -> Result<FlowPlugin, String> {
    let data = flow_data(m)?;
    let pack_dir = dir.join(&data.pack);
    let yaml = std::fs::read_to_string(pack_dir.join("xherd-flow.yaml"))
        .map_err(|e| format!("{}/xherd-flow.yaml: {e}", data.pack))?;
    let pack = Pack::parse(&yaml).map_err(|e| format!("{}: {}", e.pack, e.message))?;
    if pack.name != m.name {
        return Err(format!(
            "the pack is named {:?}: it must be named like the plugin, {:?}",
            pack.name, m.name
        ));
    }
    if pack.name == "xherd" || pack.name.starts_with("xherd-") {
        return Err(format!(
            "the pack name {:?} is reserved: `xherd` and `xherd-*` belong to the built-in pack",
            pack.name
        ));
    }
    if pack.spec_dir != "private" {
        return Err(format!(
            "specDir must be `private` (the pack has {:?})",
            pack.spec_dir
        ));
    }
    if !pack.schemas.is_empty() {
        return Err("a plugin pack cannot have `schemas`".into());
    }
    let files = read_pack_dir(&pack_dir, &pack)?;
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let missing = missing_prompts(&pack, &refs);
    if !missing.is_empty() {
        return Err(format!("missing prompt files: {}", missing.join("; ")));
    }

    let agents = match &data.agents {
        Some(file) => read_agents(&dir.join(file), file, &m.name)?,
        None => Vec::new(),
    };
    let mut assignments = Vec::new();
    for (flow, stages) in &data.assignments {
        let Some(def) = pack.flow(flow) else {
            return Err(format!("assignments: the pack has no flow {flow:?}"));
        };
        let mut runtimes = BTreeSet::new();
        for (stage, a) in stages {
            if !def.stages.iter().any(|s| &s.id == stage) {
                return Err(format!("assignments: flow {flow} has no stage {stage:?}"));
            }
            let saved = format!("{}-{}", m.name, a.agent);
            let Some(agent) = agents.iter().find(|d| d.name == saved) else {
                return Err(format!(
                    "assignments: {flow}/{stage} names agent {:?}, which the agents file does not have",
                    a.agent
                ));
            };
            runtimes.insert(agent.runtime.clone());
            assignments.push((
                flow.clone(),
                Assignment {
                    stage: stage.clone(),
                    agent_id: saved,
                    human: a.human,
                },
            ));
        }
        if runtimes.len() > 1 {
            let list: Vec<_> = runtimes.into_iter().collect();
            return Err(format!(
                "the agents of flow {flow} must share a runtime ({}); \
                 a job cannot change runtime between stages yet",
                list.join(" and ")
            ));
        }
    }
    Ok(FlowPlugin {
        pack,
        files,
        agents,
        assignments,
    })
}

/// The agents file in the export format, renamed `<plugin>-<name>` and
/// stamped with the plugin as `source`.
fn read_agents(path: &Path, shown: &str, plugin: &str) -> Result<Vec<AgentDef>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{shown}: {e}"))?;
    let import: Import = serde_json::from_str(&text).map_err(|e| format!("{shown}: {e}"))?;
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for mut def in import.agents {
        if !seen.insert(def.name.clone()) {
            return Err(format!("{shown}: the agent {:?} appears twice", def.name));
        }
        def.name = format!("{plugin}-{}", def.name);
        def.source = Some(plugin.to_string());
        def.id = String::new();
        def.created_at = Default::default();
        def.updated_at = Default::default();
        def.archived_at = None;
        agent_def::check(&mut def)
            .map_err(|e| format!("{shown}: agent {}: {}", def.name, e.message))?;
        out.push(def);
    }
    Ok(out)
}

/// What the preview of a flow plugin needs from the daemon's state, read
/// before the (sync) inspection of the fetched folder.
#[derive(Debug, Default)]
pub(crate) struct FlowContext {
    pub agents: Vec<AgentDef>,
    /// `(pack, flow, assignment)` as stored.
    pub assignments: Vec<(String, String, Assignment)>,
    /// Pack → the first job that has not ended and uses it.
    pub in_use: BTreeMap<String, String>,
}

/// The flow part of a plugin preview (R5, R8).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowPluginView {
    pub pack: String,
    pub memory_note: &'static str,
    pub flows: Vec<PackFlowView>,
    pub agents: Vec<PluginAgentView>,
    /// Saved agents of this plugin that the new version no longer has.
    pub archived: Vec<String>,
    /// The default assignments the plugin ships.
    pub assignments: Vec<AssignmentView>,
    /// Stored assignments of the pack that the defaults would change.
    pub assignment_changes: Vec<AssignmentChange>,
    pub files: Vec<RuntimeFiles>,
    pub in_use_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PackFlowView {
    pub flow: String,
    pub stages: Vec<PackStageView>,
}

#[derive(Debug, Serialize)]
pub struct PackStageView {
    pub id: String,
    pub role: xherd_core::domain::Role,
    pub class: xherd_core::domain::Class,
    pub gate: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginAgentView {
    pub name: String,
    pub saved_name: String,
    /// `new`, `taken` (hand-made or from another plugin), `reclaimed` (kept
    /// from an earlier install), `overwritten` or `unchanged`.
    pub state: &'static str,
    pub risky: Vec<&'static str>,
    pub issues: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AssignmentView {
    pub flow: String,
    pub stage: String,
    pub agent: String,
    pub human: bool,
}

#[derive(Debug, Serialize)]
pub struct AssignmentChange {
    pub flow: String,
    pub stage: String,
    pub from: String,
    /// `None`: the plugin ships no default for the stage any more.
    pub to: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeFiles {
    pub runtime: String,
    pub paths: Vec<String>,
}

/// Whether two saved agents say the same thing (ids, versions, dates and the
/// archive mark are bookkeeping).
fn same_content(a: &AgentDef, b: &AgentDef) -> bool {
    let mut a = a.clone();
    a.id = b.id.clone();
    a.version = b.version;
    a.created_at = b.created_at;
    a.updated_at = b.updated_at;
    a.archived_at = b.archived_at;
    a == *b
}

impl Daemon {
    /// R6, R7, R9: store a flow plugin's record, its agents and its default
    /// assignments in one transaction, under the agents lock. An agent of the
    /// plugin that is already saved (an earlier install) keeps its id and
    /// dates and gets a new version only when its content changed. An install
    /// brings it back from the archive; an update (R8) leaves it archived and
    /// archives the plugin's agents the new version dropped. A name held by
    /// another agent is `name_taken` and nothing is written.
    pub(crate) async fn apply_flow_plugin(
        &self,
        record: &xherd_store::PluginRecord,
        fp: &FlowPlugin,
        update: bool,
    ) -> Result<(), ActionError> {
        let store = crate::agents::store;
        let _w = self.agents_write.lock().await;
        let have = self.store.list_agent_defs().await.map_err(store)?;
        let now = self.clock.now();
        let mut defs = Vec::new();
        for def in &fp.agents {
            let mut def = def.clone();
            match have.iter().find(|a| a.name == def.name) {
                None => {
                    def.id = xherd_core::new_id();
                    def.version = 1;
                    def.created_at = now;
                    def.updated_at = now;
                }
                Some(old) if old.source.as_deref() != def.source.as_deref() => {
                    return Err(crate::create::refuse(
                        "name_taken",
                        format!(
                            "an agent named {} exists and is not from this plugin",
                            def.name
                        ),
                    ));
                }
                Some(old) => {
                    def.id = old.id.clone();
                    def.created_at = old.created_at;
                    def.archived_at = if update { old.archived_at } else { None };
                    if same_content(&def, old) {
                        def.version = old.version;
                        def.updated_at = old.updated_at;
                    } else {
                        def.version = old.version + 1;
                        def.updated_at = now;
                    }
                }
            }
            defs.push(def);
        }
        if update {
            for old in &have {
                if old.source.as_deref() == Some(record.name.as_str())
                    && old.archived_at.is_none()
                    && !defs.iter().any(|d| d.name == old.name)
                {
                    let mut gone = old.clone();
                    gone.archived_at = Some(now);
                    defs.push(gone);
                }
            }
        }
        let rows: Vec<(String, Assignment)> = fp
            .assignments
            .iter()
            .map(|(flow, a)| {
                let id = defs
                    .iter()
                    .find(|d| d.name == a.agent_id)
                    .map(|d| d.id.clone())
                    .unwrap_or_default();
                (
                    flow.clone(),
                    Assignment {
                        agent_id: id,
                        ..a.clone()
                    },
                )
            })
            .collect();
        self.store
            .put_flow_plugin(record, &defs, &fp.pack.name, &rows)
            .await
            .map_err(store)?;
        for d in &defs {
            self.sup.agent_limit(&d.id, Some(d.max_live_jobs));
        }
        self.agents_changed();
        Ok(())
    }

    pub(crate) async fn flow_context(&self) -> Result<FlowContext, ActionError> {
        let store = crate::agents::store;
        let agents = self.store.list_agent_defs().await.map_err(store)?;
        let assignments = self.store.all_flow_assignments().await.map_err(store)?;
        let mut in_use = BTreeMap::new();
        for j in self.store.list_jobs().await.map_err(store)? {
            if !matches!(j.status, Status::Done | Status::Cancelled) {
                let pack = j.pack.unwrap_or_else(|| xherd_flow::BUILTIN.to_string());
                in_use.entry(pack).or_insert(j.name);
            }
        }
        Ok(FlowContext {
            agents,
            assignments,
            in_use,
        })
    }

    /// What `validate` would refuse for a reason the install tolerates: the
    /// runtime is missing or off, a model belongs to no runtime, a skill is
    /// not in the library (R4, R9). Shown in the preview.
    pub(crate) fn plugin_agent_issues(&self, def: &AgentDef) -> Vec<String> {
        let off = self.live_config().disabled_plugins;
        let mut out = Vec::new();
        if let Err(m) = self.check_runtime(&def.runtime, &off) {
            let fallback_runs = def
                .fallback_runtime
                .as_deref()
                .is_some_and(|f| self.check_runtime(f, &off).is_ok());
            if !fallback_runs {
                out.push(format!("runtime: {m}"));
            }
        }
        if let Some(f) = &def.fallback_runtime
            && let Err(m) = self.check_runtime(f, &[])
        {
            out.push(format!("fallbackRuntime: {m}"));
        }
        for rt in def.models.keys() {
            if let Err(m) = self.check_model_runtime(rt) {
                out.push(format!("models: {m}"));
            }
        }
        for s in &def.skills {
            if !self.has_skill(s) {
                out.push(format!("skills: no skill named {s:?} in the library"));
            }
        }
        out
    }

    /// The flow part of a preview (R5, R8). `installed`: the plugin is
    /// installed already, so an agent of its own is being updated, not taken
    /// back. Refuses what no install tolerates: a permission mode the host
    /// does not offer, and install paths outside the pack's folder (R12).
    pub(crate) fn flow_plugin_view(
        &self,
        fp: &FlowPlugin,
        plugin: &str,
        installed: bool,
        ctx: &FlowContext,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Result<FlowPluginView, String> {
        let files: Vec<(&str, &str)> = fp
            .files
            .iter()
            .map(|(n, t)| (n.as_str(), t.as_str()))
            .collect();
        let builtin = self
            .packs
            .get(xherd_flow::BUILTIN)
            .ok_or("no built-in pack")?;
        xherd_flow::install::check_plugin_install(&fp.pack, builtin, &files, env)?;
        let mut runtime_files = Vec::new();
        for rt in fp.pack.install.keys() {
            let plan = fp.pack.plan_install(rt, &files, env)?;
            runtime_files.push(RuntimeFiles {
                runtime: rt.clone(),
                paths: plan
                    .iter()
                    .map(|p| p.path.to_string_lossy().into_owned())
                    .collect(),
            });
        }

        let mut agents = Vec::new();
        for def in &fp.agents {
            if def
                .permission_mode
                .is_some_and(|m| !m.allowed(self.sandbox))
            {
                return Err(format!(
                    "agent {}: permission mode all asks nothing, so it is only offered in the container",
                    def.name
                ));
            }
            let state = match ctx.agents.iter().find(|a| a.name == def.name) {
                None => "new",
                Some(a) if a.source.as_deref() != Some(plugin) => "taken",
                Some(_) if !installed => "reclaimed",
                Some(a) if same_content(def, a) => "unchanged",
                Some(_) => "overwritten",
            };
            agents.push(PluginAgentView {
                name: def
                    .name
                    .strip_prefix(&format!("{plugin}-"))
                    .unwrap_or(&def.name)
                    .to_string(),
                saved_name: def.name.clone(),
                state,
                risky: crate::agents::risky_fields(def),
                issues: self.plugin_agent_issues(def),
            });
        }
        let archived = ctx
            .agents
            .iter()
            .filter(|a| a.source.as_deref() == Some(plugin) && a.archived_at.is_none())
            .filter(|a| !fp.agents.iter().any(|d| d.name == a.name))
            .map(|a| a.name.clone())
            .collect();

        let name_of = |id: &str| {
            ctx.agents
                .iter()
                .find(|a| a.id == id)
                .map_or_else(|| id.to_string(), |a| a.name.clone())
        };
        let pack = &fp.pack.name;
        let assignments = fp
            .assignments
            .iter()
            .map(|(flow, a)| AssignmentView {
                flow: flow.clone(),
                stage: a.stage.clone(),
                agent: a.agent_id.clone(),
                human: a.human,
            })
            .collect();
        let assignment_changes = ctx
            .assignments
            .iter()
            .filter(|(p, _, _)| p == pack)
            .filter_map(|(_, flow, stored)| {
                let to = fp
                    .assignments
                    .iter()
                    .find(|(f, a)| f == flow && a.stage == stored.stage)
                    .map(|(_, a)| a.agent_id.clone());
                (to.as_deref() != Some(&name_of(&stored.agent_id))).then(|| AssignmentChange {
                    flow: flow.clone(),
                    stage: stored.stage.clone(),
                    from: name_of(&stored.agent_id),
                    to,
                })
            })
            .collect();

        Ok(FlowPluginView {
            pack: pack.clone(),
            memory_note: match fp.pack.memory_note {
                xherd_flow::manifest::MemoryNote::Required => "required",
                xherd_flow::manifest::MemoryNote::Off => "off",
            },
            flows: fp
                .pack
                .flows
                .iter()
                .map(|(flow, def)| PackFlowView {
                    flow: flow.clone(),
                    stages: def
                        .stages
                        .iter()
                        .map(|s| PackStageView {
                            id: s.id.clone(),
                            role: s.role,
                            class: s.class,
                            gate: s.gate.is_some(),
                        })
                        .collect(),
                })
                .collect(),
            agents,
            archived,
            assignments,
            assignment_changes,
            files: runtime_files,
            in_use_by: ctx.in_use.get(pack).cloned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    const PACK: &str = r#"
name: acme-sdd
version: 1.2.0
protocol: 1
specDir: private
memoryNote: required
private: [HANDOFF.md]
install:
  claude-code:
    prefix: "/acme-sdd:"
    commands: { from: commands/*.md, to: "${CLAUDE_CONFIG_DIR:-~/.claude}/commands/acme-sdd/{name}.md", format: command }
    args: "$ARGUMENTS"
commands: { resume: "{prefix}resume", handoff: "{prefix}handoff", status: "{prefix}status", ask: "{prefix}ask {prompt}" }
flows:
  feature:
    start: "{prefix}start {key} {prompt}"
    stages:
      - { id: brief,  role: spec,   class: critical, gate: { docs: [spec.md] },   next: [{ run: "{prefix}design" }] }
      - { id: design, role: design, class: routine,  command: "{prefix}design", gate: { docs: [design.md] }, next: [{ run: "{prefix}plan" }] }
      - { id: plan,   role: tasks,  class: routine,  command: "{prefix}plan",   gate: { docs: [tasks.md] },  next: [{ run: "{prefix}next" }] }
      - { id: implement, role: implement, class: code, command: "{prefix}next", gate: { kind: commit },
          next: [{ run: "{prefix}commit {commit}" }], loopUntil: allTasksDone, then: review }
      - { id: review, role: review, class: routine,  command: "{prefix}review", gate: { docs: [review.md] }, next: [{ run: "{prefix}ship" }] }
      - { id: ship,   role: ship,   class: critical, command: "{prefix}ship",   gate: { docs: [pr.md] },     next: [{ run: "{prefix}ship" }] }
"#;

    const PROMPTS: [&str; 12] = [
        "rules", "start", "design", "plan", "next", "commit", "review", "ship", "ask", "resume",
        "handoff", "status",
    ];

    const MANIFEST: &str = r#"{
        "name": "acme-sdd", "kind": "flow", "version": "1.2.0",
        "data": {"flow": {"pack": "pack", "agents": "agents.json", "assignments": {
            "feature": {
                "brief": {"agent": "analyst"},
                "design": {"agent": "architect", "human": true},
                "plan": {"agent": "planner"},
                "implement": {"agent": "coder"},
                "review": {"agent": "reviewer"}}}}}
    }"#;

    fn agents_json() -> String {
        let one = |n: &str| format!(r#"{{"name": "{n}", "runtime": "claude-code"}}"#);
        let list: Vec<_> = ["analyst", "architect", "planner", "coder", "reviewer"]
            .iter()
            .map(|n| one(n))
            .collect();
        format!(r#"{{"agents": [{}]}}"#, list.join(","))
    }

    /// The `acme-sdd` plugin of the design, in a fresh folder.
    fn plugin(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("xherd-flow-plugin-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("pack/commands")).unwrap();
        fs::write(dir.join("xherd-plugin.json"), MANIFEST).unwrap();
        fs::write(dir.join("agents.json"), agents_json()).unwrap();
        fs::write(dir.join("pack/xherd-flow.yaml"), PACK).unwrap();
        for p in PROMPTS {
            fs::write(
                dir.join(format!("pack/commands/{p}.md")),
                format!("# {p}\n"),
            )
            .unwrap();
        }
        dir
    }

    fn read(dir: &Path) -> Result<FlowPlugin, String> {
        let m = Manifest::parse(&fs::read_to_string(dir.join("xherd-plugin.json")).unwrap())?;
        read_flow_plugin(dir, &m)
    }

    fn refused(name: &str, edit: impl FnOnce(&Path)) -> String {
        let dir = plugin(name);
        edit(&dir);
        let err = read(&dir).unwrap_err();
        let _ = fs::remove_dir_all(&dir);
        err
    }

    fn replace(path: &Path, from: &str, to: &str) {
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains(from), "{from} not in {}", path.display());
        fs::write(path, text.replace(from, to)).unwrap();
    }

    #[test]
    fn the_acme_sdd_example_parses() {
        let dir = plugin("ok");
        let fp = read(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(fp.pack.name, "acme-sdd");
        let names: Vec<_> = fp.agents.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "acme-sdd-analyst",
                "acme-sdd-architect",
                "acme-sdd-planner",
                "acme-sdd-coder",
                "acme-sdd-reviewer"
            ]
        );
        assert!(
            fp.agents
                .iter()
                .all(|a| a.source.as_deref() == Some("acme-sdd"))
        );
        assert!(fp.agents.iter().all(|a| a.id.is_empty()));
        assert_eq!(fp.assignments.len(), 5);
        let design = fp
            .assignments
            .iter()
            .find(|(_, a)| a.stage == "design")
            .unwrap();
        assert_eq!(design.0, "feature");
        assert_eq!(design.1.agent_id, "acme-sdd-architect");
        assert!(design.1.human);
        assert!(fp.files.iter().any(|(p, _)| p == "commands/design.md"));
    }

    #[test]
    fn a_pack_without_agents_is_allowed() {
        let dir = plugin("noagents");
        fs::write(
            dir.join("xherd-plugin.json"),
            r#"{"name": "acme-sdd", "kind": "flow", "version": "1.2.0",
                "data": {"flow": {"pack": "pack"}}}"#,
        )
        .unwrap();
        let fp = read(&dir).unwrap();
        let _ = fs::remove_dir_all(&dir);
        assert!(fp.agents.is_empty() && fp.assignments.is_empty());
    }

    #[test]
    fn pack_names_are_checked() {
        for (case, name) in [("a", "xherd"), ("b", "xherd-x"), ("c", "other")] {
            let err = refused(case, |d| {
                replace(
                    &d.join("pack/xherd-flow.yaml"),
                    "name: acme-sdd",
                    &format!("name: {name}"),
                );
            });
            assert!(err.contains(name), "{name}: {err}");
        }
        // A plugin called `xherd-x` is refused even when the pack matches it.
        let dir = plugin("reserved");
        replace(
            &dir.join("xherd-plugin.json"),
            r#""name": "acme-sdd""#,
            r#""name": "xherd-x""#,
        );
        replace(
            &dir.join("pack/xherd-flow.yaml"),
            "name: acme-sdd",
            "name: xherd-x",
        );
        let err = read(&dir).unwrap_err();
        let _ = fs::remove_dir_all(&dir);
        assert!(err.contains("reserved"), "{err}");
    }

    #[test]
    fn the_spec_folder_and_schemas_are_checked() {
        let err = refused("spec", |d| {
            replace(
                &d.join("pack/xherd-flow.yaml"),
                "specDir: private",
                "specDir: xherd/{agent}",
            );
        });
        assert!(err.contains("specDir"), "{err}");
        let err = refused("schemas", |d| {
            replace(
                &d.join("pack/xherd-flow.yaml"),
                "private: [HANDOFF.md]",
                "private: [HANDOFF.md]\nschemas: { spec.md: schemas/spec.json }",
            );
        });
        assert!(err.contains("schemas"), "{err}");
    }

    #[test]
    fn a_command_without_a_prompt_is_refused() {
        let err = refused("prompt", |d| {
            fs::remove_file(d.join("pack/commands/plan.md")).unwrap()
        });
        assert!(err.contains("missing prompt"), "{err}");
        assert!(err.contains("plan"), "{err}");
    }

    #[test]
    fn assignments_must_name_real_stages_flows_and_agents() {
        let edit = |from: &'static str, to: &'static str| {
            move |d: &Path| replace(&d.join("xherd-plugin.json"), from, to)
        };
        let err = refused("stage", edit(r#""review": {"agent""#, r#""qa": {"agent""#));
        assert!(err.contains("qa"), "{err}");
        let err = refused("flow", edit(r#""feature": {"#, r#""bugfix": {"#));
        assert!(err.contains("bugfix"), "{err}");
        let err = refused(
            "agent",
            edit(r#""agent": "reviewer""#, r#""agent": "tester""#),
        );
        assert!(err.contains("tester"), "{err}");
    }

    #[test]
    fn the_agents_of_a_flow_share_a_runtime() {
        let err = refused("mixed", |d| {
            replace(
                &d.join("agents.json"),
                r#"{"name": "coder", "runtime": "claude-code"}"#,
                r#"{"name": "coder", "runtime": "codex"}"#,
            );
        });
        assert!(err.contains("share a runtime"), "{err}");
    }

    #[test]
    fn agent_names_are_checked() {
        let err = refused("dup", |d| {
            replace(
                &d.join("agents.json"),
                r#"{"name": "coder","#,
                r#"{"name": "planner", "#,
            );
        });
        assert!(err.contains("twice"), "{err}");
        // `acme-sdd-` is 9 characters: a 33-character name makes 42.
        let long = "a".repeat(33);
        let err = refused("long", |d| {
            replace(
                &d.join("agents.json"),
                r#""name": "coder""#,
                &format!(r#""name": "{long}""#),
            );
        });
        assert!(err.contains("acme-sdd-aaaa"), "{err}");
        let ok = "a".repeat(32);
        let dir = plugin("limit");
        replace(
            &dir.join("agents.json"),
            r#""name": "coder""#,
            &format!(r#""name": "{ok}""#),
        );
        replace(
            &dir.join("xherd-plugin.json"),
            r#""agent": "coder""#,
            &format!(r#""agent": "{ok}""#),
        );
        let res = read(&dir);
        let _ = fs::remove_dir_all(&dir);
        res.unwrap();
    }
}
