//! Push status of the recorded commits (R1, R2, R3): at every stop the actor
//! compares the branch with the local `refs/remotes/origin/<branch>` and sets
//! `pushed` on each done commit. Local git only; a bare repository is the
//! origin and nothing is fetched.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use xherd_core::domain::{Commit, CommitStatus};
use xherd_core::paths::Paths;
use xherd_core::report::{Report, ReportEvent};
use xherd_core::{Job, Status};
use xherd_daemon::*;

struct T {
    _home: tempfile::TempDir,
    ws: tempfile::TempDir,
    d: Daemon,
    /// The two commits of the branch, oldest first.
    shas: [String; 2],
    /// The job's id (not its name): reports are routed by it.
    id: String,
}

impl T {
    fn repo(&self) -> PathBuf {
        self.ws.path().join("repo")
    }
    fn wt(&self) -> PathBuf {
        self.ws.path().join("wt")
    }
    fn bare(&self) -> PathBuf {
        self.ws.path().join("origin.git")
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

fn done(id: &str, sha: &str) -> Commit {
    Commit {
        id: id.into(),
        subject: id.into(),
        message: None,
        files: vec![],
        status: CommitStatus::Done,
        tasks: vec![],
        sha: Some(sha.into()),
        needs_human: None,
        pushed: None,
    }
}

/// A repository on `main` with a bare `origin`, and the job's worktree on
/// `SC-1` with two commits recorded as done (`C01`, `C02`). The job is
/// running; nothing is pushed.
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
    let bare = ws.path().join("origin.git");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&bare).unwrap();
    git(&bare, &["init", "-q", "--bare", "-b", "main"]);
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "user.name", "t"]);
    git(&repo, &["config", "user.email", "t@t"]);
    git(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
    std::fs::write(repo.join("base.txt"), "base\n").unwrap();
    git(&repo, &["add", "base.txt"]);
    git(&repo, &["commit", "-q", "-m", "base"]);
    git(&repo, &["push", "-q", "origin", "main"]);
    git(
        &repo,
        &["worktree", "add", "-q", "-b", "SC-1", wt.to_str().unwrap()],
    );
    std::fs::write(repo.join(".git/info/exclude"), ".xherd/\n").unwrap();
    std::fs::create_dir_all(wt.join(".xherd/spec")).unwrap();
    for f in ["one.rs", "two.rs"] {
        std::fs::write(wt.join(f), f).unwrap();
        git(&wt, &["add", f]);
        git(&wt, &["commit", "-q", "-m", f]);
    }
    let shas = [
        git(&wt, &["rev-parse", "HEAD~1"]),
        git(&wt, &["rev-parse", "HEAD"]),
    ];

    let drv = Arc::new(FakeDriver::default());
    let o = Options::new(drv, Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let mut a = Job::new("SC-1", 1);
    a.launch = Some(vec!["rt".into()]);
    a.workdir = Some(wt.to_string_lossy().into_owned());
    a.repo = Some(repo.to_string_lossy().into_owned());
    a.pack = Some("xherd".into());
    a.flow = Some("feature".into());
    a.base = Some("main".into());
    a.branch = "SC-1".into();
    a.commits = vec![done("C01", &shas[0]), done("C02", &shas[1])];
    let id = d.create_job(a).await.unwrap().id;
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
        shas,
        id,
    }
}

async fn stop(t: &T, seq: u64) {
    let r = Report {
        id: format!("s{seq}"),
        job_id: t.id.clone(),
        event: ReportEvent::Stop,
        payload: json!({"session_id": format!("s-{seq}")}),
        seq,
        at: 0,
    };
    t.d.report(r).await;
}

async fn get(t: &T) -> Job {
    t.d.action("SC-1", UserAction::Get).await.unwrap()
}

