//! Shipping a job whose repository has no `origin` (R9, R10, R12): approval
//! marks it, the user then sets an origin (push, ship again) or finishes
//! locally (merge into the base checkout). Local git only.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::paths::Paths;
use xherd_core::report::{Report, ReportEvent};
use xherd_core::{Job, Status};
use xherd_daemon::*;

struct T {
    _home: tempfile::TempDir,
    ws: tempfile::TempDir,
    d: Daemon,
    drv: Arc<FakeDriver>,
    id: String,
}

impl T {
    fn repo(&self) -> PathBuf {
        self.ws.path().join("repo")
    }
    fn wt(&self) -> PathBuf {
        self.ws.path().join("wt")
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

const VALID_MEMORY: &str = "# Memory\n## State\nspec\n## Decisions\nnone\n## Code map\nsrc/a.rs: a\n## Watch out\nnothing\n";

/// A repository on `main` with one commit, and the job's worktree on `SC-1`
/// with one commit of its own (`feature.rs`). The job is running.
async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.json"),
        json!({"freshSessions": true, "workspace": ws.path()}).to_string(),
    )
    .unwrap();
    let repo = ws.path().join("repo");
    let wt = ws.path().join("wt");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    std::fs::write(repo.join("base.txt"), "base\n").unwrap();
    git(&repo, &["add", "base.txt"]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "SC-1", wt.to_str().unwrap()],
    );
    std::fs::write(repo.join(".git/info/exclude"), ".xherd/\n").unwrap();
    std::fs::create_dir_all(wt.join(".xherd/spec")).unwrap();
    std::fs::write(wt.join(".xherd/spec/memory.md"), VALID_MEMORY).unwrap();
    std::fs::write(wt.join(".xherd/spec/pr.md"), "# Add the feature\n").unwrap();
    std::fs::write(wt.join("feature.rs"), "feature\n").unwrap();
    git(&wt, &["add", "feature.rs"]);
    git(&wt, &["commit", "-q", "-m", "feature"]);

    let drv = Arc::new(FakeDriver::default());
    let mut o = Options::new(drv.clone(), Arc::new(SystemClock));
    o.pr_poll = Some(Duration::from_millis(1));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let mut a = Job::new("SC-1", 1);
    a.launch = Some(vec!["rt".into()]);
    a.workdir = Some(wt.to_string_lossy().into_owned());
    a.repo = Some(repo.to_string_lossy().into_owned());
    a.pack = Some("xherd".into());
    a.flow = Some("feature".into());
    a.base = Some("main".into());
    a.branch = "SC-1".into();
    let a = d.create_job(a).await.unwrap();
    for _ in 0..200 {
        if d.action("SC-1", UserAction::Get).await.unwrap().status == Status::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    T {
        _home: home,
        ws,
        d,
        drv,
        id: a.id,
    }
}

async fn report(t: &T, id: &str, event: ReportEvent, seq: u64) {
    let r = Report {
        id: id.into(),
        job_id: t.id.clone(),
        event,
        payload: json!({"session_id": format!("s-{seq}")}),
        seq,
        at: 0,
    };
    t.d.report(r).await;
}

async fn xap(t: &T, id: &str, cmd: &str, args: Value) {
    let c =
        serde_json::from_value(json!({"id": id, "jobId": t.id, "cmd": cmd, "args": args, "at": 1}))
            .unwrap();
    t.d.xap(c).await.unwrap();
}

async fn get(t: &T) -> Job {
    t.d.action("SC-1", UserAction::Get).await.unwrap()
}

async fn approve(t: &T) {
    t.d.action(
        "SC-1",
        UserAction::Approve {
            reason: None,
            expect: Default::default(),
            message: None,
        },
    )
    .await
    .unwrap();
}

fn sends(t: &T) -> Vec<String> {
    t.drv
        .calls()
        .into_iter()
        .filter(|c| c.starts_with("send "))
        .collect()
}

