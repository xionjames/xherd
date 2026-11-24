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

async fn installed() -> (T, Repo) {
    let t = daemon().await;
    let (repo, _) = install_repo();
    let digest = digest_of(&t, &repo).await;
    t.d.plugin_install(&repo.url(), None, None, &digest)
        .await
        .unwrap();
    (t, repo)
}

fn v2_manifest() -> String {
    all_argv_manifest().replace("1.2.3", "1.3.0")
}

#[tokio::test]
async fn update_remove_update_previews_the_manifest_diff_and_writes_nothing() {
    let (t, repo) = installed().await;
    let before = t.d.store.plugin_records().await.unwrap();
    let same = json_of(&t.d.plugin_update_preview("gitea", None).await.unwrap());
    assert_eq!(same["manifestDiff"], "");
    assert_eq!(same["installed"]["sha"], repo.sha());
    assert_eq!(same["installed"]["version"], "1.2.3");

    let new_sha = repo.commit(&[("xherd-plugin.json", &v2_manifest())]);
    let p = json_of(&t.d.plugin_update_preview("gitea", None).await.unwrap());
    assert_eq!(p["sha"], new_sha);
    assert_eq!(p["version"], "1.3.0");
    let diff = p["manifestDiff"].as_str().unwrap();
    assert!(
        diff.contains("-  \"version\": \"1.2.3\"") && diff.contains("+  \"version\": \"1.3.0\""),
        "{diff}"
    );
    assert_eq!(t.d.store.plugin_records().await.unwrap(), before);
    let on_disk =
        std::fs::read_to_string(t.home.path().join("plugins/gitea/xherd-plugin.json")).unwrap();
    assert!(on_disk.contains("1.2.3"));
    assert_eq!(scratch_left(&t), 0);
}

