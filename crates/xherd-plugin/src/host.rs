//! Host plugins (spec §10.1 host commands). The built-in `github` plugin runs
//! inside xherd through the `gh` CLI; an external host plugin with the same
//! name replaces it and answers through the command protocol.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::exec::{Cmd, run};
use xherd_core::pr::{CheckRun, Mergeable, PrState, Snapshot};

use crate::{Context, Plugin, PluginError, run_command};

/// The host plugin used for pull requests (v1 supports GitHub only).
pub const GITHUB: &str = "github";

/// A host plugin with its resolved settings and secrets (no `Debug`: it
/// holds secrets).
#[derive(Clone)]
pub struct Host {
    pub plugin: Plugin,
    pub ctx: Context,
}

impl Host {
    /// `pr-for-branch {branch} → {url|null}` (R-302): the pull request of
    /// `branch`, looked up from the worktree `dir` (which knows the remote).
    pub async fn pr_for_branch(
        &self,
        dir: &Path,
        branch: &str,
    ) -> Result<Option<String>, PluginError> {
        let name = "pr-for-branch";
        if !self.plugin.builtin() {
            let v = run_command(
                &self.plugin,
                name,
                json!({"branch": branch, "worktree": dir}),
                &self.ctx,
            )
            .await?;
            return match &v {
                Value::Null => Ok(None),
                Value::Object(o) => match o.get("url") {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(u)) => Ok(Some(u.clone())),
                    Some(_) => Err(self.shape(name, "url is not a string")),
                },
                _ => Err(self.shape(name, "expected {url} or null")),
            };
        }
        let gh = self
            .ctx
            .settings
            .get("gh")
            .and_then(Value::as_str)
            .unwrap_or("gh");
        let out = self
            .gh(
                gh,
                dir,
                &[
                    "pr", "list", "--head", branch, "--state", "all", "--json", "url", "--limit",
                    "1",
                ],
            )
            .await?;
        let list: Vec<Value> = serde_json::from_slice(&out).map_err(|e| PluginError::BadJson {
            plugin: self.plugin.name().into(),
            name: name.into(),
            error: e.to_string(),
        })?;
        Ok(list
            .first()
            .and_then(|p| p["url"].as_str())
            .map(str::to_string))
    }

    /// `pr-status {url}` and `pr-checks {url}` (R-303): what a poll of the
    /// pull request at `url` found. Built-in `github` asks `gh`.
    pub async fn pr_snapshot(&self, dir: &Path, url: &str) -> Result<Snapshot, PluginError> {
        let (status, checks) = if !self.plugin.builtin() {
            let input = json!({"url": url, "worktree": dir});
            let s = run_command(&self.plugin, "pr-status", input.clone(), &self.ctx).await?;
            let c = run_command(&self.plugin, "pr-checks", input, &self.ctx).await?;
            (s, c)
        } else {
            let gh = self
                .ctx
                .settings
                .get("gh")
                .and_then(Value::as_str)
                .unwrap_or("gh");
            let s = self
                .gh(gh, dir, &["pr", "view", url, "--json", "state,mergeable"])
                .await?;
            let s = self.json("pr view", &s)?;
            // `gh pr checks` exits 1 when a check failed and 8 while some are
            // pending, with the list on stdout; 1 with nothing on stdout is
            // "no checks reported" (or a real failure, told by stderr).
            let out = self
                .gh_raw(gh, dir, &["pr", "checks", url, "--json", "name,bucket"])
                .await?;
            let c = match out.code {
                Some(0 | 8) => self.json("pr checks", &out.stdout)?,
                Some(1) if !out.stdout.iter().all(u8::is_ascii_whitespace) => {
                    self.json("pr checks", &out.stdout)?
                }
                Some(1) if out.stderr.contains("no checks reported") => json!([]),
                code => {
                    return Err(PluginError::Failed {
                        plugin: self.plugin.name().into(),
                        name: "pr checks".into(),
                        code,
                        stderr: out.stderr.trim().to_string(),
                    });
                }
            };
            (s, c)
        };
        let (state, mergeable) = parse_status(&status).map_err(|e| self.shape("pr-status", &e))?;
        let checks = parse_checks(&checks).map_err(|e| self.shape("pr-checks", &e))?;
        Ok(Snapshot {
            state,
            mergeable,
            checks,
        })
    }

    fn json(&self, name: &str, bytes: &[u8]) -> Result<Value, PluginError> {
        serde_json::from_slice(bytes).map_err(|e| PluginError::BadJson {
            plugin: self.plugin.name().into(),
            name: name.into(),
            error: e.to_string(),
        })
    }

    fn shape(&self, name: &str, error: &str) -> PluginError {
        PluginError::Shape {
            plugin: self.plugin.name().into(),
            name: name.into(),
            error: error.into(),
        }
    }

    /// Run `gh` in `dir`, never prompting; the `token` secret, if set, is
    /// passed as `GH_TOKEN` (otherwise gh uses its own login). A timeout is
    /// an error; the exit code is the caller's to judge.
    async fn gh_raw(
        &self,
        gh: &str,
        dir: &Path,
        args: &[&str],
    ) -> Result<xherd_core::exec::Output, PluginError> {
        let secs = self.plugin.manifest.timeout_seconds;
        let mut argv = vec![gh.to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let mut cmd = Cmd::new(argv)
            .cwd(dir)
            .timeout(Duration::from_secs(secs))
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1");
        if let Some(t) = self.ctx.secrets.get("token").and_then(Value::as_str)
            && !t.is_empty()
        {
            cmd = cmd.env("GH_TOKEN", t).redact(vec![t.to_string()]);
        }
        let out = run(cmd).await?;
        if out.timed_out {
            return Err(PluginError::Timeout {
                plugin: self.plugin.name().into(),
                name: args.join(" "),
                secs,
            });
        }
        Ok(out)
    }

    /// `gh_raw`, failing on any exit code but 0.
    async fn gh(&self, gh: &str, dir: &Path, args: &[&str]) -> Result<Vec<u8>, PluginError> {
        let out = self.gh_raw(gh, dir, args).await?;
        if out.code != Some(0) {
            return Err(PluginError::Failed {
                plugin: self.plugin.name().into(),
                name: args.join(" "),
                code: out.code,
                stderr: out.stderr.trim().to_string(),
            });
        }
        Ok(out.stdout)
    }
}

