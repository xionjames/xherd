//! `Daemon::new_agent` against a real temp git repo (R-001..R-005c).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use xherd_core::new::NewAgent;
use xherd_core::paths::Paths;
use xherd_core::{Agent, Status};
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
fn req(t: &T, key: &str) -> NewAgent {
    NewAgent {
        name: key.to_ascii_lowercase(),
        ticket: Some(key.into()),
        ticket_key: None,
        slug: None,
        repo: t.repo.path().to_string_lossy().into_owned(),
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
    let a: Agent = t.d.new_agent(r).await.unwrap();
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
        serde_json::from_slice(&std::fs::read(wt.join(".xherd/agent.json")).unwrap()).unwrap();
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
    let code = |r: Result<Agent, ActionError>| r.unwrap_err().code;

    let mut r = req(&t, "SC-2");
    r.base = Some("nope".into());
    assert_eq!(code(t.d.new_agent(r).await), "base_unresolved");
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
    assert_eq!(code(t.d.new_agent(r).await), "unknown_flow");
    let mut r = req(&t, "SC-2");
    r.repo = t.ws.path().to_string_lossy().into_owned();
    assert_eq!(code(t.d.new_agent(r).await), "not_a_repo");

    t.d.new_agent(req(&t, "SC-2")).await.unwrap();
    assert_eq!(code(t.d.new_agent(req(&t, "SC-2")).await), "name_taken");

    assert!(git(t.repo.path(), &["branch", "SC-3"]).status.success());
    assert_eq!(code(t.d.new_agent(req(&t, "SC-3")).await), "branch_exists");
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
    let a = t.d.new_agent(r).await.unwrap();
    assert_eq!(a.base.as_deref(), Some("origin/main"));
    assert_eq!((a.pack, a.flow), (None, None));
    assert_eq!(a.next_prompt.as_deref(), Some("add login"));
}

#[tokio::test]
async fn starts_claude_code_with_hooks() {
    let t = setup().await;
    let wt =
        t.ws.path()
            .join("worktrees")
            .join(repo_name(&t))
            .join("sc-5");
    let a = t.d.new_agent(req(&t, "SC-5")).await.unwrap();
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
    assert_eq!(t.d.new_agent(r).await.unwrap_err().code, "mode_unavailable");
    let mut r = req(&t, "SC-8");
    r.permission_mode = Some(PermissionMode::Ask);
    let a = t.d.new_agent(r).await.unwrap();
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
    let line = |a: &Agent| a.launch.as_deref().map(|l| l.join(" ")).unwrap();
    let a = t.d.new_agent(req(&t, "SC-31")).await.unwrap();
    assert_eq!(a.model, None);
    assert_eq!(line(&a), "claude --permission-mode acceptEdits");

    let saved = t.d.save_config(
        serde_json::from_value(
            serde_json::json!({"runtimeModels": {"claude-code": "haiku", "opencode": ""}}),
        )
        .unwrap(),
    );
    assert!(saved.is_ok(), "{:?}", saved.err().map(|e| e.message));
    let a = t.d.new_agent(req(&t, "SC-32")).await.unwrap();
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
    let a = t.d.new_agent(req(&t, "SC-33")).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("sonnet"), "profile");

    let mut r = req(&t, "SC-34");
    r.model = Some(" opus ".into());
    let a = t.d.new_agent(r).await.unwrap();
    assert_eq!(a.model.as_deref(), Some("opus"), "request");

    let mut r = req(&t, "SC-35");
    r.model = Some("--evil".into());
    assert_eq!(t.d.new_agent(r).await.unwrap_err().code, "bad_model");

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
    t.d.new_agent(req(&t, "SC-7")).await.unwrap();
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
    let a = t.d.new_agent(req(&t, "SC-8")).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&trust).unwrap()).unwrap();
    assert_eq!(v["userID"], "u");
    assert_eq!(v["projects"]["/elsewhere"]["x"], 1);
    let wd = a.workdir.unwrap();
    assert_eq!(v["projects"][&wd]["hasTrustDialogAccepted"], true);
    // The worktree as stored and canonicalised (`/var` → `/private/var`), nothing else.
    assert!(v["projects"].as_object().unwrap().len() <= 3);

    // A trust file that is not JSON is never overwritten: creation refuses.
    std::fs::write(&trust, "{oops").unwrap();
    let e = t.d.new_agent(req(&t, "SC-9")).await.unwrap_err();
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
    let r = NewAgent {
        name: "sc-6".into(),
        ticket: None,
        ticket_key: None,
        slug: None,
        repo: repo.to_string_lossy().into_owned(),
        base: None,
        flow: None,
        prompt: None,
        autonomy: None,
        permission_mode: None,
        runtime: None,
        model: None,
    };
    assert_eq!(d.new_agent(r).await.unwrap_err().code, "runtime_unusable");
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
    let a = t.d.new_agent(r).await.unwrap();
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
    let a = t.d.new_agent(r).await.unwrap();
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
    let a = t.d.new_agent(req(&t, "SC-21")).await.unwrap();
    assert_eq!(a.runtime.as_deref(), Some("claude-code"), "setting default");

    let saved = t.d.save_config(
        serde_json::from_value(serde_json::json!({"defaultRuntime": "opencode"})).unwrap(),
    );
    assert!(saved.is_ok(), "{:?}", saved.err().map(|e| e.message));
    let a = t.d.new_agent(req(&t, "SC-22")).await.unwrap();
    assert_eq!(a.runtime.as_deref(), Some("opencode"), "changed setting");

    let p: xherd_core::profile::Profile = serde_json::from_value(serde_json::json!({
        "name": "app",
        "match": [t.repo.path().canonicalize().unwrap().to_string_lossy()],
        "runtime": "claude-code"
    }))
    .unwrap();
    t.d.create_profile(p).await.unwrap();
    let a = t.d.new_agent(req(&t, "SC-23")).await.unwrap();
    assert_eq!(a.runtime.as_deref(), Some("claude-code"), "profile");

    let mut r = req(&t, "SC-24");
    r.runtime = Some("opencode".into());
    assert_eq!(
        t.d.new_agent(r).await.unwrap().runtime.as_deref(),
        Some("opencode")
    );

    let mut r = req(&t, "SC-25");
    r.runtime = Some("github".into());
    assert_eq!(t.d.new_agent(r).await.unwrap_err().code, "runtime_unusable");
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
    let a = t.d.new_agent(r).await.unwrap();
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