#[tokio::test]
async fn update_remove_update_swaps_the_folder_and_moves_the_record() {
    let (t, repo) = installed().await;
    let old_sha = repo.sha();
    repo.commit(&[("xherd-plugin.json", &v2_manifest()), ("bin/new", "x")]);
    let p = json_of(&t.d.plugin_update_preview("gitea", None).await.unwrap());
    let digest = p["digest"].as_str().unwrap().to_string();

    // A source that moved on since the preview is refused, nothing changes.
    repo.commit(&[("README.md", "later")]);
    let e = t.d.plugin_update("gitea", None, &digest).await.unwrap_err();
    assert_eq!(e.code, "changed_since_preview");
    assert!(!t.home.path().join("plugins/gitea/bin/new").exists());

    let digest = json_of(&t.d.plugin_update_preview("gitea", None).await.unwrap())["digest"]
        .as_str()
        .unwrap()
        .to_string();
    let u = json_of(&t.d.plugin_update("gitea", None, &digest).await.unwrap());
    assert_eq!(u["sha"], repo.sha());
    assert_eq!(u["previousSha"], old_sha);
    let dir = t.home.path().join("plugins/gitea");
    assert!(dir.join("bin/new").is_file());
    let left: Vec<_> = std::fs::read_dir(t.home.path().join("plugins"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["gitea"]);

    let recs = t.d.store.plugin_records().await.unwrap();
    assert_eq!(recs.len(), 1);
    assert_eq!(recs[0].sha.as_deref(), Some(repo.sha().as_str()));
    assert_eq!(recs[0].data["version"], "1.3.0");
    assert_eq!(recs[0].data["digest"], digest);
    assert!(recs[0].data["installedAt"].as_i64().unwrap() > 0);
    let events = t.d.store.events("", 10).await.unwrap();
    assert_eq!(
        events.iter().filter(|e| e.kind == "plugin-update").count(),
        1
    );
}

#[tokio::test]
async fn update_remove_update_takes_a_new_ref_and_keeps_the_name() {
    let (t, repo) = installed().await;
    repo.git(&["checkout", "--quiet", "-b", "next"]);
    repo.commit(&[("xherd-plugin.json", &v2_manifest())]);
    let digest = json_of(
        &t.d.plugin_update_preview("gitea", Some("next"))
            .await
            .unwrap(),
    )["digest"]
        .as_str()
        .unwrap()
        .to_string();
    t.d.plugin_update("gitea", Some("next"), &digest)
        .await
        .unwrap();
    let r = &t.d.store.plugin_records().await.unwrap()[0];
    assert_eq!(r.data["ref"], "next");

    // A source that now holds another plugin is refused.
    repo.commit(&[("xherd-plugin.json", &manifest("other", "host", json!({})))]);
    let e =
        t.d.plugin_update_preview("gitea", Some("next"))
            .await
            .unwrap_err();
    assert_eq!(e.code, "bad_manifest");
}

#[tokio::test]
async fn update_remove_update_refuses_built_ins_and_unknown_plugins() {
    let t = daemon().await;
    for (name, code) in [("github", "builtin_plugin"), ("nope", "unknown_plugin")] {
        let e = t.d.plugin_update_preview(name, None).await.unwrap_err();
        assert_eq!(e.code, code, "{name}");
        let e = t.d.plugin_update(name, None, "x").await.unwrap_err();
        assert_eq!(e.code, code, "{name}");
    }
}

#[tokio::test]
async fn update_remove_remove_deletes_folder_and_record_and_keeps_settings() {
    let (t, _repo) = installed().await;
    let mut cfg = xherd_core::config::Config::load(&t.d.paths.config())
        .unwrap()
        .0;
    cfg.plugin_settings
        .insert("gitea".into(), json!({"url": "https://git.example"}));
    cfg.save(&t.d.paths.config()).unwrap();
    let mut s = xherd_core::config::Secrets::default();
    s.0.insert("gitea".into(), json!({"token": "t"}));
    s.save(&t.d.paths.secrets()).unwrap();

    let r = json_of(&t.d.plugin_remove("gitea", false).await.unwrap());
    assert_eq!(
        (r["name"].as_str(), r["forgot"].as_bool()),
        (Some("gitea"), Some(false))
    );
    assert!(!t.home.path().join("plugins/gitea").exists());
    assert!(t.d.store.plugin_records().await.unwrap().is_empty());
    let events = t.d.store.events("", 10).await.unwrap();
    assert_eq!(
        events.iter().filter(|e| e.kind == "plugin-remove").count(),
        1
    );
    let cfg = xherd_core::config::Config::load(&t.d.paths.config())
        .unwrap()
        .0;
    assert!(cfg.plugin_settings.contains_key("gitea"));
    assert!(
        xherd_core::config::Secrets::load(&t.d.paths.secrets())
            .unwrap()
            .0
            .contains_key("gitea")
    );
    let e = t.d.plugin_remove("gitea", false).await.unwrap_err();
    assert_eq!(e.code, "unknown_plugin");
}

#[tokio::test]
async fn update_remove_forget_drops_settings_and_secrets() {
    let (t, _repo) = installed().await;
    let mut cfg = xherd_core::config::Config::load(&t.d.paths.config())
        .unwrap()
        .0;
    cfg.plugin_settings
        .insert("gitea".into(), json!({"url": "https://git.example"}));
    cfg.plugin_settings.insert("other".into(), json!({"a": 1}));
    cfg.save(&t.d.paths.config()).unwrap();
    let mut s = xherd_core::config::Secrets::default();
    s.0.insert("gitea".into(), json!({"token": "t"}));
    s.0.insert("other".into(), json!({"token": "o"}));
    s.save(&t.d.paths.secrets()).unwrap();

    let r = json_of(&t.d.plugin_remove("gitea", true).await.unwrap());
    assert_eq!(r["forgot"], true);
    let cfg = xherd_core::config::Config::load(&t.d.paths.config())
        .unwrap()
        .0;
    assert!(!cfg.plugin_settings.contains_key("gitea"));
    assert!(cfg.plugin_settings.contains_key("other"));
    let s = xherd_core::config::Secrets::load(&t.d.paths.secrets()).unwrap();
    assert!(!s.0.contains_key("gitea") && s.0.contains_key("other"));
}

#[tokio::test]
async fn update_remove_is_refused_for_a_live_job_and_allowed_when_paused() {
    let t = daemon().await;
    // A runtime plugin, installed from a source.
    let m = manifest(
        "gemini",
        "runtime",
        json!({"data": {"terminal": {"launch": ["gemini"]}}}),
    );
    let repo = Repo::new(&[("xherd-plugin.json", &m)]);
    let digest = digest_of(&t, &repo).await;
    t.d.plugin_install(&repo.url(), None, None, &digest)
        .await
        .unwrap();

    let mut job = xherd_core::Job::new("rt-1", 1);
    job.runtime = Some("gemini".into());
    job.status = xherd_core::Status::Running;
    t.d.store.put_job(&job).await.unwrap();
    let e = t.d.plugin_remove("gemini", false).await.unwrap_err();
    assert_eq!(e.code, "plugin_in_use");
    assert!(e.message.contains("rt-1"), "{}", e.message);
    assert!(t.home.path().join("plugins/gemini").is_dir());
    assert_eq!(t.d.store.plugin_records().await.unwrap().len(), 1);

    job.status = xherd_core::Status::Paused;
    t.d.store.put_job(&job).await.unwrap();
    t.d.plugin_remove("gemini", false).await.unwrap();
    assert!(!t.home.path().join("plugins/gemini").exists());
}

#[tokio::test]
async fn update_remove_refuses_built_ins_and_works_with_the_switch_off() {
    let t = daemon().await;
    assert_eq!(
        t.d.plugin_remove("github", false).await.unwrap_err().code,
        "builtin_plugin"
    );
    assert_eq!(
        t.d.plugin_remove("../x", false).await.unwrap_err().code,
        "unknown_plugin"
    );

    // A plugin copied by hand (no record) can be removed, switch off or not.
    let t = daemon_with(|o| o.plugin_install = false).await;
    let dir = t.home.path().join("plugins/hand");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("xherd-plugin.json"),
        manifest("hand", "host", json!({})),
    )
    .unwrap();
    t.d.plugin_remove("hand", false).await.unwrap();
    assert!(!dir.exists());
    let e = t.d.plugin_update_preview("hand", None).await.unwrap_err();
    assert_eq!(e.code, "unknown_plugin");
}

fn restart_needed(t: &T) -> bool {
    t.d.config_view().unwrap()["restartNeeded"] == true
}

async fn restarted(t: &T) -> Daemon {
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.allow_local_plugin_sources = true;
    Daemon::start(Paths::at(t.home.path()), o).await.unwrap()
}

