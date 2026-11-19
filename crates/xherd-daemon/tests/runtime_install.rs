//! Runtime CLIs install themselves (R3, R6, R7, R9). A fake runtime whose
//! tool is a version file in a temp folder; never the real HOME or CLIs.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use xherd_core::paths::Paths;
use xherd_daemon::runtime_install::InstallState;
use xherd_daemon::*;

struct T {
    home: tempfile::TempDir,
    ver: PathBuf,
}

impl T {
    fn runs(&self) -> usize {
        std::fs::read_to_string(self.home.path().join("runs"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }
    fn installed(&self) -> String {
        std::fs::read_to_string(&self.ver)
            .unwrap_or_default()
            .trim()
            .into()
    }
}

/// `claude-code` only, requiring `fake` (version 1.2.3): its check prints the
/// version file, its install writes the pinned version and counts its runs.
async fn start(
    runtime_install: bool,
    have: Option<&str>,
    install: &str,
    disabled: &[&str],
) -> (T, Daemon) {
    let home = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path().join("data"));
    paths.ensure().unwrap();
    xherd_core::config::Config {
        workspace: Some(home.path().join("ws").to_string_lossy().into_owned()),
        disabled_plugins: disabled.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
    .save(&paths.config())
    .unwrap();
    let t = T {
        ver: home.path().join("ver"),
        home,
    };
    if let Some(v) = have {
        std::fs::write(&t.ver, v).unwrap();
    }
    let mut o = Options::new(Arc::new(FakeDriver::default()), Arc::new(SystemClock));
    o.runtime_install = runtime_install;
    o.builtin_plugins.retain(|m| m.name == "claude-code");
    let rt = &mut o.builtin_plugins[0];
    let trust = t.home.path().join("claude.json");
    std::fs::write(&trust, "{}").unwrap();
    rt.data.get_mut("terminal").unwrap()["trust"]["file"] = trust.to_string_lossy().into();
    let runs = t.home.path().join("runs");
    rt.requires = serde_json::from_value(json!([{
        "name": "fake",
        "check": ["sh", "-c", "cat \"$1\"", "_", t.ver],
        "version": "1.2.3",
        "install": ["sh", "-c", format!("{install}; echo run >> \"$2\""), "_", t.ver, runs],
    }]))
    .unwrap();
    let d = Daemon::start(paths, o).await.unwrap();
    (t, d)
}

const WRITE: &str = r#"echo "$XHERD_INSTALL_VERSION" > "$1""#;

async fn settle(d: &Daemon) {
    for _ in 0..100 {
        if d.install_state("claude-code", "fake").is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("install still running");
}

async fn until_runs(t: &T, n: usize) {
    for _ in 0..200 {
        if t.runs() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("install did not run");
}

async fn quiet() {
    tokio::time::sleep(Duration::from_millis(700)).await;
}

#[tokio::test]
async fn a_missing_runtime_is_installed_at_start() {
    let (t, d) = start(true, None, WRITE, &[]).await;
    until_runs(&t, 1).await;
    settle(&d).await;
    assert_eq!((t.installed().as_str(), t.runs()), ("1.2.3", 1));
    // The check passes now.
    let env = |_: &str| None;
    let cs = d.checks(&env).await;
    assert!(
        cs.iter()
            .any(|c| c.id == "plugin:claude-code:requires:fake" && c.ok)
    );
}

#[tokio::test]
async fn a_disabled_runtime_never_installs() {
    let (t, d) = start(true, None, WRITE, &["claude-code"]).await;
    quiet().await;
    assert_eq!(
        (t.runs(), d.install_state("claude-code", "fake")),
        (0, None)
    );
}

#[tokio::test]
async fn an_older_version_is_replaced_and_an_equal_or_newer_one_is_not() {
    let (t, d) = start(true, Some("1.0.0"), WRITE, &[]).await;
    until_runs(&t, 1).await;
    settle(&d).await;
    assert_eq!(t.installed(), "1.2.3");
    for have in ["1.2.3", "2.0.0", "odd output"] {
        let (t, _d) = start(true, Some(have), WRITE, &[]).await;
        quiet().await;
        assert_eq!(t.runs(), 0, "{have}");
        assert_eq!(t.installed(), have);
    }
}

#[tokio::test]
async fn without_runtime_install_nothing_runs() {
    let (t, d) = start(false, None, WRITE, &[]).await;
    quiet().await;
    assert_eq!(
        (t.runs(), d.install_state("claude-code", "fake")),
        (0, None)
    );
}

#[tokio::test]
async fn a_failed_install_keeps_its_tail_and_is_not_retried() {
    let (t, d) = start(true, None, "echo boom >&2; exit 1", &[]).await;
    for _ in 0..100 {
        if matches!(
            d.install_state("claude-code", "fake"),
            Some(InstallState::Failed { .. })
        ) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let Some(InstallState::Failed { tail }) = d.install_state("claude-code", "fake") else {
        panic!("not failed");
    };
    assert!(tail.contains("boom"), "{tail}");
    quiet().await;
    assert_eq!(t.runs(), 0);
}