/// `{state, mergeable?}` of `pr-status`. State is `open`, `merged` or `closed`
/// in any case; `mergeable` is a bool or text (`MERGEABLE`, `CONFLICTING`,
/// `yes`, `no`…) and unknown when absent or not recognised.
pub fn parse_status(v: &Value) -> Result<(PrState, Mergeable), String> {
    let state = match v["state"].as_str().map(str::to_ascii_lowercase).as_deref() {
        Some("open") => PrState::Open,
        Some("merged") => PrState::Merged,
        Some("closed") => PrState::Closed,
        other => return Err(format!("state {other:?} is not open, merged or closed")),
    };
    let mergeable = match &v["mergeable"] {
        Value::Bool(true) => Mergeable::Yes,
        Value::Bool(false) => Mergeable::No,
        Value::String(s) => match s.to_ascii_lowercase().as_str() {
            "mergeable" | "yes" | "true" | "clean" => Mergeable::Yes,
            "conflicting" | "no" | "false" | "dirty" => Mergeable::No,
            _ => Mergeable::Unknown,
        },
        _ => Mergeable::Unknown,
    };
    Ok((state, mergeable))
}

/// `[{name, bucket}]` of `pr-checks`.
pub fn parse_checks(v: &Value) -> Result<Vec<CheckRun>, String> {
    let list = v.as_array().ok_or("expected a list of checks")?;
    list.iter()
        .map(|c| {
            Ok(CheckRun {
                name: c["name"].as_str().ok_or("a check has no name")?.to_string(),
                bucket: c["bucket"]
                    .as_str()
                    .ok_or("a check has no bucket")?
                    .to_ascii_lowercase(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parses_gh_and_plugin_shapes() {
        let s = |v: Value| parse_status(&v).unwrap();
        assert_eq!(
            s(json!({"state": "OPEN", "mergeable": "CONFLICTING"})),
            (PrState::Open, Mergeable::No)
        );
        assert_eq!(
            s(json!({"state": "merged", "mergeable": true})),
            (PrState::Merged, Mergeable::Yes)
        );
        assert_eq!(
            s(json!({"state": "Closed"})),
            (PrState::Closed, Mergeable::Unknown)
        );
        assert_eq!(
            s(json!({"state": "open", "mergeable": "UNKNOWN"})).1,
            Mergeable::Unknown
        );
        assert!(parse_status(&json!({"state": "draft"})).is_err());
    }

    #[test]
    fn checks_parse_and_lowercase_buckets() {
        let c = parse_checks(&json!([{"name": "ci", "bucket": "FAIL"}])).unwrap();
        assert_eq!((c[0].name.as_str(), c[0].bucket.as_str()), ("ci", "fail"));
        assert!(parse_checks(&json!([{"bucket": "pass"}])).is_err());
        assert!(parse_checks(&json!({})).is_err());
    }
}