#[tokio::test]
async fn restart_is_needed_after_install_update_and_remove_and_not_after_a_start() {
    let t = daemon().await;
    assert!(!restart_needed(&t));
    let (repo, _) = install_repo();
    let digest = digest_of(&t, &repo).await;
    t.d.plugin_install(&repo.url(), None, None, &digest)
        .await
        .unwrap();
    assert!(restart_needed(&t));
    assert!(t.d.plugin_changes());

    // A new start loads the installed plugin: nothing is pending.
    let d2 = restarted(&t).await;
    assert!(
        !d2.config_view().unwrap()["restartNeeded"]
            .as_bool()
            .unwrap()
    );
    assert!(d2.plugins.get("gitea").is_some());

    // An update moves the sha, a remove drops the plugin.
    repo.commit(&[("xherd-plugin.json", &v2_manifest())]);
    let p = d2.plugin_update_preview("gitea", None).await.unwrap();
    d2.plugin_update("gitea", None, json_of(&p)["digest"].as_str().unwrap())
        .await
        .unwrap();
    assert!(d2.plugin_changes());
    let d3 = restarted(&t).await;
    assert!(!d3.plugin_changes());
    d3.plugin_remove("gitea", false).await.unwrap();
    assert!(
        d3.config_view().unwrap()["restartNeeded"]
            .as_bool()
            .unwrap()
    );
    assert!(!restarted(&t).await.plugin_changes());
}

#[tokio::test]
async fn restart_changes_cancel_out_and_a_hand_copied_plugin_counts() {
    let (t, _repo) = installed().await;
    // Installed after start, then removed again: same as at start.
    assert!(t.d.plugin_changes());
    t.d.plugin_remove("gitea", false).await.unwrap();
    assert!(!t.d.plugin_changes());

    let dir = t.home.path().join("plugins").join("hand");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("xherd-plugin.json"),
        manifest("hand", "tracker", json!({})),
    )
    .unwrap();
    let t2 = restarted(&t).await;
    assert!(!t2.plugin_changes());
    t2.plugin_remove("hand", false).await.unwrap();
    assert!(t2.plugin_changes());
}

#[tokio::test]
async fn restart_request_wakes_the_waiter_even_when_made_first() {
    let t = daemon().await;
    t.d.request_restart();
    tokio::time::timeout(std::time::Duration::from_secs(5), t.d.restart_requested())
        .await
        .expect("a request made before waiting still counts");
    let waiter = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        t.d.restart_requested(),
    )
    .await;
    assert!(waiter.is_err(), "one request wakes one wait");
}

#[tokio::test]
async fn security_lists_external_plugins_with_their_source_or_as_by_hand() {
    let (t, repo) = installed().await;
    let hand = t.home.path().join("plugins/hand");
    std::fs::create_dir_all(&hand).unwrap();
    std::fs::write(
        hand.join("xherd-plugin.json"),
        manifest("hand", "host", json!({})),
    )
    .unwrap();

    // Plugins are listed once loaded: a new start sees both.
    let d2 = restarted(&t).await;
    let found = d2.security_check().await;
    let p = found.iter().find(|f| f.id == "plugins").unwrap();
    assert!(!p.ok);
    let pinned = format!("gitea ({}@{})", repo.url(), &repo.sha()[..7]);
    assert!(p.detail.contains(&pinned), "{}", p.detail);
    assert!(
        p.detail.contains("hand (installed by hand, not pinned)"),
        "{}",
        p.detail
    );
}

// ---- Jobs on a plugin pack (R13, R14) ----

const ACME_PACK: &str = r#"
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
      - { id: brief, role: spec, class: critical, gate: { docs: [spec.md] }, next: [{ run: "{prefix}design" }] }
      - { id: design, role: design, class: routine, command: "{prefix}design", gate: { docs: [design.md] }, next: [{ run: "{prefix}design" }] }
"#;

const ACME_PROMPTS: [&str; 12] = [
    "rules", "start", "design", "plan", "next", "commit", "review", "ship", "ask", "resume",
    "handoff", "status",
];

struct Acme {
    t: T,
    repo: tempfile::TempDir,
}

/// A daemon that loaded the `acme-sdd` flow plugin from its plugins folder,
/// and a git repository to start jobs in.
async fn acme() -> Acme {
    acme_with(|_, _| {}).await
}

