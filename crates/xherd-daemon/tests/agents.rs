//! Saved agents (issue #141): CRUD, versions, archive, delete, the checks that
//! need plugins and packs, `broken`, export and import.

use std::sync::Arc;

use xherd_core::agent_def::{AgentDef, AgentStatus};
use xherd_core::paths::Paths;
use xherd_daemon::agents::Import;
use xherd_daemon::*;

struct T {
    _home: tempfile::TempDir,
    d: Daemon,
}

async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(home.path().join("ws").to_string_lossy().into_owned()),
        // A runtime that is installed and switched off: its agents are broken.
        disabled_plugins: vec!["opencode".into()],
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let d = Daemon::start(paths, o).await.unwrap();
    T { _home: home, d }
}

fn agent(name: &str) -> AgentDef {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "description": "Reviews branches",
        "runtime": "claude-code",
    }))
    .unwrap()
}

#[tokio::test]
async fn create_get_list_and_the_derived_fields() {
    let t = setup().await;
    let mut rx = t.d.hub.subscribe();
    let a = t.d.create_agent(agent("reviewer")).await.unwrap();
    assert_eq!(rx.recv().await.unwrap(), Change::Agents);
    assert_eq!((a.def.version, a.def.name.as_str()), (1, "reviewer"));
    assert!(!a.def.id.is_empty() && a.def.created_at > 0);
    assert_eq!(a.def.created_at, a.def.updated_at);
    assert_eq!(a.status, AgentStatus::Idle);
    assert_eq!((a.live_jobs, a.last_job, a.broken), (0, None, None));
    // by id or by name
    assert_eq!(t.d.agent("reviewer").await.unwrap().def.id, a.def.id);
    assert_eq!(t.d.agent(&a.def.id).await.unwrap().def.name, "reviewer");
    assert_eq!(t.d.agent("nope").await.unwrap_err().code, "unknown_agent");
    assert_eq!(t.d.agents().await.unwrap().len(), 1);
    // ids and versions the caller sends are ignored
    let mut b = agent("second");
    b.id = "mine".into();
    b.version = 9;
    let b = t.d.create_agent(b).await.unwrap();
    assert_ne!(b.def.id, "mine");
    assert_eq!(b.def.version, 1);
    // a name is unique
    assert_eq!(
        t.d.create_agent(agent("reviewer")).await.unwrap_err().code,
        "name_taken"
    );
}

#[tokio::test]
async fn checks_that_need_plugins_and_packs() {
    let t = setup().await;
    type Edit = Box<dyn Fn(&mut AgentDef)>;
    let cases: Vec<(&str, Edit)> = vec![
        ("bad_name", Box::new(|d| d.name = "Bad Name".into())),
        (
            "bad_fallback",
            Box::new(|d| d.fallback_runtime = Some("nothing".into())),
        ),
        // a plugin that exists but is no runtime
        (
            "bad_fallback",
            Box::new(|d| d.fallback_runtime = Some("github".into())),
        ),
        (
            "unknown_runtime",
            Box::new(|d| d.runtime = "nothing".into()),
        ),
        // a plugin that is not a runtime
        ("unknown_runtime", Box::new(|d| d.runtime = "github".into())),
        (
            "mode_unavailable",
            Box::new(|d| d.permission_mode = Some(xherd_core::permission::PermissionMode::All)),
        ),
        (
            "bad_agent",
            Box::new(|d| {
                // a runtime that exists but takes no model: github is no runtime
                d.models.insert("github".into(), "x".into());
            }),
        ),
        (
            "autonomy_unavailable",
            Box::new(|d| d.autonomy = Some(xherd_core::autonomy::Autonomy::Never)),
        ),
    ];
    for (code, edit) in cases {
        let mut d = agent("checked");
        edit(&mut d);
        assert_eq!(
            t.d.create_agent(d.clone()).await.unwrap_err().code,
            code,
            "{d:?}"
        );
    }
    assert!(t.d.agents().await.unwrap().is_empty(), "nothing was saved");
    // a fallback runtime that exists is fine
    let mut d = agent("plain");
    d.fallback_runtime = Some("opencode".into());
    t.d.create_agent(d).await.unwrap();
}

