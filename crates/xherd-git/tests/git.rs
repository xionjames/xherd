//! Integration tests with real git in temporary folders.

use std::path::Path;

use xherd_git::Git;

async fn init_repo(dir: &Path) -> Git {
    let g = Git::default();
    for args in [
        &["init", "--quiet", "-b", "main"][..],
        &["config", "user.email", "t@example.com"],
        &["config", "user.name", "t"],
        &["config", "commit.gpgsign", "false"],
    ] {
        g.run(dir, args).await.unwrap();
    }
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    g.run(dir, &["add", "."]).await.unwrap();
    g.run(dir, &["commit", "--quiet", "-m", "init"])
        .await
        .unwrap();
    g
}

#[tokio::test]
async fn worktree_lifecycle_diff_and_commits() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let g = init_repo(&repo).await;
    assert!(g.is_repo(&repo).await);
    assert!(!g.is_repo(d.path()).await);

    let wt = d.path().join("SC-1");
    g.worktree_add(&repo, &wt, "SC-1", "main").await.unwrap();
    assert!(g.branch_exists(&repo, "SC-1").await);
    assert_eq!(g.current_branch(&wt).await.as_deref(), Some("SC-1"));
    // --no-track: no upstream configured.
    assert!(
        g.run(&wt, &["rev-parse", "--abbrev-ref", "@{u}"])
            .await
            .is_err()
    );

    let list = g.worktree_list(&repo).await.unwrap();
    assert_eq!(list.len(), 2);
    assert!(list.iter().any(|w| w.branch.as_deref() == Some("SC-1")));

    std::fs::write(wt.join("a.txt"), "two\n").unwrap();
    std::fs::write(wt.join("b.txt"), "new\n").unwrap();
    let diff = g.diff(&wt, Some("main"), &["a.txt"]).await.unwrap();
    assert!(diff.contains("+two"));
    assert_eq!(
        g.changed_files(&wt, "main").await.unwrap(),
        vec!["a.txt", "b.txt"]
    );

    g.run(&wt, &["add", "a.txt"]).await.unwrap();
    g.run(&wt, &["commit", "--quiet", "-m", "change a"])
        .await
        .unwrap();
    let sha = g.rev_parse(&wt, "HEAD").await.unwrap();
    assert_eq!(g.commit_files(&wt, &sha).await.unwrap(), vec!["a.txt"]);
    assert!(g.is_ancestor(&wt, &sha, "SC-1").await);
    assert!(!g.is_ancestor(&repo, &sha, "main").await);
    assert_eq!(g.count_commits(&repo, "SC-1", "main").await.unwrap(), 1);

    let bundle = d.path().join("SC-1.bundle");
    g.bundle_create(&repo, &bundle, "SC-1", Some("main"))
        .await
        .unwrap();
    g.bundle_verify(&repo, &bundle).await.unwrap();

    g.worktree_remove(&repo, &wt, true).await.unwrap();
    assert!(!wt.exists());
    assert!(g.branch_exists(&repo, "SC-1").await, "branch is kept");
}

#[tokio::test]
async fn excludes_are_shared_and_not_duplicated() {
    let d = tempfile::tempdir().unwrap();
    let repo = d.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let g = init_repo(&repo).await;
    let wt = d.path().join("SC-2");
    g.worktree_add(&repo, &wt, "SC-2", "main").await.unwrap();

    g.add_excludes(&wt, &[".xherd/", ".claude/settings.local.json"])
        .await
        .unwrap();
    g.add_excludes(&wt, &[".xherd/"]).await.unwrap();
    let ex = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
    assert_eq!(ex.matches(".xherd/").count(), 1);

    std::fs::create_dir(wt.join(".xherd")).unwrap();
    std::fs::write(wt.join(".xherd/agent.json"), "{}").unwrap();
    assert!(g.changed_files(&wt, "main").await.unwrap().is_empty());
}