/// `acme()` with a say before the daemon starts: in the home (to add more
/// plugins) and in the options.
async fn acme_with(setup: impl FnOnce(&Path, &mut Options)) -> Acme {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    let cfg = xherd_core::config::Config {
        workspace: Some(workspace.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    cfg.save(&paths.config()).unwrap();
    let dir = home.path().join("plugins/acme-sdd");
    std::fs::create_dir_all(dir.join("pack/commands")).unwrap();
    std::fs::write(
        dir.join("xherd-plugin.json"),
        manifest(
            "acme-sdd",
            "flow",
            json!({"data": {"flow": {"pack": "pack"}}}),
        ),
    )
    .unwrap();
    std::fs::write(dir.join("pack/xherd-flow.yaml"), ACME_PACK).unwrap();
    for p in ACME_PROMPTS {
        std::fs::write(
            dir.join(format!("pack/commands/{p}.md")),
            format!("# {p}\n"),
        )
        .unwrap();
    }
    let repo = tempfile::tempdir().unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["commit", "-q", "--allow-empty", "-m", "base"][..],
    ] {
        let out = Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    // Never the real Claude Code config: a fixture that trusts the workspace.
    let trust = home.path().join("claude.json");
    std::fs::write(
        &trust,
        json!({"projects": {workspace.path().to_string_lossy(): {"hasTrustDialogAccepted": true}}})
            .to_string(),
    )
    .unwrap();
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    setup(home.path(), &mut o);
    let d = Daemon::start(paths, o).await.unwrap();
    Acme {
        t: T { home, workspace, d },
        repo,
    }
}

fn new_on(a: &Acme, name: &str, pack: Option<&str>, flow: Option<&str>) -> xherd_core::new::NewJob {
    xherd_core::new::NewJob {
        name: name.into(),
        ticket: None,
        ticket_key: None,
        slug: None,
        repo: a.repo.path().to_string_lossy().into_owned(),
        agent: None,
        agents: Default::default(),
        base: None,
        flow: flow.map(Into::into),
        pack: pack.map(Into::into),
        prompt: Some("add login".into()),
        autonomy: None,
        permission_mode: None,
        runtime: None,
        model: None,
    }
}

#[tokio::test]
async fn a_job_runs_on_a_plugin_pack_to_a_gate_and_an_approval() {
    let a = acme().await;
    assert!(a.t.d.pack_selectable("acme-sdd"));
    let job =
        a.t.d
            .new_job(new_on(&a, "login", Some("acme-sdd"), None))
            .await
            .unwrap();
    assert_eq!(
        (job.pack.as_deref(), job.flow.as_deref()),
        (Some("acme-sdd"), Some("feature"))
    );
    assert_eq!(
        job.next_prompt.as_deref(),
        Some("/acme-sdd:start  add login")
    );
    // The hook file allows reading the plugin's installed commands.
    let wt = std::path::PathBuf::from(job.workdir.as_deref().unwrap());
    let settings = std::fs::read_to_string(wt.join(".claude/settings.local.json")).unwrap();
    assert!(settings.contains("commands/acme-sdd"), "{settings}");

    // A gate of the plugin's flow, then the user's approval.
    let name = job.name.clone();
    let memory = "# Memory\n## State\nbrief\n## Decisions\nnone\n## Code map\nsrc/a.rs: a\n## Watch out\nnothing\n";
    std::fs::write(wt.join(".xherd/spec/memory.md"), memory).unwrap();
    std::fs::write(wt.join(".xherd/spec/spec.md"), "# Spec\n").unwrap();
    let call = |id: &str, cmd: &str, args: Value| -> xherd_core::xap::XapCall {
        serde_json::from_value(
            json!({"id": id, "jobId": job.id, "cmd": cmd, "args": args, "at": 1}),
        )
        .unwrap()
    };
    a.t.d
        .xap(call("c1", "gate.open", json!({"stage": "brief"})))
        .await
        .unwrap();
    a.t.d
        .report(xherd_core::report::Report {
            id: "stop-1".into(),
            job_id: job.id.clone(),
            event: xherd_core::report::ReportEvent::Stop,
            payload: Value::Null,
            seq: 1,
            at: 0,
        })
        .await;
    let waiting = a.t.d.action(&name, UserAction::Get).await.unwrap();
    assert_eq!(waiting.status, xherd_core::Status::WaitingInput);
    let approved =
        a.t.d
            .action(
                &name,
                UserAction::Approve {
                    reason: None,
                    expect: Default::default(),
                    message: None,
                },
            )
            .await
            .unwrap();
    assert_eq!(approved.pack.as_deref(), Some("acme-sdd"));
    assert!(
        approved
            .next_prompt
            .as_deref()
            .is_some_and(|p| p.starts_with("/acme-sdd:design")),
        "{:?}",
        approved.next_prompt
    );
}

#[tokio::test]
async fn an_unknown_pack_a_missing_flow_and_a_disabled_plugin_are_refused() {
    let a = acme().await;
    let e =
        a.t.d
            .new_job(new_on(&a, "one", Some("nope"), None))
            .await
            .unwrap_err();
    assert_eq!(e.code, "unknown_pack");
    let e =
        a.t.d
            .new_job(new_on(&a, "two", Some("acme-sdd"), Some("bugfix")))
            .await
            .unwrap_err();
    assert_eq!(e.code, "unknown_flow");
    // `flow: none` ignores the pack.
    let plain =
        a.t.d
            .new_job(new_on(&a, "three", Some("nope"), Some("none")))
            .await
            .unwrap();
    assert_eq!(plain.pack, None);

    // Switched off while running: refused at once.
    let paths = Paths::at(a.t.home.path());
    let (mut cfg, _) = xherd_core::config::Config::load(&paths.config()).unwrap();
    cfg.disabled_plugins.push("acme-sdd".into());
    cfg.save(&paths.config()).unwrap();
    let e =
        a.t.d
            .new_job(new_on(&a, "four", Some("acme-sdd"), None))
            .await
            .unwrap_err();
    assert_eq!(e.code, "unknown_pack");
    // The built-in pack is still there.
    assert_eq!(
        a.t.d
            .new_job(new_on(&a, "five", None, None))
            .await
            .unwrap()
            .pack
            .as_deref(),
        Some("xherd")
    );
}

#[tokio::test]
async fn start_installs_every_selectable_pack_and_a_broken_one_does_not_stop_the_others() {
    let user = tempfile::tempdir().unwrap();
    let h = user.path().to_string_lossy().into_owned();
    let a = acme_with(|home, o| {
        // A second flow plugin whose pack cannot be read.
        let bad = home.join("plugins/bad-sdd");
        std::fs::create_dir_all(bad.join("pack")).unwrap();
        std::fs::write(
            bad.join("xherd-plugin.json"),
            manifest(
                "bad-sdd",
                "flow",
                json!({"data": {"flow": {"pack": "pack"}}}),
            ),
        )
        .unwrap();
        std::fs::write(bad.join("pack/xherd-flow.yaml"), "name: [").unwrap();
        o.install_env = Some(Arc::new(move |k: &str| (k == "HOME").then(|| h.clone())));
    })
    .await;
    assert!(a.t.d.pack_selectable("acme-sdd"));
    assert!(!a.t.d.pack_selectable("bad-sdd"));
    assert_eq!(a.t.d.pack_errors.len(), 1);
    let commands = user.path().join(".claude/commands");
    assert!(commands.join("xherd/status.md").exists());
    assert!(commands.join("acme-sdd/start.md").exists());
    assert!(!commands.join("bad-sdd").exists());
}

/// A git repository holding the `acme-sdd` flow plugin; `agents` is the text
/// of its agents file.
fn acme_repo(agents: &str) -> Repo {
    flow_repo("acme-sdd", agents)
}

/// The same plugin under another name (the pack is named like the plugin).
fn flow_repo(name: &str, agents: &str) -> Repo {
    let m = manifest(
        name,
        "flow",
        json!({"data": {"flow": {"pack": "pack", "agents": "agents.json", "assignments": {
            "feature": {
                "brief": {"agent": "analyst"},
                "design": {"agent": "architect", "human": true}}}}}}),
    );
    let prompts: Vec<(String, String)> = ACME_PROMPTS
        .iter()
        .map(|p| (format!("pack/commands/{p}.md"), format!("# {p}\n")))
        .collect();
    let pack = ACME_PACK.replace("acme-sdd", name);
    let mut files: Vec<(&str, &str)> = vec![
        ("xherd-plugin.json", &m),
        ("agents.json", agents),
        ("pack/xherd-flow.yaml", &pack),
    ];
    files.extend(prompts.iter().map(|(p, t)| (p.as_str(), t.as_str())));
    Repo::new(&files)
}

const ACME_AGENTS: &str = r#"{"agents": [
    {"name": "analyst", "runtime": "claude-code"},
    {"name": "architect", "runtime": "claude-code", "instructions": "Draw boxes."}]}"#;

