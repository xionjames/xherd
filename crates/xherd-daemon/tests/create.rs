//! `Daemon::new_job` against a real temp git repo (R-001..R-005c).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use xherd_core::new::NewJob;
use xherd_core::paths::Paths;
use xherd_core::{Job, Status};
use xherd_daemon::*;

struct T {
    home: tempfile::TempDir,
    ws: tempfile::TempDir,
    repo: tempfile::TempDir,
    d: Daemon,
    fake: Arc<FakeDriver>,
}

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap()
}

async fn setup() -> T {
    setup_with(|_| {}).await
}

async fn setup_with(f: impl FnOnce(&mut Options)) -> T {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    assert!(
        git(repo.path(), &["init", "-q", "-b", "main"])
            .status
            .success()
    );
    assert!(
        git(
            repo.path(),
            &["commit", "-q", "--allow-empty", "-m", "base"]
        )
        .status
        .success()
    );
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    let cfg = xherd_core::config::Config {
        workspace: Some(ws.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    cfg.save(&paths.config()).unwrap();
    let fake = Arc::new(FakeDriver::default());
    let mut o = Options::new(fake.clone(), Arc::new(SystemClock));
    // Never the real Claude Code config: a fixture that trusts the workspace.
    let trust = home.path().join("claude.json");
    std::fs::write(&trust, trusting(ws.path())).unwrap();
    let rt = o
        .builtin_plugins
        .iter_mut()
        .find(|m| m.name == "claude-code")
        .unwrap();
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    f(&mut o);
    let d = Daemon::start(paths, o).await.unwrap();
    T {
        home,
        ws,
        repo,
        d,
        fake,
    }
}

fn trusting(dir: &Path) -> String {
    serde_json::json!({"projects": {dir.to_string_lossy(): {"hasTrustDialogAccepted": true}}})
        .to_string()
}

/// An agent named after its ticket key, lower-cased: its branch is the key.
fn req(t: &T, key: &str) -> NewJob {
    NewJob {
        name: key.to_ascii_lowercase(),
        ticket: Some(key.into()),
        ticket_key: None,
        slug: None,
        repo: t.repo.path().to_string_lossy().into_owned(),
        agent: None,
        agents: Default::default(),
        base: None,
        flow: None,
        prompt: Some("add login".into()),
        autonomy: None,
        permission_mode: None,
        runtime: None,
        model: None,
    }
}

fn repo_name(t: &T) -> String {
    t.repo
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[tokio::test]
async fn creates_worktree_and_private_folder() {
    let t = setup().await;
    let mut r = req(&t, "sc-1");
    r.name = "login".into();
    let a: Job = t.d.new_job(r).await.unwrap();
    assert_eq!(
        (a.name.as_str(), a.ticket_key(), a.branch.as_str()),
        ("login", Some("SC-1"), "SC-1-login")
    );
    assert_eq!(
        (a.pack.as_deref(), a.flow.as_deref()),
        (Some("xherd"), Some("feature"))
    );
    assert_eq!(
        a.next_prompt.as_deref(),
        Some("/xherd:feature SC-1 add login")
    );
    assert_eq!(a.base.as_deref(), Some("main"));

    let wt =
        t.ws.path()
            .join("worktrees")
            .join(repo_name(&t))
            .join("login");
    assert_eq!(a.workdir.as_deref(), Some(wt.to_string_lossy().as_ref()));
    let head = git(&wt, &["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(String::from_utf8_lossy(&head.stdout).trim(), "SC-1-login");
    assert!(
        !git(&wt, &["rev-parse", "--abbrev-ref", "@{u}"])
            .status
            .success(),
        "no upstream (R-004)"
    );

    let j: serde_json::Value =
        serde_json::from_slice(&std::fs::read(wt.join(".xherd/job.json")).unwrap()).unwrap();
    assert_eq!(
        (j["id"].as_str(), j["ticket"]["key"].as_str()),
        (Some(a.id.as_str()), Some("SC-1"))
    );
    assert!(
        std::fs::read_to_string(wt.join(".xherd/instructions.md"))
            .unwrap()
            .contains("SC-1")
    );
    assert!(wt.join(".xherd/spec").is_dir());
    let status = git(&wt, &["status", "--porcelain", "--untracked-files=all"]);
    assert!(
        status.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&status.stdout)
    );
}

#[tokio::test]
async fn refusals_leave_nothing_behind() {
    let t = setup().await;
    let code = |r: Result<Job, ActionError>| r.unwrap_err().code;

    let mut r = req(&t, "SC-2");
    r.base = Some("nope".into());
    assert_eq!(code(t.d.new_job(r).await), "base_unresolved");
    assert!(
        !t.ws
            .path()
            .join("worktrees")
            .join(repo_name(&t))
            .join("sc-2")
            .exists()
    );
    assert!(
        !git(
            t.repo.path(),
            &["show-ref", "--verify", "--quiet", "refs/heads/SC-2"]
        )
        .status
        .success()
    );

    let mut r = req(&t, "SC-2");
    r.flow = Some("bug".into());
    assert_eq!(code(t.d.new_job(r).await), "unknown_flow");
    let mut r = req(&t, "SC-2");
    r.repo = t.ws.path().to_string_lossy().into_owned();
    assert_eq!(code(t.d.new_job(r).await), "not_a_repo");

    t.d.new_job(req(&t, "SC-2")).await.unwrap();
    assert_eq!(code(t.d.new_job(req(&t, "SC-2")).await), "name_taken");

    assert!(git(t.repo.path(), &["branch", "SC-3"]).status.success());
    assert_eq!(code(t.d.new_job(req(&t, "SC-3")).await), "branch_exists");
}

#[tokio::test]
async fn a_bugfix_agent_starts_with_the_bugfix_command() {
    let t = setup().await;
    let mut r = req(&t, "SC-5");
    r.flow = Some("bugfix".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(
        (a.pack.as_deref(), a.flow.as_deref()),
        (Some("xherd"), Some("bugfix"))
    );
    assert_eq!(
        a.next_prompt.as_deref(),
        Some("/xherd:bugfix SC-5 add login")
    );
}

#[tokio::test]
async fn flow_none_and_origin_base() {
    let t = setup().await;
    // A clone whose origin/HEAD points at main.
    let clone = tempfile::tempdir().unwrap();
    let src = t.repo.path().to_string_lossy().into_owned();
    assert!(
        git(clone.path(), &["clone", "-q", &src, "c"])
            .status
            .success()
    );
    let c = clone.path().join("c");
    assert!(
        git(&c, &["checkout", "-q", "-b", "local-work"])
            .status
            .success()
    );
    let mut r = req(&t, "SC-4");
    r.repo = c.to_string_lossy().into_owned();
    r.flow = Some("none".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("origin/main"));
    assert_eq!((a.pack, a.flow), (None, None));
    assert_eq!(a.next_prompt.as_deref(), Some("add login"));
}

/// A repository given as a path gets its `origin` fetched before the job (R-002).
#[tokio::test]
async fn path_repo_fetches_origin() {
    let t = setup().await;
    let clone = tempfile::tempdir().unwrap();
    let src = t.repo.path().to_string_lossy().into_owned();
    assert!(
        git(clone.path(), &["clone", "-q", &src, "c"])
            .status
            .success()
    );
    let c = clone.path().join("c");
    // The remote moves on after the clone.
    assert!(
        git(
            t.repo.path(),
            &["commit", "-q", "--allow-empty", "-m", "newer"]
        )
        .status
        .success()
    );
    let newest = String::from_utf8(git(t.repo.path(), &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string();

    let mut r = req(&t, "SC-8");
    r.repo = c.to_string_lossy().into_owned();
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("origin/main"));
    let wt = t.ws.path().join("worktrees/c/sc-8");
    let head = String::from_utf8(git(&wt, &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(head, newest);
}

/// A failed fetch never refuses: resolve warns, the job is created.
#[tokio::test]
async fn path_repo_fetch_failure_is_a_hint() {
    let t = setup().await;
    let clone = tempfile::tempdir().unwrap();
    let src = t.repo.path().to_string_lossy().into_owned();
    assert!(
        git(clone.path(), &["clone", "-q", &src, "c"])
            .status
            .success()
    );
    let c = clone.path().join("c");
    let gone = clone.path().join("gone");
    assert!(
        git(
            &c,
            &["remote", "set-url", "origin", &gone.to_string_lossy()]
        )
        .status
        .success()
    );
    let v = t.d.resolve_repo(&c.to_string_lossy()).await.unwrap();
    assert!(
        v["warning"]
            .as_str()
            .unwrap()
            .starts_with("could not fetch origin")
    );
    let mut r = req(&t, "SC-9");
    r.repo = c.to_string_lossy().into_owned();
    assert!(t.d.new_job(r).await.is_ok());
}

/// No `origin`: nothing to fetch, no warning.
#[tokio::test]
async fn path_repo_without_remote_is_quiet() {
    let t = setup().await;
    let v =
        t.d.resolve_repo(&t.repo.path().to_string_lossy())
            .await
            .unwrap();
    assert!(v["warning"].is_null());
    let a = t.d.new_job(req(&t, "SC-10")).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("main"));
}

fn rev(dir: &Path, r: &str) -> String {
    String::from_utf8(git(dir, &["rev-parse", r]).stdout)
        .unwrap()
        .trim()
        .to_string()
}

/// A clone of the test repo (kept alive by the returned dir), after which
/// the remote gets one more commit that adds `new.txt`.
fn clone_then_remote_moves(t: &T) -> (tempfile::TempDir, PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let src = t.repo.path().to_string_lossy().into_owned();
    assert!(
        git(dir.path(), &["clone", "-q", &src, "c"])
            .status
            .success()
    );
    let c = dir.path().join("c");
    std::fs::write(t.repo.path().join("new.txt"), "remote\n").unwrap();
    assert!(git(t.repo.path(), &["add", "new.txt"]).status.success());
    assert!(
        git(t.repo.path(), &["commit", "-q", "-m", "newer"])
            .status
            .success()
    );
    let newest = rev(t.repo.path(), "HEAD");
    (dir, c, newest)
}

/// A plain local base branch that is behind `origin` is fast-forwarded (R-002).
#[tokio::test]
async fn local_base_behind_is_fast_forwarded() {
    let t = setup().await;
    let (_dir, c, newest) = clone_then_remote_moves(&t);
    // `main` is not checked out in the clone.
    assert!(git(&c, &["checkout", "-q", "-b", "other"]).status.success());
    let mut r = req(&t, "SC-11");
    r.repo = c.to_string_lossy().into_owned();
    r.base = Some("main".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("main"));
    assert_eq!(rev(&c, "main"), newest);
    assert_eq!(rev(&t.ws.path().join("worktrees/c/sc-11"), "HEAD"), newest);
}

#[tokio::test]
async fn checked_out_local_base_behind_is_fast_forwarded() {
    let t = setup().await;
    let (_dir, c, newest) = clone_then_remote_moves(&t);
    let mut r = req(&t, "SC-12");
    r.repo = c.to_string_lossy().into_owned();
    r.base = Some("main".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("main"));
    assert_eq!(rev(&c, "HEAD"), newest);
    assert!(c.join("new.txt").exists());
    assert_eq!(rev(&t.ws.path().join("worktrees/c/sc-12"), "HEAD"), newest);
}

/// Diverged: never reset, rebase or merge; the job starts from the local branch.
#[tokio::test]
async fn diverged_local_base_is_left_alone() {
    let t = setup().await;
    let (_dir, c, newest) = clone_then_remote_moves(&t);
    assert!(
        git(&c, &["commit", "-q", "--allow-empty", "-m", "unpushed"])
            .status
            .success()
    );
    let local = rev(&c, "main");
    let mut r = req(&t, "SC-13");
    r.repo = c.to_string_lossy().into_owned();
    r.base = Some("main".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("main"));
    assert_eq!(rev(&c, "main"), local);
    assert_ne!(local, newest);
    assert_eq!(rev(&t.ws.path().join("worktrees/c/sc-13"), "HEAD"), local);
}

/// A dirty file the fast-forward would overwrite: nothing changes, the job is created.
#[tokio::test]
async fn dirty_checked_out_base_is_left_alone() {
    let t = setup().await;
    let (_dir, c, newest) = clone_then_remote_moves(&t);
    let local = rev(&c, "main");
    std::fs::write(c.join("new.txt"), "mine\n").unwrap();
    let mut r = req(&t, "SC-14");
    r.repo = c.to_string_lossy().into_owned();
    r.base = Some("main".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("main"));
    assert_eq!(rev(&c, "main"), local);
    assert_eq!(
        std::fs::read_to_string(c.join("new.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(rev(&c, "origin/main"), newest);
}

/// A remote base leaves the local branch alone.
#[tokio::test]
async fn remote_base_leaves_local_branch() {
    let t = setup().await;
    let (_dir, c, newest) = clone_then_remote_moves(&t);
    let local = rev(&c, "main");
    let mut r = req(&t, "SC-15");
    r.repo = c.to_string_lossy().into_owned();
    r.base = Some("origin/main".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("origin/main"));
    assert_eq!(rev(&c, "main"), local);
    assert_eq!(rev(&t.ws.path().join("worktrees/c/sc-15"), "HEAD"), newest);
}

#[tokio::test]
async fn starts_claude_code_with_hooks() {
    let t = setup().await;
    let wt =
        t.ws.path()
            .join("worktrees")
            .join(repo_name(&t))
            .join("sc-5");
    let a = t.d.new_job(req(&t, "SC-5")).await.unwrap();
    // The default mode (accept edits) is passed explicitly, never the runtime's own default.
    assert_eq!(
        a.launch.as_deref().map(|l| l.join(" ")).as_deref(),
        Some("claude --permission-mode acceptEdits")
    );
    let settings: serde_json::Value =
        serde_json::from_slice(&std::fs::read(wt.join(".claude/settings.local.json")).unwrap())
            .unwrap();
    assert_eq!(
        settings["hooks"]["Stop"][0]["hooks"][0]["command"],
        "'xherd' hook stop # xherd"
    );
    // The XAP calls go through plain `xherd`, allowed by name (a variable in
    // the command would make the runtime ask every time).
    let allow = settings["permissions"]["allow"].as_array().unwrap();
    for rule in [
        "Bash(xherd stage *)",
        "Bash(xherd gate *)",
        "Bash(xherd commit *)",
    ] {
        assert!(allow.iter().any(|r| r == rule), "{rule} in {allow:?}");
    }
    assert!(
        allow
            .iter()
            .any(|r| r.as_str().is_some_and(|r| r.starts_with("Read(//")))
    );
    let status = git(&wt, &["status", "--porcelain", "--untracked-files=all"]);
    assert!(status.stdout.is_empty(), "hook file is excluded from git");
    for _ in 0..200 {
        let a = t.d.action("SC-5", UserAction::Get).await.unwrap();
        if a.status == Status::Running {
            assert!(
                t.fake
                    .calls()
                    .contains(&"start sc-5 claude --permission-mode acceptEdits".to_string())
            );
            let ev = t.d.store.events(&a.id, 10).await.unwrap();
            assert!(ev.iter().any(|e| e.kind == "session-started"));
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("agent never started");
}

/// The tool-permission mode (R-002a): from the request, `all` refused
/// outside the container, and changed later for the next start.
#[tokio::test]
async fn permission_mode_from_the_request_and_changed_later() {
    use xherd_core::permission::PermissionMode;
    let t = setup().await;
    let mut r = req(&t, "SC-8");
    r.permission_mode = Some(PermissionMode::All);
    assert_eq!(t.d.new_job(r).await.unwrap_err().code, "mode_unavailable");
    let mut r = req(&t, "SC-8");
    r.permission_mode = Some(PermissionMode::Ask);
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.permission_mode, Some(PermissionMode::Ask));
    assert_eq!(
        a.launch.as_deref().map(|l| l.join(" ")).as_deref(),
        Some("claude --permission-mode manual")
    );
    let a =
        t.d.set_permission_mode("SC-8", PermissionMode::Auto)
            .await
            .unwrap();
    assert_eq!(a.permission_mode, Some(PermissionMode::Auto));
    assert_eq!(
        a.launch.as_deref().map(|l| l.join(" ")).as_deref(),
        Some("claude --permission-mode auto")
    );
    let e =
        t.d.set_permission_mode("SC-8", PermissionMode::All)
            .await
            .unwrap_err();
    assert_eq!(e.code, "mode_unavailable");
}

/// The model (issue #112): request, then the profile's entry for the
/// runtime, then Settings' `runtimeModels`; it goes on the launch line and
/// can be changed (or cleared) later, keeping the permission mode.
#[tokio::test]
async fn the_model_comes_from_the_request_then_the_profile_then_settings() {
    let t = setup().await;
    let line = |a: &Job| a.launch.as_deref().map(|l| l.join(" ")).unwrap();
    let a = t.d.new_job(req(&t, "SC-31")).await.unwrap();
    assert_eq!(a.model, None);
    assert_eq!(line(&a), "claude --permission-mode acceptEdits");

    let saved = t.d.save_config(
        serde_json::from_value(
            serde_json::json!({"runtimeModels": {"claude-code": "haiku", "opencode": ""}}),
        )
        .unwrap(),
    );
    assert!(saved.is_ok(), "{:?}", saved.err().map(|e| e.message));
    let a = t.d.new_job(req(&t, "SC-32")).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("haiku"), "setting");
    assert_eq!(
        line(&a),
        "claude --permission-mode acceptEdits --model haiku"
    );

    let p: xherd_core::profile::Profile = serde_json::from_value(serde_json::json!({
        "name": "app",
        "match": [t.repo.path().canonicalize().unwrap().to_string_lossy()],
        "models": {"claude-code": "sonnet", "opencode": "opencode/x"}
    }))
    .unwrap();
    t.d.create_profile(p).await.unwrap();
    let a = t.d.new_job(req(&t, "SC-33")).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("sonnet"), "profile");

    let mut r = req(&t, "SC-34");
    r.model = Some(" opus ".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("opus"), "request");

    let mut r = req(&t, "SC-35");
    r.model = Some("--evil".into());
    assert_eq!(t.d.new_job(r).await.unwrap_err().code, "bad_model");

    use xherd_core::permission::PermissionMode;
    t.d.set_permission_mode("SC-34", PermissionMode::Ask)
        .await
        .unwrap();
    let a = t.d.set_model("SC-34", Some("haiku")).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("haiku"));
    assert_eq!(
        line(&a),
        "claude --permission-mode manual --model haiku",
        "mode kept"
    );
    let a =
        t.d.set_permission_mode("SC-34", PermissionMode::Auto)
            .await
            .unwrap();
    assert_eq!(
        line(&a),
        "claude --permission-mode auto --model haiku",
        "model kept"
    );
    let a = t.d.set_model("SC-34", Some("")).await.unwrap();
    assert_eq!(a.model, None);
    assert_eq!(line(&a), "claude --permission-mode auto");
    assert_eq!(
        t.d.set_model("SC-34", Some("a b")).await.unwrap_err().code,
        "bad_model"
    );

    let bad = t.d.save_config(
        serde_json::from_value(serde_json::json!({"runtimeModels": {"github": "x"}})).unwrap(),
    );
    assert!(bad.unwrap_err().message.contains("runtimeModels"));
}

/// Model suggestions (issue #112): a runtime's fixed list, or what its list
/// command prints; a failing command gives none and says why.
#[tokio::test]
async fn model_suggestions_come_from_the_list_or_the_command() {
    let t = setup_with(|o| {
        let set = |o: &mut Options, name: &str, v: serde_json::Value| {
            let m = o
                .builtin_plugins
                .iter_mut()
                .find(|m| m.name == name)
                .unwrap();
            m.data.get_mut("terminal").unwrap()["models"] = v;
        };
        set(
            o,
            "opencode",
            serde_json::json!({"command": "printf 'a/b\\tA B\\n-x\\nc/d\\n' | grep /"}),
        );
        set(
            o,
            "codex",
            serde_json::json!({"command": "echo nope >&2; exit 3"}),
        );
    })
    .await;
    let fixed = t.d.runtime_models("claude-code").await.unwrap();
    assert!(fixed.choose && fixed.models.iter().any(|m| m.id == "sonnet"));
    let listed = t.d.runtime_models("opencode").await.unwrap();
    let ids: Vec<&str> = listed.models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["a/b", "c/d"]);
    assert_eq!(listed.models[0].label.as_deref(), Some("A B"));
    assert_eq!(listed.error, None);
    let failed = t.d.runtime_models("codex").await.unwrap();
    assert!(failed.choose && failed.models.is_empty());
    assert!(failed.error.unwrap().contains("nope"));
    assert_eq!(
        t.d.runtime_models("github").await.unwrap_err().code,
        "unknown_runtime"
    );
}

/// A worktree with version files starts through `xherd toolchain exec`, with
/// the plan written for it (R-652); one without starts the runtime directly.
#[tokio::test]
async fn version_files_wrap_the_launch_in_the_toolchain_step() {
    let t = setup().await;
    std::fs::write(t.repo.path().join(".nvmrc"), "20\n").unwrap();
    git(t.repo.path(), &["add", ".nvmrc"]);
    assert!(
        git(t.repo.path(), &["commit", "-q", "-m", "node"])
            .status
            .success()
    );
    t.d.new_job(req(&t, "SC-7")).await.unwrap();
    let wt =
        t.ws.path()
            .join("worktrees")
            .join(repo_name(&t))
            .join("sc-7");
    for _ in 0..200 {
        if let Some(c) = t.fake.calls().iter().find(|c| c.starts_with("start sc-7 ")) {
            assert_eq!(
                c,
                "start sc-7 xherd toolchain exec -- claude --permission-mode acceptEdits"
            );
            let plan: serde_json::Value =
                serde_json::from_slice(&std::fs::read(wt.join(".xherd/toolchain.json")).unwrap())
                    .unwrap();
            assert_eq!(plan["install"], false);
            assert_eq!(plan["steps"][0]["tool"], "node");
            assert_eq!(plan["steps"][0]["version"], "20");
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("agent never started");
}

/// Each new worktree is marked trusted in the runtime's file, other entries
/// kept, so no agent stops at the trust dialog (design §4.1).
#[tokio::test]
async fn trusts_each_new_worktree_in_the_runtime_file() {
    let t = setup().await;
    let trust = t.home.path().join("claude.json");
    std::fs::write(
        &trust,
        r#"{"userID": "u", "projects": {"/elsewhere": {"x": 1}}}"#,
    )
    .unwrap();
    let a = t.d.new_job(req(&t, "SC-8")).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&trust).unwrap()).unwrap();
    assert_eq!(v["userID"], "u");
    assert_eq!(v["projects"]["/elsewhere"]["x"], 1);
    let wd = a.workdir.unwrap();
    assert_eq!(v["projects"][&wd]["hasTrustDialogAccepted"], true);
    // The worktree as stored and canonicalised (`/var` → `/private/var`), nothing else.
    assert!(v["projects"].as_object().unwrap().len() <= 3);

    // A trust file that is not JSON is never overwritten: creation refuses.
    std::fs::write(&trust, "{oops").unwrap();
    let e = t.d.new_job(req(&t, "SC-9")).await.unwrap_err();
    assert_eq!(e.code, "runtime_trust");
    assert_eq!(std::fs::read_to_string(&trust).unwrap(), "{oops");
    assert!(
        !git(t.repo.path(), &["rev-parse", "--verify", "SC-9"])
            .status
            .success(),
        "nothing left behind"
    );
}

#[tokio::test]
async fn trust_is_written_only_under_the_worktrees_root() {
    let tmp = tempfile::tempdir().unwrap();
    let w = TrustWriter {
        trust: serde_json::from_value(serde_json::json!({
            "file": tmp.path().join("t.json").to_string_lossy(),
            "projects": "projects", "key": "ok"
        }))
        .unwrap(),
        root: tmp.path().join("worktrees"),
    };
    assert!(w.ensure(tmp.path()).await.is_err());
    assert!(w.ensure(&tmp.path().join("worktrees")).await.is_err());
    assert!(w.ensure(&tmp.path().join("worktrees/r/SC-1")).await.is_ok());
    let text = std::fs::read_to_string(tmp.path().join("t.json")).unwrap();
    assert!(text.contains("worktrees/r/SC-1"), "{text}");
}

#[tokio::test]
async fn refuses_without_a_runtime_plugin() {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path());
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.builtin_plugins.clear();
    let d = Daemon::start(paths, o).await.unwrap();
    // The runtime is chosen after the repository (a profile may name it).
    let repo = home.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    let git = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&repo)
        .status()
        .unwrap();
    assert!(git.success());
    let r = NewJob {
        name: "sc-6".into(),
        ticket: None,
        ticket_key: None,
        slug: None,
        repo: repo.to_string_lossy().into_owned(),
        agent: None,
        agents: Default::default(),
        base: None,
        flow: None,
        prompt: None,
        autonomy: None,
        permission_mode: None,
        runtime: None,
        model: None,
    };
    assert_eq!(d.new_job(r).await.unwrap_err().code, "runtime_unusable");
}

#[tokio::test]
async fn url_uses_and_fetches_xherds_clone() {
    let t = setup().await;
    // xherd's clone of the URL already exists (cloning over the network is
    // covered by xherd-git's tests); its origin is the local test repo.
    let clone = t.ws.path().join("repos/git.example.com/acme/app");
    std::fs::create_dir_all(clone.parent().unwrap()).unwrap();
    let src = t.repo.path().to_string_lossy().into_owned();
    assert!(
        git(
            t.ws.path(),
            &["clone", "-q", &src, &clone.to_string_lossy()]
        )
        .status
        .success()
    );
    git(t.repo.path(), &["branch", "dev"]);

    let clone = std::fs::canonicalize(&clone).unwrap();
    let url = "https://git.example.com/acme/app.git";
    let v = t.d.resolve_repo(url).await.unwrap();
    assert_eq!(v["path"], clone.to_string_lossy().as_ref());
    assert_eq!(
        (v["cloned"].as_bool(), v["warning"].is_null()),
        (Some(false), true)
    );
    assert_eq!(v["base"], "origin/main");
    // Fetched: the branch made after the clone is there.
    assert!(
        v["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b == "origin/dev")
    );

    let mut r = req(&t, "SC-7");
    r.repo = "git@git.example.com:acme/app".into();
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.repo.as_deref(), Some(clone.to_string_lossy().as_ref()));
    assert!(t.ws.path().join("worktrees/app/sc-7").is_dir());

    let known = t.d.known_repos().await.unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].path, clone.to_string_lossy());
    assert_eq!(known[0].last_used, Some(a.created_at));

    let e =
        t.d.resolve_repo("https://git.example.com/acme")
            .await
            .unwrap_err();
    assert_eq!(e.code, "bad_repo_url");
    let e = t.d.resolve_repo("relative/app").await.unwrap_err();
    assert_eq!(e.code, "not_a_repo");
}

#[tokio::test]
async fn opencode_agents_get_its_plugin_file_and_mode() {
    let t = setup().await;
    let mut r = req(&t, "SC-20");
    r.runtime = Some("opencode".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.runtime.as_deref(), Some("opencode"));
    assert_eq!(
        a.next_prompt.as_deref(),
        Some("/xherd/feature SC-20 add login")
    );
    // Default mode `edits`, through opencode's environment variable.
    let launch = a.launch.clone().unwrap();
    assert_eq!(launch[0], "env");
    assert!(launch[1].starts_with("OPENCODE_PERMISSION="), "{launch:?}");
    assert_eq!(launch[2..], ["opencode"]);
    let wt = Path::new(a.workdir.as_deref().unwrap());
    let js = std::fs::read_to_string(wt.join(".opencode/plugin/xherd.js")).unwrap();
    assert!(js.starts_with("// Written by xherd"));
    assert!(js.contains(r#""session.idle":"stop""#));
    assert!(js.contains(r#""xherd task *""#));
    assert!(!wt.join(".claude/settings.local.json").exists());
    let ignored = git(wt, &["check-ignore", ".opencode/plugin/xherd.js"]);
    assert!(ignored.status.success(), "kept out of git");
}

#[tokio::test]
async fn the_runtime_comes_from_the_request_then_the_profile_then_settings() {
    let t = setup().await;
    let a = t.d.new_job(req(&t, "SC-21")).await.unwrap();
    assert_eq!(a.runtime.as_deref(), Some("claude-code"), "setting default");

    let saved = t.d.save_config(
        serde_json::from_value(serde_json::json!({"defaultRuntime": "opencode"})).unwrap(),
    );
    assert!(saved.is_ok(), "{:?}", saved.err().map(|e| e.message));
    let a = t.d.new_job(req(&t, "SC-22")).await.unwrap();
    assert_eq!(a.runtime.as_deref(), Some("opencode"), "changed setting");

    let p: xherd_core::profile::Profile = serde_json::from_value(serde_json::json!({
        "name": "app",
        "match": [t.repo.path().canonicalize().unwrap().to_string_lossy()],
        "runtime": "claude-code"
    }))
    .unwrap();
    t.d.create_profile(p).await.unwrap();
    let a = t.d.new_job(req(&t, "SC-23")).await.unwrap();
    assert_eq!(a.runtime.as_deref(), Some("claude-code"), "profile");

    let mut r = req(&t, "SC-24");
    r.runtime = Some("opencode".into());
    assert_eq!(
        t.d.new_job(r).await.unwrap().runtime.as_deref(),
        Some("opencode")
    );

    let mut r = req(&t, "SC-25");
    r.runtime = Some("github".into());
    assert_eq!(t.d.new_job(r).await.unwrap_err().code, "runtime_unusable");
    let bad = t.d.save_config(
        serde_json::from_value(serde_json::json!({"defaultRuntime": "nope"})).unwrap(),
    );
    assert!(bad.unwrap_err().message.contains("defaultRuntime"));
}

/// A ticket URL gets its key from the tracker that reads it, and the key
/// prefixes the branch (#114); a URL no tracker reads stays a plain link.
#[tokio::test]
async fn a_ticket_url_names_the_branch() {
    let t = setup().await;
    let mut r = req(&t, "x");
    r.name = "fix-login".into();
    r.ticket = Some("https://github.com/acme/api/issues/12".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.ticket_key(), Some("#12"));
    assert_eq!(a.branch, "12-fix-login");
    assert_eq!(
        a.next_prompt.as_deref(),
        Some("/xherd:feature https://github.com/acme/api/issues/12 add login")
    );
    let v =
        t.d.resolve_ticket_input("https://github.com/acme/api/issues/12", None)
            .await
            .unwrap();
    assert_eq!(
        (&v["key"], &v["tracker"]),
        (
            &serde_json::json!("#12"),
            &serde_json::json!("github-issues")
        )
    );
    // Jira reads only URLs of its own site, which is not set here.
    let v =
        t.d.resolve_ticket_input("https://acme.atlassian.net/browse/PAY-4", None)
            .await
            .unwrap();
    assert_eq!(
        (&v["key"], &v["tracker"]),
        (&serde_json::Value::Null, &serde_json::Value::Null)
    );
    let v = t.d.resolve_ticket_input("pay-4", None).await.unwrap();
    assert_eq!(
        (&v["key"], &v["tracker"]),
        (&serde_json::json!("PAY-4"), &serde_json::json!("jira"))
    );
}

/// A ticket URL names its repository (#148): the resolve answer carries it,
/// a key or a URL no tracker reads gives none, and with no repository at all
/// the agent is refused as `repo_required`.
#[tokio::test]
async fn a_ticket_url_names_the_repository() {
    let t = setup().await;
    let repo = |v: serde_json::Value| v["repo"].clone();
    let code = |r: Result<Job, ActionError>| r.unwrap_err().code;
    let v =
        t.d.resolve_ticket_input("https://github.com/acme/api/issues/12", None)
            .await
            .unwrap();
    assert_eq!(repo(v), serde_json::json!("https://github.com/acme/api"));
    let v =
        t.d.resolve_ticket_input("https://gitlab.com/acme/sub/api/-/issues/3", None)
            .await
            .unwrap();
    assert_eq!(
        repo(v),
        serde_json::json!("https://gitlab.com/acme/sub/api")
    );
    // No tracker reads a Jira URL here, and a key names no repository.
    for ticket in ["https://acme.atlassian.net/browse/PAY-4", "pay-4", ""] {
        let v = t.d.resolve_ticket_input(ticket, None).await.unwrap();
        assert_eq!(repo(v), serde_json::Value::Null, "{ticket}");
    }

    // No repository and nothing to derive one from.
    for ticket in [
        None,
        Some("PAY-4"),
        Some("https://acme.atlassian.net/browse/PAY-4"),
    ] {
        let mut r = req(&t, "x");
        r.repo = String::new();
        r.ticket = ticket.map(Into::into);
        assert_eq!(code(t.d.new_job(r).await), "repo_required", "{ticket:?}");
    }

    // With the tracker switched off the URL names nothing either.
    let patch =
        serde_json::from_value(serde_json::json!({"disabledPlugins": ["github-issues"]})).unwrap();
    t.d.save_config(patch).unwrap();
    let v =
        t.d.resolve_ticket_input("https://github.com/acme/api/issues/12", None)
            .await
            .unwrap();
    assert_eq!(repo(v), serde_json::Value::Null);
    let mut r = req(&t, "x");
    r.repo = String::new();
    r.ticket = Some("https://github.com/acme/api/issues/12".into());
    assert_eq!(code(t.d.new_job(r).await), "repo_required");

    // An explicit repository wins over the one the ticket names.
    let mut r = req(&t, "x");
    r.ticket = Some("https://github.com/acme/api/issues/12".into());
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.repo.as_deref(), Some(t.repo.path().to_str().unwrap()));
}

#[tokio::test]
async fn cancel_can_remove_the_worktree_and_keeps_the_branch() {
    let t = setup().await;
    let a = t.d.new_job(req(&t, "SC-7")).await.unwrap();
    let wt = std::path::PathBuf::from(a.workdir.clone().unwrap());
    assert!(wt.is_dir());
    let a =
        t.d.action(
            "SC-7",
            UserAction::Cancel {
                remove_worktree: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(a.status, Status::Cancelled);
    assert!(a.worktree_removed_at.is_some());
    assert!(!wt.exists());
    // The branch and the record stay.
    let branches = git(t.repo.path(), &["branch", "--list", "SC-7"]);
    assert!(String::from_utf8_lossy(&branches.stdout).contains("SC-7"));
    assert_eq!(a.workdir.as_deref(), Some(wt.to_string_lossy().as_ref()));
    // Every reader of the worktree answers the same code.
    assert_eq!(
        t.d.diff("SC-7", None, None).await.unwrap_err().code,
        "worktree_removed"
    );
    assert_eq!(t.d.docs("SC-7").await.unwrap_err().code, "worktree_removed");
    assert_eq!(
        t.d.open_targets("SC-7").await.unwrap_err().code,
        "worktree_removed"
    );
    assert_eq!(
        t.d.action("SC-7", UserAction::Resume)
            .await
            .unwrap_err()
            .code,
        "worktree_removed"
    );
    // A second removal changes nothing.
    let again =
        t.d.action("SC-7", UserAction::RemoveWorktree)
            .await
            .unwrap();
    assert_eq!(again.worktree_removed_at, a.worktree_removed_at);
}

#[tokio::test]
async fn remove_worktree_only_once_the_agent_is_finished() {
    let t = setup().await;
    let a = t.d.new_job(req(&t, "SC-8")).await.unwrap();
    let wt = std::path::PathBuf::from(a.workdir.clone().unwrap());
    let err =
        t.d.action("SC-8", UserAction::RemoveWorktree)
            .await
            .unwrap_err();
    assert_eq!(err.code, "not_allowed");
    assert!(wt.is_dir());
    t.d.action(
        "SC-8",
        UserAction::Cancel {
            remove_worktree: false,
        },
    )
    .await
    .unwrap();
    assert!(wt.is_dir());
    // Deleted by hand: the removal prunes git's record of it.
    std::fs::remove_dir_all(&wt).unwrap();
    let a =
        t.d.action("SC-8", UserAction::RemoveWorktree)
            .await
            .unwrap();
    assert!(a.worktree_removed_at.is_some());
    let list = git(t.repo.path(), &["worktree", "list", "--porcelain"]);
    assert!(!String::from_utf8_lossy(&list.stdout).contains(wt.to_string_lossy().as_ref()));
    // The Ticket tab keeps working: it reads the main checkout's origin.
    let status = t.d.ticket_status("SC-8").await.unwrap();
    assert_eq!(status["posted"], false);
}

// ---- jobs from saved agents (#141, T10) ----

use xherd_core::agent_def::AgentDef;
use xherd_core::autonomy::Autonomy;
use xherd_core::permission::PermissionMode;

fn saved(name: &str) -> AgentDef {
    serde_json::from_value(serde_json::json!({
        "name": name,
        "runtime": "claude-code",
        "autonomy": "important",
        "permissionMode": "auto",
        "instructions": "Be terse.",
        "sensitivePaths": ["secrets/**"],
        "maxDiffLines": 300,
    }))
    .unwrap()
}

fn from_agent(t: &T, agent: &str) -> NewJob {
    let mut r = req(t, "A-1");
    r.name = String::new();
    r.ticket = None;
    r.agent = Some(agent.into());
    r
}

#[tokio::test]
async fn a_job_from_an_agent_keeps_a_snapshot_and_its_instructions() {
    let t = setup().await;
    let def = t.d.create_agent(saved("rev")).await.unwrap().def;
    let j =
        t.d.new_agent_job("rev", from_agent(&t, "ignored"))
            .await
            .unwrap();
    // the default name (R21) and the agent's settings
    assert_eq!(j.name, "rev-1");
    assert_eq!(
        (j.autonomy, j.permission_mode),
        (Autonomy::Important, Some(PermissionMode::Auto))
    );
    let snap = j.agent().cloned().unwrap();
    assert_eq!(
        (snap.id.as_str(), snap.name.as_str(), snap.version),
        (def.id.as_str(), "rev", 1)
    );
    assert!(j.lowered.is_empty());
    let md = std::fs::read_to_string(
        Path::new(j.workdir.as_deref().unwrap()).join(".xherd/instructions.md"),
    )
    .unwrap();
    assert!(md.contains("## Agent instructions") && md.contains("Be terse."));
    // the next default name, and one from a ticket
    let mut r = from_agent(&t, "rev");
    r.ticket = Some("pay-7".into());
    assert_eq!(t.d.new_agent_job("rev", r).await.unwrap().name, "rev-pay-7");
    let k =
        t.d.new_agent_job("rev", from_agent(&t, "rev"))
            .await
            .unwrap();
    assert_eq!(k.name, "rev-2");
    // editing the agent leaves the job as it was
    let mut v2 = saved("rev");
    v2.instructions = Some("Be verbose.".into());
    v2.autonomy = Some(Autonomy::Ask);
    t.d.update_agent("rev", v2).await.unwrap();
    let again = t.d.action(&j.name, UserAction::Get).await.unwrap();
    assert_eq!(again.agent().unwrap().version, 1);
    assert_eq!(again.autonomy, Autonomy::Important);
    // the agent now sees its jobs
    let v = t.d.agent("rev").await.unwrap();
    assert_eq!(v.def.version, 2);
    assert!(t.d.agent_jobs("rev").await.unwrap().len() >= 3);
}

#[tokio::test]
async fn an_agent_job_is_refused_when_the_agent_cannot_start_it() {
    let t = setup().await;
    let code = |r: Result<Job, ActionError>| r.unwrap_err().code;
    assert_eq!(
        code(t.d.new_job(from_agent(&t, "nope")).await),
        "unknown_agent"
    );
    t.d.create_agent(saved("rev")).await.unwrap();
    t.d.archive_agent("rev", true).await.unwrap();
    assert_eq!(
        code(t.d.new_job(from_agent(&t, "rev")).await),
        "agent_archived"
    );
    t.d.archive_agent("rev", false).await.unwrap();
    let mut r = from_agent(&t, "rev");
    r.repo = String::new();
    r.name = "no-repo".into();
    assert_eq!(code(t.d.new_job(r.clone()).await), "repo_required");
    // an agent has no repository of its own: the request names one
    r.repo = t.repo.path().to_string_lossy().into_owned();
    r.name = "from-rev".into();
    assert_eq!(t.d.new_job(r).await.unwrap().agent().unwrap().name, "rev");
}

#[tokio::test]
async fn the_profile_is_a_ceiling_for_autonomy_and_mode() {
    let t = setup().await;
    let real = std::fs::canonicalize(t.repo.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let mut p = xherd_core::profile::Profile {
        name: "Local".into(),
        matches: vec![real],
        ..Default::default()
    };
    p.autonomy = Some(Autonomy::Ask);
    p.permission_mode = Some(PermissionMode::Edits);
    t.d.create_profile(p).await.unwrap();
    t.d.create_agent(saved("rev")).await.unwrap();
    let j =
        t.d.new_agent_job("rev", from_agent(&t, "rev"))
            .await
            .unwrap();
    assert_eq!(
        (j.autonomy, j.permission_mode),
        (Autonomy::Ask, Some(PermissionMode::Edits))
    );
    assert_eq!(j.lowered.len(), 2);
    assert_eq!(
        (j.lowered[0].field.as_str(), j.lowered[0].to.as_str()),
        ("autonomy", "ask")
    );
    // live changes cannot go above it either
    let e =
        t.d.set_permission_mode(&j.name, PermissionMode::Auto)
            .await
            .unwrap_err();
    assert_eq!(e.code, "above_profile");
    let e =
        t.d.action(
            &j.name,
            UserAction::SetAutonomy {
                level: Autonomy::Important,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "above_profile");
    // at or below it is fine, and clears that note
    let j =
        t.d.set_permission_mode(&j.name, PermissionMode::Ask)
            .await
            .unwrap();
    assert_eq!(j.permission_mode, Some(PermissionMode::Ask));
    assert_eq!(j.lowered.len(), 1);
}

#[tokio::test]
async fn a_busy_agent_counts_its_live_jobs_and_cannot_be_deleted() {
    let t = setup().await;
    t.d.create_agent(saved("rev")).await.unwrap();
    let j =
        t.d.new_agent_job("rev", from_agent(&t, "rev"))
            .await
            .unwrap();
    for _ in 0..200 {
        if t.d.action(&j.name, UserAction::Get).await.unwrap().status == Status::Running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let v = t.d.agent("rev").await.unwrap();
    assert_eq!(
        (v.live_jobs, v.last_job.as_deref()),
        (1, Some(j.name.as_str()))
    );
    assert_eq!(
        t.d.delete_agent("rev").await.unwrap_err().code,
        "agent_busy"
    );
}

// ---- skills copied into the worktree (#141, T16) ----

use xherd_daemon::skills::{NewSkill, NewSkillFile};

/// A skill with a nested file, in the library.
async fn add_skill(t: &T, name: &str) -> String {
    let main = format!("---\nname: {name}\ndescription: Does {name}\n---\nSteps.\n");
    t.d.create_skill(NewSkill {
        name: name.into(),
        text: None,
        files: Some(vec![
            NewSkillFile {
                path: "SKILL.md".into(),
                text: Some(main),
                base64: None,
            },
            NewSkillFile {
                path: "ref/notes.md".into(),
                text: Some("notes".into()),
                base64: None,
            },
        ]),
        replace: false,
    })
    .await
    .unwrap()
    .info
    .sha256
}

fn skilled(name: &str, runtime: &str, skills: &[&str]) -> AgentDef {
    let mut v = serde_json::to_value(saved(name)).unwrap();
    v["runtime"] = runtime.into();
    v["skills"] = serde_json::json!(skills);
    serde_json::from_value(v).unwrap()
}

fn worktree(j: &Job) -> &Path {
    Path::new(j.workdir.as_deref().unwrap())
}

#[tokio::test]
async fn an_agents_skills_are_copied_and_kept_out_of_git() {
    let t = setup().await;
    let sha = add_skill(&t, "lint-fix").await;
    add_skill(&t, "other").await;
    t.d.create_agent(skilled("rev", "claude-code", &["lint-fix", "other"]))
        .await
        .unwrap();
    let exclude = t.repo.path().join(".git/info/exclude");
    let before = std::fs::read_to_string(&exclude).unwrap_or_default();

    let j =
        t.d.new_agent_job("rev", from_agent(&t, "rev"))
            .await
            .unwrap();
    let wt = worktree(&j);
    let dir = wt.join(".claude/skills/lint-fix");
    assert!(
        std::fs::read_to_string(dir.join("SKILL.md"))
            .unwrap()
            .contains("Does lint-fix")
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("ref/notes.md")).unwrap(),
        "notes"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
        "*\n"
    );

    // recorded on the job, hash included
    let skills = j.agent().unwrap().skills.clone();
    assert_eq!(skills.len(), 2);
    assert!(skills.iter().all(|s| s.installed && s.skipped.is_none()));
    assert_eq!(
        (skills[0].name.as_str(), skills[0].sha256.as_str()),
        ("lint-fix", sha.as_str())
    );
    // and stored: the next read of the job has it
    let again = t.d.action(&j.name, UserAction::Get).await.unwrap();
    assert_eq!(again.agent().unwrap().skills, skills);

    // git sees nothing, in the worktree or in the main checkout
    for dir in [wt, t.repo.path()] {
        let st = git(dir, &["status", "--porcelain", "--untracked-files=all"]);
        let out = String::from_utf8_lossy(&st.stdout).into_owned();
        assert!(!out.contains("skills"), "{dir:?}: {out}");
    }
    assert!(!t.repo.path().join(".claude").exists());
    // xherd adds its own lines to the shared exclude at creation; none is for skills
    let after = std::fs::read_to_string(&exclude).unwrap_or_default();
    assert!(!after.contains("skills"), "{after}");
    assert_eq!(after.lines().count(), before.lines().count() + 2, "{after}");

    // the prompt never carries them
    let prompt = j.next_prompt.unwrap_or_default();
    assert!(!prompt.contains("Steps."), "{prompt}");
}

#[tokio::test]
async fn a_skill_the_repository_already_has_is_left_alone() {
    let t = setup().await;
    add_skill(&t, "lint-fix").await;
    add_skill(&t, "other").await;
    let own = t.repo.path().join(".claude/skills/lint-fix");
    std::fs::create_dir_all(&own).unwrap();
    std::fs::write(own.join("SKILL.md"), "the repo's own").unwrap();
    assert!(git(t.repo.path(), &["add", "-A"]).status.success());
    assert!(
        git(t.repo.path(), &["commit", "-q", "-m", "skill"])
            .status
            .success()
    );
    t.d.create_agent(skilled("rev", "claude-code", &["lint-fix", "other"]))
        .await
        .unwrap();

    let j =
        t.d.new_agent_job("rev", from_agent(&t, "rev"))
            .await
            .unwrap();
    let wt = worktree(&j);
    assert_eq!(
        std::fs::read_to_string(wt.join(".claude/skills/lint-fix/SKILL.md")).unwrap(),
        "the repo's own"
    );
    assert!(!wt.join(".claude/skills/lint-fix/.gitignore").exists());
    assert!(wt.join(".claude/skills/other/SKILL.md").is_file());
    let skills = j.agent().cloned().unwrap().skills;
    assert_eq!(
        (skills[0].installed, skills[0].skipped.as_deref()),
        (false, Some("the repository has its own"))
    );
    assert!(skills[1].installed);
    let st = git(wt, &["status", "--porcelain", "--untracked-files=all"]);
    assert!(
        st.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&st.stdout)
    );
}

#[tokio::test]
async fn a_runtime_without_a_skill_folder_skips_them_and_still_creates_the_job() {
    // opencode with its target (built in), then without it
    let t = setup().await;
    add_skill(&t, "lint-fix").await;
    t.d.create_agent(skilled("oc", "opencode", &["lint-fix"]))
        .await
        .unwrap();
    let j = t.d.new_agent_job("oc", from_agent(&t, "oc")).await.unwrap();
    assert!(
        worktree(&j)
            .join(".opencode/skills/lint-fix/SKILL.md")
            .is_file()
    );
    assert!(j.agent().unwrap().skills[0].installed);

    let t = setup_with(|o| {
        let rt = o
            .builtin_plugins
            .iter_mut()
            .find(|m| m.name == "opencode")
            .unwrap();
        rt.data
            .get_mut("terminal")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("skills");
    })
    .await;
    let sha = add_skill(&t, "lint-fix").await;
    t.d.create_agent(skilled("oc", "opencode", &["lint-fix"]))
        .await
        .unwrap();
    let j = t.d.new_agent_job("oc", from_agent(&t, "oc")).await.unwrap();
    let wt = worktree(&j);
    assert!(!wt.join(".opencode/skills").exists());
    let s = j.agent().unwrap().skills.clone();
    assert_eq!(s.len(), 1);
    assert_eq!(
        (
            s[0].installed,
            s[0].skipped.as_deref(),
            s[0].sha256.as_str()
        ),
        (false, Some("opencode has no skill folder"), sha.as_str())
    );
}

#[tokio::test]
async fn agent_memory_is_copied_into_the_job_when_it_is_created() {
    let t = setup().await;
    t.d.create_agent(saved("mem")).await.unwrap();
    // nothing remembered yet: no section
    assert_eq!(t.d.agent_memory("mem").await.unwrap()["version"], 0);
    let first =
        t.d.new_agent_job("mem", from_agent(&t, "mem"))
            .await
            .unwrap();
    assert!(first.agent().unwrap().memory.is_none());
    let md = |j: &xherd_core::Job| {
        std::fs::read_to_string(
            Path::new(j.workdir.as_deref().unwrap()).join(".xherd/instructions.md"),
        )
        .unwrap()
    };
    assert!(!md(&first).contains("## Agent memory"));
    // saved memory reaches the next job, after the instructions
    t.d.set_agent_memory("mem", "Tests live in qa/.")
        .await
        .unwrap();
    let second =
        t.d.new_agent_job("mem", from_agent(&t, "mem"))
            .await
            .unwrap();
    let text = md(&second);
    assert!(text.contains("## Agent memory") && text.contains("Tests live in qa/."));
    assert!(text.find("## Agent instructions") < text.find("## Agent memory"));
    // a later edit leaves the job's copy alone
    let v = t.d.set_agent_memory("mem", "Changed.").await.unwrap();
    assert_eq!(v["version"], 2);
    let again = t.d.action(&second.name, UserAction::Get).await.unwrap();
    assert_eq!(
        again.agent().unwrap().memory.as_deref(),
        Some("Tests live in qa/.")
    );
    // too long, and unknown agents
    let long = "x".repeat(4001);
    assert_eq!(
        t.d.set_agent_memory("mem", &long).await.unwrap_err().code,
        "bad_memory"
    );
    assert!(t.d.set_agent_memory("nobody", "x").await.is_err());
}

/// R30: the third job without an agent with the same setup in the same repo
/// gets a suggestion and one notice; the first two and a fourth do not, and
/// neither does a different setup or `suggestAgents` off.
#[tokio::test]
async fn the_third_job_with_the_same_setup_gets_a_suggestion() {
    let t = setup().await;
    let mut rx = t.d.hub.subscribe();
    let mut made = Vec::new();
    for n in ["one", "two", "three", "four"] {
        let mut r = req(&t, "");
        r.name = n.into();
        made.push(t.d.new_job(r).await.unwrap());
    }
    let reasons: Vec<_> = made
        .iter()
        .map(|j| j.suggestion.as_ref().map(|s| s.reason.as_str()))
        .collect();
    assert_eq!(reasons, [None, None, Some("repeated"), None]);
    let mut notices = 0;
    while let Ok(c) = rx.try_recv() {
        if let Change::Notify { kind, .. } = c
            && kind == "suggestion"
        {
            notices += 1;
        }
    }
    assert_eq!(notices, 1);
}

#[tokio::test]
async fn a_different_setup_does_not_count_for_the_third_job_rule() {
    let t = setup().await;
    for (n, autonomy) in [("one", "ask"), ("two", "ask"), ("three", "important")] {
        let mut r = req(&t, "");
        r.name = n.into();
        r.autonomy = serde_json::from_value(serde_json::json!(autonomy)).unwrap();
        let j = t.d.new_job(r).await.unwrap();
        assert!(j.suggestion.is_none(), "{n}");
    }
}

// ---- several agents in one job (#141, D26, T23) ----

fn assign(stage: &str, agent: &str, human: bool) -> xherd_core::resolve::Assignment {
    xherd_core::resolve::Assignment {
        stage: stage.into(),
        agent_id: agent.into(),
        human,
    }
}

#[tokio::test]
async fn a_job_bakes_every_agent_of_its_workflow_and_starts_on_the_first_stages() {
    let t = setup().await;
    let dev = t.d.create_agent(saved("dev")).await.unwrap().def;
    let arch = t.d.create_agent(saved("architect")).await.unwrap().def;
    let rev = t.d.create_agent(saved("reviewer")).await.unwrap().def;
    t.d.set_assignments(
        "xherd",
        "feature",
        vec![
            assign("design", "architect", false),
            assign("review", "reviewer", true),
        ],
    )
    .await
    .unwrap();
    // `--agent dev` fills the unassigned stages (before the previous one);
    // the request gives `ship` its own agent.
    let mut r = from_agent(&t, "dev");
    r.name = "multi".into();
    r.agents.insert("ship".into(), "architect".into());
    let j = t.d.new_job(r).await.unwrap();
    let at = |s: &str| j.stage_agents[s].agent.as_str();
    assert_eq!(at("spec"), dev.id);
    assert_eq!(at("design"), arch.id);
    assert_eq!(at("tasks"), dev.id);
    assert_eq!(at("implement"), dev.id);
    assert_eq!(at("review"), rev.id);
    assert_eq!(at("ship"), arch.id);
    assert!(j.stage_agents["review"].human && !j.stage_agents["design"].human);
    assert_eq!(j.default_agent.as_deref(), Some(dev.id.as_str()));
    assert_eq!(j.current_agent.as_deref(), Some(dev.id.as_str()));
    assert_eq!(j.agents.len(), 3);
    // each agent carries what it resolved to
    let g = &j.agents[&rev.id];
    assert_eq!(g.runtime, "claude-code");
    assert_eq!(g.autonomy, Some(Autonomy::Important));
    assert_eq!(g.permission_mode, Some(PermissionMode::Auto));
    assert!(!g.launch.is_empty() && !g.fell_back);
    // the job starts as its first stage's agent
    assert_eq!(j.autonomy, Autonomy::Important);
    assert_eq!(j.launch.as_deref(), Some(&j.agent().unwrap().launch[..]));
    // every agent sees the job
    for a in ["dev", "architect", "reviewer"] {
        assert_eq!(t.d.agent_jobs(a).await.unwrap().len(), 1, "{a}");
    }
}

#[tokio::test]
async fn without_a_default_an_unassigned_stage_follows_the_one_before() {
    let t = setup().await;
    let arch = t.d.create_agent(saved("architect")).await.unwrap().def;
    t.d.set_assignments(
        "xherd",
        "feature",
        vec![assign("design", "architect", false)],
    )
    .await
    .unwrap();
    let mut r = req(&t, "A-2");
    r.name = "later".into();
    r.ticket = None;
    let j = t.d.new_job(r).await.unwrap();
    // `spec` has no agent: the session starts plain, the agent comes at design
    assert!(!j.stage_agents.contains_key("spec"));
    assert_eq!(j.stage_agents["design"].agent, arch.id);
    assert_eq!(j.stage_agents["tasks"].agent, arch.id);
    assert_eq!(j.stage_agents["review"].agent, arch.id);
    assert_eq!(j.current_agent, None);
    assert_eq!(j.default_agent, None);
    assert!(j.agent().is_none() && j.agents.len() == 1);
    assert_eq!(j.autonomy, Autonomy::Ask);
}

#[tokio::test]
async fn an_agent_on_its_fallback_runtime_is_recorded_once() {
    let t = setup().await;
    let mut d = saved("flaky");
    d.runtime = "opencode".into();
    d.fallback_runtime = Some("claude-code".into());
    t.d.create_agent(d).await.unwrap();
    // the preferred runtime works: no fallback
    let mut r = from_agent(&t, "flaky");
    r.name = "first".into();
    let a = t.d.new_job(r).await.unwrap();
    assert_eq!(a.runtime.as_deref(), Some("opencode"));
    assert!(!a.agent().unwrap().fell_back);
    // switched off: the fallback runs it
    let off = t.d.save_config(
        serde_json::from_value(serde_json::json!({"disabledPlugins": ["opencode"]})).unwrap(),
    );
    assert!(off.is_ok(), "{:?}", off.err().map(|e| e.message));
    let mut r = from_agent(&t, "flaky");
    r.name = "second".into();
    let b = t.d.new_job(r).await.unwrap();
    assert_eq!(b.runtime.as_deref(), Some("claude-code"));
    let g = b.agent().unwrap();
    assert!(g.fell_back && g.runtime == "claude-code");
    let ev = t.d.store.events(&b.id, 20).await.unwrap();
    assert_eq!(ev.iter().filter(|e| e.kind == "agent-fallback").count(), 1);
    assert!(
        t.d.store
            .events(&a.id, 20)
            .await
            .unwrap()
            .iter()
            .all(|e| e.kind != "agent-fallback")
    );
}

#[tokio::test]
async fn agents_on_different_runtimes_and_unknown_stages_are_refused() {
    let t = setup().await;
    t.d.create_agent(saved("dev")).await.unwrap();
    let mut oc = saved("oc");
    oc.runtime = "opencode".into();
    t.d.create_agent(oc).await.unwrap();
    let mut r = from_agent(&t, "dev");
    r.name = "mixed".into();
    r.agents.insert("review".into(), "oc".into());
    assert_eq!(t.d.new_job(r).await.unwrap_err().code, "mixed_runtime");
    let mut r = from_agent(&t, "dev");
    r.name = "nostage".into();
    r.agents.insert("deploy".into(), "dev".into());
    assert_eq!(t.d.new_job(r).await.unwrap_err().code, "unknown_stage");
    let mut r = from_agent(&t, "dev");
    r.name = "ghost".into();
    r.agents.insert("review".into(), "nobody".into());
    assert_eq!(t.d.new_job(r).await.unwrap_err().code, "unknown_agent");
    // nothing was left behind by the refused ones
    assert!(t.d.store.list_jobs().await.unwrap().is_empty());
}
