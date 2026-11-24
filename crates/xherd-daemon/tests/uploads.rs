//! Terminal uploads and downloads (#192, R1–R6): `Daemon::begin_upload` and
//! `Daemon::download_path`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use xherd_core::Job;
use xherd_core::new::NewJob;
use xherd_core::paths::Paths;
use xherd_daemon::settings::ConfigPatch;
use xherd_daemon::*;

fn git(dir: &Path, args: &[&str]) {
    let o = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}");
}

fn req(repo: &Path, name: &str) -> NewJob {
    NewJob {
        name: name.into(),
        ticket: None,
        ticket_key: None,
        slug: None,
        repo: repo.to_string_lossy().into_owned(),
        agent: None,
        agents: Default::default(),
        base: None,
        flow: None,
        pack: None,
        prompt: Some("add login".into()),
        autonomy: None,
        permission_mode: None,
        runtime: None,
        model: None,
        new_project: None,
    }
}

struct T {
    d: Daemon,
    job: Job,
    wd: PathBuf,
    _dirs: Vec<tempfile::TempDir>,
}

impl T {
    fn uploads(&self) -> PathBuf {
        self.wd.join(".xherd/uploads")
    }

    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.uploads())
            .map(|r| {
                r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    git(repo.path(), &["init", "-q", "-b", "main"]);
    git(
        repo.path(),
        &["commit", "-q", "--allow-empty", "-m", "base"],
    );
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(ws.path().to_string_lossy().into_owned()),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
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
    let job = d.new_job(req(repo.path(), "up")).await.unwrap();
    let wd = PathBuf::from(job.workdir.clone().unwrap());
    T {
        d,
        job,
        wd,
        _dirs: vec![home, ws, repo],
    }
}

fn limits(t: &T, file: u64, job: u64, files: usize) {
    t.d.save_config(ConfigPatch {
        upload_max_bytes: Some(file),
        upload_max_job_bytes: Some(job),
        upload_max_files: Some(files),
        ..Default::default()
    })
    .unwrap();
}

#[tokio::test]
async fn files_are_stored_in_order_under_the_uploads_folder() {
    let t = setup().await;
    let mut u = t.d.begin_upload("up").await.unwrap();
    let a = u.add("shot.png", b"one").await.unwrap();
    let b = u.add("notes.txt", b"two").await.unwrap();
    assert_eq!(u.finish(), vec![a.clone(), b.clone()]);
    assert_eq!(a, t.uploads().join("shot.png"));
    assert_eq!(std::fs::read(&a).unwrap(), b"one");
    assert_eq!(std::fs::read(&b).unwrap(), b"two");
    assert_eq!(t.names(), ["notes.txt", "shot.png"]);
}

#[tokio::test]
async fn hostile_names_become_plain_and_a_taken_name_is_numbered() {
    let t = setup().await;
    let mut u = t.d.begin_upload("up").await.unwrap();
    for n in ["../x", ".env", "a b?.png", "shot.png", "shot.png"] {
        u.add(n, b"x").await.unwrap();
    }
    u.finish();
    assert_eq!(
        t.names(),
        ["a_b_.png", "env", "shot-2.png", "shot.png", "x"]
    );
    // The folder is the only place written: nothing escaped.
    assert!(!t.wd.join("x").exists());
    assert!(!t.wd.join(".xherd/x").exists());

    // A later upload numbers against what is already there.
    let mut u = t.d.begin_upload("up").await.unwrap();
    let p = u.add("shot.png", b"y").await.unwrap();
    assert_eq!(p, t.uploads().join("shot-3.png"));
    u.finish();
}

#[tokio::test]
async fn a_planted_symlink_is_neither_followed_nor_overwritten() {
    let t = setup().await;
    let target = t.wd.join("victim.txt");
    std::fs::write(&target, "keep").unwrap();
    std::fs::create_dir_all(t.uploads()).unwrap();
    std::os::unix::fs::symlink(&target, t.uploads().join("shot.png")).unwrap();
    let mut u = t.d.begin_upload("up").await.unwrap();
    let p = u.add("shot.png", b"new").await.unwrap();
    u.finish();
    assert_eq!(p, t.uploads().join("shot-2.png"));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
}

#[tokio::test]
async fn an_uploads_folder_that_is_a_symlink_is_refused() {
    let t = setup().await;
    let out = t.wd.join("elsewhere");
    std::fs::create_dir(&out).unwrap();
    std::os::unix::fs::symlink(&out, t.uploads()).unwrap();
    let e = t.d.begin_upload("up").await.err().unwrap();
    assert_eq!(e.code, "bad_path");
    assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
}

#[tokio::test]
async fn limits_refuse_and_a_refused_request_leaves_nothing() {
    let t = setup().await;
    limits(&t, 10, 25, 3);

    // One file over the file limit.
    let mut u = t.d.begin_upload("up").await.unwrap();
    u.add("a.txt", b"12345").await.unwrap();
    let e = u.add("big.bin", &[0u8; 11]).await.unwrap_err();
    assert_eq!(e.code, "too_large");
    u.abort().await;
    assert!(t.names().is_empty(), "{:?}", t.names());

    // Over the bytes of the job in all (counts what is already stored).
    let mut u = t.d.begin_upload("up").await.unwrap();
    for n in ["a", "b"] {
        u.add(n, &[0u8; 10]).await.unwrap();
    }
    u.finish();
    let mut u = t.d.begin_upload("up").await.unwrap();
    u.add("c", b"1234").await.unwrap();
    let e = u.add("d", b"12").await.unwrap_err();
    assert_eq!(e.code, "too_large");
    u.abort().await;
    assert_eq!(t.names(), ["a", "b"]);

    // Over the number of files.
    limits(&t, 10, 250, 3);
    let mut u = t.d.begin_upload("up").await.unwrap();
    u.add("c", b"1").await.unwrap();
    let e = u.add("d", b"1").await.unwrap_err();
    assert_eq!(e.code, "too_large");
    u.abort().await;
    assert_eq!(t.names(), ["a", "b"]);
}

#[tokio::test]
async fn a_lowered_limit_applies_to_the_next_upload() {
    let t = setup().await;
    let mut u = t.d.begin_upload("up").await.unwrap();
    assert_eq!(u.max_bytes(), 25 * 1024 * 1024);
    u.add("a", &[0u8; 100]).await.unwrap();
    u.finish();
    limits(&t, 50, 250 * 1024 * 1024, 200);
    let mut u = t.d.begin_upload("up").await.unwrap();
    assert_eq!(u.max_bytes(), 50);
    let e = u.add("b", &[0u8; 100]).await.unwrap_err();
    assert_eq!(e.code, "too_large");
    u.abort().await;
}

#[tokio::test]
async fn a_job_without_a_worktree_takes_no_uploads() {
    let t = setup().await;
    t.d.action(
        "up",
        UserAction::Cancel {
            remove_worktree: false,
        },
    )
    .await
    .unwrap();
    std::fs::remove_dir_all(&t.wd).unwrap();
    t.d.action("up", UserAction::RemoveWorktree).await.unwrap();
    assert_eq!(
        t.d.begin_upload("up").await.err().unwrap().code,
        "worktree_removed"
    );
    assert_eq!(
        t.d.download_path("up", "x").await.unwrap_err().code,
        "worktree_removed"
    );
}

#[tokio::test]
async fn a_job_without_a_workdir_is_no_workspace() {
    let t = setup().await;
    let mut b = Job::new("bare", 2);
    b.launch = Some(vec!["rt".into()]);
    t.d.create_job(b).await.unwrap();
    assert_eq!(
        t.d.begin_upload("bare").await.err().unwrap().code,
        "no_workspace"
    );
    assert_eq!(
        t.d.download_path("bare", "x").await.unwrap_err().code,
        "no_workspace"
    );
    let _ = &t.job;
}

#[tokio::test]
async fn a_stalled_upload_does_not_block_another() {
    let t = setup().await;
    // Begun, then the client stops before sending any file.
    let _stalled = t.d.begin_upload("up").await.unwrap();
    let other = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut u = t.d.begin_upload("up").await.unwrap();
        u.add("a.txt", b"x").await.unwrap();
        u
    })
    .await
    .expect("a second upload waited for the first one's body");
    other.finish();
    assert_eq!(t.names(), ["a.txt"]);
}