#[tokio::test]
async fn preview_of_a_flow_plugin_lists_flows_agents_assignments_and_files() {
    let t = daemon().await;
    let repo = acme_repo(ACME_AGENTS);
    let p = json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap());
    assert_eq!(p["kind"], "flow");
    let f = &p["flow"];
    assert_eq!(f["pack"], "acme-sdd");
    assert_eq!(f["memoryNote"], "required");
    assert_eq!(f["flows"][0]["flow"], "feature");
    assert_eq!(f["flows"][0]["stages"][0]["id"], "brief");
    assert_eq!(f["flows"][0]["stages"][0]["role"], "spec");
    assert_eq!(f["flows"][0]["stages"][0]["class"], "critical");
    assert_eq!(f["flows"][0]["stages"][0]["gate"], true);
    assert_eq!(f["agents"][0]["name"], "analyst");
    assert_eq!(f["agents"][0]["savedName"], "acme-sdd-analyst");
    assert_eq!(f["agents"][0]["state"], "new");
    assert_eq!(f["agents"][1]["risky"], json!(["instructions"]));
    assert_eq!(f["agents"][1]["issues"], json!([]));
    assert_eq!(f["assignments"][1]["agent"], "acme-sdd-architect");
    assert_eq!(f["assignments"][1]["human"], true);
    assert_eq!(f["assignmentChanges"], json!([]));
    assert_eq!(f["archived"], json!([]));
    assert_eq!(f["inUseBy"], Value::Null);
    assert_eq!(f["files"][0]["runtime"], "claude-code");
    let paths = f["files"][0]["paths"].as_array().unwrap();
    assert_eq!(paths.len(), ACME_PROMPTS.len());
    assert!(
        paths
            .iter()
            .all(|p| p.as_str().unwrap().contains("commands/acme-sdd/"))
    );
    assert_eq!(scratch_left(&t), 0);
}

#[tokio::test]
async fn preview_of_a_flow_plugin_shows_a_taken_name_and_an_unknown_skill_as_findings() {
    let t = daemon().await;
    let hand: xherd_core::agent_def::AgentDef =
        serde_json::from_value(json!({"name": "acme-sdd-architect", "runtime": "claude-code"}))
            .unwrap();
    t.d.create_agent(hand).await.unwrap();
    let agents = r#"{"agents": [
        {"name": "analyst", "runtime": "claude-code", "skills": ["no-such-skill"]},
        {"name": "architect", "runtime": "claude-code"}]}"#;
    let repo = acme_repo(agents);
    let p = json_of(&t.d.plugin_preview(&repo.url(), None, None).await.unwrap());
    let a = &p["flow"]["agents"];
    assert_eq!(a[0]["state"], "new");
    assert_eq!(a[0]["risky"], json!(["skills"]));
    assert_eq!(
        a[0]["issues"],
        json!(["skills: no skill named \"no-such-skill\" in the library"])
    );
    assert_eq!(a[1]["state"], "taken");
}