#[tokio::test]
async fn editing_keeps_every_version() {
    let t = setup().await;
    let a = t.d.create_agent(agent("reviewer")).await.unwrap();
    let mut edit = agent("reviewer");
    edit.description = "Reviews more".into();
    edit.max_live_jobs = 3;
    let b = t.d.update_agent("reviewer", edit).await.unwrap();
    assert_eq!((b.def.version, b.def.id.clone()), (2, a.def.id.clone()));
    assert_eq!(b.def.created_at, a.def.created_at);
    assert_eq!(b.def.max_live_jobs, 3);
    let versions = t.d.agent_versions("reviewer").await.unwrap();
    assert_eq!(
        versions.iter().map(|v| v.version).collect::<Vec<_>>(),
        [2, 1]
    );
    assert_eq!(versions[1].description, "Reviews branches");
    // the name is fixed, a refused edit changes nothing
    let mut rename = agent("other");
    rename.description = "x".into();
    assert_eq!(
        t.d.update_agent("reviewer", rename).await.unwrap_err().code,
        "name_fixed"
    );
    let mut bad = agent("reviewer");
    bad.runtime = "nothing".into();
    assert_eq!(
        t.d.update_agent(&a.def.id, bad).await.unwrap_err().code,
        "unknown_runtime"
    );
    assert_eq!(t.d.agent("reviewer").await.unwrap().def.version, 2);
    assert_eq!(
        t.d.update_agent("nope", agent("nope"))
            .await
            .unwrap_err()
            .code,
        "unknown_agent"
    );
}

#[tokio::test]
async fn archive_and_delete() {
    let t = setup().await;
    t.d.create_agent(agent("reviewer")).await.unwrap();
    let a = t.d.archive_agent("reviewer", true).await.unwrap();
    assert!(a.def.archived_at.is_some());
    assert_eq!((a.status, a.def.version), (AgentStatus::Archived, 1));
    let a = t.d.archive_agent("reviewer", false).await.unwrap();
    assert_eq!((a.def.archived_at, a.status), (None, AgentStatus::Idle));
    // an edit keeps the archive state
    t.d.archive_agent("reviewer", true).await.unwrap();
    let e =
        t.d.update_agent("reviewer", agent("reviewer"))
            .await
            .unwrap();
    assert!(e.def.archived_at.is_some());
    // delete removes it with its versions
    t.d.delete_agent("reviewer").await.unwrap();
    assert_eq!(
        t.d.agent("reviewer").await.unwrap_err().code,
        "unknown_agent"
    );
    assert_eq!(
        t.d.delete_agent("reviewer").await.unwrap_err().code,
        "unknown_agent"
    );
    // the name is free again
    t.d.create_agent(agent("reviewer")).await.unwrap();
}

#[tokio::test]
async fn an_agent_whose_runtime_is_off_is_broken_but_can_be_saved() {
    let t = setup().await;
    let mut d = agent("offline");
    d.runtime = "opencode".into();
    let a = t.d.create_agent(d).await.unwrap();
    assert!(a.broken.as_deref().unwrap().contains("switched off"));
    assert!(t.d.agent("offline").await.unwrap().broken.is_some());
    assert!(t.d.agent_broken(&agent("fine")).is_none());
}

