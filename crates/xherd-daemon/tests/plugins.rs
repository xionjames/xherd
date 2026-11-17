//! Installing plugins from a git URL (R1–R5a): previews from a local
//! repository served through `allow_local_plugin_sources` (no network).

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use serde_json::{Value, json};
use xherd_core::paths::Paths;
use xherd_daemon::*;

/// A git repository with one commit holding `files`; its `file://` URL.
struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new(files: &[(&str, &str)]) -> Repo {
        let r = Repo {
            dir: tempfile::tempdir().unwrap(),
        };
        r.git(&["init", "--quiet", "-b", "main"]);
        r.commit(files);
        r
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
            .args(["-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    fn commit(&self, files: &[(&str, &str)]) -> String {
        for (path, text) in files {
            let p = self.dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        self.git(&["add", "--all"]);
        self.git(&["commit", "--quiet", "-m", "x"]);
        self.sha()
    }

    fn sha(&self) -> String {
        self.git(&["rev-parse", "HEAD"])
    }

    fn url(&self) -> String {
        format!("file://{}", self.dir.path().display())
    }
}

struct T {
    home: tempfile::TempDir,
    workspace: tempfile::TempDir,
    d: Daemon,
}

async fn daemon_with(tweak: impl FnOnce(&mut Options)) -> T {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    let cfg = xherd_core::config::Config {
        workspace: Some(workspace.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    cfg.save(&paths.config()).unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.allow_local_plugin_sources = true;
    tweak(&mut o);
    let d = Daemon::start(paths, o).await.unwrap();
    T { home, workspace, d }
}

async fn daemon() -> T {
    daemon_with(|_| {}).await
}

fn manifest(name: &str, kind: &str, extra: Value) -> String {
    let mut m = json!({"name": name, "kind": kind, "version": "1.2.3"});
    m.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::to_string_pretty(&m).unwrap()
}

fn json_of(p: &impl serde::Serialize) -> Value {
    serde_json::to_value(p).unwrap()
}

fn scratch_left(t: &T) -> usize {
    std::fs::read_dir(t.workspace.path().join("repos"))
        .map(|rd| rd.count())
        .unwrap_or(0)
}

fn all_argv_manifest() -> String {
    manifest(
        "gitea",
        "host",
        json!({
            "label": "Gitea", "description": "PRs on Gitea",
            "settings": [
                {"key": "url", "label": "Server"},
                {"key": "token", "label": "Token", "secret": true, "required": true}
            ],
            "commands": {"pr-create": ["./bin/pr", "create"]},
            "hooks": {"job-done": ["./bin/hook"]},
            "checks": [{"name": "tea", "argv": ["tea", "--version"], "fix": ["./bin/fix"]}],
            "requires": [{"name": "tea", "check": ["tea", "-v"], "install": ["brew", "install", "tea"]}],
            "login": ["tea", "login", "add"],
        }),
    )
}

fn bins() -> Vec<(&'static str, String)> {
    vec![
        ("bin/pr", "#!/bin/sh\n".into()),
        ("bin/hook", "#!/bin/sh\n".into()),
        ("bin/fix", "#!/bin/sh\n".into()),
    ]
}

fn files<'a>(
    manifest_path: &'a str,
    m: &'a str,
    extra: &'a [(&'a str, String)],
) -> Vec<(&'a str, &'a str)> {
    let mut v = vec![(manifest_path, m)];
    v.extend(extra.iter().map(|(p, t)| (*p, t.as_str())));
    v
}

#[tokio::test]
async fn preview_of_a_root_plugin_lists_every_argv() {
    let t = daemon().await;
    let m = all_argv_manifest();
    let extra = bins();
    let repo = Repo::new(&files("xherd-plugin.json", &m, &extra));
    let p = json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap());
    assert_eq!(p["name"], "gitea");
    assert_eq!(p["kind"], "host");
    assert_eq!(p["version"], "1.2.3");
    assert_eq!(p["label"], "Gitea");
    assert_eq!(p["compatible"], true);
    assert_eq!(p["sha"], repo.sha());
    assert_eq!(p["files"], 4);
    assert_eq!(p["digest"].as_str().unwrap().len(), 64);
    assert!(p["replaces"].is_null());
    assert!(p["path"].is_null());
    assert!(p.get("choices").is_none());
    assert!(
        p["warning"]
            .as_str()
            .unwrap()
            .contains("no further sandbox")
    );
    let runs: Vec<String> = p["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["what"].as_str().unwrap().to_string())
        .collect();
    for what in [
        "command pr-create",
        "hook job-done",
        "check tea",
        "fix tea",
        "requires tea check",
        "requires tea install",
        "login",
    ] {
        assert!(runs.contains(&what.to_string()), "{what} in {runs:?}");
    }
    let install = p["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["what"] == "requires tea install")
        .unwrap();
    assert_eq!(install["argv"], json!(["brew", "install", "tea"]));
    let secret = &p["settings"][1];
    assert_eq!(
        (secret["key"].as_str(), secret["secret"].as_bool()),
        (Some("token"), Some(true))
    );
    assert_eq!(secret["required"], true);
    // Nothing is left behind and nothing was installed.
    assert_eq!(scratch_left(&t), 0);
    assert!(!t.home.path().join("plugins/gitea").exists());
}

#[tokio::test]
async fn preview_pins_the_ref_to_a_sha_and_the_digest_follows_the_content() {
    let t = daemon().await;
    let m = manifest("demo", "tracker", json!({}));
    let repo = Repo::new(&[("xherd-plugin.json", &m)]);
    let first = repo.sha();
    let a = json_of(
        &t.d.plugin_preview(&repo.url(), Some("main"), None)
            .await
            .unwrap(),
    );
    assert_eq!(
        (a["ref"].as_str(), a["sha"].as_str()),
        (Some("main"), Some(first.as_str()))
    );
    let second = repo.commit(&[("README.md", "hello")]);
    let b = json_of(
        &t.d.plugin_preview(&repo.url(), Some("main"), None)
            .await
            .unwrap(),
    );
    assert_eq!(b["sha"], second);
    assert_ne!(a["digest"], b["digest"]);
    // A full commit sha is a ref too.
    let c = json_of(
        &t.d.plugin_preview(&repo.url(), Some(&first), None)
            .await
            .unwrap(),
    );
    assert_eq!(c["digest"], a["digest"]);
    assert_eq!(scratch_left(&t), 0);
}

#[tokio::test]
async fn preview_of_several_plugins_offers_choices_and_a_path_picks_one() {
    let t = daemon().await;
    let one = manifest("one", "tracker", json!({}));
    let two = manifest("two", "toolchain", json!({}));
    let repo = Repo::new(&[
        ("plugins/one/xherd-plugin.json", &one),
        ("plugins/two/xherd-plugin.json", &two),
        ("tools/xherd-plugin.json", "{ not json"),
    ]);
    let p = json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap());
    assert!(p.get("name").is_none());
    assert!(p["path"].is_null());
    let choices = p["choices"].as_array().unwrap();
    let got: Vec<(&str, &str)> = choices
        .iter()
        .map(|c| (c["path"].as_str().unwrap(), c["name"].as_str().unwrap()))
        .collect();
    assert_eq!(
        got,
        [
            ("tools", "tools"),
            ("plugins/one", "one"),
            ("plugins/two", "two")
        ]
    );
    assert!(choices[0]["error"].as_str().is_some());
    assert_eq!(choices[2]["kind"], "toolchain");

    let q = json_of(
        &t.d.plugin_preview(&repo.url(), None, Some("plugins/two"))
            .await
            .unwrap(),
    );
    assert_eq!(
        (q["name"].as_str(), q["path"].as_str()),
        (Some("two"), Some("plugins/two"))
    );
    assert_eq!(q["files"], 1);
}

#[tokio::test]
async fn preview_of_a_single_plugin_in_a_sub_folder_takes_it() {
    let t = daemon().await;
    let m = manifest("solo", "tracker", json!({}));
    let repo = Repo::new(&[("plugins/solo/xherd-plugin.json", &m), ("README.md", "x")]);
    let p = json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap());
    assert_eq!(
        (p["name"].as_str(), p["path"].as_str()),
        (Some("solo"), Some("plugins/solo"))
    );
}