#[tokio::test]
async fn preview_of_a_flow_plugin_refuses_a_broken_pack() {
    let t = daemon().await;
    let m = manifest(
        "acme-sdd",
        "flow",
        json!({"data": {"flow": {"pack": "pack"}}}),
    );
    let repo = Repo::new(&[
        ("xherd-plugin.json", &m),
        ("pack/xherd-flow.yaml", "name: ["),
    ]);
    assert_eq!(refused(&t, &repo).await, "bad_manifest");
    // The structure is checked: a flow plugin without a `flow` section.
    let m = manifest("acme-sdd", "flow", json!({}));
    assert_eq!(
        refused(&t, &Repo::new(&[("xherd-plugin.json", &m)])).await,
        "bad_manifest"
    );
}

#[tokio::test]
async fn install_of_a_flow_plugin_saves_its_agents_and_assignments() {
    let t = daemon().await;
    let repo = acme_repo(ACME_AGENTS);
    let digest = digest_of(&t, &repo).await;
    let i =
        t.d.plugin_install(&repo.url(), None, None, &digest)
            .await
            .unwrap();
    assert_eq!(i.name, "acme-sdd");
    assert!(t.home.path().join("plugins/acme-sdd/agents.json").exists());
    assert_eq!(t.d.store.plugin_records().await.unwrap().len(), 1);

    let agents = t.d.store.list_agent_defs().await.unwrap();
    let mut names: Vec<_> = agents.iter().map(|a| a.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["acme-sdd-analyst", "acme-sdd-architect"]);
    assert!(
        agents
            .iter()
            .all(|a| a.source.as_deref() == Some("acme-sdd") && a.version == 1 && !a.id.is_empty())
    );
    let architect = agents
        .iter()
        .find(|a| a.name.ends_with("architect"))
        .unwrap();

    let rows = t.d.store.all_flow_assignments().await.unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|(pack, flow, _)| pack == "acme-sdd" && flow == "feature")
    );
    let design = rows.iter().find(|(_, _, a)| a.stage == "design").unwrap();
    assert_eq!(design.2.agent_id, architect.id);
    assert!(design.2.human);
    assert_eq!(scratch_left(&t), 0);
}

#[tokio::test]
async fn install_of_a_flow_plugin_refuses_a_taken_name_and_leaves_nothing() {
    let t = daemon().await;
    let hand: xherd_core::agent_def::AgentDef =
        serde_json::from_value(json!({"name": "acme-sdd-architect", "runtime": "claude-code"}))
            .unwrap();
    t.d.create_agent(hand).await.unwrap();
    let repo = acme_repo(ACME_AGENTS);
    let digest = digest_of(&t, &repo).await;
    let e =
        t.d.plugin_install(&repo.url(), None, None, &digest)
            .await
            .unwrap_err();
    assert_eq!(e.code, "name_taken");
    assert!(!t.home.path().join("plugins/acme-sdd").exists());
    assert!(t.d.store.plugin_records().await.unwrap().is_empty());
    let agents = t.d.store.list_agent_defs().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].source, None);
    assert!(t.d.store.all_flow_assignments().await.unwrap().is_empty());
}

#[tokio::test]
async fn install_of_a_flow_plugin_with_an_unknown_skill_works_and_the_agent_is_broken() {
    let t = daemon().await;
    let agents = r#"{"agents": [
        {"name": "analyst", "runtime": "claude-code", "skills": ["no-such-skill"]},
        {"name": "architect", "runtime": "claude-code"}]}"#;
    let repo = acme_repo(agents);
    let digest = digest_of(&t, &repo).await;
    t.d.plugin_install(&repo.url(), None, None, &digest)
        .await
        .unwrap();
    let views = t.d.agents().await.unwrap();
    let by = |n: &str| views.iter().find(|v| v.def.name == n).unwrap();
    assert!(by("acme-sdd-analyst").broken.is_some());
    assert!(by("acme-sdd-architect").broken.is_none());
}

#[tokio::test]
async fn two_flow_plugins_with_an_architect_live_side_by_side() {
    let t = daemon().await;
    for name in ["acme-sdd", "beta-sdd"] {
        let repo = flow_repo(name, ACME_AGENTS);
        let digest = digest_of(&t, &repo).await;
        t.d.plugin_install(&repo.url(), None, None, &digest)
            .await
            .unwrap();
    }
    let agents = t.d.store.list_agent_defs().await.unwrap();
    let mut names: Vec<_> = agents.iter().map(|a| a.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "acme-sdd-analyst",
            "acme-sdd-architect",
            "beta-sdd-analyst",
            "beta-sdd-architect"
        ]
    );
    let rows = t.d.store.all_flow_assignments().await.unwrap();
    assert_eq!(rows.iter().filter(|(p, _, _)| p == "acme-sdd").count(), 2);
    assert_eq!(rows.iter().filter(|(p, _, _)| p == "beta-sdd").count(), 2);
}

/// The `acme-sdd` manifest with other default assignments (stage → agent).
fn acme_manifest(assign: &[(&str, &str)]) -> String {
    let stages: serde_json::Map<String, Value> = assign
        .iter()
        .map(|(s, a)| (s.to_string(), json!({"agent": a})))
        .collect();
    manifest(
        "acme-sdd",
        "flow",
        json!({"data": {"flow": {"pack": "pack", "agents": "agents.json",
            "assignments": {"feature": stages}}}}),
    )
}

