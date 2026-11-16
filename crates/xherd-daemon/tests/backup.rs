//! Backup, verify and restore with real git (R-1401, R-1403, R-1404): an agent
//! with a commit, an uncommitted change and an untracked file is restored on
//! "another machine" (new data root and workspace; the repository is cloned
//! again from its remote).

use std::path::Path;

use xherd_core::paths::Paths;
use xherd_core::{Job, Status};
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
    let mut a = Job::new("ag1", 1);
    a.status = Status::Running;
    a.workdir = Some(wt1.to_string_lossy().into_owned());
    a.repo = Some(repo1.to_string_lossy().into_owned());
    a.base = Some("origin/main".into());
    store.put_job(&a).await.unwrap();

    // A saved agent (issue #141): two versions, a memory and a skill in the
    // library. They live in the database snapshot and `data/skills`.
    let mut def = xherd_core::agent_def::AgentDef {
        id: "ad1".into(),
        name: "reviewer".into(),
        description: String::new(),
        version: 1,
        runtime: "claude-code".into(),
        models: Default::default(),
        workflow: xherd_core::agent_def::Workflow {
            pack: "xherd".into(),
            flow: "feature".into(),
        },
        autonomy: None,
        permission_mode: None,
        instructions: Some("Be brief.".into()),
        repo: None,
        base_branch: None,
        job_token_budget: None,
        sensitive_paths: vec![],
        max_diff_lines: None,
        stage_overrides: Default::default(),
        skills: vec!["lint".into()],
        max_live_jobs: 1,
        created_at: 1,
        updated_at: 10,
        archived_at: None,
    };
    store.put_agent_def(&def).await.unwrap();
    def.version = 2;
    def.updated_at = 20;
    store.put_agent_def(&def).await.unwrap();
    store
        .put_agent_memory("ad1", "Use cargo nextest.", 30)
        .await
        .unwrap();
    std::fs::create_dir_all(p1.skills().join("lint")).unwrap();
    std::fs::write(p1.skills().join("lint/SKILL.md"), "---\nname: lint\n---\n").unwrap();

    let dest = t.join("out");
    let made = backup::create(&p1, &ws1, &store, &g, &dest, "manual", 1_000_000)
        .await
        .unwrap();
    assert_eq!(made.jobs, 1, "{:?}", made.warnings);
    assert!(made.path.is_file());
    // Nothing is left beside the archive.
    assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 1);

    let m = backup::verify(&made.path, &p1.scratch(), &g, 1)
        .await
        .unwrap();
    assert!(m.jobs[0].bundle && m.jobs[0].patch);
    assert!(!m.files.keys().any(|k| k.contains("secrets")));
    assert!(m.files.keys().any(|k| k.starts_with("jobs/ag1/")));
    assert!(!m.files.keys().any(|k| k.starts_with("agents/")));

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
    assert_eq!(r.jobs, ["ag1"]);
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
    let back = s2.get_job(&a.id).await.unwrap().unwrap();
    assert_eq!(back.status, Status::Paused);
    assert_eq!(back.workdir.as_deref(), Some(wt2.to_str().unwrap()));
    assert_eq!(
        back.repo.as_deref(),
        Some(ws2.join("repos/r").to_str().unwrap())
    );
    let ad = s2.get_agent_def("ad1").await.unwrap().unwrap();
    assert_eq!((ad.name.as_str(), ad.version), ("reviewer", 2));
    assert_eq!(s2.agent_versions("ad1").await.unwrap().len(), 2);
    assert_eq!(
        s2.agent_memory("ad1").await.unwrap().map(|m| m.0),
        Some("Use cargo nextest.".into())
    );
    assert!(p2.skills().join("lint/SKILL.md").is_file());
    let cfg: serde_json::Value =
        serde_json::from_slice(&std::fs::read(p2.config()).unwrap()).unwrap();
    assert!(cfg.get("userToken").is_none());
    assert_eq!(cfg["workspace"], ws2.to_string_lossy().as_ref());
    assert!(!p2.secrets().exists());

    // A second restore over a database needs --force.
    let again = backup::restore(&p2, &g, &made.path, &opts, 3).await;
    assert!(again.unwrap_err().contains("--force"));

    // An archive made before the rename (`agents/<name>`, manifest key `agents`).
    let old = legacy_archive(&made.path, &t.join("legacy"));
    let ws3 = t.join("ws3");
    let p3 = Paths::at(t.join("data3"));
    let opts3 = RestoreOptions {
        workspace: Some(ws3.to_string_lossy().into_owned()),
        ..RestoreOptions::default()
    };
    let m3 = backup::verify(&old, &p3.scratch(), &g, 4).await.unwrap();
    assert_eq!(m3.jobs.len(), 1);
    let r3 = backup::restore(&p3, &g, &old, &opts3, 5).await.unwrap();
    assert_eq!(r3.jobs, ["ag1"]);
    assert!(r3.warnings.is_empty(), "{:?}", r3.warnings);
    let wt3 = ws3.join("worktrees/r/ag1");
    assert_eq!(git(&g, &wt3, &["log", "-1", "--format=%s"]).await, "work");
    assert_eq!(
        std::fs::read_to_string(wt3.join(".xherd/spec/spec.md")).unwrap(),
        "# spec\n"
    );
}

/// Repack `archive` the way xherd made them before jobs: `agents/<name>` folders and
/// `agents` in the manifest.
fn legacy_archive(archive: &Path, work: &Path) -> std::path::PathBuf {
    let dir = work.join("tree");
    std::fs::create_dir_all(&dir).unwrap();
    let dec = zstd::Decoder::new(std::fs::File::open(archive).unwrap()).unwrap();
    tar::Archive::new(dec).unpack(&dir).unwrap();
    std::fs::rename(dir.join("jobs"), dir.join("agents")).unwrap();
    let mp = dir.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&mp).unwrap()).unwrap();
    let o = m.as_object_mut().unwrap();
    let jobs = o.remove("jobs").unwrap();
    o.insert("agents".into(), jobs);
    let files = o["files"].as_object().unwrap().clone();
    let renamed: serde_json::Map<String, serde_json::Value> = files
        .into_iter()
        .map(|(k, v)| (k.replacen("jobs/", "agents/", 1), v))
        .collect();
    o.insert("files".into(), renamed.into());
    std::fs::write(&mp, serde_json::to_vec(&m).unwrap()).unwrap();
    let out = work.join("old.tar.zst");
    let enc = zstd::Encoder::new(std::fs::File::create(&out).unwrap(), 3).unwrap();
    let mut b = tar::Builder::new(enc);
    b.append_dir_all(".", &dir).unwrap();
    b.into_inner().unwrap().finish().unwrap();
    out
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