#[tokio::test]
async fn preview_says_what_a_plugin_replaces() {
    let t = daemon().await;
    let m = manifest("github", "host", json!({}));
    let repo = Repo::new(&[("xherd-plugin.json", &m)]);
    let p = json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap());
    assert_eq!(p["replaces"], "builtin");

    // A folder of that name already in the plugins folder counts as external.
    let m = manifest("demo", "tracker", json!({}));
    let repo = Repo::new(&[("xherd-plugin.json", &m)]);
    std::fs::create_dir_all(t.home.path().join("plugins/demo")).unwrap();
    let p = json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap());
    assert_eq!(p["replaces"], "external");
}

#[tokio::test]
async fn preview_shows_a_plugin_that_needs_a_newer_xherd() {
    let t = daemon().await;
    let m = manifest("future", "tracker", json!({"min_xherd_version": "999.0.0"}));
    let repo = Repo::new(&[("xherd-plugin.json", &m)]);
    let p = json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap());
    assert_eq!(p["compatible"], false);
    assert_eq!(p["minXherdVersion"], "999.0.0");
}

async fn refused(t: &T, repo: &Repo) -> &'static str {
    t.d.plugin_preview(&repo.url(), None, None)
        .await
        .unwrap_err()
        .code
}

#[tokio::test]
async fn preview_refuses_what_cannot_be_installed() {
    let t = daemon().await;
    let at = |m: String| Repo::new(&[("xherd-plugin.json", &m)]);

    let build = manifest("b", "tracker", json!({"build": ["make"]}));
    assert_eq!(refused(&t, &at(build)).await, "build_not_supported");
    let flow = manifest("f", "flow", json!({}));
    assert_eq!(refused(&t, &at(flow)).await, "kind_not_supported");
    assert_eq!(refused(&t, &at("{ not json".into())).await, "bad_manifest");
    let git = manifest("git", "tracker", json!({}));
    assert_eq!(refused(&t, &at(git)).await, "bad_manifest");
    let missing = manifest("m", "tracker", json!({"commands": {"x": ["./nope"]}}));
    assert_eq!(refused(&t, &at(missing)).await, "bad_manifest");
    assert_eq!(
        refused(&t, &Repo::new(&[("README.md", "nothing")])).await,
        "no_manifest"
    );
    assert_eq!(scratch_left(&t), 0);
}

