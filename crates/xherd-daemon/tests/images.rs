//! `Daemon::image` and the file reads next to it, with real git in a temp
//! worktree (R1, R2, R7c, R7e, R9). The setup helpers are copied from
//! `tests/xap.rs`: integration test files do not share code.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::paths::Paths;
use xherd_core::report::{Report, ReportEvent};
use xherd_core::xap::{Refusal, XapCall};
use xherd_core::{Job, Status};
use xherd_daemon::*;

struct T {
    _home: tempfile::TempDir,
    wt: tempfile::TempDir,
    d: Daemon,
    a: Job,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
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

async fn setup() -> T {
    let home = tempfile::tempdir().unwrap();
    let ws = home.path().join("ws");
    std::fs::write(
        home.path().join("config.json"),
        json!({"freshSessions": true, "workspace": ws}).to_string(),
    )
    .unwrap();
    let wt = tempfile::tempdir().unwrap();
    git(wt.path(), &["init", "-q", "-b", "main"]);
    git(wt.path(), &["config", "user.email", "t@t"]);
    git(wt.path(), &["config", "user.name", "t"]);
    git(wt.path(), &["commit", "-q", "--allow-empty", "-m", "base"]);
    std::fs::create_dir_all(wt.path().join(".xherd/spec")).unwrap();
    std::fs::write(wt.path().join(".xherd/spec/memory.md"), VALID_MEMORY).unwrap();
    let drv = Arc::new(FakeDriver::default());
    let o = Options::new(drv, Arc::new(SystemClock));
    let d = Daemon::start(Paths::at(home.path()), o).await.unwrap();
    let mut a = Job::new("SC-1", 1);
    a.launch = Some(vec!["rt".into()]);
    a.workdir = Some(wt.path().to_string_lossy().into_owned());
    a.pack = Some("xherd".into());
    a.flow = Some("feature".into());
    a.base = Some("main".into());
    let a = d.create_job(a).await.unwrap();
    for _ in 0..200 {
        if d.action("SC-1", UserAction::Get).await.unwrap().status == Status::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    T {
        _home: home,
        wt,
        d,
        a,
    }
}

async fn xap(t: &T, id: &str, cmd: &str, args: Value) -> Result<Value, Refusal> {
    let call: XapCall = serde_json::from_value(
        json!({"id": id, "jobId": t.a.id, "cmd": cmd, "args": args, "at": 1}),
    )
    .unwrap();
    t.d.xap(call).await
}

/// Propose `files` as C01, stop, approve: xherd makes the commit.
async fn commit_files(t: &T, files: &[&str]) {
    std::fs::write(
        t.wt.path().join(".xherd/spec/tasks.md"),
        format!("## T01: change\n- files: {}\n", files.join(", ")),
    )
    .unwrap();
    xap(t, "g1", "tasks.sync", json!({})).await.unwrap();
    xap(t, "g2", "task.start", json!({"id": "T01"}))
        .await
        .unwrap();
    xap(
        t,
        "g3",
        "commit.propose",
        json!({"id": "C01", "subject": "Change", "message": "Change",
               "files": files, "tasks": ["T01"]}),
    )
    .await
    .unwrap();
    t.d.report(Report {
        id: "stop-1".into(),
        job_id: t.a.id.clone(),
        event: ReportEvent::Stop,
        payload: Value::Null,
        seq: 1,
        at: 0,
    })
    .await;
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

/// Bytes that are not UTF-8 and differ per `n`.
fn png(n: u8) -> Vec<u8> {
    vec![0x89, b'P', b'N', b'G', 0xff, 0x00, n, 0xfe]
}

fn put(t: &T, rel: &str, bytes: &[u8]) {
    std::fs::write(t.wt.path().join(rel), bytes).unwrap();
}

const MIB: usize = 1024 * 1024;

/// Fixtures on `main` (the merge base), then a feature branch whose C01
/// changes them: `mod` (v1 → v2), `new`, `gone`, `ren-old` → `ren-new`, and
/// `doc.md`. Afterwards the worktree has moved on: `mod.png` is v3 and
/// `wt.png` is untracked.
async fn repo() -> T {
    let t = setup().await;
    put(&t, "mod.png", &png(1));
    put(&t, "gone.png", &png(4));
    put(&t, "ren-old.png", &png(5));
    put(&t, "doc.md", b"old text\n");
    put(&t, "note.txt", b"just text\n");
    put(&t, "ok5.png", &vec![7u8; 5 * MIB]);
    put(&t, "big.png", &vec![7u8; 9 * MIB]);
    put(&t, "big.txt", &vec![7u8; 9 * MIB]);
    put(&t, "big.md", &vec![b'x'; 3 * MIB]);
    std::os::unix::fs::symlink("mod.png", t.wt.path().join("link.png")).unwrap();
    git(t.wt.path(), &["add", "--all"]);
    git(t.wt.path(), &["commit", "-q", "-m", "fixtures"]);
    git(t.wt.path(), &["checkout", "-q", "-b", "feature"]);
    std::fs::write(t.wt.path().join(".git/info/exclude"), ".xherd/\n").unwrap();

    put(&t, "mod.png", &png(2));
    put(&t, "new.png", &png(3));
    std::fs::remove_file(t.wt.path().join("gone.png")).unwrap();
    std::fs::remove_file(t.wt.path().join("ren-old.png")).unwrap();
    put(&t, "ren-new.png", &png(5));
    put(&t, "doc.md", b"new text\n");
    commit_files(
        &t,
        &[
            "mod.png",
            "new.png",
            "gone.png",
            "ren-old.png",
            "ren-new.png",
            "doc.md",
        ],
    )
    .await;

    put(&t, "mod.png", &png(3));
    put(&t, "wt.png", &png(6));
    t
}

/// What `image` answers, as (bytes or refusal code).
async fn read(
    t: &T,
    path: &str,
    side: Option<&str>,
    view: Option<&str>,
    commit: Option<&str>,
) -> Result<Image, String> {
    t.d.image("SC-1", path, side, view, commit)
        .await
        .map_err(|e| e.code.to_string())
}

async fn bytes(
    t: &T,
    path: &str,
    side: Option<&str>,
    view: Option<&str>,
    commit: Option<&str>,
) -> Result<Vec<u8>, String> {
    read(t, path, side, view, commit).await.map(|i| i.bytes)
}

#[tokio::test]
async fn each_view_reads_its_old_and_new_side() {
    let t = repo().await;
    let c = Some("C01");
    // (side, view, commit) → the version of mod.png.
    for (side, view, commit, want) in [
        (None, None, None, 3),
        (Some("new"), None, None, 3),
        (Some("old"), None, None, 2),
        (Some("new"), Some("uncommitted"), None, 3),
        (Some("old"), Some("uncommitted"), None, 2),
        (Some("new"), Some("branch"), None, 3),
        (Some("old"), Some("branch"), None, 1),
        (Some("new"), Some("commit"), c, 2),
        (Some("old"), Some("commit"), c, 1),
    ] {
        assert_eq!(
            bytes(&t, "./mod.png", side, view, commit).await,
            Ok(png(want)),
            "{side:?} {view:?} {commit:?}"
        );
    }

    // Added: no old side. Untracked in the worktree: only in the branch view's new side.
    assert_eq!(
        bytes(&t, "./new.png", Some("old"), Some("commit"), c).await,
        Err("no_such_doc".into())
    );
    assert_eq!(
        bytes(&t, "./new.png", Some("new"), Some("commit"), c).await,
        Ok(png(3))
    );
    assert_eq!(
        bytes(&t, "./wt.png", Some("new"), Some("branch"), None).await,
        Ok(png(6))
    );
    assert_eq!(
        bytes(&t, "./wt.png", Some("old"), Some("branch"), None).await,
        Err("no_such_doc".into())
    );
    // Deleted: only an old side.
    assert_eq!(
        bytes(&t, "./gone.png", Some("old"), Some("commit"), c).await,
        Ok(png(4))
    );
    for (side, view, commit) in [
        (Some("new"), Some("commit"), c),
        (None, None, None),
        (Some("new"), Some("branch"), None),
    ] {
        assert_eq!(
            bytes(&t, "./gone.png", side, view, commit).await,
            Err("no_such_doc".into()),
            "{side:?} {view:?}"
        );
    }
    // Renamed: the old side is read at the old path, the new side at the new one.
    assert_eq!(
        bytes(&t, "./ren-old.png", Some("old"), Some("commit"), c).await,
        Ok(png(5))
    );
    assert_eq!(
        bytes(&t, "./ren-new.png", Some("new"), Some("commit"), c).await,
        Ok(png(5))
    );
}

#[tokio::test]
async fn the_blob_id_is_the_one_git_gives_the_bytes() {
    let t = repo().await;
    let wt = t.wt.path();
    let sha = git(wt, &["rev-parse", "HEAD"]);
    let base = git(wt, &["merge-base", "main", "HEAD"]);

    let i = read(&t, "./mod.png", Some("new"), Some("commit"), Some("C01"))
        .await
        .unwrap();
    assert_eq!(i.blob, git(wt, &["rev-parse", &format!("{sha}:mod.png")]));
    assert_eq!(i.mime, "image/png");
    let i = read(&t, "./mod.png", Some("old"), Some("branch"), None)
        .await
        .unwrap();
    assert_eq!(i.blob, git(wt, &["rev-parse", &format!("{base}:mod.png")]));
    // The worktree side: what `git hash-object` says of the file now (v3).
    let i = read(&t, "./mod.png", None, None, None).await.unwrap();
    assert_eq!(i.blob, git(wt, &["hash-object", "mod.png"]));
    assert_ne!(
        i.blob,
        git(wt, &["rev-parse", &format!("{sha}:mod.png")]),
        "the worktree moved on since the commit"
    );
}

#[tokio::test]
async fn paths_the_worktree_does_not_serve() {
    let t = repo().await;
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("x.png"), png(9)).unwrap();
    std::os::unix::fs::symlink(outside.path().join("x.png"), t.wt.path().join("escape.png"))
        .unwrap();
    std::fs::create_dir(t.wt.path().join("dir.png")).unwrap();
    let abs_out = outside.path().join("x.png").to_string_lossy().into_owned();

    for (path, want) in [
        ("./../x.png", "bad_path"),
        ("./a/../../x.png", "bad_path"),
        (abs_out.as_str(), "bad_path"),
        ("./", "no_such_doc"),
        ("./escape.png", "no_such_doc"),
        ("./dir.png", "no_such_doc"),
        ("./missing.png", "no_such_doc"),
        ("./missing.txt", "no_such_doc"),
        ("./note.txt", "not_an_image"),
        ("./big.txt", "not_an_image"),
        ("./big.png", "too_large"),
    ] {
        assert_eq!(
            bytes(&t, path, None, None, None).await,
            Err(want.into()),
            "worktree {path}"
        );
    }
    // The same paths on a revision (those that name a worktree-only thing are
    // simply missing there).
    let (side, view, c) = (Some("new"), Some("commit"), Some("C01"));
    for (path, want) in [
        ("./../x.png", "bad_path"),
        ("./", "no_such_doc"),
        ("./missing.png", "no_such_doc"),
        ("./missing.txt", "no_such_doc"),
        ("./note.txt", "not_an_image"),
        ("./big.txt", "not_an_image"),
        ("./link.png", "not_an_image"),
        ("./big.png", "too_large"),
    ] {
        assert_eq!(
            bytes(&t, path, side, view, c).await,
            Err(want.into()),
            "revision {path}"
        );
    }
}

#[tokio::test]
async fn images_up_to_8_mib_are_read_on_both_paths() {
    let t = repo().await;
    let want = vec![7u8; 5 * MIB];
    assert_eq!(
        bytes(&t, "./ok5.png", None, None, None).await,
        Ok(want.clone())
    );
    assert_eq!(
        bytes(&t, "./ok5.png", Some("new"), Some("commit"), Some("C01")).await,
        Ok(want)
    );
    // Exactly the limit is still served, one byte more is not.
    put(&t, "edge.png", &vec![1u8; 8 * MIB]);
    assert_eq!(
        bytes(&t, "./edge.png", None, None, None)
            .await
            .map(|b| b.len()),
        Ok(8 * MIB)
    );
    put(&t, "edge.png", &vec![1u8; 8 * MIB + 1]);
    assert_eq!(
        bytes(&t, "./edge.png", None, None, None).await,
        Err("too_large".into())
    );
}

#[tokio::test]
async fn arguments_that_make_no_sense_are_refused() {
    let t = repo().await;
    for (side, view, commit, want) in [
        (Some("both"), None, None, "bad_request"),
        (Some("new"), Some("nope"), None, "bad_request"),
        (Some("new"), Some("commit"), None, "bad_request"),
        (Some("new"), Some("commit"), Some("C99"), "commit_unknown"),
    ] {
        assert_eq!(
            bytes(&t, "./mod.png", side, view, commit).await,
            Err(want.into()),
            "{side:?} {view:?} {commit:?}"
        );
    }
}

#[tokio::test]
async fn a_root_commit_has_no_old_side() {
    let t = setup().await;
    git(t.wt.path(), &["checkout", "-q", "--orphan", "solo"]);
    std::fs::write(t.wt.path().join(".git/info/exclude"), ".xherd/\n").unwrap();
    put(&t, "root.png", &png(1));
    commit_files(&t, &["root.png"]).await;
    assert_eq!(
        bytes(&t, "./root.png", Some("old"), Some("commit"), Some("C01")).await,
        Err("no_such_doc".into())
    );
    assert_eq!(
        bytes(&t, "./root.png", Some("new"), Some("commit"), Some("C01")).await,
        Ok(png(1))
    );
}

#[tokio::test]
async fn diff_source_at_a_sha_gives_that_commits_text() {
    let t = repo().await;
    put(&t, "doc.md", b"edited later\n");
    let v =
        t.d.diff_source("SC-1", "doc.md", Some("commit"), Some("C01"))
            .await
            .unwrap();
    assert_eq!(v["text"], "new text\n");
    let v =
        t.d.diff_source("SC-1", "doc.md", Some("uncommitted"), None)
            .await
            .unwrap();
    assert_eq!(v["text"], "edited later\n");
    // Text keeps its 2 MiB limit, on a revision and in the worktree, while an
    // image of the same size is served.
    for (view, commit) in [(Some("commit"), Some("C01")), (Some("uncommitted"), None)] {
        let e =
            t.d.diff_source("SC-1", "big.md", view, commit)
                .await
                .unwrap_err();
        assert_eq!(e.code, "too_large", "{view:?}");
    }
    assert!(bytes(&t, "./ok5.png", None, None, None).await.is_ok());
}

#[tokio::test]
async fn documents_keep_their_2_mib_limit_and_data_url() {
    let t = repo().await;
    let e = t.d.doc("SC-1", "./big.md").await.unwrap_err();
    assert_eq!(e.code, "too_large");
    let v = t.d.doc("SC-1", "./mod.png").await.unwrap();
    assert!(
        v["dataUrl"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
}
