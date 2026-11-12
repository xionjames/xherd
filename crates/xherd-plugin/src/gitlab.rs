//! The built-in `gitlab` host plugin's pull-request commands (D24 step 3):
//! merge requests through `glab api`, which answers GitLab's REST API as
//! JSON. The parsers are pure and turn GitLab's shapes into the ones the
//! command protocol uses (`parse_status`, `parse_checks`, `Comment`).
//!
//! Every part that reaches a command line (project path, branch, ids) is a
//! plain name checked here; nothing goes through a shell.

use std::path::Path;

use serde_json::{Value, json};
use xherd_core::exec::{Cmd, run};
use xherd_core::pr::{Comment, Snapshot};

use crate::PluginError;
use crate::host::{Host, parse_checks, parse_status, parse_stream};

/// The plugin's name in the registry.
pub const GITLAB: &str = "gitlab";

fn plain(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// `a/b/c` → `a%2Fb%2Fc`, the form of a project path in an API URL.
fn project_id(path: &str) -> String {
    path.replace('/', "%2F")
}

/// `https://host/group/sub/project/-/merge_requests/7` → (host, project
/// path, 7). Plain names only.
pub fn parse_mr_url(url: &str) -> Option<(String, String, u64)> {
    let rest = url.strip_prefix("https://")?.trim_end_matches('/');
    let (host_path, number) = rest.split_once("/-/merge_requests/")?;
    let (host, path) = host_path.split_once('/')?;
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | ':'));
    (host_ok && path.split('/').all(plain))
        .then(|| number.parse().ok())
        .flatten()
        .map(|n| (host.into(), path.into(), n))
}

/// (host, project path) of a remote URL (`https://h/g/p.git`,
/// `git@h:g/p.git`, `ssh://git@h/g/p`).
pub fn remote_project(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let rest = rest.rsplit_once('@').map_or(rest, |(_, r)| r);
        rest.split_once('/')?
    } else {
        url.split_once('@')?.1.split_once(':')?
    };
    // A port (`host:8443`) is not part of the API host's name for ssh.
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    (!host.is_empty() && path.split('/').all(plain)).then(|| (host.into(), path.into()))
}

/// A branch name usable in a query string: plain names with `/`.
fn branch_query(b: &str) -> Option<String> {
    b.split('/').all(plain).then(|| b.replace('/', "%2F"))
}

