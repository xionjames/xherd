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

    // Most settings apply at once: the actors' snapshot changes, no restart.
    let patch =
        serde_json::from_value(json!({"maxReviewLoops": 7, "freshSessions": false})).unwrap();
    let v = d.save_config(patch).unwrap();
    assert_eq!(v["running"]["maxReviewLoops"], 7);
    assert_eq!(d.settings.get().max_review_loops, 7);
    assert!(!d.settings.get().fresh_sessions);
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

/// Git providers are optional, like trackers: all of them may be off, while
/// the last runtime still may not.
#[tokio::test]
async fn git_providers_may_all_be_off() {
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
    o.builtin_plugins.retain(|m| {
        ["claude-code", "opencode", "github", "gitlab", "bitbucket"].contains(&m.name.as_str())
    });
    let d = Daemon::start(paths, o).await.unwrap();

    let patch =
        serde_json::from_value(json!({"disabledPlugins": ["github", "gitlab", "bitbucket"]}))
            .unwrap();
    d.save_config(patch).unwrap();
    assert!(!d.plugin_enabled("github") && !d.plugin_enabled("bitbucket"));

    let patch = serde_json::from_value(
        json!({"disabledPlugins": ["github", "gitlab", "bitbucket", "claude-code", "opencode"]}),
    )
    .unwrap();
    let e = d.save_config(patch).unwrap_err();
    assert_eq!(e.code, "bad_config");
    assert!(e.message.contains("runtime"), "{}", e.message);
}

/// R7: view plugins are optional too: the only one may be switched off.
#[tokio::test]
async fn view_plugins_may_all_be_off() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path().join("data"));
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(home.path().join("ws").to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let dir = paths.plugins().join("my-view");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("xherd-plugin.json"),
        r#"{"name": "my-view", "kind": "view", "version": "0.1.0",
            "data": {"view": {"entry": "index.html"}}}"#,
    )
    .unwrap();
    std::fs::write(dir.join("index.html"), "<p>view</p>").unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.scheduler = None;
    o.builtin_plugins.retain(|m| m.name == "claude-code");
    let d = Daemon::start(paths, o).await.unwrap();
    assert!(
        d.plugin_enabled("my-view"),
        "the external view plugin loaded"
    );

    let patch = serde_json::from_value(json!({"disabledPlugins": ["my-view"]})).unwrap();
    d.save_config(patch).unwrap();
    assert!(!d.plugin_enabled("my-view"));
}

/// R7, R8: a Git provider that is on gives its host an implicit Git access
/// row (in `GET /git/hosts`, and the managed gitconfig in the container only;
/// never saved); a saved
/// row for that host replaces it, and switching the provider off removes it.
#[tokio::test]
async fn providers_that_are_on_give_implicit_git_access_rows() {
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
    o.builtin_plugins
        .retain(|m| ["claude-code", "github", "gitlab"].contains(&m.name.as_str()));
    let d = Daemon::start(paths, o).await.unwrap();
    let gitconfig = || std::fs::read_to_string(d.gitconfig_path().unwrap()).unwrap();
    let rows = || d.git_hosts()["hosts"].as_array().unwrap().clone();

    // GitHub on: the row is listed from the plugin. Natively the gitconfig is
    // included from the user's own, so an implicit row writes no helper there
    // (it would replace theirs); the container's gets it (not testable here).
    assert!(!gitconfig().contains("github.com"));
    let r = rows();
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!(
        (&r[0]["host"], &r[0]["from"], &r[0]["hasToken"]),
        (&json!("github.com"), &json!("github"), &json!(false))
    );
    let saved = std::fs::read_to_string(home.path().join("data/config.json")).unwrap();
    assert!(!saved.contains("github.com"), "never saved: {saved}");

    // A saved row for the host takes its place, whatever its case, and a
    // saved row is what writes the provider's helper natively.
    let u = serde_json::from_value(json!({"hosts": [{
        "host": "GitHub.com", "protocols": ["https"], "credential": "provider"
    }]}))
    .unwrap();
    d.set_git_hosts(u).unwrap();
    let r = rows();
    assert_eq!(r.len(), 1, "{r:?}");
    assert!(r[0].get("from").is_none());
    assert!(gitconfig().contains("gh auth git-credential"));

    // Off: no implicit row, no helper.
    d.set_git_hosts(serde_json::from_value(json!({"hosts": []})).unwrap())
        .unwrap();
    let patch = serde_json::from_value(json!({"disabledPlugins": ["github"]})).unwrap();
    d.save_config(patch).unwrap();
    assert!(rows().is_empty());
    assert!(!gitconfig().contains("github.com"));
}

/// R-691: a tool a plugin requires is a check; natively a missing one shows
/// the instructions (the host is never installed into), and installing it
/// from the UI is refused.
#[tokio::test]
async fn a_missing_required_tool_shows_its_instructions_natively() {
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
    o.builtin_plugins
        .retain(|m| ["claude-code", "nvm"].contains(&m.name.as_str()));
    for m in &mut o.builtin_plugins {
        m.checks = vec![];
        if m.name == "nvm" {
            m.requires = serde_json::from_value(json!([{
                "name": "xherd-no-such-tool",
                "optional": true,
                "install": ["sh", "-c", "echo installing"],
                "instructions": {"any": "Install it from example.test"}
            }]))
            .unwrap();
        }
    }
    let d = Daemon::start(paths, o).await.unwrap();
    let h = home.path().to_string_lossy().into_owned();
    let env = move |k: &str| (k == "HOME").then(|| h.clone());

    let cs = d.checks(&env).await;
    let c = cs
        .iter()
        .find(|c| c.id == "plugin:nvm:requires:xherd-no-such-tool")
        .expect("the requirement is a check");
    assert!(!c.ok && !c.required);
    assert_eq!(
        c.fix,
        Some(checks::Fix::Command {
            text: "Install it from example.test".into()
        })
    );
    let e = d
        .fix_check("plugin:nvm:requires:xherd-no-such-tool", &env)
        .await
        .unwrap_err();
    assert_eq!(e.code, "not_fixable");
}

/// Tracker link rules (D24-style `data.tracker`): `{baseUrl}` comes from the
/// plugin's setting, a rule that needs one it lacks is left out, and no
/// tracker at all is allowed.
#[tokio::test]
async fn tracker_links_use_the_base_url_and_trackers_may_all_be_off() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path().join("data"));
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(home.path().join("ws").to_string_lossy().into_owned()),
        plugin_settings: serde_json::from_value(
            json!({"jira": {"baseUrl": "https://acme.atlassian.net/"}}),
        )
        .unwrap(),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.scheduler = None;
    let d = Daemon::start(paths, o).await.unwrap();

    let links = d.tracker_links();
    let jira = links.iter().find(|l| l["plugin"] == "jira").unwrap();
    assert_eq!(jira["url"], "https://acme.atlassian.net/browse/{0}");
    // GitHub's needs no base URL; GitLab's has its default.
    assert!(links.iter().any(|l| l["plugin"] == "github-issues"));
    assert!(
        links
            .iter()
            .any(|l| l["url"] == "https://gitlab.com/{1}/-/issues/{2}")
    );

    let patch = serde_json::from_value(
        json!({"disabledPlugins": ["jira", "github-issues", "gitlab-issues"]}),
    )
    .unwrap();
    d.save_config(patch).unwrap();
    assert!(d.tracker_links().is_empty());
}
