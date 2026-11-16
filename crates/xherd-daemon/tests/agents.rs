//! Saved agents (issue #141): CRUD, versions, archive, delete, the checks that
//! need plugins and packs, `broken`, export and import.

use std::sync::Arc;

use xherd_core::agent_def::{AgentDef, AgentStatus, StageOverride, Workflow};
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
        "workflow": {"pack": "xherd", "flow": "feature"},
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
            "unknown_pack",
            Box::new(|d| d.workflow.pack = "other".into()),
        ),
        (
            "unknown_flow",
            Box::new(|d| d.workflow.flow = "hotfix".into()),
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
            "unknown_stage",
            Box::new(|d| {
                d.stage_overrides
                    .insert("nope".into(), StageOverride { human: true });
            }),
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
    // flow none needs no pack; a known stage override is fine
    let mut d = agent("plain");
    d.workflow = Workflow {
        pack: "xherd".into(),
        flow: "none".into(),
    };
    t.d.create_agent(d).await.unwrap();
    let mut d = agent("strict");
    d.stage_overrides
        .insert("ship".into(), StageOverride { human: true });
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
    bad.workflow.flow = "hotfix".into();
    assert_eq!(
        t.d.update_agent(&a.def.id, bad).await.unwrap_err().code,
        "unknown_flow"
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
    };
    assert_eq!(
        u.d.import_agents(mixed, false).await.unwrap_err().code,
        "name_taken"
    );
    assert_eq!(u.d.agents().await.unwrap().len(), 2);
    // the dry run says so without refusing
    let mixed = Import {
        agents: vec![agent("plain")],
    };
    assert_eq!(
        u.d.import_agents(mixed, true).await.unwrap()["nameTaken"],
        true
    );
    // one invalid agent imports none
    let mut bad = agent("broken-one");
    bad.workflow.flow = "hotfix".into();
    let mixed = Import {
        agents: vec![agent("good"), bad],
    };
    let e = u.d.import_agents(mixed, false).await.unwrap_err();
    assert_eq!(e.code, "unknown_flow");
    assert!(e.message.starts_with("broken-one: "));
    assert_eq!(u.d.agents().await.unwrap().len(), 2);
    // the same name twice in one import
    let twice = Import {
        agents: vec![agent("dup"), agent("dup")],
    };
    assert_eq!(
        u.d.import_agents(twice, false).await.unwrap_err().code,
        "name_taken"
    );
}
