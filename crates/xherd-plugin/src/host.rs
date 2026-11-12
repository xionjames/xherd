//! Host plugins (spec §10.1 host commands). The built-in `github` plugin runs
//! inside xherd through the `gh` CLI; an external host plugin with the same
//! name replaces it and answers through the command protocol.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use xherd_core::exec::{Cmd, run};
use xherd_core::pr::{CheckRun, Comment, Mergeable, PrState, ReviewState, Reviewer, Snapshot};

use crate::{Context, Plugin, PluginError, run_command};

/// A pull-request template of the repository (R-153).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Template {
    /// Relative to the worktree.
    pub path: String,
    pub name: String,
    pub body: String,
}

const TEMPLATE_FILES: &[&str] = &[
    ".github/pull_request_template.md",
    ".github/PULL_REQUEST_TEMPLATE.md",
    "docs/pull_request_template.md",
    "docs/PULL_REQUEST_TEMPLATE.md",
    "pull_request_template.md",
    "PULL_REQUEST_TEMPLATE.md",
];
const TEMPLATE_DIRS: &[&str] = &[
    ".github/PULL_REQUEST_TEMPLATE",
    ".gitlab/merge_request_templates",
];
const TEMPLATE_MAX_BYTES: u64 = 64 * 1024;
const TEMPLATE_MAX: usize = 20;