#[tokio::test]
async fn export_and_import() {
    let t = setup().await;
    let mut a = agent("reviewer");
    a.instructions = Some("be brief".into());
    a.permission_mode = Some(xherd_core::permission::PermissionMode::Auto);
    t.d.create_agent(a).await.unwrap();
    t.d.create_agent(agent("plain")).await.unwrap();
    t.d.update_agent("plain", agent("plain")).await.unwrap();
    let all = t.d.export_agents(None).await.unwrap();
    assert_eq!(all["agents"].as_array().unwrap().len(), 2);
    let one = t.d.export_agents(Some("plain")).await.unwrap();
    let e = &one["agents"][0];
    assert_eq!(
        (
            e["id"].as_str(),
            e["version"].as_u64(),
            e["createdAt"].as_i64()
        ),
        (Some(""), Some(1), Some(0))
    );
    assert_eq!(
        t.d.export_agents(Some("nope")).await.unwrap_err().code,
        "unknown_agent"
    );

    // into another daemon: a dry run flags the risky fields, writes nothing
    let u = setup().await;
    let import: Import = serde_json::from_value(all.clone()).unwrap();
    let dry = u.d.import_agents(import, true).await.unwrap();
    assert_eq!(
        dry["agents"][0]["risky"],
        serde_json::json!(["permissionMode", "instructions"])
    );
    assert_eq!(dry["agents"][1]["risky"], serde_json::json!([]));
    assert_eq!(dry["nameTaken"], false);
    assert!(u.d.agents().await.unwrap().is_empty());
    let import: Import = serde_json::from_value(all.clone()).unwrap();
    assert_eq!(u.d.import_agents(import, false).await.unwrap()["added"], 2);
    let got = u.d.agents().await.unwrap();
    assert_eq!(got.len(), 2);
    assert!(
        got.iter()
            .all(|a| a.def.version == 1 && !a.def.id.is_empty())
    );

    // an existing name is refused, and nothing of that import is saved
    let mut fresh = agent("fresh");
    fresh.description = "new".into();
    let mixed = Import {
        agents: vec![fresh, agent("plain")],
        ..Default::default()
    };
    assert_eq!(
        u.d.import_agents(mixed, false).await.unwrap_err().code,
        "name_taken"
    );
    assert_eq!(u.d.agents().await.unwrap().len(), 2);
    // the dry run says so without refusing
    let mixed = Import {
        agents: vec![agent("plain")],
        ..Default::default()
    };
    assert_eq!(
        u.d.import_agents(mixed, true).await.unwrap()["nameTaken"],
        true
    );
    // one invalid agent imports none
    let mut bad = agent("broken-one");
    bad.runtime = "nothing".into();
    let mixed = Import {
        agents: vec![agent("good"), bad],
        ..Default::default()
    };
    let e = u.d.import_agents(mixed, false).await.unwrap_err();
    assert_eq!(e.code, "unknown_runtime");
    assert!(e.message.starts_with("broken-one: "));
    assert_eq!(u.d.agents().await.unwrap().len(), 2);
    // the same name twice in one import
    let twice = Import {
        agents: vec![agent("dup"), agent("dup")],
        ..Default::default()
    };
    assert_eq!(
        u.d.import_agents(twice, false).await.unwrap_err().code,
        "name_taken"
    );
}

/// An export from before stage agents still imports: the workflow, repository,
/// base branch and stage overrides are dropped, and the dry run says so.
#[tokio::test]
async fn an_old_export_imports_without_the_dropped_keys() {
    let t = setup().await;
    let old: Import = serde_json::from_value(serde_json::json!({"agents": [{
        "name": "old", "runtime": "claude-code",
        "workflow": {"pack": "xherd", "flow": "bugfix"}, "repo": "/r",
        "baseBranch": "main", "stageOverrides": {"ship": {"human": true}},
        "maxDiffLines": 7
    }]}))
    .unwrap();
    assert_eq!(
        old.ignored,
        [["workflow", "repo", "baseBranch", "stageOverrides"]]
    );
    let dry = t.d.import_agents(old, true).await.unwrap();
    assert_eq!(
        dry["agents"][0]["ignored"],
        serde_json::json!(["workflow", "repo", "baseBranch", "stageOverrides"])
    );
    let again: Import = serde_json::from_value(serde_json::json!({"agents": [{
        "name": "old", "runtime": "claude-code", "workflow": {"pack": "xherd", "flow": "none"}
    }]}))
    .unwrap();
    assert_eq!(t.d.import_agents(again, false).await.unwrap()["added"], 1);
    assert_eq!(t.d.agent("old").await.unwrap().def.name, "old");
}

fn row(stage: &str, agent: &str, human: bool) -> xherd_core::resolve::Assignment {
    xherd_core::resolve::Assignment {
        stage: stage.into(),
        agent_id: agent.into(),
        human,
    }
}

