//! Backup, verify and restore with real git (R-1401, R-1403, R-1404): an agent
//! with a commit, an uncommitted change and an untracked file is restored on
//! "another machine" (new data root and workspace; the repository is cloned
//! again from its remote).

use std::path::Path;

use xherd_core::paths::Paths;
use xherd_core::{Agent, Status};
use xherd_daemon::backup::{self, RestoreOptions};
use xherd_git::Git;
use xherd_store::Store;

async fn git(g: &Git, dir: &Path, args: &[&str]) -> String {
    g.run(dir, args)
        .await
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"))
}

#[tokio::test]
async fn backup_and_restore_on_another_machine() {
    let tmp = tempfile::tempdir().unwrap();
    let t = tmp.path();
    let g = Git::new("git");

    // A remote with one commit on main, a clone of it, and an agent worktree.
    let origin = t.join("origin.git");
    std::fs::create_dir_all(&origin).unwrap();
    git(&g, &origin, &["init", "--bare", "--quiet", "-b", "main"]).await;
    let ws1 = t.join("ws1");
    let repo1 = ws1.join("repos/r");
    std::fs::create_dir_all(repo1.parent().unwrap()).unwrap();
    g.clone_repo(&origin.to_string_lossy(), &repo1)
        .await
        .unwrap();
    for (k, v) in [("user.name", "t"), ("user.email", "t@example.com")] {
        git(&g, &repo1, &["config", k, v]).await;
    }
    git(&g, &repo1, &["checkout", "--quiet", "-b", "main"]).await;
    std::fs::write(repo1.join("README"), "one\n").unwrap();
    git(&g, &repo1, &["add", "."]).await;
    git(&g, &repo1, &["commit", "--quiet", "-m", "init"]).await;
    git(&g, &repo1, &["push", "--quiet", "origin", "main"]).await;
    git(&g, &repo1, &["fetch", "--quiet", "origin"]).await;
    let wt1 = ws1.join("worktrees/r/ag1");
    g.worktree_add(&repo1, &wt1, "ag1", "origin/main")
        .await
        .unwrap();
    std::fs::write(wt1.join("c.txt"), "committed\n").unwrap();
    git(&g, &wt1, &["add", "c.txt"]).await;
    git(&g, &wt1, &["commit", "--quiet", "-m", "work"]).await;
    g.add_excludes(&wt1, &[".xherd/"]).await.unwrap();
    std::fs::write(wt1.join("README"), "one\nedited\n").unwrap();
    std::fs::write(wt1.join("u.txt"), "untracked\n").unwrap();
    std::fs::create_dir_all(wt1.join(".xherd/spec")).unwrap();
    std::fs::write(wt1.join(".xherd/spec/spec.md"), "# spec\n").unwrap();
    std::fs::write(wt1.join(".xherd/agent.json"), "{}").unwrap();

    // The first machine's data root.
    let p1 = Paths::at(t.join("data1"));
    p1.ensure().unwrap();
    std::fs::write(
        p1.config(),
        format!(
            r#"{{"userToken":"secret","workspace":"{}"}}"#,
            ws1.display()
        ),
    )
    .unwrap();
    std::fs::write(p1.secrets(), r#"{"git":{"token":"nope"}}"#).unwrap();
    let store = Store::open(&p1.db(), &p1.backups()).unwrap();
    let mut a = Agent::new("ag1", 1);
    a.status = Status::Running;
    a.workdir = Some(wt1.to_string_lossy().into_owned());
    a.repo = Some(repo1.to_string_lossy().into_owned());
    a.base = Some("origin/main".into());
    store.put_agent(&a).await.unwrap();

    let dest = t.join("out");
    let made = backup::create(&p1, &ws1, &store, &g, &dest, "manual", 1_000_000)
        .await
        .unwrap();
    assert_eq!(made.agents, 1, "{:?}", made.warnings);
    assert!(made.path.is_file());
    // Nothing is left beside the archive.
    assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 1);

    let m = backup::verify(&made.path, &p1.scratch(), &g, 1)
        .await
        .unwrap();
    assert!(m.agents[0].bundle && m.agents[0].patch);
    assert!(!m.files.keys().any(|k| k.contains("secrets")));

    // The second machine: no repository, another workspace.
    let ws2 = t.join("ws2");
    let p2 = Paths::at(t.join("data2"));
    let opts = RestoreOptions {
        workspace: Some(ws2.to_string_lossy().into_owned()),
        ..RestoreOptions::default()
    };
    let r = backup::restore(&p2, &g, &made.path, &opts, 2)
        .await
        .unwrap();
    assert_eq!(r.agents, ["ag1"]);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);

    let wt2 = ws2.join("worktrees/r/ag1");
    assert_eq!(git(&g, &wt2, &["log", "-1", "--format=%s"]).await, "work");
    assert_eq!(
        std::fs::read_to_string(wt2.join("README")).unwrap(),
        "one\nedited\n"
    );
    assert_eq!(
        std::fs::read_to_string(wt2.join("u.txt")).unwrap(),
        "untracked\n"
    );
    assert_eq!(
        std::fs::read_to_string(wt2.join(".xherd/spec/spec.md")).unwrap(),
        "# spec\n"
    );
    assert!(
        !wt2.join(".xherd/agent.json").exists(),
        "xherd writes it again at the next start"
    );

    let s2 = Store::open(&p2.db(), &p2.backups()).unwrap();
    let back = s2.get_agent(&a.id).await.unwrap().unwrap();
    assert_eq!(back.status, Status::Paused);
    assert_eq!(back.workdir.as_deref(), Some(wt2.to_str().unwrap()));
    assert_eq!(
        back.repo.as_deref(),
        Some(ws2.join("repos/r").to_str().unwrap())
    );
    let cfg: serde_json::Value =
        serde_json::from_slice(&std::fs::read(p2.config()).unwrap()).unwrap();
    assert!(cfg.get("userToken").is_none());
    assert_eq!(cfg["workspace"], ws2.to_string_lossy().as_ref());
    assert!(!p2.secrets().exists());

    // A second restore over a database needs --force.
    let again = backup::restore(&p2, &g, &made.path, &opts, 3).await;
    assert!(again.unwrap_err().contains("--force"));
}

