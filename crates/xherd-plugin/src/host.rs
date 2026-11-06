//! Host plugins (spec §10.1 host commands). The built-in `github` plugin runs
//! inside xherd through the `gh` CLI; an external host plugin with the same
//! name replaces it and answers through the command protocol.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::exec::{Cmd, run};

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

    fn shape(&self, name: &str, error: &str) -> PluginError {
        PluginError::Shape {
            plugin: self.plugin.name().into(),
            name: name.into(),
            error: error.into(),
        }
    }

    /// Run `gh` in `dir`, never prompting; the `token` secret, if set, is
    /// passed as `GH_TOKEN` (otherwise gh uses its own login).
    async fn gh(&self, gh: &str, dir: &Path, args: &[&str]) -> Result<Vec<u8>, PluginError> {
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
        let (plugin, name) = (self.plugin.name().to_string(), args.join(" "));
        if out.timed_out {
            return Err(PluginError::Timeout { plugin, name, secs });
        }
        if out.code != Some(0) {
            return Err(PluginError::Failed {
                plugin,
                name,
                code: out.code,
                stderr: out.stderr.trim().to_string(),
            });
        }
        Ok(out.stdout)
    }
}