#[tokio::test]
async fn merges_cleanly_tells_conflicts_apart() {
    let d = tempfile::tempdir().unwrap();
    let g = init_repo(d.path()).await;
    let dir = d.path();
    let commit = |branch: &'static str, file: &'static str, text: &'static str| {
        let g = g.clone();
        async move {
            g.run(dir, &["checkout", "--quiet", branch]).await.unwrap();
            std::fs::write(dir.join(file), text).unwrap();
            g.run(dir, &["commit", "--quiet", "-am", "x"])
                .await
                .unwrap();
        }
    };
    g.run(dir, &["branch", "clean"]).await.unwrap();
    g.run(dir, &["branch", "clash"]).await.unwrap();
    commit("main", "a.txt", "main\n").await;
    commit("clash", "a.txt", "clash\n").await;
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    g.run(dir, &["checkout", "--quiet", "clean"]).await.unwrap();
    g.run(dir, &["add", "b.txt"]).await.unwrap();
    g.run(dir, &["commit", "--quiet", "-m", "b"]).await.unwrap();

    assert_eq!(g.merges_cleanly(dir, "main", "clean").await, Some(true));
    assert_eq!(g.merges_cleanly(dir, "main", "clash").await, Some(false));
    assert_eq!(g.merges_cleanly(dir, "main", "no-such").await, None);
    assert_eq!(
        g.run(dir, &["status", "--porcelain"]).await.unwrap(),
        "",
        "nothing in the worktree changed"
    );
}

#[tokio::test]
async fn failures_carry_stderr() {
    let d = tempfile::tempdir().unwrap();
    let g = init_repo(d.path()).await;
    let err = g.rev_parse(d.path(), "no-such-branch").await;
    assert!(err.is_err());
    let err = g
        .run(d.path(), &["checkout", "nope"])
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("nope"), "{err}");
}

#[tokio::test]
async fn diff_proposed_covers_new_deleted_and_leaves_the_index_alone() {
    let d = tempfile::tempdir().unwrap();
    let g = init_repo(d.path()).await;
    let dir = d.path();
    std::fs::write(dir.join("b.txt"), "gone\n").unwrap();
    std::fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
    g.run(dir, &["add", "."]).await.unwrap();
    g.run(dir, &["commit", "--quiet", "-m", "b"]).await.unwrap();

    std::fs::write(dir.join("a.txt"), "two\n").unwrap();
    std::fs::remove_file(dir.join("b.txt")).unwrap();
    std::fs::write(dir.join("new [1].txt"), "fresh\n").unwrap();
    std::fs::write(dir.join("x.log"), "ignored but proposed\n").unwrap();
    std::fs::write(dir.join("other.txt"), "not proposed\n").unwrap();
    let files: Vec<String> = ["a.txt", "b.txt", "new [1].txt", "x.log", "missing.txt"]
        .map(String::from)
        .to_vec();
    let diff = g.diff_proposed(dir, &files).await.unwrap();
    assert!(diff.contains("+two"), "{diff}");
    assert!(
        diff.contains("deleted file mode") && diff.contains("-gone"),
        "{diff}"
    );
    assert!(diff.contains("+fresh"), "{diff}");
    assert!(diff.contains("+ignored but proposed"), "{diff}");
    assert!(!diff.contains("not proposed"), "{diff}");
    assert!(!diff.contains("missing.txt"), "{diff}");

    // The worktree's own index is untouched.
    assert_eq!(
        g.run(dir, &["diff", "--cached", "--name-only"])
            .await
            .unwrap(),
        ""
    );
    assert_eq!(g.diff_proposed(dir, &[]).await.unwrap(), "");
    assert_eq!(
        g.diff_proposed(dir, &["missing.txt".into()]).await.unwrap(),
        ""
    );
}