#[tokio::test]
async fn preview_refuses_symlinks_and_unknown_paths() {
    let t = daemon().await;
    let m = manifest("demo", "tracker", json!({}));
    let repo = Repo::new(&[("xherd-plugin.json", &m)]);
    std::os::unix::fs::symlink("/etc/passwd", repo.dir.path().join("link")).unwrap();
    repo.git(&["add", "--all"]);
    repo.git(&["commit", "--quiet", "-m", "link"]);
    assert_eq!(refused(&t, &repo).await, "bad_plugin_folder");

    let repo = Repo::new(&[("xherd-plugin.json", &m)]);
    let e =
        t.d.plugin_preview(&repo.url(), None, Some("nope"))
            .await
            .unwrap_err();
    assert_eq!(e.code, "no_manifest");
    // A sub-folder that is a symlink out of the repository is not followed.
    std::os::unix::fs::symlink("/etc", repo.dir.path().join("out")).unwrap();
    repo.git(&["add", "--all"]);
    repo.git(&["commit", "--quiet", "-m", "link"]);
    let e =
        t.d.plugin_preview(&repo.url(), None, Some("out"))
            .await
            .unwrap_err();
    assert_eq!(e.code, "no_manifest");
}

#[tokio::test]
async fn preview_reports_a_fetch_that_fails() {
    let t = daemon().await;
    let m = manifest("demo", "tracker", json!({}));
    let repo = Repo::new(&[("xherd-plugin.json", &m)]);
    let e =
        t.d.plugin_preview(&repo.url(), Some("no-such-branch"), None)
            .await
            .unwrap_err();
    assert_eq!(e.code, "fetch_failed");
    let gone = format!("file://{}/missing", t.home.path().display());
    let e = t.d.plugin_preview(&gone, None, None).await.unwrap_err();
    assert_eq!(e.code, "fetch_failed");
    assert_eq!(scratch_left(&t), 0);
}