/// The ship gate is approved, the agent finds no remote and stops with
/// nothing pushed: the job is idle with `noOrigin` set.
async fn stopped_without_origin(t: &T) {
    xap(t, "g1", "gate.open", json!({"stage": "ship"})).await;
    report(t, "s1", ReportEvent::Stop, 1).await;
    assert_eq!(get(t).await.status, Status::WaitingInput);
    approve(t).await;
    report(t, "ss", ReportEvent::SessionStart, 2).await;
    report(t, "s3", ReportEvent::Stop, 3).await;
    let a = get(t).await;
    assert_eq!(
        (a.status, a.gate.is_none(), a.no_origin),
        (Status::Idle, true, true)
    );
}

/// An empty bare repository, to be the new origin.
fn bare(t: &T, name: &str) -> String {
    let p = t.ws.path().join(name);
    std::fs::create_dir_all(&p).unwrap();
    git(&p, &["init", "-q", "--bare", "-b", "main"]);
    p.to_string_lossy().into_owned()
}

/// A bare repository behind an https URL: the repository's own git config
/// rewrites the URL to the bare repo's path (`set-origin` refuses paths).
fn remote(t: &T, name: &str) -> (String, PathBuf) {
    let path = bare(t, name);
    let url = format!("https://example.test/acme/{name}");
    git(
        &t.repo(),
        &["config", &format!("url.{path}.insteadOf"), &url],
    );
    (url, PathBuf::from(path))
}

#[tokio::test]
async fn approving_the_ship_gate_without_origin_marks_the_job() {
    let t = setup().await;
    assert!(!get(&t).await.no_origin);
    xap(&t, "g1", "gate.open", json!({"stage": "ship"})).await;
    report(&t, "s1", ReportEvent::Stop, 1).await;
    approve(&t).await;
    assert!(get(&t).await.no_origin);
}

#[tokio::test]
async fn approving_the_ship_gate_with_origin_does_not() {
    let t = setup().await;
    let url = bare(&t, "origin.git");
    git(&t.repo(), &["remote", "add", "origin", &url]);
    xap(&t, "g1", "gate.open", json!({"stage": "ship"})).await;
    report(&t, "s1", ReportEvent::Stop, 1).await;
    approve(&t).await;
    assert!(!get(&t).await.no_origin);
}

#[tokio::test]
async fn set_origin_pushes_the_base_and_ships_again() {
    let t = setup().await;
    stopped_without_origin(&t).await;
    let (url, bare) = remote(&t, "origin.git");

    let a = t.d.set_origin("SC-1", &url).await.unwrap();
    assert!(!a.no_origin);
    assert_eq!(git(&t.repo(), &["config", "remote.origin.url"]), url);
    assert_eq!(
        git(&bare, &["rev-parse", "main"]),
        git(&t.repo(), &["rev-parse", "main"])
    );

    // A new ship session types the ship command again.
    report(&t, "ss2", ReportEvent::SessionStart, 4).await;
    assert_eq!(
        sends(&t).last().map(String::as_str),
        Some("send SC-1 \"/xherd:ship\" 1")
    );
    assert_ne!(get(&t).await.status, Status::Idle);
}

#[tokio::test]
async fn set_origin_refuses_other_history_and_leaves_no_remote() {
    let t = setup().await;
    stopped_without_origin(&t).await;
    // A bare repository that already has a `main` with unrelated history.
    let other = t.ws.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "main"]);
    git(&other, &["config", "user.name", "t"]);
    git(&other, &["config", "user.email", "t@t"]);
    git(
        &other,
        &["commit", "-q", "--allow-empty", "-m", "elsewhere"],
    );
    let (url, busy) = remote(&t, "busy.git");
    git(&other, &["push", "-q", &busy.to_string_lossy(), "main"]);

    let e = t.d.set_origin("SC-1", &url).await.unwrap_err();
    assert_eq!(e.code, "push_failed");
    let remotes = git(&t.repo(), &["remote"]);
    assert!(remotes.is_empty(), "left a remote: {remotes}");
    let a = get(&t).await;
    assert_eq!((a.status, a.no_origin), (Status::Idle, true));
}

#[tokio::test]
async fn set_origin_is_refused_when_the_job_is_not_stopped() {
    let t = setup().await;
    let (url, _) = remote(&t, "origin.git");
    let e = t.d.set_origin("SC-1", &url).await.unwrap_err();
    assert_eq!(e.code, "not_stopped");
    assert!(git(&t.repo(), &["remote"]).is_empty());
}