#[tokio::test]
async fn uploads_to_one_job_share_the_limits() {
    let t = setup().await;
    limits(&t, 10, 250 * 1024 * 1024, 3);
    let mut a = t.d.begin_upload("up").await.unwrap();
    let mut b = t.d.begin_upload("up").await.unwrap();
    b.add("b1", b"1").await.unwrap();
    b.add("b2", b"1").await.unwrap();
    a.add("a1", b"1").await.unwrap();
    // The file count includes the other request's files.
    assert_eq!(a.add("a2", b"1").await.unwrap_err().code, "too_large");
    b.finish();
    a.abort().await;
    assert_eq!(t.names(), ["b1", "b2"]);

    // The byte limit is shared the same way.
    limits(&t, 2, 3, 200);
    let mut c = t.d.begin_upload("up").await.unwrap();
    assert_eq!(c.add("c", b"12").await.unwrap_err().code, "too_large");
    c.add("c", b"1").await.unwrap();
    c.finish();
    assert_eq!(t.names(), ["b1", "b2", "c"]);
}

#[tokio::test]
async fn a_request_dropped_midway_keeps_nothing() {
    let t = setup().await;
    let mut u = t.d.begin_upload("up").await.unwrap();
    u.add("half.txt", b"x").await.unwrap();
    // The client went away before the request finished.
    drop(u);
    let gone = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !t.names().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(gone.is_ok(), "{:?}", t.names());
}