/// The rows' `pushed` flags once the check after a stop has been applied.
async fn pushed_after_stop(t: &T, seq: u64, want: [Option<bool>; 2]) -> Vec<Option<bool>> {
    stop(t, seq).await;
    let mut got = vec![];
    for _ in 0..200 {
        got = get(t).await.commits.iter().map(|c| c.pushed).collect();
        if got == want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    got
}

#[tokio::test]
async fn a_stop_marks_pushed_and_unpushed_commits() {
    let t = setup().await;
    // The first commit is on origin/SC-1, the second is not.
    git(
        &t.wt(),
        &[
            "push",
            "-q",
            "origin",
            &format!("{}:refs/heads/SC-1", t.shas[0]),
        ],
    );
    assert_eq!(
        git(&t.repo(), &["rev-parse", "refs/remotes/origin/SC-1"]),
        t.shas[0]
    );
    let got = pushed_after_stop(&t, 1, [Some(true), Some(false)]).await;
    assert_eq!(got, vec![Some(true), Some(false)]);

    // The job JSON carries it, camelCase, only where known.
    let v = serde_json::to_value(get(&t).await).unwrap();
    assert_eq!(v["commits"][0]["pushed"], true);
    assert_eq!(v["commits"][1]["pushed"], false);
}

#[tokio::test]
async fn a_stop_with_nothing_new_still_updates_pushed() {
    let t = setup().await;
    git(
        &t.wt(),
        &[
            "push",
            "-q",
            "origin",
            &format!("{}:refs/heads/SC-1", t.shas[0]),
        ],
    );
    let got = pushed_after_stop(&t, 1, [Some(true), Some(false)]).await;
    assert_eq!(got, vec![Some(true), Some(false)]);
    let rows = get(&t).await.commits.len();

    // Someone pushes the branch from outside xherd; no commit is unknown, so
    // the stop finds nothing to record, yet the flags follow the ref.
    git(&t.wt(), &["push", "-q", "origin", "SC-1"]);
    let got = pushed_after_stop(&t, 2, [Some(true), Some(true)]).await;
    assert_eq!(got, vec![Some(true), Some(true)]);
    assert_eq!(get(&t).await.commits.len(), rows);
}

#[tokio::test]
async fn without_a_tracking_ref_no_commit_has_a_flag() {
    let t = setup().await;
    assert!(
        std::process::Command::new("git")
            .args(["rev-parse", "--verify", "-q", "refs/remotes/origin/SC-1"])
            .current_dir(t.repo())
            .output()
            .unwrap()
            .stdout
            .is_empty()
    );
    let got = pushed_after_stop(&t, 1, [None, None]).await;
    assert_eq!(got, vec![None, None]);
    let v = serde_json::to_value(get(&t).await).unwrap();
    assert!(v["commits"][0].get("pushed").is_none());
    assert!(v["commits"][1].get("pushed").is_none());
    // Nothing was fetched or pushed behind the user's back.
    assert!(git(&t.bare(), &["branch", "--list", "SC-1"]).is_empty());
}

/// Stop the job and wait until it is no longer running.
async fn stopped(t: &T, seq: u64) {
    stop(t, seq).await;
    for _ in 0..200 {
        if get(t).await.status != Status::Running {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("the job never stopped");
}

async fn pushed_flags(t: &T) -> Vec<Option<bool>> {
    get(t).await.commits.iter().map(|c| c.pushed).collect()
}

async fn push_events(t: &T) -> usize {
    t.d.store
        .events(&t.id, 100)
        .await
        .unwrap()
        .iter()
        .filter(|e| e.kind == "user.push")
        .count()
}

async fn refusal(t: &T) -> &'static str {
    t.d.push("SC-1").await.unwrap_err().code
}

#[tokio::test]
async fn push_sends_the_branch_and_marks_every_row() {
    let t = setup().await;
    git(
        &t.wt(),
        &[
            "push",
            "-q",
            "origin",
            &format!("{}:refs/heads/SC-1", t.shas[0]),
        ],
    );
    stopped(&t, 1).await;
    let got = pushed_after_stop(&t, 2, [Some(true), Some(false)]).await;
    assert_eq!(got, vec![Some(true), Some(false)]);

    t.d.push("SC-1").await.unwrap();
    assert_eq!(git(&t.bare(), &["rev-parse", "SC-1"]), t.shas[1]);
    for _ in 0..200 {
        if pushed_flags(&t).await == [Some(true), Some(true)] {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(pushed_flags(&t).await, vec![Some(true), Some(true)]);
    let ev =
        t.d.store
            .events(&t.id, 100)
            .await
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "user.push")
            .expect("user.push event");
    assert!(format!("{:?}", ev.payload).contains(&t.shas[1]));
}

#[tokio::test]
async fn a_running_job_cannot_push() {
    let t = setup().await;
    assert_eq!(refusal(&t).await, "not_stopped");
    assert!(git(&t.bare(), &["branch", "--list", "SC-1"]).is_empty());
    assert_eq!(push_events(&t).await, 0);
}

#[tokio::test]
async fn without_an_origin_remote_nothing_is_pushed() {
    let t = setup().await;
    stopped(&t, 1).await;
    git(&t.repo(), &["remote", "remove", "origin"]);
    assert_eq!(refusal(&t).await, "no_origin");
    assert!(git(&t.bare(), &["branch", "--list", "SC-1"]).is_empty());
    assert_eq!(push_events(&t).await, 0);
}

#[tokio::test]
async fn nothing_to_push_is_refused() {
    let t = setup().await;
    git(&t.wt(), &["push", "-q", "origin", "SC-1"]);
    stopped(&t, 1).await;
    assert_eq!(refusal(&t).await, "nothing_to_push");
    assert_eq!(push_events(&t).await, 0);
}

#[tokio::test]
async fn without_a_tracking_ref_nothing_is_pushed() {
    let t = setup().await;
    stopped(&t, 1).await;
    assert_eq!(refusal(&t).await, "nothing_to_push");
    assert!(git(&t.bare(), &["branch", "--list", "SC-1"]).is_empty());
    assert_eq!(push_events(&t).await, 0);
}

#[tokio::test]
async fn a_refused_push_leaves_the_job_unchanged() {
    let t = setup().await;
    git(
        &t.wt(),
        &[
            "push",
            "-q",
            "origin",
            &format!("{}:refs/heads/SC-1", t.shas[0]),
        ],
    );
    // Someone else moved origin's branch to a commit this one does not have.
    let other = t.ws.path().join("other");
    git(
        t.ws.path(),
        &["clone", "-q", t.bare().to_str().unwrap(), "other"],
    );
    git(&other, &["config", "user.name", "o"]);
    git(&other, &["config", "user.email", "o@o"]);
    git(
        &other,
        &["checkout", "-q", "-b", "elsewhere", "origin/main"],
    );
    std::fs::write(other.join("x.txt"), "x").unwrap();
    git(&other, &["add", "x.txt"]);
    git(&other, &["commit", "-q", "-m", "x"]);
    git(
        &other,
        &["push", "-q", "--force", "origin", "elsewhere:SC-1"],
    );
    let theirs = git(&t.bare(), &["rev-parse", "SC-1"]);

    let got = pushed_after_stop(&t, 1, [Some(true), Some(false)]).await;
    assert_eq!(got, vec![Some(true), Some(false)]);
    assert_eq!(refusal(&t).await, "push_failed");
    assert_eq!(git(&t.bare(), &["rev-parse", "SC-1"]), theirs);
    assert_eq!(pushed_flags(&t).await, vec![Some(true), Some(false)]);
    assert_eq!(push_events(&t).await, 0);
}
