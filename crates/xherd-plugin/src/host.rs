//! Host plugins (spec §10.1 host commands). The built-in `github` plugin runs
//! inside xherd through the `gh` CLI; an external host plugin with the same
//! name replaces it and answers through the command protocol.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::exec::{Cmd, run};
use xherd_core::pr::{CheckRun, Comment, Mergeable, PrState, Snapshot};

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

    /// `pr-comments {url}` (R-304): every comment on the pull request at
    /// `url`: line comments, review texts and the conversation. Built-in
    /// `github` reads three endpoints through `gh api`.
    pub async fn pr_comments(&self, dir: &Path, url: &str) -> Result<Vec<Comment>, PluginError> {
        let name = "pr-comments";
        if !self.plugin.builtin() {
            let v = run_command(
                &self.plugin,
                name,
                json!({"url": url, "worktree": dir}),
                &self.ctx,
            )
            .await?;
            return serde_json::from_value(v).map_err(|e| self.shape(name, &e.to_string()));
        }
        let (host, owner, repo, number) = parse_pr_url(url)
            .ok_or_else(|| self.shape(name, &format!("{url} is not a pull request URL")))?;
        let gh = self
            .ctx
            .settings
            .get("gh")
            .and_then(Value::as_str)
            .unwrap_or("gh");
        let mut out = Vec::new();
        for (kind, path) in [
            (
                "review-comment",
                format!("repos/{owner}/{repo}/pulls/{number}/comments"),
            ),
            (
                "issue",
                format!("repos/{owner}/{repo}/issues/{number}/comments"),
            ),
            (
                "review",
                format!("repos/{owner}/{repo}/pulls/{number}/reviews"),
            ),
        ] {
            let mut args = vec!["api", "--paginate"];
            if host != "github.com" {
                args.extend(["--hostname", &host]);
            }
            args.push(&path);
            let bytes = self.gh(gh, dir, &args).await?;
            let items = parse_stream(&bytes).map_err(|e| self.shape(name, &e))?;
            out.extend(items.iter().filter_map(|v| github_comment(kind, v)));
        }
        Ok(out)
    }

    /// `pr-reply {url, to, body}` (R-306): answer comment `to` (an id as
    /// `pr-comments` gave it). `body` is posted as it is: the caller adds the
    /// marker. Built-in `github` replies in the thread of a line comment and
    /// otherwise adds a comment to the conversation.
    pub async fn pr_reply(
        &self,
        dir: &Path,
        url: &str,
        to: &str,
        body: &str,
    ) -> Result<(), PluginError> {
        let name = "pr-reply";
        if !self.plugin.builtin() {
            run_command(
                &self.plugin,
                name,
                json!({"url": url, "to": to, "body": body, "worktree": dir}),
                &self.ctx,
            )
            .await?;
            return Ok(());
        }
        let (host, owner, repo, number) = parse_pr_url(url)
            .ok_or_else(|| self.shape(name, &format!("{url} is not a pull request URL")))?;
        let path = reply_path(to, &owner, &repo, number)
            .ok_or_else(|| self.shape(name, &format!("{to} is not a comment id")))?;
        let gh = self
            .ctx
            .settings
            .get("gh")
            .and_then(Value::as_str)
            .unwrap_or("gh");
        let field = format!("body={body}");
        let mut args = vec!["api", "-X", "POST"];
        if host != "github.com" {
            args.extend(["--hostname", &host]);
        }
        args.extend([path.as_str(), "-f", &field]);
        self.gh(gh, dir, &args).await?;
        Ok(())
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

/// `https://host/owner/repo/pull/7` → (host, owner, repo, 7). Plain names
/// only: the parts go into an API path.
pub fn parse_pr_url(url: &str) -> Option<(String, String, String, u64)> {
    let rest = url.strip_prefix("https://")?;
    let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    let [host, owner, repo, "pull", number] = parts[..] else {
        return None;
    };
    let plain = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    };
    (plain(host) && plain(owner) && plain(repo))
        .then(|| number.parse().ok())
        .flatten()
        .map(|n| (host.into(), owner.into(), repo.into(), n))
}