#[tokio::test]
async fn diff_proposed_leaves_out_files_over_the_limit() {
    let d = tempfile::tempdir().unwrap();
    let g = init_repo(d.path()).await;
    let dir = d.path();
    std::fs::write(dir.join("small.txt"), "shown\n").unwrap();
    // A heap dump: sparse, so it takes no disk, but reading it would be slow.
    std::fs::File::create(dir.join("java.hprof"))
        .unwrap()
        .set_len(6 << 30)
        .unwrap();
    let files: Vec<String> = ["java.hprof", "small.txt"].map(String::from).to_vec();
    let diff = g.diff_proposed(dir, &files).await.unwrap();
    assert!(diff.contains("+shown"), "{diff}");
    assert!(
        diff.ends_with(
            "diff --git a/java.hprof b/java.hprof\n\
             Not shown: java.hprof is 6.0 GiB, over the 8.0 MiB limit\n"
        ),
        "{diff}"
    );
    assert!(diff.contains(xherd_git::TOO_LARGE));
    // Only the large file: no git call stages anything.
    let only = g.diff_proposed(dir, &files[..1]).await.unwrap();
    assert!(only.starts_with("diff --git a/java.hprof"), "{only}");
}

#[tokio::test]
async fn clone_and_branches() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("src");
    std::fs::create_dir(&src).unwrap();
    let g = init_repo(&src).await;
    g.run(&src, &["branch", "dev"]).await.unwrap();

    let dest = d.path().join("clone");
    g.clone_repo(&src.to_string_lossy(), &dest).await.unwrap();
    assert!(g.is_repo(&dest).await);
    let mut b = g.branches(&dest).await.unwrap();
    b.sort();
    assert_eq!(b, ["main", "origin/dev", "origin/main"]);
    assert_eq!(
        g.remote_default_branch(&dest, "origin").await.as_deref(),
        Some("origin/main")
    );
    g.fetch(&dest, "origin").await.unwrap();
    // Cloning onto an existing repository fails and leaves it alone.
    assert!(g.clone_repo(&src.to_string_lossy(), &dest).await.is_err());
    assert!(g.is_repo(&dest).await);
}

#[tokio::test]
async fn commit_paths_takes_exactly_the_given_files() {
    let d = tempfile::tempdir().unwrap();
    let g = init_repo(d.path()).await;
    std::fs::write(d.path().join("keep.txt"), "k\n").unwrap();
    g.run(d.path(), &["add", "keep.txt"]).await.unwrap();
    g.run(d.path(), &["commit", "--quiet", "-m", "keep"])
        .await
        .unwrap();
    // changed, new, deleted: all proposed; `other.txt` is staged but not.
    std::fs::write(d.path().join("a.txt"), "two\n").unwrap();
    std::fs::write(d.path().join("new.txt"), "n\n").unwrap();
    std::fs::remove_file(d.path().join("keep.txt")).unwrap();
    std::fs::write(d.path().join("other.txt"), "o\n").unwrap();
    g.run(d.path(), &["add", "other.txt"]).await.unwrap();

    let files = ["a.txt", "new.txt", "keep.txt"].map(String::from);
    let sha = g
        .commit_paths(d.path(), &files, "Subject\n\nBody line")
        .await
        .unwrap();
    assert_eq!(g.rev_parse(d.path(), "HEAD").await.unwrap(), sha);
    let mut got = g.commit_files(d.path(), &sha).await.unwrap();
    got.sort();
    assert_eq!(got, ["a.txt", "keep.txt", "new.txt"]);
    let msg = g
        .run(d.path(), &["log", "-1", "--format=%B"])
        .await
        .unwrap();
    assert_eq!(msg, "Subject\n\nBody line");
    // The unrelated staged file is still staged, not committed.
    let staged = g
        .run(d.path(), &["diff", "--cached", "--name-only"])
        .await
        .unwrap();
    assert_eq!(staged, "other.txt");

    // A failing hook is an error carrying its output; nothing is committed.
    let hook = d.path().join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho 'nope: formatter' >&2\nexit 1\n").unwrap();
    std::process::Command::new("chmod")
        .arg("+x")
        .arg(&hook)
        .status()
        .unwrap();
    std::fs::write(d.path().join("a.txt"), "three\n").unwrap();
    let e = g
        .commit_paths(d.path(), &["a.txt".to_string()], "x")
        .await
        .unwrap_err();
    assert!(e.to_string().contains("nope: formatter"), "{e}");
    assert_eq!(g.rev_parse(d.path(), "HEAD").await.unwrap(), sha);
}
