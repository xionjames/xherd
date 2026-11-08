//! Commit identity by remote: the managed gitconfig is read by real git.

use std::process::Command;
use std::sync::Arc;

use serde_json::json;
use xherd_core::paths::Paths;
use xherd_daemon::*;

fn git(dir: &std::path::Path, cfg: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", cfg)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn identity_follows_the_remote() {
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
    let d = Daemon::start(paths, o).await.unwrap();

    let patch = serde_json::from_value(json!({
        "gitUserName": "Me", "gitUserEmail": "me@home.dev",
        "gitIdentities": [{"scope": "github.com/my-company", "name": "Me Work", "email": "me@company.com"}]
    }))
    .unwrap();
    d.save_config(patch).unwrap();
    let cfg = d.gitconfig_path().unwrap();

    let email = |remote: &str| {
        let repo = tempfile::tempdir().unwrap();
        git(repo.path(), &cfg, &["init", "-q"]);
        git(repo.path(), &cfg, &["remote", "add", "origin", remote]);
        git(repo.path(), &cfg, &["config", "user.email"])
    };
    for work in [
        "https://github.com/my-company/app.git",
        "git@github.com:my-company/app.git",
        "ssh://git@github.com/my-company/app",
    ] {
        assert_eq!(email(work), "me@company.com", "{work}");
    }
    for personal in [
        "https://github.com/me/app.git",
        "https://github.com/my-company-x/app.git",
        "git@github.com:me/app.git",
    ] {
        assert_eq!(email(personal), "me@home.dev", "{personal}");
    }

    // Bad scopes are refused; removing the rules removes the files.
    for scope in [
        "",
        "github.com/a b",
        "github.com/a/b",
        "https://github.com/x",
    ] {
        let patch = serde_json::from_value(json!({
            "gitIdentities": [{"scope": scope, "name": "N", "email": "n@x.io"}]
        }))
        .unwrap();
        assert_eq!(
            d.save_config(patch).unwrap_err().code,
            "bad_config",
            "{scope:?}"
        );
    }
    let patch = serde_json::from_value(json!({"gitIdentities": []})).unwrap();
    d.save_config(patch).unwrap();
    assert!(
        !cfg.parent()
            .unwrap()
            .join("identities/0.gitconfig")
            .exists()
    );
    assert_eq!(
        email("https://github.com/my-company/app.git"),
        "me@home.dev"
    );
}
