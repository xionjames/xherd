//! `Daemon::new_job` with `newProject`: the daemon creates the repository
//! (R1, R3, R4, R5, R6, R7). Local git only.

use std::path::Path;
use std::sync::Arc;

use xherd_core::new::NewJob;
use xherd_core::paths::Paths;
use xherd_daemon::*;

struct T {
    _home: tempfile::TempDir,
    ws: tempfile::TempDir,
    d: Daemon,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {o:?}");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

async fn setup() -> T {
    setup_with(|_| {}).await
}

/// `before` runs on the workspace before the daemon starts.
async fn setup_with(before: impl FnOnce(&Path)) -> T {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    before(ws.path());
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    let cfg = xherd_core::config::Config {
        workspace: Some(ws.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    cfg.save(&paths.config()).unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    // Never the real Claude Code config: a fixture that trusts the workspace.
    let trust = home.path().join("claude.json");
    std::fs::write(
        &trust,
        serde_json::json!({"projects": {ws.path().to_string_lossy(): {"hasTrustDialogAccepted": true}}})
            .to_string(),
    )
    .unwrap();
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    let d = Daemon::start(paths, o).await.unwrap();
    T { _home: home, ws, d }
}

fn req(name: &str, project: &str) -> NewJob {
    NewJob {
        name: name.into(),
        ticket: None,
        ticket_key: None,
        slug: None,
        repo: String::new(),
        agent: None,
        agents: Default::default(),
        base: None,
        flow: None,
        prompt: Some("build a tool".into()),
        autonomy: None,
        permission_mode: None,
        runtime: None,
        model: None,
        new_project: Some(project.into()),
    }
}

fn projects(t: &T) -> std::path::PathBuf {
    t.ws.path().join("projects")
}

/// No folder is left under `projects/` (a temporary one included).
fn no_project_folders(t: &T) {
    let left: Vec<_> = std::fs::read_dir(projects(t))
        .map(|d| d.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "left behind: {left:?}");
}

#[tokio::test]
async fn creates_the_project_and_its_job() {
    let t = setup().await;
    let job = t.d.new_job(req("shop", "")).await.unwrap();
    let repo = projects(&t).join("shop");
    assert_eq!(job.repo.as_deref(), Some(repo.to_string_lossy().as_ref()));
    assert_eq!(job.base.as_deref(), Some("main"));
    assert_eq!(git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]), "main");
    assert_eq!(git(&repo, &["rev-list", "--count", "HEAD"]), "1");
    assert!(t.ws.path().join("worktrees/shop/shop").is_dir());

    let known = t.d.known_repos().await.unwrap();
    assert!(known.iter().any(|k| Path::new(&k.path) == repo));
}

#[tokio::test]
async fn a_named_project_is_a_folder_of_that_name() {
    let t = setup().await;
    let job = t.d.new_job(req("first-job", "tool")).await.unwrap();
    let repo = projects(&t).join("tool");
    assert_eq!(job.repo.as_deref(), Some(repo.to_string_lossy().as_ref()));
    assert!(t.ws.path().join("worktrees/tool/first-job").is_dir());
}

#[tokio::test]
async fn an_existing_folder_is_refused_and_left_alone() {
    let t = setup().await;
    let dir = projects(&t).join("shop");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("keep.txt"), "mine").unwrap();
    let e = t.d.new_job(req("shop", "")).await.unwrap_err();
    assert_eq!(e.code, "path_exists");
    assert_eq!(
        std::fs::read_to_string(dir.join("keep.txt")).unwrap(),
        "mine"
    );
    assert!(!dir.join(".git").exists());
    assert!(!t.ws.path().join("worktrees/shop").exists());
}

#[tokio::test]
async fn a_refused_job_leaves_no_project() {
    let t = setup().await;
    t.d.new_job(req("taken", "one")).await.unwrap();

    // The name is taken.
    let e = t.d.new_job(req("taken", "two")).await.unwrap_err();
    assert_eq!(e.code, "name_taken");
    assert!(!projects(&t).join("two").exists());

    // The runtime is not there.
    let mut r = req("other", "three");
    r.runtime = Some("no-such-runtime".into());
    assert!(t.d.new_job(r).await.is_err());
    assert!(!projects(&t).join("three").exists());

    // A bad name is refused before anything exists.
    let e = t.d.new_job(req("bad", "Bad Name")).await.unwrap_err();
    assert_eq!(e.code, "bad_name");
    assert!(!projects(&t).join("Bad Name").exists());

    let left: Vec<_> = std::fs::read_dir(projects(&t))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["one"]);
}

#[tokio::test]
async fn a_failure_after_the_project_was_made_removes_it() {
    let t = setup().await;
    // `worktrees/shop` is a file: the job's folder cannot be created.
    std::fs::create_dir_all(t.ws.path().join("worktrees")).unwrap();
    std::fs::write(t.ws.path().join("worktrees/shop"), "in the way").unwrap();
    assert!(t.d.new_job(req("shop", "")).await.is_err());
    assert!(!projects(&t).join("shop").exists());
    no_project_folders(&t);
}

#[tokio::test]
async fn a_ticket_url_does_not_clone_a_repository() {
    let t = setup().await;
    let mut r = req("fix-login", "");
    r.ticket = Some("https://github.com/acme/api/issues/12".into());
    let job = t.d.new_job(r).await.unwrap();
    let repo = projects(&t).join("fix-login");
    assert_eq!(job.repo.as_deref(), Some(repo.to_string_lossy().as_ref()));
    assert!(!t.ws.path().join("repos").exists());
}

#[tokio::test]
async fn two_creations_of_one_project_make_it_once() {
    let t = setup().await;
    let (a, b) = tokio::join!(
        t.d.new_job(req("one", "tool")),
        t.d.new_job(req("two", "tool"))
    );
    let codes: Vec<_> = [&a, &b]
        .iter()
        .map(|r| r.as_ref().err().map(|e| e.code))
        .collect();
    assert!(
        codes.contains(&None) && codes.contains(&Some("path_exists")),
        "{codes:?}"
    );
    let left: Vec<_> = std::fs::read_dir(projects(&t))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["tool"]);
}

#[tokio::test]
async fn start_removes_half_made_projects() {
    let t = setup_with(|ws| {
        let p = ws.join("projects");
        std::fs::create_dir_all(p.join(".tool.creating-1/.git")).unwrap();
        std::fs::create_dir_all(p.join("kept/.git")).unwrap();
    })
    .await;
    let left: Vec<_> = std::fs::read_dir(projects(&t))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["kept"]);
}
