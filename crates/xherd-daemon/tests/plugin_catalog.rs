//! The plugin catalogue (R2-R5, R8): fetching through a fake `curl` script
//! that logs its arguments and serves canned answers by URL. Never the network.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};
use xherd_core::paths::Paths;
use xherd_daemon::*;

const GITHUB: &str =
    "https://api.github.com/search/repositories?q=topic:xherd-plugin&sort=stars&per_page=100";
const INDEX: &str = "https://example.org/xherd-catalog.json";
const START: i64 = 1_800_000_000_000;
const DAY: i64 = 24 * 60 * 60 * 1000;

struct T {
    home: tempfile::TempDir,
    fake: tempfile::TempDir,
    clock: Arc<ManualClock>,
    d: Daemon,
}

impl T {
    /// What curl answers for `url`: the status and the body.
    fn serve(&self, url: &str, status: u16, body: &str) {
        let k = fake_key(url);
        std::fs::write(
            self.fake.path().join(format!("{k}.status")),
            status.to_string(),
        )
        .unwrap();
        std::fs::write(self.fake.path().join(format!("{k}.body")), body).unwrap();
    }

    /// Every curl call so far, one line of arguments each.
    fn calls(&self) -> Vec<String> {
        std::fs::read_to_string(self.fake.path().join("log"))
            .map(|s| s.lines().map(String::from).collect())
            .unwrap_or_default()
    }

    fn cache(&self) -> Value {
        let p: PathBuf = Paths::at(self.home.path()).plugin_catalog();
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    }
}

/// A script that logs `$@`, then answers with the canned status and body of
/// the last argument (the URL); the key is the URL with every character that
/// is not a letter or digit turned into `_` (see `fake_key`).
fn write_fake_curl(dir: &std::path::Path) -> String {
    let script = dir.join("curl");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
D='{d}'
echo "$@" >> "$D/log"
for a in "$@"; do URL="$a"; done
K=$(printf %s "$URL" | tr -c 'A-Za-z0-9' '_')
S=200
[ -f "$D/$K.status" ] && S=$(cat "$D/$K.status")
if [ ! -f "$D/$K.body" ]; then echo "curl: (6) Could not resolve host" >&2; exit 6; fi
printf 'HTTP/2 %s\r\nx-ratelimit-reset: 1800003600\r\n\r\n' "$S" >&2
cat "$D/$K.body"
"#,
            d = dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    script.display().to_string()
}

async fn daemon_with(sources: &[&str], tweak: impl FnOnce(&mut Options)) -> T {
    let home = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let fake = tempfile::tempdir().unwrap();
    let paths = Paths::at(home.path());
    paths.ensure().unwrap();
    let cfg = xherd_core::config::Config {
        workspace: Some(workspace.path().to_string_lossy().into_owned()),
        plugin_catalogs: sources.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    cfg.save(&paths.config()).unwrap();
    let clock = Arc::new(ManualClock::new(START));
    let mut o = Options::new(Arc::new(FakeDriver::default()), clock.clone());
    o.catalog_curl = write_fake_curl(fake.path());
    tweak(&mut o);
    let d = Daemon::start(paths, o).await.unwrap();
    // Keep the workspace alive as long as the daemon.
    std::mem::forget(workspace);
    T {
        home,
        fake,
        clock,
        d,
    }
}

fn github_body() -> String {
    json!({"items": [
        {"name": "xherd-forgejo", "owner": {"login": "acme"}, "description": "Forgejo host",
         "clone_url": "https://github.com/acme/xherd-forgejo.git", "stargazers_count": 12,
         "pushed_at": "2026-10-01T10:00:00Z"}
    ]})
    .to_string()
}

fn index_body() -> String {
    json!({"xherdCatalog": 1, "plugins": [
        {"name": "forgejo", "url": "https://codeberg.org/acme/xherd-forgejo",
         "ref": "v0.3.0", "path": "plugin", "description": "Forgejo host", "kind": "host"},
        {"name": "bad", "url": "http://example.org/x/y"}
    ]})
    .to_string()
}

/// The fake's file key for a URL (the script's `tr -c 'A-Za-z0-9' _`).
fn fake_key(url: &str) -> String {
    url.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[tokio::test]
async fn entries_are_parsed_and_the_cache_is_written() {
    let t = daemon_with(&["github"], |_| {}).await;
    t.serve(GITHUB, 200, &github_body());
    let c = t.d.plugin_catalog(None, false).await.unwrap();
    assert_eq!(c.entries.len(), 1);
    assert_eq!(c.entries[0].name, "xherd-forgejo");
    assert_eq!(c.entries[0].owner, "acme");
    assert_eq!(c.entries[0].source, "github");
    assert_eq!(c.sources.len(), 1);
    assert_eq!(c.sources[0].fetched_at, Some(START));
    assert!(c.sources[0].error.is_none());
    assert_eq!(t.calls().len(), 1);
    let cache = t.cache();
    assert_eq!(cache["sources"]["github"]["fetchedAt"], START);
    assert_eq!(
        cache["sources"]["github"]["entries"][0]["name"],
        "xherd-forgejo"
    );
}

#[tokio::test]
async fn a_second_call_within_a_day_does_not_fetch_and_a_later_one_does() {
    let t = daemon_with(&["github", INDEX], |_| {}).await;
    t.serve(GITHUB, 200, &github_body());
    t.serve(INDEX, 200, &index_body());
    let first = t.d.plugin_catalog(None, false).await.unwrap();
    assert_eq!(first.entries.len(), 2);
    assert_eq!(first.sources[1].skipped, 1);
    assert_eq!(t.calls().len(), 2);

    t.clock.advance(DAY - 1);
    let again = t.d.plugin_catalog(Some("forge"), false).await.unwrap();
    assert_eq!(again.entries.len(), 2);
    assert_eq!(t.calls().len(), 2, "still fresh: no curl call");

    t.clock.advance(1);
    t.d.plugin_catalog(None, false).await.unwrap();
    assert_eq!(t.calls().len(), 4, "a day later both sources are fetched");
    assert_eq!(t.cache()["sources"]["github"]["fetchedAt"], START + DAY);
}