#[tokio::test]
async fn preview_checks_the_source_and_the_switch() {
    // `file://` is only for tests that ask for it.
    let t = daemon_with(|o| o.allow_local_plugin_sources = false).await;
    let e =
        t.d.plugin_preview("file:///tmp/x", None, None)
            .await
            .unwrap_err();
    assert_eq!(e.code, "bad_plugin_source");
    let e =
        t.d.plugin_preview("http://example.com/o/r", None, None)
            .await
            .unwrap_err();
    assert_eq!(e.code, "bad_plugin_source");

    let t = daemon_with(|o| o.plugin_install = false).await;
    let e =
        t.d.plugin_preview("https://github.com/o/r", None, None)
            .await
            .unwrap_err();
    assert_eq!(e.code, "plugin_install_off");
    assert!(Path::new(t.home.path()).exists());
}

fn install_repo() -> (Repo, String) {
    let m = all_argv_manifest();
    let extra = bins();
    let repo = Repo::new(&files("xherd-plugin.json", &m, &extra));
    // The scripts are executable in the repository.
    repo.git(&[
        "update-index",
        "--chmod=+x",
        "bin/pr",
        "bin/hook",
        "bin/fix",
    ]);
    repo.git(&["commit", "--quiet", "-m", "exec"]);
    (repo, m)
}

