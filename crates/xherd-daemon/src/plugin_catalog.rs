//! The plugin catalogue: fetch the configured sources with `curl`, cache the
//! answers for a day and answer searches (R2, R3, R4, R5, R8). The pure parts
//! (parsing, merge, filter, the curl argv) live in `xherd_plugin::catalog`.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use xherd_core::exec::{Cmd, run};
use xherd_plugin::catalog::{self, Entry, Source};

use crate::create::refuse;
use crate::{ActionError, Daemon};

/// A source is fetched again after a day (R4).
const FRESH_MS: i64 = 24 * 60 * 60 * 1000;
/// A source whose last try failed is tried again after 15 minutes (R4).
const RETRY_MS: i64 = 15 * 60 * 1000;
/// The most a catalogue answer may hold (curl's `--max-filesize` as well).
const MAX_BODY: usize = 1024 * 1024;
const GITHUB_URL: &str =
    "https://api.github.com/search/repositories?q=topic:xherd-plugin&sort=stars&per_page=100";

/// `<data>/plugin-catalog.json`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default)]
    sources: BTreeMap<String, Cached>,
}

/// What one source gave the last time it was tried.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    /// When it last succeeded (ms).
    #[serde(default)]
    fetched_at: Option<i64>,
    /// When it was last tried, successfully or not (ms).
    #[serde(default)]
    tried_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(default)]
    skipped: usize,
    #[serde(default)]
    entries: Vec<Entry>,
}

/// One source in the answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub source: String,
    /// `null` when it never succeeded.
    pub fetched_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "is_zero")]
    pub skipped: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// `GET /plugins/catalog`.
#[derive(Debug, Clone, Serialize)]
pub struct Catalog {
    pub entries: Vec<Entry>,
    pub sources: Vec<SourceStatus>,
}

/// Whether a source must be fetched now.
fn due(c: Option<&Cached>, now: i64, refresh: bool, started: i64) -> bool {
    let Some(c) = c else { return true };
    if refresh && c.tried_at < started {
        return true;
    }
    let wait = if c.error.is_some() {
        RETRY_MS
    } else {
        FRESH_MS
    };
    now.saturating_sub(c.tried_at) >= wait
}

/// One fetch: the entries and how many were skipped, or why it failed.
async fn fetch_source(
    curl: &str,
    source: &Source,
    name: &str,
) -> Result<(Vec<Entry>, usize), String> {
    let (url, github) = match source {
        Source::Github => (GITHUB_URL, true),
        Source::Index(u) => (u.as_str(), false),
    };
    let cmd = Cmd::new(catalog::curl_argv(curl, url, github)).timeout(Duration::from_secs(20));
    let out = run(cmd).await.map_err(|e| e.to_string())?;
    if out.timed_out {
        return Err("timed out".into());
    }
    if out.truncated || out.stdout.len() > MAX_BODY {
        return Err("the answer is larger than 1 MiB".into());
    }
    if !out.success() {
        let why = out
            .stderr
            .lines()
            .rev()
            .find(|l| l.starts_with("curl:"))
            .map(|l| l.trim_start_matches("curl:").trim().to_string())
            .unwrap_or_else(|| format!("curl failed (exit {})", out.code.unwrap_or(-1)));
        return Err(why);
    }
    let (status, headers, body) = catalog::response(&out.stdout, &out.stderr);
    if status == 0 {
        return Err("no HTTP answer".into());
    }
    if status != 200 {
        return Err(if github {
            catalog::github_error(status, &headers)
        } else {
            format!("HTTP {status}")
        });
    }
    if github {
        catalog::parse_github(&body, name).map(|e| (e, 0))
    } else {
        catalog::parse_index(&body, name)
    }
}

impl Daemon {
    /// The catalogue entries matching `q`, from the cache; sources that are
    /// due (or all of them with `refresh`) are fetched first. A failing
    /// source keeps its old entries and reports its error (R4).
    pub async fn plugin_catalog(
        &self,
        q: Option<&str>,
        refresh: bool,
    ) -> Result<Catalog, ActionError> {
        if !self.plugin_install {
            return Err(refuse(
                "plugin_install_off",
                "installing plugins is turned off (XHERD_PLUGIN_INSTALL=off)",
            ));
        }
        let started = self.clock.now();
        let _guard = self.catalog_lock.lock().await;
        let listed: Vec<(String, Result<Source, String>)> = self
            .live_config()
            .plugin_catalogs
            .into_iter()
            .map(|s| {
                let parsed = Source::parse(&s);
                (s, parsed)
            })
            .collect();

        let path = self.paths.plugin_catalog();
        let mut cache: Cache = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "plugin-catalog.json is not readable; starting empty");
                Cache::default()
            }),
            Err(_) => Cache::default(),
        };
        let before = cache.sources.len();
        cache
            .sources
            .retain(|k, _| listed.iter().any(|(s, _)| s == k));
        let mut changed = cache.sources.len() != before;

        let now = self.clock.now();
        let mut set = tokio::task::JoinSet::new();
        for (name, source) in &listed {
            if let Ok(source) = source
                && due(cache.sources.get(name), now, refresh, started)
            {
                let (curl, name, source) =
                    (self.catalog_curl.clone(), name.clone(), source.clone());
                set.spawn(async move {
                    let got = fetch_source(&curl, &source, &name).await;
                    (name, got)
                });
            }
        }
        while let Some(done) = set.join_next().await {
            let Ok((name, got)) = done else { continue };
            let slot = cache.sources.entry(name).or_default();
            slot.tried_at = now;
            match got {
                Ok((entries, skipped)) => {
                    slot.fetched_at = Some(now);
                    slot.error = None;
                    slot.skipped = skipped;
                    slot.entries = entries;
                }
                Err(e) => slot.error = Some(e),
            }
            changed = true;
        }
        if changed {
            let bytes = serde_json::to_vec_pretty(&cache).unwrap_or_default();
            if let Err(e) = xherd_core::paths::write_atomic(&path, &bytes, None) {
                tracing::warn!(error = %e, "plugin-catalog.json not written");
            }
        }
        drop(_guard);

        let mut sources = Vec::new();
        let mut per_source = Vec::new();
        for (name, parsed) in &listed {
            match (parsed, cache.sources.remove(name)) {
                (Err(e), _) => sources.push(SourceStatus {
                    source: name.clone(),
                    fetched_at: None,
                    error: Some(e.clone()),
                    skipped: 0,
                }),
                (Ok(_), Some(c)) => {
                    sources.push(SourceStatus {
                        source: name.clone(),
                        fetched_at: c.fetched_at,
                        error: c.error,
                        skipped: c.skipped,
                    });
                    per_source.push(c.entries);
                }
                (Ok(_), None) => {}
            }
        }
        let records: Vec<(String, String, Option<String>)> = self
            .store
            .plugin_records()
            .await
            .map_err(|e| refuse("store", e.to_string()))?
            .into_iter()
            .map(|r| {
                let path = r
                    .data
                    .get("path")
                    .and_then(|p| p.as_str())
                    .map(String::from);
                (r.name, r.source, path)
            })
            .collect();
        let q = q.unwrap_or("").trim();
        let entries = catalog::merge(per_source)
            .into_iter()
            .map(|mut e| {
                e.installed = catalog::installed_names(&e, &records);
                e
            })
            .filter(|e| catalog::matches(e, q))
            .collect();
        Ok(Catalog { entries, sources })
    }
}
