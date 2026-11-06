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