/// The API path that answers comment `to` (`review-comment:11`, `issue:12`,
/// `review:13`): a reply in the thread of a line comment, else a new comment
/// in the conversation.
fn reply_path(to: &str, owner: &str, repo: &str, number: u64) -> Option<String> {
    let (kind, id) = to.split_once(':')?;
    let id: u64 = id.parse().ok()?;
    match kind {
        "review-comment" => Some(format!(
            "repos/{owner}/{repo}/pulls/{number}/comments/{id}/replies"
        )),
        "issue" | "review" => Some(format!("repos/{owner}/{repo}/issues/{number}/comments")),
        _ => None,
    }
}

/// `gh api --paginate` prints one JSON array per page, back to back: all
/// their items.
pub fn parse_stream(bytes: &[u8]) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for page in serde_json::Deserializer::from_slice(bytes).into_iter::<Value>() {
        match page.map_err(|e| e.to_string())? {
            Value::Array(items) => out.extend(items),
            other => return Err(format!("expected a list, got {other}")),
        }
    }
    Ok(out)
}

/// A GitHub comment, review or issue comment, as a `Comment`; reviews
/// without text are skipped. Ids carry the kind, since the three lists number
/// independently.
fn github_comment(kind: &str, v: &Value) -> Option<Comment> {
    let body = v["body"].as_str().filter(|b| !b.trim().is_empty())?;
    let id = v["id"].as_u64()?;
    Some(Comment {
        id: format!("{kind}:{id}"),
        kind: kind.into(),
        author: v["user"]["login"].as_str().unwrap_or("unknown").into(),
        body: body.into(),
        url: v["html_url"].as_str().unwrap_or_default().into(),
        created_at: v["created_at"]
            .as_str()
            .or_else(|| v["submitted_at"].as_str())
            .unwrap_or_default()
            .into(),
        path: v["path"].as_str().map(str::to_string),
        line: v["line"]
            .as_u64()
            .or_else(|| v["original_line"].as_u64())
            .map(|n| n as u32),
        in_reply_to: v["in_reply_to_id"].as_u64().map(|n| format!("{kind}:{n}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr_urls() {
        assert_eq!(
            parse_pr_url("https://github.com/acme/app/pull/7"),
            Some(("github.com".into(), "acme".into(), "app".into(), 7))
        );
        assert!(parse_pr_url("https://ghe.corp:8443/a/b.c/pull/12/").is_some());
        assert_eq!(parse_pr_url("http://github.com/a/b/pull/7"), None);
        assert_eq!(parse_pr_url("https://github.com/a/b/issues/7"), None);
        assert_eq!(parse_pr_url("https://github.com/a/b c/pull/7"), None);
        assert_eq!(parse_pr_url("https://github.com/a/../pull/x"), None);
    }

    #[test]
    fn replies_go_where_the_comment_is() {
        let p = |to| reply_path(to, "acme", "app", 7);
        assert_eq!(
            p("review-comment:11").as_deref(),
            Some("repos/acme/app/pulls/7/comments/11/replies")
        );
        assert_eq!(
            p("issue:12").as_deref(),
            Some("repos/acme/app/issues/7/comments")
        );
        assert_eq!(
            p("review:13").as_deref(),
            Some("repos/acme/app/issues/7/comments")
        );
        assert_eq!(p("issue:x"), None);
        assert_eq!(p("nope:1"), None);
        assert_eq!(p("12"), None);
    }

    #[test]
    fn paginated_pages_and_github_comments() {
        let pages = br#"[{"id":1,"user":{"login":"bo"},"body":"rename","path":"a.rs","line":4,"html_url":"u","created_at":"t","in_reply_to_id":9}]
[{"id":2,"user":{"login":"cy"},"body":"   "},{"id":3,"user":{"login":"cy"},"body":"ok","submitted_at":"s"}]"#;
        let items = parse_stream(pages).unwrap();
        assert_eq!(items.len(), 3);
        let c = github_comment("review-comment", &items[0]).unwrap();
        assert_eq!(c.id, "review-comment:1");
        assert_eq!((c.path.as_deref(), c.line), (Some("a.rs"), Some(4)));
        assert_eq!(c.in_reply_to.as_deref(), Some("review-comment:9"));
        assert!(
            github_comment("review", &items[1]).is_none(),
            "empty review text"
        );
        assert_eq!(github_comment("review", &items[2]).unwrap().created_at, "s");
        assert!(parse_stream(b"{}").is_err());
        assert!(parse_stream(b"").unwrap().is_empty());
    }

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