/// The templates the repository itself carries: GitHub's file locations and
/// folder, and GitLab's merge request templates. Plain files only (a symlink
/// could point out of the worktree) and none over 64 KiB. Organisation-wide
/// defaults live on the host, not here.
pub fn local_templates(dir: &Path) -> Vec<Template> {
    let mut found: Vec<String> = TEMPLATE_FILES.iter().map(|s| s.to_string()).collect();
    for d in TEMPLATE_DIRS {
        let mut names: Vec<String> = std::fs::read_dir(dir.join(d))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.ends_with(".md"))
            .collect();
        names.sort();
        found.extend(names.into_iter().map(|n| format!("{d}/{n}")));
    }
    let mut out: Vec<Template> = Vec::new();
    for rel in found {
        let path = dir.join(&rel);
        let plain = path
            .symlink_metadata()
            .is_ok_and(|m| m.is_file() && m.len() <= TEMPLATE_MAX_BYTES);
        let Some(body) = plain.then(|| std::fs::read_to_string(&path).ok()).flatten() else {
            continue;
        };
        let name = rel
            .rsplit('/')
            .next()
            .unwrap_or(&rel)
            .trim_end_matches(".md")
            .to_string();
        // A case-insensitive filesystem lists one file under both spellings.
        if out.iter().any(|t| t.path.eq_ignore_ascii_case(&rel)) {
            continue;
        }
        out.push(Template {
            path: rel,
            name,
            body,
        });
        if out.len() == TEMPLATE_MAX {
            break;
        }
    }
    out
}

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
    /// A built-in host with no pull-request commands (`bitbucket`: git access
    /// only) says so instead of being asked through `gh`.
    fn no_prs(&self, name: &str) -> Result<(), PluginError> {
        if self.plugin.builtin() && !matches!(self.plugin.name(), GITHUB | crate::gitlab::GITLAB) {
            return Err(self.shape(
                name,
                &format!("{} has no pull request support yet", self.plugin.name()),
            ));
        }
        Ok(())
    }

    /// The built-in GitLab plugin (merge requests through `glab api`).
    fn is_gitlab(&self) -> bool {
        self.plugin.builtin() && self.plugin.name() == crate::gitlab::GITLAB
    }

    /// `pr-for-branch {branch} → {url|null}` (R-302): the pull request of
    /// `branch`, looked up from the worktree `dir` (which knows the remote).
    pub async fn pr_for_branch(
        &self,
        dir: &Path,
        branch: &str,
    ) -> Result<Option<String>, PluginError> {
        let name = "pr-for-branch";
        self.no_prs(name)?;
        if self.is_gitlab() {
            return self.gl_pr_for_branch(dir, branch).await;
        }
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
        self.no_prs("pr-status")?;
        if self.is_gitlab() {
            return self.gl_pr_snapshot(dir, url).await;
        }
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
                .gh(
                    gh,
                    dir,
                    &[
                        "pr",
                        "view",
                        url,
                        "--json",
                        "state,mergeable,mergeStateStatus,baseRefName,reviewRequests,latestReviews",
                    ],
                )
                .await?;
            let s = github_status(&self.json("pr view", &s)?);
            // `gh pr checks` exits 1 when a check failed and 8 while some are
            // pending, with the list on stdout; 1 with nothing on stdout is
            // "no checks reported" (or a real failure, told by stderr).
            let out = self
                .gh_raw(
                    gh,
                    dir,
                    &["pr", "checks", url, "--json", "name,bucket,link,workflow"],
                )
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
        let mut snap = parse_status(&status).map_err(|e| self.shape("pr-status", &e))?;
        snap.checks = parse_checks(&checks).map_err(|e| self.shape("pr-checks", &e))?;
        Ok(snap)
    }

    /// `pr-comments {url}` (R-304): every comment on the pull request at
    /// `url`: line comments, review texts and the conversation. Built-in
    /// `github` reads three endpoints through `gh api`.
    pub async fn pr_comments(&self, dir: &Path, url: &str) -> Result<Vec<Comment>, PluginError> {
        let name = "pr-comments";
        self.no_prs(name)?;
        if self.is_gitlab() {
            return self.gl_pr_comments(dir, url).await;
        }
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
        self.no_prs(name)?;
        if self.is_gitlab() {
            return self.gl_pr_reply(dir, url, to, body).await;
        }
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

    /// `pr-template {} → [{path, name, body}]` (R-153): the repository's
    /// pull-request templates. Built-in `github` reads the files of the
    /// worktree (it does not look up organisation defaults).
    pub async fn pr_template(&self, dir: &Path) -> Result<Vec<Template>, PluginError> {
        let name = "pr-template";
        if self.plugin.builtin() {
            return Ok(local_templates(dir));
        }
        let v = run_command(&self.plugin, name, json!({"worktree": dir}), &self.ctx).await?;
        serde_json::from_value(v).map_err(|e| self.shape(name, &e.to_string()))
    }

    pub(crate) fn json(&self, name: &str, bytes: &[u8]) -> Result<Value, PluginError> {
        serde_json::from_slice(bytes).map_err(|e| PluginError::BadJson {
            plugin: self.plugin.name().into(),
            name: name.into(),
            error: e.to_string(),
        })
    }

    pub(crate) fn shape(&self, name: &str, error: &str) -> PluginError {
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
        self.cli_raw(gh, "GH_PROMPT_DISABLED", "GH_TOKEN", dir, args)
            .await
    }

    /// Run a host's CLI (`gh`, `glab`) in `dir`: colour and prompts off
    /// (`prompt_off` is the variable the CLI reads for that), the `token`
    /// secret, if set, as `token_env`.
    pub(crate) async fn cli_raw(
        &self,
        bin: &str,
        prompt_off: &str,
        token_env: &str,
        dir: &Path,
        args: &[&str],
    ) -> Result<xherd_core::exec::Output, PluginError> {
        let secs = self.plugin.manifest.timeout_seconds;
        let mut argv = vec![bin.to_string()];
        argv.extend(args.iter().map(|s| s.to_string()));
        let mut cmd = Cmd::new(argv)
            .cwd(dir)
            .timeout(Duration::from_secs(secs))
            .env(prompt_off, "1")
            .env("NO_COLOR", "1")
            .as_agent();
        if let Some(t) = self.ctx.secrets.get("token").and_then(Value::as_str)
            && !t.is_empty()
        {
            cmd = cmd.env(token_env, t).redact(vec![t.to_string()]);
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
        self.ok(out, args)
    }

    /// The stdout of a CLI run that exited 0, else the failure.
    pub(crate) fn ok(
        &self,
        out: xherd_core::exec::Output,
        args: &[&str],
    ) -> Result<Vec<u8>, PluginError> {
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

/// `{state, mergeable?, base?, reviewers?}` of `pr-status`, as a snapshot
/// without checks. State is `open`, `merged` or `closed` in any case;
/// `mergeable` is a bool or text (`MERGEABLE`, `CONFLICTING`, `yes`, `no`…)
/// and unknown when absent or not recognised. `reviewers` is
/// `[{name, state}]`, state one of `pending`, `approved`,
/// `changes-requested`, `commented`, `dismissed` (any case, `_` for `-`).
pub fn parse_status(v: &Value) -> Result<Snapshot, String> {
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
    let base = match &v["base"] {
        Value::Null => None,
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => return Err("base is not a branch name".into()),
    };
    let reviewers = match &v["reviewers"] {
        Value::Null => Vec::new(),
        Value::Array(list) => list
            .iter()
            .map(|r| {
                let name = r["name"].as_str().ok_or("a reviewer has no name")?;
                let s = r["state"].as_str().unwrap_or_default();
                let state = match s.to_ascii_lowercase().replace('_', "-").as_str() {
                    "pending" => ReviewState::Pending,
                    "approved" => ReviewState::Approved,
                    "changes-requested" => ReviewState::ChangesRequested,
                    "commented" => ReviewState::Commented,
                    "dismissed" => ReviewState::Dismissed,
                    _ => return Err(format!("reviewer {name} has an unknown state {s:?}")),
                };
                Ok(Reviewer {
                    name: name.to_string(),
                    state,
                })
            })
            .collect::<Result<_, String>>()?,
        _ => return Err("reviewers is not a list".into()),
    };
    Ok(Snapshot {
        state,
        mergeable,
        checks: Vec::new(),
        reviewers,
        base,
    })
}

/// `gh pr view --json state,mergeable,mergeStateStatus,baseRefName,
/// reviewRequests,latestReviews` in the `pr-status` shape. A reviewer asked
/// again after reviewing is `pending`; a merge state of `DIRTY` stands in
/// for a `mergeable` GitHub has not computed yet.
fn github_status(v: &Value) -> Value {
    let mut reviewers: Vec<Value> = Vec::new();
    let requested = v["reviewRequests"].as_array().into_iter().flatten();
    for r in requested {
        // Users have a login, teams a slug (or at least a name).
        if let Some(name) = ["login", "slug", "name"]
            .iter()
            .find_map(|k| r[*k].as_str().filter(|s| !s.is_empty()))
        {
            reviewers.push(json!({"name": name, "state": "pending"}));
        }
    }
    for r in v["latestReviews"].as_array().into_iter().flatten() {
        let (Some(name), Some(state)) = (r["author"]["login"].as_str(), r["state"].as_str()) else {
            continue;
        };
        if !reviewers.iter().any(|x| x["name"] == name) {
            reviewers.push(json!({"name": name, "state": state}));
        }
    }
    let mergeable = match (v["mergeable"].as_str(), v["mergeStateStatus"].as_str()) {
        (Some("UNKNOWN") | None, Some("DIRTY")) => json!("CONFLICTING"),
        (m, _) => json!(m),
    };
    json!({
        "state": v["state"],
        "mergeable": mergeable,
        "base": v["baseRefName"],
        "reviewers": reviewers,
    })
}

/// `[{name, bucket, workflow?, link?}]` of `pr-checks`; an empty `workflow`
/// or `link` counts as absent.
pub fn parse_checks(v: &Value) -> Result<Vec<CheckRun>, String> {
    let list = v.as_array().ok_or("expected a list of checks")?;
    let text = |c: &Value, k: &str| c[k].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    list.iter()
        .map(|c| {
            Ok(CheckRun {
                name: c["name"].as_str().ok_or("a check has no name")?.to_string(),
                bucket: c["bucket"]
                    .as_str()
                    .ok_or("a check has no bucket")?
                    .to_ascii_lowercase(),
                workflow: text(c, "workflow"),
                link: text(c, "link"),
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
    fn templates_are_found_where_hosts_look() {
        let d = tempfile::tempdir().unwrap();
        let w = |p: &str, t: &str| {
            let f = d.path().join(p);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        };
        assert!(local_templates(d.path()).is_empty());
        w(
            ".github/pull_request_template.md",
            "## Summary\n- [ ] tests\n",
        );
        w(".github/PULL_REQUEST_TEMPLATE/bug.md", "## Bug\n");
        w(".github/PULL_REQUEST_TEMPLATE/feature.md", "## Feature\n");
        w(".github/PULL_REQUEST_TEMPLATE/notes.txt", "not markdown");
        w(".gitlab/merge_request_templates/Default.md", "## MR\n");
        let t = local_templates(d.path());
        let names: Vec<_> = t.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(
            names,
            [
                ".github/pull_request_template.md",
                ".github/PULL_REQUEST_TEMPLATE/bug.md",
                ".github/PULL_REQUEST_TEMPLATE/feature.md",
                ".gitlab/merge_request_templates/Default.md",
            ]
        );
        assert_eq!(
            (t[0].name.as_str(), t[0].body.as_str()),
            ("pull_request_template", "## Summary\n- [ ] tests\n")
        );
        // Too large, and a symlink out of the worktree: skipped.
        w("docs/pull_request_template.md", &"x".repeat(70_000));
        std::os::unix::fs::symlink("/etc/hosts", d.path().join("pull_request_template.md"))
            .unwrap();
        assert_eq!(local_templates(d.path()).len(), 4);
    }

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
        let s = |v: Value| {
            let s = parse_status(&v).unwrap();
            (s.state, s.mergeable)
        };
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

        let p = parse_status(&json!({"state": "open", "base": "main", "reviewers": [
            {"name": "ann", "state": "CHANGES_REQUESTED"},
            {"name": "core", "state": "pending"},
        ]}))
        .unwrap();
        assert_eq!(p.base.as_deref(), Some("main"));
        assert_eq!(
            p.reviewers
                .iter()
                .map(|r| (r.name.as_str(), r.state))
                .collect::<Vec<_>>(),
            [
                ("ann", ReviewState::ChangesRequested),
                ("core", ReviewState::Pending)
            ]
        );
        let bad = json!({"state": "open", "reviewers": [{"name": "x", "state": "meh"}]});
        assert!(parse_status(&bad).is_err());
    }

    #[test]
    fn gh_view_becomes_the_status_shape() {
        let v = json!({
            "state": "OPEN",
            "mergeable": "UNKNOWN",
            "mergeStateStatus": "DIRTY",
            "baseRefName": "main",
            "reviewRequests": [
                {"__typename": "User", "login": "ann"},
                {"__typename": "Team", "name": "Core", "slug": "core"},
            ],
            "latestReviews": [
                {"author": {"login": "ann"}, "state": "COMMENTED"},
                {"author": {"login": "bob"}, "state": "APPROVED"},
            ],
        });
        let s = parse_status(&github_status(&v)).unwrap();
        assert_eq!(
            s.mergeable,
            Mergeable::No,
            "DIRTY when GitHub has no answer"
        );
        assert_eq!(s.base.as_deref(), Some("main"));
        assert_eq!(
            s.reviewers
                .iter()
                .map(|r| (r.name.as_str(), r.state))
                .collect::<Vec<_>>(),
            [
                ("ann", ReviewState::Pending),
                ("core", ReviewState::Pending),
                ("bob", ReviewState::Approved)
            ],
            "asked again = pending"
        );
        let plain =
            json!({"state": "OPEN", "mergeable": "MERGEABLE", "mergeStateStatus": "BLOCKED"});
        assert_eq!(
            parse_status(&github_status(&plain)).unwrap().mergeable,
            Mergeable::Yes
        );
    }

    #[test]
    fn checks_parse_and_lowercase_buckets() {
        let c = parse_checks(&json!([
            {"name": "ci", "bucket": "FAIL", "workflow": "", "link": "https://x/1"}
        ]))
        .unwrap();
        assert_eq!((c[0].name.as_str(), c[0].bucket.as_str()), ("ci", "fail"));
        assert_eq!(
            (c[0].workflow.as_deref(), c[0].link.as_deref()),
            (None, Some("https://x/1"))
        );
        assert!(parse_checks(&json!([{"bucket": "pass"}])).is_err());
        assert!(parse_checks(&json!({})).is_err());
    }
}
