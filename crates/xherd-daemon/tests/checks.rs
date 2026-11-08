//! Setup checks and settings (`/checks`, `/config`). Never the real HOME,
//! Claude Code config or `gh`.

use std::sync::Arc;

use serde_json::json;
use xherd_core::paths::Paths;
use xherd_daemon::*;

#[tokio::test]
async fn checks_report_and_fix_the_pack_install() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path().join("data"));
    paths.ensure().unwrap();
    let ws = home.path().join("ws");
    xherd_core::config::Config {
        workspace: Some(ws.to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.scheduler = None;
    // Only the runtime, with a check that always passes and a fixture trust file.
    o.builtin_plugins.retain(|m| m.name == "claude-code");
    let trust = home.path().join("claude.json");
    std::fs::write(&trust, "{}").unwrap();
    let rt = &mut o.builtin_plugins[0];
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    rt.checks =
        serde_json::from_value(json!([{"name": "runs", "argv": ["git", "--version"]}])).unwrap();
    let d = Daemon::start(paths, o).await.unwrap();

    let h = home.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let by_id = |cs: &[checks::Check], id: &str| cs.iter().find(|c| c.id == id).cloned().unwrap();

    let cs = d.checks(&env).await;
    assert!(by_id(&cs, "git").ok);
    let rt = by_id(&cs, "plugin:claude-code:runs");
    assert!(rt.ok && rt.required);
    // Trust is written per worktree; there is nothing to check up front.
    assert!(cs.iter().all(|c| c.id != "trust"));
    let pack = by_id(&cs, "pack");
    assert!(!pack.ok);
    assert!(matches!(pack.fix, Some(checks::Fix::Action { .. })));

    assert_eq!(
        d.fix_check("git", &env).await.unwrap_err().code,
        "not_fixable"
    );

    d.fix_check("pack", &env).await.unwrap();
    assert!(
        home.path()
            .join(".claude/commands/xherd/feature.md")
            .exists()
    );
    let cs = d.checks(&env).await;
    assert!(by_id(&cs, "pack").ok);

    // Settings: saved to config.json, applied at the next start.
    let v = d.config_view().unwrap();
    assert_eq!(v["restartNeeded"], false);
    assert!(v["config"].get("userToken").is_none());
    let patch = serde_json::from_value(json!({"welcomed": true})).unwrap();
    assert_eq!(d.save_config(patch).unwrap()["restartNeeded"], false);
    let patch = serde_json::from_value(json!({"maxConcurrentAgents": 2})).unwrap();
    let v = d.save_config(patch).unwrap();
    assert_eq!(
        (
            v["config"]["maxConcurrentAgents"].as_u64(),
            v["restartNeeded"].as_bool()
        ),
        (Some(2), Some(true))
    );
    assert_eq!(v["running"]["maxConcurrentAgents"], 4);
}

#[tokio::test]
async fn disabled_plugins_apply_live() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path().join("data"));
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(home.path().join("ws").to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.scheduler = None;
    // A runtime and two toolchains, harmless checks.
    o.builtin_plugins
        .retain(|m| ["claude-code", "nvm", "uv"].contains(&m.name.as_str()));
    for m in &mut o.builtin_plugins {
        m.checks = serde_json::from_value(json!([{"name": "runs", "argv": ["git", "--version"]}]))
            .unwrap();
    }
    let d = Daemon::start(paths, o).await.unwrap();
    let h = home.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());
    let ids = |cs: Vec<checks::Check>| cs.into_iter().map(|c| c.id).collect::<Vec<_>>();

    assert!(ids(d.checks(&env).await).contains(&"plugin:nvm:runs".to_string()));

    // Switching a plugin off applies at once and needs no restart.
    let patch = serde_json::from_value(json!({"disabledPlugins": ["nvm"]})).unwrap();
    let v = d.save_config(patch).unwrap();
    assert_eq!(v["restartNeeded"], false);
    assert!(!d.plugin_enabled("nvm") && d.plugin_enabled("claude-code"));
    let after = ids(d.checks(&env).await);
    assert!(!after.contains(&"plugin:nvm:runs".to_string()));
    assert!(after.contains(&"plugin:claude-code:runs".to_string()));

    // The last plugin of a kind, and unknown names, are refused.
    for off in [json!(["claude-code"]), json!(["nope"])] {
        let patch = serde_json::from_value(json!({ "disabledPlugins": off })).unwrap();
        assert_eq!(d.save_config(patch).unwrap_err().code, "bad_config");
    }
}
