//! Repo profiles (spec §16.1): CRUD, matching and what a profile changes.

use std::path::Path;
use std::sync::Arc;

use xherd_core::new::NewAgent;
use xherd_core::paths::Paths;
use xherd_daemon::*;

#[allow(dead_code)]
struct T {
    home: tempfile::TempDir,
    ws: tempfile::TempDir,
    repo: tempfile::TempDir,
    d: Daemon,
    fake: Arc<FakeDriver>,
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
}

async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    assert!(
        git(repo.path(), &["init", "-q", "-b", "main"])
            .status
            .success()
    );
    assert!(
        git(
            repo.path(),
            &["commit", "-q", "--allow-empty", "-m", "base"]
        )
        .status
        .success()
    );
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    let cfg = xherd_core::config::Config {
        workspace: Some(ws.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    cfg.save(&paths.config()).unwrap();
    let fake = Arc::new(FakeDriver::default());
    let mut o = Options::new(fake.clone(), Arc::new(SystemClock));
    // Never the real Claude Code config: a fixture that trusts the workspace.
    let trust = home.path().join("claude.json");
    std::fs::write(&trust, trusting(ws.path())).unwrap();
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    let d = Daemon::start(paths, o).await.unwrap();
    T {
        home,
        ws,
        repo,
        d,
        fake,
    }
}

fn trusting(dir: &Path) -> String {
    serde_json::json!({"projects": {dir.to_string_lossy(): {"hasTrustDialogAccepted": true}}})
        .to_string()
}

fn req(t: &T, key: &str) -> NewAgent {
    NewAgent {
        ticket_key: key.into(),
        slug: None,
        repo: t.repo.path().to_string_lossy().into_owned(),
        base: None,
        flow: None,
        prompt: Some("add login".into()),
        autonomy: None,
        permission_mode: None,
    }
}

fn profile(name: &str, matches: &[&str]) -> xherd_core::profile::Profile {
    xherd_core::profile::Profile {
        name: name.into(),
        matches: matches.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn real(t: &T) -> String {
    std::fs::canonicalize(t.repo.path())
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[tokio::test]
async fn crud_validates_and_survives() {
    let t = setup().await;
    let a =
        t.d.create_profile(profile("Acme", &["github.com/acme/*"]))
            .await
            .unwrap();
    assert!(!a.id.is_empty());
    // a second one with the same name, and an invalid one, are refused
    assert_eq!(
        t.d.create_profile(profile("acme", &["/x"]))
            .await
            .unwrap_err()
            .code,
        "bad_profile"
    );
    assert_eq!(
        t.d.create_profile(profile("Bad", &[]))
            .await
            .unwrap_err()
            .code,
        "bad_profile"
    );
    // update keeps the id, may keep its own name
    let mut p = profile("Acme", &["github.com/acme/*", "/work"]);
    p.max_diff_lines = Some(50);
    let u = t.d.update_profile(&a.id, p).await.unwrap();
    assert_eq!((u.id.as_str(), u.max_diff_lines), (a.id.as_str(), Some(50)));
    assert_eq!(
        t.d.update_profile("nope", profile("Z", &["/z"]))
            .await
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(t.d.profiles().await.unwrap().len(), 1);
    t.d.delete_profile(&a.id).await.unwrap();
    assert_eq!(
        t.d.delete_profile(&a.id).await.unwrap_err().code,
        "not_found"
    );
}

#[tokio::test]
async fn import_is_all_or_nothing_and_replaces_by_name() {
    let t = setup().await;
    t.d.create_profile(profile("A", &["/a"])).await.unwrap();
    let imp =
        |v: serde_json::Value| serde_json::from_value::<xherd_daemon::profiles::Import>(v).unwrap();
    let bad = imp(
        serde_json::json!({"profiles": [{"name": "B", "match": ["/b"]}, {"name": "C", "match": []}]}),
    );
    assert_eq!(
        t.d.import_profiles(bad).await.unwrap_err().code,
        "bad_profile"
    );
    assert_eq!(t.d.profiles().await.unwrap().len(), 1);
    let ok = imp(
        serde_json::json!({"profiles": [{"name": "a", "match": ["/a2"]}, {"name": "B", "match": ["/b"]}]}),
    );
    let r = t.d.import_profiles(ok).await.unwrap();
    assert_eq!(
        (r["added"].as_u64(), r["replaced"].as_u64()),
        (Some(1), Some(1))
    );
    let all = t.d.profiles().await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].matches, ["/a2"]);
    // export has no ids and imports back
    let ex = t.d.export_profiles().await.unwrap();
    assert!(ex["profiles"][0].get("id").is_none() || ex["profiles"][0]["id"] == "");
}

#[tokio::test]
async fn a_matching_profile_sets_base_autonomy_and_instructions() {
    let t = setup().await;
    assert!(git(t.repo.path(), &["branch", "develop"]).status.success());
    let mut p = profile("Local", &[&real(&t)]);
    p.base_branch = Some("develop".into());
    p.autonomy = Some(xherd_core::autonomy::Autonomy::Important);
    p.instructions = Some("Use tabs.".into());
    let p = t.d.create_profile(p).await.unwrap();

    let a = t.d.new_agent(req(&t, "P-1")).await.unwrap();
    assert_eq!(a.profile.as_deref(), Some(p.id.as_str()));
    assert_eq!(a.base.as_deref(), Some("develop"));
    assert_eq!(a.autonomy, xherd_core::autonomy::Autonomy::Important);
    let md = std::fs::read_to_string(
        std::path::Path::new(a.workdir.as_deref().unwrap()).join(".xherd/instructions.md"),
    )
    .unwrap();
    assert!(md.contains("## Repo instructions") && md.contains("Use tabs."));

    // the request wins over the profile
    let mut r = req(&t, "P-2");
    r.base = Some("main".into());
    r.autonomy = Some(xherd_core::autonomy::Autonomy::Ask);
    let b = t.d.new_agent(r).await.unwrap();
    assert_eq!(b.base.as_deref(), Some("main"));
    assert_eq!(b.autonomy, xherd_core::autonomy::Autonomy::Ask);

    // the repo picker shows what would apply
    let v = t.d.resolve_repo(&real(&t)).await.unwrap();
    assert_eq!(v["profile"]["name"], "Local");
    assert_eq!(v["base"], "develop");
    // no remote: the suggestion is the folder
    assert_eq!(v["suggestedMatch"], real(&t));
}

#[tokio::test]
async fn without_a_match_the_global_settings_apply() {
    let t = setup().await;
    t.d.create_profile(profile("Other", &["github.com/other/*"]))
        .await
        .unwrap();
    let a = t.d.new_agent(req(&t, "N-1")).await.unwrap();
    assert_eq!(a.profile, None);
    assert_eq!(a.base.as_deref(), Some("main"));
}