/// A merge request (and its approvals) in the `pr-status` shape.
/// `has_conflicts` says conflict; a merge status GitLab has not computed
/// yet, or a branch that needs a rebase, is unknown.
pub fn status(mr: &Value, approvals: &Value) -> Value {
    let state = match mr["state"].as_str() {
        Some("opened" | "locked") => json!("open"),
        other => json!(other),
    };
    let mergeable = match (
        mr["has_conflicts"].as_bool(),
        mr["detailed_merge_status"].as_str(),
    ) {
        (Some(true), _) => "conflicting",
        (_, Some("checking" | "unchecked" | "preparing" | "need_rebase") | None) => "unknown",
        _ => "mergeable",
    };
    let approved: Vec<&str> = approvals["approved_by"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a["user"]["username"].as_str())
        .collect();
    let mut reviewers: Vec<Value> = mr["reviewers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["username"].as_str())
        .map(|n| json!({"name": n, "state": if approved.contains(&n) { "approved" } else { "pending" }}))
        .collect();
    for n in approved {
        if !reviewers.iter().any(|r| r["name"] == n) {
            reviewers.push(json!({"name": n, "state": "approved"}));
        }
    }
    json!({
        "state": state,
        "mergeable": mergeable,
        "base": mr["target_branch"],
        "reviewers": reviewers,
    })
}

/// A pipeline's jobs in the `pr-checks` shape. A failed job that is allowed
/// to fail does not count as failing.
pub fn checks(jobs: &[Value]) -> Value {
    Value::Array(
        jobs.iter()
            .filter_map(|j| {
                let bucket = match j["status"].as_str()? {
                    "success" => "pass",
                    "failed" if j["allow_failure"].as_bool() == Some(true) => "skipping",
                    "failed" => "fail",
                    "canceled" | "canceling" => "cancel",
                    "skipped" => "skipping",
                    _ => "pending",
                };
                Some(json!({
                    "name": j["name"],
                    "bucket": bucket,
                    "workflow": j["stage"],
                    "link": j["web_url"],
                }))
            })
            .collect(),
    )
}

/// Comments of `merge_requests/N/discussions`, system notes left out. The
/// id is `<kind>:<discussion>:<note>`: a reply goes to the discussion, or
/// to the conversation for an individual note (which cannot be threaded).
pub fn comments(discussions: &[Value], mr_url: &str) -> Vec<Comment> {
    let mut out = Vec::new();
    for d in discussions {
        let (Some(did), Some(notes)) = (d["id"].as_str(), d["notes"].as_array()) else {
            continue;
        };
        let kind = if d["individual_note"].as_bool() == Some(true) {
            "issue"
        } else {
            "review-comment"
        };
        let root = notes
            .first()
            .filter(|n| n["system"].as_bool() != Some(true))
            .and_then(|n| n["id"].as_u64())
            .map(|id| format!("{kind}:{did}:{id}"));
        for (i, n) in notes.iter().enumerate() {
            let (Some(id), Some(body)) = (
                n["id"].as_u64(),
                n["body"].as_str().filter(|b| !b.trim().is_empty()),
            ) else {
                continue;
            };
            if n["system"].as_bool() == Some(true) {
                continue;
            }
            let pos = &n["position"];
            out.push(Comment {
                id: format!("{kind}:{did}:{id}"),
                kind: kind.into(),
                author: n["author"]["username"].as_str().unwrap_or("unknown").into(),
                body: body.into(),
                url: format!("{mr_url}#note_{id}"),
                created_at: n["created_at"].as_str().unwrap_or_default().into(),
                path: pos["new_path"]
                    .as_str()
                    .or_else(|| pos["old_path"].as_str())
                    .map(str::to_string),
                line: pos["new_line"]
                    .as_u64()
                    .or_else(|| pos["old_line"].as_u64())
                    .map(|n| n as u32),
                in_reply_to: if i > 0 { root.clone() } else { None },
            });
        }
    }
    out
}

/// Where a reply to comment `to` goes, under the MR's API path.
fn reply_path(to: &str, mr: &str) -> Option<String> {
    let mut p = to.splitn(3, ':');
    let (kind, disc, note) = (p.next()?, p.next()?, p.next()?);
    let hex = !disc.is_empty() && disc.chars().all(|c| c.is_ascii_hexdigit());
    (hex && note.parse::<u64>().is_ok()).then_some(())?;
    match kind {
        "issue" => Some(format!("{mr}/notes")),
        "review-comment" => Some(format!("{mr}/discussions/{disc}/notes")),
        _ => None,
    }
}

impl Host {
    fn glab_bin(&self) -> String {
        self.ctx
            .settings
            .get("glab")
            .and_then(Value::as_str)
            .unwrap_or("glab")
            .to_string()
    }

    /// `glab api` against `host`; `args` follow `api --hostname <host>`.
    async fn glab_api(
        &self,
        dir: &Path,
        host: &str,
        args: &[&str],
    ) -> Result<xherd_core::exec::Output, PluginError> {
        let mut a = vec!["api", "--hostname", host];
        a.extend(args);
        self.cli_raw(&self.glab_bin(), "GLAB_NO_PROMPT", "GITLAB_TOKEN", dir, &a)
            .await
    }

    async fn glab_get(&self, dir: &Path, host: &str, path: &str) -> Result<Value, PluginError> {
        let args = [path];
        let out = self.glab_api(dir, host, &args).await?;
        let bytes = self.ok(out, &args)?;
        self.json(path, &bytes)
    }

    async fn glab_list(
        &self,
        dir: &Path,
        host: &str,
        path: &str,
    ) -> Result<Vec<Value>, PluginError> {
        let args = ["--paginate", path];
        let out = self.glab_api(dir, host, &args).await?;
        let bytes = self.ok(out, &args)?;
        parse_stream(&bytes).map_err(|e| self.shape(path, &e))
    }

    fn mr(&self, name: &str, url: &str) -> Result<(String, String, u64), PluginError> {
        parse_mr_url(url)
            .ok_or_else(|| self.shape(name, &format!("{url} is not a merge request URL")))
    }

    pub(crate) async fn gl_pr_for_branch(
        &self,
        dir: &Path,
        branch: &str,
    ) -> Result<Option<String>, PluginError> {
        let name = "pr-for-branch";
        let remote = run(Cmd::new([
            "git",
            "-C",
            &dir.display().to_string(),
            "config",
            "--get",
            "remote.origin.url",
        ])
        .as_agent())
        .await?;
        let (host, project) = remote_project(String::from_utf8_lossy(&remote.stdout).trim())
            .ok_or_else(|| self.shape(name, "the worktree's origin is not a GitLab project"))?;
        let b = branch_query(branch)
            .ok_or_else(|| self.shape(name, &format!("{branch:?} is not a plain branch name")))?;
        let path = format!(
            "projects/{}/merge_requests?source_branch={b}&state=all&per_page=1",
            project_id(&project)
        );
        let list = self.glab_get(dir, &host, &path).await?;
        Ok(list
            .as_array()
            .and_then(|l| l.first())
            .and_then(|m| m["web_url"].as_str())
            .map(str::to_string))
    }

    pub(crate) async fn gl_pr_snapshot(
        &self,
        dir: &Path,
        url: &str,
    ) -> Result<Snapshot, PluginError> {
        let (host, project, n) = self.mr("pr-status", url)?;
        let base = format!("projects/{}/merge_requests/{n}", project_id(&project));
        let mr = self.glab_get(dir, &host, &base).await?;
        let approvals = self
            .glab_get(dir, &host, &format!("{base}/approvals"))
            .await
            .unwrap_or(Value::Null);
        let jobs = match mr["head_pipeline"]["id"].as_u64() {
            Some(id) => {
                let p = format!(
                    "projects/{}/pipelines/{id}/jobs?per_page=100",
                    project_id(&project)
                );
                self.glab_list(dir, &host, &p).await?
            }
            None => Vec::new(),
        };
        let mut snap =
            parse_status(&status(&mr, &approvals)).map_err(|e| self.shape("pr-status", &e))?;
        snap.checks = parse_checks(&checks(&jobs)).map_err(|e| self.shape("pr-checks", &e))?;
        Ok(snap)
    }

    pub(crate) async fn gl_pr_comments(
        &self,
        dir: &Path,
        url: &str,
    ) -> Result<Vec<Comment>, PluginError> {
        let (host, project, n) = self.mr("pr-comments", url)?;
        let path = format!(
            "projects/{}/merge_requests/{n}/discussions?per_page=100",
            project_id(&project)
        );
        let list = self.glab_list(dir, &host, &path).await?;
        Ok(comments(&list, url))
    }

    pub(crate) async fn gl_pr_reply(
        &self,
        dir: &Path,
        url: &str,
        to: &str,
        body: &str,
    ) -> Result<(), PluginError> {
        let name = "pr-reply";
        let (host, project, n) = self.mr(name, url)?;
        let mr = format!("projects/{}/merge_requests/{n}", project_id(&project));
        let path = reply_path(to, &mr)
            .ok_or_else(|| self.shape(name, &format!("{to} is not a comment id")))?;
        let field = format!("body={body}");
        let args = ["-X", "POST", path.as_str(), "-f", &field];
        let out = self.glab_api(dir, &host, &args).await?;
        self.ok(out, &args)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_and_remotes() {
        assert_eq!(
            parse_mr_url("https://gitlab.com/g/sub/p/-/merge_requests/7"),
            Some(("gitlab.com".into(), "g/sub/p".into(), 7))
        );
        assert_eq!(
            parse_mr_url("https://gitlab.com/g/p/-/merge_requests/x"),
            None
        );
        assert_eq!(parse_mr_url("https://h/g/p/pull/7"), None);
        assert_eq!(parse_mr_url("https://h/g/p q/-/merge_requests/7"), None);
        for r in [
            "https://gitlab.com/g/p.git",
            "git@gitlab.com:g/p.git",
            "ssh://git@gitlab.com/g/p",
        ] {
            assert_eq!(
                remote_project(r),
                Some(("gitlab.com".into(), "g/p".into())),
                "{r}"
            );
        }
        assert_eq!(remote_project("/local/path"), None);
        assert_eq!(branch_query("feat/x-1").as_deref(), Some("feat%2Fx-1"));
        assert_eq!(branch_query("a&b=c"), None);
    }

    #[test]
    fn a_merge_request_becomes_the_status_shape() {
        let mr = json!({
            "state": "opened", "target_branch": "main", "has_conflicts": false,
            "detailed_merge_status": "ci_must_pass",
            "reviewers": [{"username": "ann"}, {"username": "bob"}]
        });
        let ap =
            json!({"approved_by": [{"user": {"username": "bob"}}, {"user": {"username": "cy"}}]});
        let snap = parse_status(&status(&mr, &ap)).unwrap();
        assert_eq!(snap.state, xherd_core::pr::PrState::Open);
        assert_eq!(snap.mergeable, xherd_core::pr::Mergeable::Yes);
        assert_eq!(snap.base.as_deref(), Some("main"));
        let names: Vec<_> = snap.reviewers.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["ann", "bob", "cy"]);
        use xherd_core::pr::ReviewState::*;
        let states: Vec<_> = snap.reviewers.iter().map(|r| r.state).collect();
        assert_eq!(states, [Pending, Approved, Approved]);

        let conflict = json!({"state": "opened", "has_conflicts": true, "detailed_merge_status": "broken_status"});
        assert_eq!(
            parse_status(&status(&conflict, &Value::Null))
                .unwrap()
                .mergeable,
            xherd_core::pr::Mergeable::No
        );
        let fresh =
            json!({"state": "opened", "has_conflicts": false, "detailed_merge_status": "checking"});
        assert_eq!(
            parse_status(&status(&fresh, &Value::Null))
                .unwrap()
                .mergeable,
            xherd_core::pr::Mergeable::Unknown
        );
        let merged = json!({"state": "merged"});
        assert_eq!(
            parse_status(&status(&merged, &Value::Null)).unwrap().state,
            xherd_core::pr::PrState::Merged
        );
    }

    #[test]
    fn jobs_become_checks() {
        let jobs = [
            json!({"name": "build", "status": "success", "stage": "build", "web_url": "https://x/1"}),
            json!({"name": "lint", "status": "failed", "allow_failure": true, "stage": "test"}),
            json!({"name": "test", "status": "failed", "stage": "test"}),
            json!({"name": "deploy", "status": "manual", "stage": "deploy"}),
            json!({"name": "e2e", "status": "running", "stage": "test"}),
        ];
        let c = parse_checks(&checks(&jobs)).unwrap();
        let b: Vec<_> = c.iter().map(|c| c.bucket.as_str()).collect();
        assert_eq!(b, ["pass", "skipping", "fail", "pending", "pending"]);
        assert_eq!(c[0].link.as_deref(), Some("https://x/1"));
        assert_eq!(c[0].workflow.as_deref(), Some("build"));
    }

    #[test]
    fn discussions_become_comments_and_replies_find_their_thread() {
        let d = [
            json!({"id": "abc1", "individual_note": true, "notes": [
                {"id": 1, "body": "looks off", "author": {"username": "ann"}, "created_at": "t"}]}),
            json!({"id": "def2", "individual_note": false, "notes": [
                {"id": 2, "body": "rename this", "author": {"username": "bob"},
                 "position": {"new_path": "src/a.rs", "new_line": 12}},
                {"id": 3, "body": "done?", "author": {"username": "ann"}},
                {"id": 4, "body": "assigned", "system": true, "author": {"username": "x"}}]}),
        ];
        let c = comments(&d, "https://h/g/p/-/merge_requests/7");
        assert_eq!(c.len(), 3);
        assert_eq!(
            (c[0].id.as_str(), c[0].kind.as_str()),
            ("issue:abc1:1", "issue")
        );
        assert_eq!(c[1].path.as_deref(), Some("src/a.rs"));
        assert_eq!(c[1].line, Some(12));
        assert_eq!(c[2].in_reply_to.as_deref(), Some("review-comment:def2:2"));
        assert_eq!(c[1].url, "https://h/g/p/-/merge_requests/7#note_2");

        let mr = "projects/g%2Fp/merge_requests/7";
        assert_eq!(
            reply_path("issue:abc1:1", mr).unwrap(),
            format!("{mr}/notes")
        );
        assert_eq!(
            reply_path("review-comment:def2:2", mr).unwrap(),
            format!("{mr}/discussions/def2/notes")
        );
        assert_eq!(reply_path("review-comment:../x:2", mr), None);
        assert_eq!(reply_path("nonsense", mr), None);
    }

    /// The whole path through a fake `glab`: what it is asked and how its
    /// answers are read.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_poll_and_a_reply_through_a_fake_glab() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("calls");
        let bin = d.path().join("glab");
        std::fs::write(
            &bin,
            format!(
                r#"#!/bin/sh
echo "$@" >> {}
for a; do last="$a"; done
case "$*" in
  *"-X POST"*) echo '{{}}' ;;
  *"/approvals") echo '{{"approved_by":[]}}' ;;
  *"/pipelines/9/jobs"*) echo '[{{"name":"t","status":"failed","stage":"test"}}]' ;;
  *"merge_requests?source_branch=feat%2Fx"*) echo '[{{"web_url":"https://gl.test/g/p/-/merge_requests/3"}}]' ;;
  *"/discussions"*) echo '[{{"id":"ab","individual_note":false,"notes":[{{"id":5,"body":"fix it","author":{{"username":"u"}}}}]}}]' ;;
  *"/merge_requests/3") echo '{{"state":"opened","target_branch":"main","has_conflicts":false,"detailed_merge_status":"mergeable","head_pipeline":{{"id":9}}}}' ;;