async fn install_acme(t: &T, repo: &Repo) {
    let digest = digest_of(t, repo).await;
    t.d.plugin_install(&repo.url(), None, None, &digest)
        .await
        .unwrap();
}

/// Preview the update of `acme-sdd`, then apply it; the preview as JSON.
async fn update_acme(t: &T) -> Value {
    let p = t.d.plugin_update_preview("acme-sdd", None).await.unwrap();
    let digest = json_of(&p)["digest"].as_str().unwrap().to_string();
    t.d.plugin_update("acme-sdd", None, &digest).await.unwrap();
    json_of(&p)
}

async fn agent_named(t: &T, name: &str) -> Option<xherd_core::agent_def::AgentDef> {
    let all = t.d.store.list_agent_defs().await.unwrap();
    all.into_iter().find(|a| a.name == name)
}

#[tokio::test]
async fn update_of_a_flow_plugin_overwrites_adds_archives_and_replaces_assignments() {
    let t = daemon().await;
    let repo = acme_repo(ACME_AGENTS);
    install_acme(&t, &repo).await;
    let analyst = agent_named(&t, "acme-sdd-analyst").await.unwrap();
    let mut architect = agent_named(&t, "acme-sdd-architect").await.unwrap();
    architect.instructions = Some("Draw circles.".into());
    t.d.store.put_agent_def(&architect).await.unwrap();

    // v2: the analyst is gone, a planner arrives, the architect is as shipped.
    repo.commit(&[
        (
            "agents.json",
            r#"{"agents": [
                {"name": "architect", "runtime": "claude-code", "instructions": "Draw boxes."},
                {"name": "planner", "runtime": "claude-code"}]}"#,
        ),
        (
            "xherd-plugin.json",
            &acme_manifest(&[("brief", "planner"), ("design", "architect")]),
        ),
    ]);
    let p = t.d.plugin_update_preview("acme-sdd", None).await.unwrap();
    let json = json_of(&p);
    let flow = &json["flow"];
    let state = |n: &str| {
        flow["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["savedName"] == n)
            .unwrap()["state"]
            .clone()
    };
    assert_eq!(state("acme-sdd-architect"), "overwritten");
    assert_eq!(state("acme-sdd-planner"), "new");
    assert_eq!(flow["archived"], json!(["acme-sdd-analyst"]));
    let changes = flow["assignmentChanges"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["stage"], "brief");
    assert_eq!(changes[0]["from"], "acme-sdd-analyst");
    assert_eq!(changes[0]["to"], "acme-sdd-planner");
    // The preview writes nothing.
    assert!(agent_named(&t, "acme-sdd-planner").await.is_none());

    update_acme(&t).await;
    let architect_now = agent_named(&t, "acme-sdd-architect").await.unwrap();
    assert_eq!(architect_now.id, architect.id);
    assert_eq!(architect_now.version, architect.version + 1);
    assert_eq!(architect_now.instructions.as_deref(), Some("Draw boxes."));
    let planner = agent_named(&t, "acme-sdd-planner").await.unwrap();
    assert_eq!(
        (planner.version, planner.source.as_deref()),
        (1, Some("acme-sdd"))
    );
    let gone = agent_named(&t, "acme-sdd-analyst").await.unwrap();
    assert_eq!(gone.id, analyst.id);
    assert!(gone.archived_at.is_some());
    let rows = t.d.store.all_flow_assignments().await.unwrap();
    assert_eq!(rows.len(), 2);
    let of = |stage: &str| {
        rows.iter()
            .find(|(_, _, a)| a.stage == stage)
            .unwrap()
            .2
            .agent_id
            .clone()
    };
    assert_eq!(of("brief"), planner.id);
    assert_eq!(of("design"), architect.id);
    assert!(t.d.store.plugin_records().await.unwrap()[0].data["updatedAt"].is_number());
    assert_eq!(scratch_left(&t), 0);
}

#[tokio::test]
async fn update_of_a_flow_plugin_keeps_an_archived_agent_archived_and_recreates_a_deleted_one() {
    let t = daemon().await;
    let repo = acme_repo(ACME_AGENTS);
    install_acme(&t, &repo).await;
    let analyst = agent_named(&t, "acme-sdd-analyst").await.unwrap();

    // v2 drops the analyst: archived. v3 ships it again: still archived.
    let architect_only = r#"{"agents": [{"name": "architect", "runtime": "claude-code"}]}"#;
    let both = r#"{"agents": [
        {"name": "analyst", "runtime": "claude-code"},
        {"name": "architect", "runtime": "claude-code"}]}"#;
    let only_design = acme_manifest(&[("design", "architect")]);
    repo.commit(&[
        ("agents.json", architect_only),
        ("xherd-plugin.json", &only_design),
    ]);
    update_acme(&t).await;
    assert!(
        agent_named(&t, "acme-sdd-analyst")
            .await
            .unwrap()
            .archived_at
            .is_some()
    );
    repo.commit(&[("agents.json", both)]);
    update_acme(&t).await;
    let still = agent_named(&t, "acme-sdd-analyst").await.unwrap();
    assert_eq!(still.id, analyst.id);
    assert!(still.archived_at.is_some());

    // Deleted by the user (it is unassigned): the next update creates it again.
    t.d.delete_agent("acme-sdd-analyst").await.unwrap();
    repo.commit(&[("README.md", "v4")]);
    update_acme(&t).await;
    let again = agent_named(&t, "acme-sdd-analyst").await.unwrap();
    assert_ne!(again.id, analyst.id);
    assert_eq!((again.version, again.archived_at), (1, None));
}