#[tokio::test]
async fn a_damaged_archive_does_not_verify() {
    let tmp = tempfile::tempdir().unwrap();
    let p = Paths::at(tmp.path().join("d"));
    p.ensure().unwrap();
    let store = Store::open(&p.db(), &p.backups()).unwrap();
    let g = Git::new("git");
    let made = backup::create(
        &p,
        tmp.path(),
        &store,
        &g,
        &tmp.path().join("out"),
        "manual",
        5_000,
    )
    .await
    .unwrap();
    backup::verify(&made.path, &p.scratch(), &g, 1)
        .await
        .unwrap();

    let mut bytes = std::fs::read(&made.path).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff;
    std::fs::write(&made.path, bytes).unwrap();
    assert!(
        backup::verify(&made.path, &p.scratch(), &g, 2)
            .await
            .is_err()
    );
}

#[test]
fn only_the_last_update_backups_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 1..=5 {
        std::fs::write(
            tmp.path().join(format!("xherd-update-{i:010}.tar.zst")),
            b"x",
        )
        .unwrap();
    }
    std::fs::write(tmp.path().join("xherd-manual-0000000001.tar.zst"), b"x").unwrap();
    backup::prune(tmp.path(), "update", 3);
    let names: Vec<String> = backup::list(tmp.path()).into_iter().map(|e| e.0).collect();
    assert_eq!(names.len(), 4);
    assert!(names.contains(&"xherd-update-0000000005.tar.zst".to_string()));
    assert!(!names.contains(&"xherd-update-0000000002.tar.zst".to_string()));
    assert!(names.contains(&"xherd-manual-0000000001.tar.zst".to_string()));
}
