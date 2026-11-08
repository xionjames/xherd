//! R-1204 end to end: a worktree with the installed guard pushes its own
//! branch and nothing else. Real git, real hook, the built `xherd` binary.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> (bool, String) {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    (o.status.success(), text)
}

#[tokio::test]
async fn an_agent_pushes_only_its_own_branch_as_a_fast_forward() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let remote = root.join("remote.git");
    let repo = root.join("repo");
    let wt = root.join("wt");
    std::fs::create_dir_all(&remote).unwrap();
    git(&remote, &["init", "-q", "--bare", "-b", "main"]);
    git(&root, &["clone", "-q", remote.to_str().unwrap(), "repo"]);
    git(&repo, &["checkout", "-q", "-b", "main"]);
    // The repository's own hooks must keep running in the worktree.
    let hooks = repo.join(".git/hooks");
    std::fs::write(
        hooks.join("pre-commit"),
        "#!/bin/sh\necho ran > \"$PWD/pre-commit-ran\"\n",
    )
    .unwrap();
    Command::new("chmod")
        .args(["+x", hooks.join("pre-commit").to_str().unwrap()])
        .status()
        .unwrap();
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    assert!(git(&repo, &["push", "-q", "origin", "main"]).0);
    assert!(
        git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "--no-track",
                "-b",
                "feat",
                wt.to_str().unwrap(),
                "main"
            ]
        )
        .0
    );

    let g = xherd_git::Git::default();
    let bin = env!("CARGO_BIN_EXE_xherd");
    g.install_push_guard(&wt, &wt.join(".xherd/hooks"), bin, "feat")
        .await
        .unwrap();
    // Installing twice changes nothing.
    g.install_push_guard(&wt, &wt.join(".xherd/hooks"), bin, "feat")
        .await
        .unwrap();

    git(&wt, &["commit", "-q", "--allow-empty", "-m", "one"]);
    assert!(
        wt.join("pre-commit-ran").exists(),
        "the repo's pre-commit still runs"
    );

    let (ok, out) = git(&wt, &["push", "-q", "origin", "feat"]);
    assert!(ok, "own branch: {out}");
    git(&wt, &["commit", "-q", "--allow-empty", "-m", "two"]);
    let (ok, out) = git(&wt, &["push", "-q", "origin", "feat"]);
    assert!(ok, "fast-forward: {out}");

    let (ok, out) = git(&wt, &["push", "-q", "origin", "feat:main"]);
    assert!(!ok && out.contains("may push only feat"), "{out}");
    let (ok, out) = git(&wt, &["push", "-q", "origin", "HEAD:refs/tags/v1"]);
    assert!(!ok && out.contains("refs/tags/v1"), "{out}");
    let (ok, out) = git(&wt, &["push", "-q", "origin", ":feat"]);
    assert!(!ok && out.contains("deleting"), "{out}");
    // Rewrite history, then force.
    git(&wt, &["reset", "-q", "--hard", "HEAD~1"]);
    git(&wt, &["commit", "-q", "--allow-empty", "-m", "other two"]);
    let (ok, out) = git(&wt, &["push", "-q", "--force", "origin", "feat"]);
    assert!(!ok && out.contains("fast-forward"), "{out}");
    // The main checkout has no guard.
    assert!(git(&repo, &["push", "-q", "origin", "HEAD:refs/heads/other"]).0);
}