async fn digest_of(t: &T, repo: &Repo) -> String {
    json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap())["digest"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn install_puts_the_previewed_plugin_on_disk_and_records_it() {
    use std::os::unix::fs::PermissionsExt;
    let t = daemon().await;
    let (repo, _) = install_repo();
    let digest = digest_of(&t, &repo).await;
    let i = json_of(
        &t.d.plugin_install(&repo.url(), None, None, &digest)
            .await
            .unwrap(),
    );
    assert_eq!(
        (i["name"].as_str(), i["sha"].as_str()),
        (Some("gitea"), Some(repo.sha().as_str()))
    );
    let dir = t.home.path().join("plugins/gitea");
    assert!(dir.join("xherd-plugin.json").is_file());
    assert!(!dir.join(".git").exists());
    assert_ne!(
        dir.join("bin/pr").metadata().unwrap().permissions().mode() & 0o111,
        0
    );
    assert_eq!(
        dir.join("xherd-plugin.json")
            .metadata()
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    // No staging folder and no scratch fetch is left.
    let left: Vec<_> = std::fs::read_dir(t.home.path().join("plugins"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["gitea"]);
    assert_eq!(scratch_left(&t), 0);

    let recs = t.d.store.plugin_records().await.unwrap();
    assert_eq!(recs.len(), 1);
    let r = &recs[0];
    assert_eq!(
        (r.name.as_str(), r.kind.as_str(), r.source.as_str()),
        ("gitea", "host", repo.url().as_str())
    );
    assert_eq!(r.sha.as_deref(), Some(repo.sha().as_str()));
    assert!(r.enabled);
    assert_eq!(r.data["digest"], digest);
    assert_eq!(r.data["version"], "1.2.3");
    assert!(r.data["installedAt"].as_i64().unwrap() > 0);
    assert!(r.data["ref"].is_null() && r.data["path"].is_null());

    let events = t.d.store.events("", 10).await.unwrap();
    let installs: Vec<_> = events
        .iter()
        .filter(|e| e.kind == "plugin-install")
        .collect();
    assert_eq!(installs.len(), 1);
    assert_eq!(installs[0].payload.as_ref().unwrap()["name"], "gitea");
}

#[tokio::test]
async fn install_refuses_a_source_that_changed_since_the_preview() {
    let t = daemon().await;
    let (repo, _) = install_repo();
    let digest = digest_of(&t, &repo).await;
    repo.commit(&[("README.md", "changed")]);
    let e =
        t.d.plugin_install(&repo.url(), None, None, &digest)
            .await
            .unwrap_err();
    assert_eq!(e.code, "changed_since_preview");
    assert!(!t.home.path().join("plugins/gitea").exists());
    assert!(t.d.store.plugin_records().await.unwrap().is_empty());
    assert_eq!(scratch_left(&t), 0);
}

#[tokio::test]
async fn install_does_not_replace_an_installed_plugin() {
    let t = daemon().await;
    let (repo, _) = install_repo();
    let digest = digest_of(&t, &repo).await;
    t.d.plugin_install(&repo.url(), None, None, &digest)
        .await
        .unwrap();
    let e =
        t.d.plugin_install(&repo.url(), None, None, &digest)
            .await
            .unwrap_err();
    assert_eq!(e.code, "plugin_exists");
    assert_eq!(t.d.store.plugin_records().await.unwrap().len(), 1);
}

#[tokio::test]
async fn install_takes_a_plugin_from_a_sub_folder_and_a_ref() {
    let t = daemon().await;
    let one = manifest("one", "tracker", json!({}));
    let two = manifest("two", "toolchain", json!({}));
    let repo = Repo::new(&[
        ("plugins/one/xherd-plugin.json", &one),
        ("plugins/two/xherd-plugin.json", &two),
    ]);
    let preview = json_of(
        &t.d.plugin_preview(&repo.url(), Some("main"), Some("plugins/two"))
            .await
            .unwrap(),
    );
    let digest = preview["digest"].as_str().unwrap();
    // Without a path the repository offers a choice: nothing is installed.
    let e =
        t.d.plugin_install(&repo.url(), Some("main"), None, digest)
            .await
            .unwrap_err();
    assert_eq!(e.code, "no_manifest");
    t.d.plugin_install(&repo.url(), Some("main"), Some("plugins/two"), digest)
        .await
        .unwrap();
    assert!(
        t.home
            .path()
            .join("plugins/two/xherd-plugin.json")
            .is_file()
    );
    assert!(!t.home.path().join("plugins/one").exists());
    let r = &t.d.store.plugin_records().await.unwrap()[0];
    assert_eq!(
        (r.data["ref"].as_str(), r.data["path"].as_str()),
        (Some("main"), Some("plugins/two"))
    );
    assert_eq!(r.kind, "toolchain");
}

#[tokio::test]
async fn install_is_off_with_the_switch_and_cleans_staging_at_start() {
    let t = daemon_with(|o| o.plugin_install = false).await;
    let e =
        t.d.plugin_install("https://github.com/o/r", None, None, "x")
            .await
            .unwrap_err();
    assert_eq!(e.code, "plugin_install_off");

    // Leftovers of an interrupted install are removed at start.
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    for n in [".a.staging-1", ".b.old-2"] {
        std::fs::create_dir_all(paths.plugins().join(n)).unwrap();
    }
    std::fs::create_dir_all(paths.plugins().join("keep")).unwrap();
    let ws = tempfile::tempdir().unwrap();
    let cfg = xherd_core::config::Config {
        workspace: Some(ws.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    cfg.save(&paths.config()).unwrap();
    let o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    let _d = Daemon::start(paths.clone(), o).await.unwrap();
    let mut left: Vec<_> = std::fs::read_dir(paths.plugins())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["keep"]);
}