#[tokio::test]
async fn download_path_resolves_inside_the_workdir_only() {
    let t = setup().await;
    let root = t.wd.canonicalize().unwrap();
    std::fs::create_dir_all(t.wd.join("sub")).unwrap();
    std::fs::write(t.wd.join("sub/a.txt"), "a").unwrap();

    // Relative and absolute names of a regular file.
    assert_eq!(
        t.d.download_path("up", "sub/a.txt").await.unwrap(),
        root.join("sub/a.txt")
    );
    let abs = t.wd.join("sub/a.txt");
    assert_eq!(
        t.d.download_path("up", abs.to_str().unwrap())
            .await
            .unwrap(),
        root.join("sub/a.txt")
    );

    // `..` that stays inside works, one that leaves is refused.
    assert!(t.d.download_path("up", "sub/../sub/a.txt").await.is_ok());
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("s.txt"), "s").unwrap();
    let up = format!(
        "{}{}",
        "../".repeat(40),
        outside.path().join("s.txt").display()
    );
    assert_eq!(
        t.d.download_path("up", &up).await.unwrap_err().code,
        "bad_path"
    );
    let abs_out = outside.path().join("s.txt");
    assert_eq!(
        t.d.download_path("up", abs_out.to_str().unwrap())
            .await
            .unwrap_err()
            .code,
        "bad_path"
    );

    // A symlink pointing out is refused; one pointing in is fine.
    std::os::unix::fs::symlink(&abs_out, t.wd.join("out.txt")).unwrap();
    assert_eq!(
        t.d.download_path("up", "out.txt").await.unwrap_err().code,
        "bad_path"
    );
    std::os::unix::fs::symlink(t.wd.join("sub/a.txt"), t.wd.join("in.txt")).unwrap();
    assert_eq!(
        t.d.download_path("up", "in.txt").await.unwrap(),
        root.join("sub/a.txt")
    );

    // A folder, a missing file and an empty path.
    assert_eq!(
        t.d.download_path("up", "sub").await.unwrap_err().code,
        "no_such_file"
    );
    assert_eq!(
        t.d.download_path("up", "nope").await.unwrap_err().code,
        "no_such_file"
    );
    assert_eq!(
        t.d.download_path("up", "").await.unwrap_err().code,
        "bad_path"
    );
}