#[tokio::test]
async fn stages_are_assigned_and_resolved_through_the_previous_one() {
    let t = setup().await;
    let arch = t.d.create_agent(agent("architect")).await.unwrap();
    let rev = t.d.create_agent(agent("reviewer")).await.unwrap();
    let mut rx = t.d.hub.subscribe();
    // by name or id; the rows keep the id
    let v =
        t.d.set_assignments(
            "xherd",
            "feature",
            vec![
                row("design", "architect", false),
                row("review", &rev.def.id, true),
            ],
        )
        .await
        .unwrap();
    assert_eq!(rx.recv().await.unwrap(), Change::Agents);
    let by = |id: &str| v.stages.iter().find(|s| s.id == id).unwrap();
    assert_eq!(by("spec").agent, None);
    assert!(!by("spec").assigned);
    assert_eq!(by("design").agent.as_deref(), Some("architect"));
    assert_eq!(by("tasks").agent.as_deref(), Some("architect"));
    assert!(!by("tasks").assigned);
    assert_eq!(by("review").agent.as_deref(), Some("reviewer"));
    assert!(by("review").human && by("review").assigned);
    assert_eq!(by("ship").agent.as_deref(), Some("reviewer"));
    assert_eq!(by("design").command.as_deref(), Some("{prefix}design"));
    let stored = t.d.assignments("xherd", "feature").await.unwrap();
    assert_eq!(stored[0].agent_id, arch.def.id);
    // another flow is untouched; the listing has every flow
    assert!(t.d.assignments("xherd", "bugfix").await.unwrap().is_empty());
    assert_eq!(t.d.flows().await.unwrap().len(), 3);
    // PUT replaces
    t.d.set_assignments("xherd", "feature", vec![])
        .await
        .unwrap();
    assert!(
        t.d.assignments("xherd", "feature")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn fresh_check_is_set_with_the_assignments() {
    let t = setup().await;
    t.d.create_agent(agent("architect")).await.unwrap();
    let on = |v: &xherd_daemon::agents::FlowView| -> Vec<String> {
        v.stages
            .iter()
            .filter(|s| s.fresh_check)
            .map(|s| s.id.clone())
            .collect()
    };
    // off by default; only spec and design stages are allowed
    let v = t.d.flow("xherd", "feature").await.unwrap();
    assert!(on(&v).is_empty());
    let allowed: Vec<&str> = v
        .stages
        .iter()
        .filter(|s| s.fresh_check_allowed)
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(allowed, ["spec", "design"]);
    // set, with a duplicate dropped, and read back
    let v =
        t.d.set_flow_settings(
            "xherd",
            "feature",
            vec![row("design", "architect", false)],
            Some(vec!["spec".into(), "design".into(), "spec".into()]),
        )
        .await
        .unwrap();
    assert_eq!(on(&v), ["spec", "design"]);
    assert_eq!(
        on(&t.d.flow("xherd", "feature").await.unwrap()),
        ["spec", "design"]
    );
    // another flow is untouched
    assert!(on(&t.d.flow("xherd", "bugfix").await.unwrap()).is_empty());
    // absent keeps the set
    let v =
        t.d.set_assignments("xherd", "feature", vec![])
            .await
            .unwrap();
    assert_eq!(on(&v), ["spec", "design"]);
    // an empty list clears it
    let v =
        t.d.set_flow_settings("xherd", "feature", vec![], Some(vec![]))
            .await
            .unwrap();
    assert!(on(&v).is_empty());
    // refusals leave the set as it was
    let code = |r: Result<_, ActionError>| {
        r.map(|_: xherd_daemon::agents::FlowView| ())
            .unwrap_err()
            .code
    };
    t.d.set_flow_settings("xherd", "feature", vec![], Some(vec!["spec".into()]))
        .await
        .unwrap();
    assert_eq!(
        code(
            t.d.set_flow_settings("xherd", "feature", vec![], Some(vec!["nope".into()]))
                .await
        ),
        "unknown_stage"
    );
    assert_eq!(
        code(
            t.d.set_flow_settings("xherd", "feature", vec![], Some(vec!["review".into()]))
                .await
        ),
        "fresh_check_not_allowed"
    );
    assert_eq!(on(&t.d.flow("xherd", "feature").await.unwrap()), ["spec"]);
}

#[tokio::test]
async fn assignments_are_checked() {
    let t = setup().await;
    let a = t.d.create_agent(agent("architect")).await.unwrap();
    let mut other = agent("opencoder");
    other.runtime = "opencode".into();
    let o = t.d.create_agent(other).await.unwrap();
    let code = |r: Result<_, ActionError>| r.map(|_: FlowViewAny| ()).unwrap_err().code;
    type FlowViewAny = xherd_daemon::agents::FlowView;
    let set = |rows| t.d.set_assignments("xherd", "feature", rows);
    assert_eq!(
        code(t.d.set_assignments("nope", "feature", vec![]).await),
        "unknown_pack"
    );
    assert_eq!(
        code(t.d.set_assignments("xherd", "nope", vec![]).await),
        "unknown_flow"
    );
    assert_eq!(
        code(set(vec![row("nope", "architect", false)]).await),
        "unknown_stage"
    );
    assert_eq!(
        code(set(vec![row("design", "ghost", false)]).await),
        "unknown_agent"
    );
    assert_eq!(
        code(
            set(vec![
                row("design", "architect", false),
                row("design", "architect", false)
            ])
            .await
        ),
        "bad_assignment"
    );
    assert_eq!(
        code(
            set(vec![
                row("design", "architect", false),
                row("review", "opencoder", false)
            ])
            .await
        ),
        "mixed_runtime"
    );
    // one agent of the other runtime alone is fine
    set(vec![
        row("review", "opencoder", false),
        row("ship", "opencoder", false),
    ])
    .await
    .unwrap();
    // the same agent in two flows is fine; archived ones are refused
    t.d.set_assignments("xherd", "bugfix", vec![row("review", "opencoder", false)])
        .await
        .unwrap();
    set(vec![]).await.unwrap();
    t.d.set_assignments("xherd", "bugfix", vec![])
        .await
        .unwrap();
    t.d.archive_agent(&a.def.id, true).await.unwrap();
    assert_eq!(
        code(set(vec![row("design", "architect", false)]).await),
        "agent_archived"
    );
    let _ = o;
}

#[tokio::test]
async fn an_assigned_agent_cannot_be_archived_deleted_or_moved_to_another_runtime() {
    let t = setup().await;
    let a = t.d.create_agent(agent("architect")).await.unwrap();
    let b = t.d.create_agent(agent("reviewer")).await.unwrap();
    t.d.set_assignments(
        "xherd",
        "feature",
        vec![
            row("design", "architect", false),
            row("review", "reviewer", false),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        t.d.archive_agent("architect", true).await.unwrap_err().code,
        "agent_assigned"
    );
    assert_eq!(
        t.d.delete_agent("architect").await.unwrap_err().code,
        "agent_assigned"
    );
    // reviewer moving to opencode would leave the flow on two runtimes
    let mut moved = b.def.clone();
    moved.runtime = "opencode".into();
    assert_eq!(
        t.d.update_agent("reviewer", moved).await.unwrap_err().code,
        "mixed_runtime"
    );
    // both moving, one after the other, is refused at the first; so free one
    t.d.set_assignments("xherd", "feature", vec![row("design", "architect", false)])
        .await
        .unwrap();
    let mut moved = b.def.clone();
    moved.runtime = "opencode".into();
    t.d.update_agent("reviewer", moved).await.unwrap();
    // releasing the stage frees the agent
    t.d.set_assignments("xherd", "feature", vec![])
        .await
        .unwrap();
    t.d.archive_agent("architect", true).await.unwrap();
    t.d.delete_agent("architect").await.unwrap();
    let _ = a;
}

/// `source` marks an agent a flow plugin installed (R7): a hand-made agent
/// never gets one, an edit keeps the stored one, an export/import round trip
/// keeps it.
#[tokio::test]
async fn source_is_set_by_plugins_only_and_kept_by_edit_and_import() {
    let t = setup().await;
    let mut a = agent("acme-architect");
    a.source = Some("acme".into());
    let made = t.d.create_agent(a).await.unwrap();
    assert_eq!(made.def.source, None);

    let from_plugin = Import {
        agents: vec![{
            let mut a = agent("acme-sdd-architect");
            a.source = Some("acme-sdd".into());
            a
        }],
        ..Default::default()
    };
    t.d.import_agents(from_plugin, false).await.unwrap();
    let got = t.d.agent("acme-sdd-architect").await.unwrap();
    assert_eq!(got.def.source.as_deref(), Some("acme-sdd"));

    let mut edit = agent("acme-sdd-architect");
    edit.description = "Edited".into();
    edit.source = Some("other".into());
    let edited = t.d.update_agent("acme-sdd-architect", edit).await.unwrap();
    assert_eq!(edited.def.source.as_deref(), Some("acme-sdd"));
    assert_eq!(edited.def.version, 2);

    let export = t.d.export_agents(Some("acme-sdd-architect")).await.unwrap();
    assert_eq!(export["agents"][0]["source"], "acme-sdd");
    let u = setup().await;
    let import: Import = serde_json::from_value(export).unwrap();
    u.d.import_agents(import, false).await.unwrap();
    let back = u.d.agent("acme-sdd-architect").await.unwrap();
    assert_eq!(back.def.source.as_deref(), Some("acme-sdd"));
}