#[tokio::test]
async fn update_of_a_flow_plugin_is_refused_while_a_job_uses_the_pack() {
    let t = daemon().await;
    let repo = acme_repo(ACME_AGENTS);
    install_acme(&t, &repo).await;
    repo.commit(&[("README.md", "v2")]);
    let mut job = xherd_core::Job::new("sdd-1", 1);
    job.pack = Some("acme-sdd".into());
    job.status = xherd_core::Status::Paused;
    t.d.store.put_job(&job).await.unwrap();

    let e =
        t.d.plugin_update_preview("acme-sdd", None)
            .await
            .unwrap_err();
    assert_eq!(e.code, "plugin_in_use");
    assert!(e.message.contains("sdd-1"), "{}", e.message);
    let e = t.d.plugin_update("acme-sdd", None, "x").await.unwrap_err();
    assert_eq!(e.code, "plugin_in_use");
    assert!(t.home.path().join("plugins/acme-sdd").is_dir());

    job.status = xherd_core::Status::Done;
    t.d.store.put_job(&job).await.unwrap();
    update_acme(&t).await;
}

#[tokio::test]
async fn remove_of_a_flow_plugin_is_refused_while_a_job_uses_it_then_keeps_the_agents() {
    let t = daemon().await;
    let repo = acme_repo(ACME_AGENTS);
    install_acme(&t, &repo).await;
    let architect = agent_named(&t, "acme-sdd-architect").await.unwrap();
    let mut job = xherd_core::Job::new("sdd-1", 1);
    job.pack = Some("acme-sdd".into());
    job.status = xherd_core::Status::Paused;
    t.d.store.put_job(&job).await.unwrap();

    let e = t.d.plugin_remove("acme-sdd", false).await.unwrap_err();
    assert_eq!(e.code, "plugin_in_use");
    assert!(e.message.contains("sdd-1"), "{}", e.message);
    assert!(t.home.path().join("plugins/acme-sdd").is_dir());
    assert_eq!(t.d.store.all_flow_assignments().await.unwrap().len(), 2);

    job.status = xherd_core::Status::Done;
    t.d.store.put_job(&job).await.unwrap();
    t.d.plugin_remove("acme-sdd", false).await.unwrap();
    assert!(!t.home.path().join("plugins/acme-sdd").exists());
    assert!(t.d.store.plugin_records().await.unwrap().is_empty());
    assert!(t.d.store.all_flow_assignments().await.unwrap().is_empty());
    let kept = agent_named(&t, "acme-sdd-architect").await.unwrap();
    assert_eq!(kept.source, architect.source);
    assert!(kept.source.is_some());

    // Installing it again takes back its own agents; a hand-made one with the
    // plugin's prefix stays the user's and does not block it.
    let hand: xherd_core::agent_def::AgentDef =
        serde_json::from_value(json!({"name": "acme-sdd-x", "runtime": "claude-code"})).unwrap();
    t.d.create_agent(hand).await.unwrap();
    install_acme(&t, &repo).await;
    assert_eq!(
        agent_named(&t, "acme-sdd-architect").await.unwrap().id,
        architect.id
    );
    assert_eq!(agent_named(&t, "acme-sdd-x").await.unwrap().source, None);
    assert_eq!(t.d.store.all_flow_assignments().await.unwrap().len(), 2);
}

#[tokio::test]
async fn a_finished_job_of_a_removed_pack_still_loads_after_a_restart() {
    let t = daemon().await;
    let repo = acme_repo(ACME_AGENTS);
    install_acme(&t, &repo).await;
    let mut job = xherd_core::Job::new("sdd-1", 1);
    job.pack = Some("acme-sdd".into());
    job.status = xherd_core::Status::Done;
    t.d.store.put_job(&job).await.unwrap();
    t.d.plugin_remove("acme-sdd", false).await.unwrap();

    let d2 = restarted(&t).await;
    assert!(!d2.packs.contains_key("acme-sdd"));
    let a = d2.action("sdd-1", UserAction::Get).await.unwrap();
    assert_eq!(a.pack.as_deref(), Some("acme-sdd"));
    assert!(
        d2.store
            .list_jobs()
            .await
            .unwrap()
            .iter()
            .any(|j| j.name == "sdd-1")
    );
}

#[tokio::test]
async fn flows_name_the_plugin_that_provides_the_pack() {
    let a = acme().await;
    let flows = a.t.d.flows().await.unwrap();
    let source_of = |pack: &str| {
        let f = flows.iter().find(|f| f.pack == pack).unwrap();
        serde_json::to_value(f).unwrap().get("source").cloned()
    };
    assert_eq!(source_of("acme-sdd"), Some(json!("acme-sdd")));
    assert_eq!(source_of("xherd"), None);
    assert!(a.t.d.pack_selectable("acme-sdd"));
    assert!(a.t.d.pack_selectable("xherd"));
}