esac
"#,
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A worktree whose origin is a GitLab project.
        let repo = d.path().join("repo");
        for args in [
            vec!["init", "-q", repo.to_str().unwrap()],
            vec![
                "-C",
                repo.to_str().unwrap(),
                "remote",
                "add",
                "origin",
                "git@gl.test:g/p.git",
            ],
        ] {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let m = crate::builtins()
            .into_iter()
            .find(|m| m.name == "gitlab")
            .unwrap();
        let mut ctx = crate::Context::default();
        ctx.settings
            .insert("glab".into(), bin.to_str().unwrap().into());
        let host = Host {
            plugin: crate::Registry::load(vec![m], d.path())
                .get("gitlab")
                .unwrap()
                .clone(),
            ctx,
        };
        let url = "https://gl.test/g/p/-/merge_requests/3";

        assert_eq!(
            host.pr_for_branch(&repo, "feat/x")
                .await
                .unwrap()
                .as_deref(),
            Some(url)
        );
        let snap = host.pr_snapshot(&repo, url).await.unwrap();
        assert_eq!(snap.state, xherd_core::pr::PrState::Open);
        assert_eq!(snap.checks.len(), 1);
        assert_eq!(snap.checks[0].bucket, "fail");
        let c = host.pr_comments(&repo, url).await.unwrap();
        assert_eq!(c[0].id, "review-comment:ab:5");
        host.pr_reply(&repo, url, &c[0].id, "ok").await.unwrap();

        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("api --hostname gl.test projects/g%2Fp/merge_requests?source_branch=feat%2Fx&state=all&per_page=1"), "{calls}");
        assert!(
            calls.contains(
                "-X POST projects/g%2Fp/merge_requests/3/discussions/ab/notes -f body=ok"
            ),
            "{calls}"
        );
        // A reply to something that is not a comment id never reaches glab.
        assert!(host.pr_reply(&repo, url, "issue:../:1", "x").await.is_err());
    }
}