#[tokio::test]
async fn set_origin_refuses_what_is_not_a_repository_url() {
    let t = setup().await;
    stopped_without_origin(&t).await;
    let path = bare(&t, "origin.git");
    for url in [
        path.as_str(),
        &format!("file://{path}"),
        "ext::sh -c touch% /tmp/x",
        "https://example.test/acme/app\nx",
        "",
    ] {
        let e = t.d.set_origin("SC-1", url).await.unwrap_err();
        assert_eq!(e.code, "bad_remote", "{url}");
    }
    assert!(git(&t.repo(), &["remote"]).is_empty());
    let a = get(&t).await;
    assert_eq!((a.status, a.no_origin), (Status::Idle, true));
}

#[tokio::test]
async fn finish_local_merges_into_the_base_and_keeps_the_worktree() {
    let t = setup().await;
    stopped_without_origin(&t).await;

    let a = t.d.finish_local("SC-1").await.unwrap();
    assert_eq!(a.status, Status::Done);
    assert_eq!(
        std::fs::read_to_string(t.repo().join("feature.rs")).unwrap(),
        "feature\n"
    );
    assert_eq!(
        git(&t.repo(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "main"
    );
    assert!(git(&t.repo(), &["branch", "--merged", "main"]).contains("SC-1"));
    // The worktree and the branch are still there.
    assert!(t.wt().join("feature.rs").exists());
    assert!(git(&t.repo(), &["branch", "--list", "SC-1"]).contains("SC-1"));
}

#[tokio::test]
async fn finish_local_refuses_a_dirty_base_checkout() {
    let t = setup().await;
    stopped_without_origin(&t).await;
    std::fs::write(t.repo().join("base.txt"), "edited\n").unwrap();
    let before = git(&t.repo(), &["rev-parse", "main"]);

    let e = t.d.finish_local("SC-1").await.unwrap_err();
    assert_eq!(e.code, "dirty");
    assert_eq!(git(&t.repo(), &["rev-parse", "main"]), before);
    assert_eq!(
        std::fs::read_to_string(t.repo().join("base.txt")).unwrap(),
        "edited\n"
    );
    assert_eq!(get(&t).await.status, Status::Idle);
}

#[tokio::test]
async fn finish_local_refuses_when_the_base_is_not_checked_out() {
    let t = setup().await;
    stopped_without_origin(&t).await;
    git(&t.repo(), &["checkout", "-q", "-b", "elsewhere"]);
    let before = git(&t.repo(), &["rev-parse", "main"]);

    let e = t.d.finish_local("SC-1").await.unwrap_err();
    assert_eq!(e.code, "base_not_checked_out");
    assert_eq!(git(&t.repo(), &["rev-parse", "main"]), before);
    assert_eq!(get(&t).await.status, Status::Idle);
}

#[tokio::test]
async fn finish_local_aborts_a_conflicting_merge() {
    let t = setup().await;
    // The branch and the base both add `feature.rs`, differently.
    stopped_without_origin(&t).await;
    std::fs::write(t.repo().join("feature.rs"), "other\n").unwrap();
    git(&t.repo(), &["add", "feature.rs"]);
    git(&t.repo(), &["commit", "-q", "-m", "base feature"]);
    let before = git(&t.repo(), &["rev-parse", "main"]);

    let e = t.d.finish_local("SC-1").await.unwrap_err();
    assert_eq!(e.code, "merge_failed");
    assert_eq!(git(&t.repo(), &["rev-parse", "main"]), before);
    assert!(git(&t.repo(), &["status", "--porcelain"]).is_empty());
    assert_eq!(get(&t).await.status, Status::Idle);
}

#[tokio::test]
async fn finish_local_is_refused_when_the_job_is_not_stopped() {
    let t = setup().await;
    let e = t.d.finish_local("SC-1").await.unwrap_err();
    assert_eq!(e.code, "not_stopped");
    assert!(!t.repo().join("feature.rs").exists());
}
