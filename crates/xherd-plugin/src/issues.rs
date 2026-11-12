//! Reading a ticket and its comments through a tracker plugin (`ticket-get`).
//! The built-in `github-issues` asks `gh issue view`, `gitlab-issues` asks
//! `glab api`; an external tracker answers the command through the protocol.
//! Everything here is read-only: nothing is posted to a tracker.
//!
//! The text is written by other people, so it is untrusted: the UI renders it
//! through the sanitiser, and anything that hands it to an agent must wrap it
//! (R-1201).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xherd_core::exec::{Cmd, run};
use xherd_core::pr::Comment;

use crate::gitlab::{GITLAB_ISSUES, remote_project};
use crate::host::Host;
use crate::{PluginError, run_command};

pub const GITHUB_ISSUES: &str = "github-issues";

/// The ticket and its comments, oldest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketView {
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// `open` or `closed` (or the tracker's own word).
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub comments: Vec<Comment>,
}

/// `gh issue view --json title,body,state,url,author,comments` as a view.
pub fn github_view(v: &Value) -> Result<TicketView, String> {
    let title = v["title"].as_str().ok_or("the issue has no title")?;
    let comments = v["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let body = c["body"].as_str().filter(|b| !b.trim().is_empty())?;
            let id = c["id"]
                .as_str()
                .map(str::to_string)
                .or_else(|| c["id"].as_u64().map(|n| n.to_string()))?;
            Some(Comment {
                id: format!("issue:{id}"),
                kind: "issue".into(),
                author: c["author"]["login"].as_str().unwrap_or("unknown").into(),
                body: body.into(),
                url: c["url"].as_str().unwrap_or_default().into(),
                created_at: c["createdAt"].as_str().unwrap_or_default().into(),
                path: None,
                line: None,
                in_reply_to: None,
            })
        })
        .collect();
    Ok(TicketView {
        title: title.into(),
        body: v["body"].as_str().unwrap_or_default().into(),
        state: v["state"].as_str().unwrap_or_default().to_ascii_lowercase(),
        url: v["url"].as_str().unwrap_or_default().into(),
        author: v["author"]["login"].as_str().unwrap_or_default().into(),
        comments,
    })
}

/// A GitLab issue (`projects/:p/issues/N`) and its notes as a view; system
/// notes ("changed the description") are left out.
pub fn gitlab_view(issue: &Value, notes: &[Value]) -> Result<TicketView, String> {
    let title = issue["title"].as_str().ok_or("the issue has no title")?;
    let url = issue["web_url"].as_str().unwrap_or_default();
    let mut comments: Vec<Comment> = notes
        .iter()
        .filter(|n| n["system"].as_bool() != Some(true))
        .filter_map(|n| {
            let (id, body) = (
                n["id"].as_u64()?,
                n["body"].as_str().filter(|b| !b.trim().is_empty())?,
            );
            Some(Comment {
                id: format!("issue:{id}"),
                kind: "issue".into(),
                author: n["author"]["username"].as_str().unwrap_or("unknown").into(),
                body: body.into(),
                url: format!("{url}#note_{id}"),
                created_at: n["created_at"].as_str().unwrap_or_default().into(),
                path: None,
                line: None,
                in_reply_to: None,
            })
        })
        .collect();
    // The API lists newest first unless asked; keep them oldest first.
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(TicketView {
        title: title.into(),
        body: issue["description"].as_str().unwrap_or_default().into(),
        state: match issue["state"].as_str() {
            Some("opened") => "open".into(),
            Some(s) => s.into(),
            None => String::new(),
        },
        url: url.into(),
        author: issue["author"]["username"]
            .as_str()
            .unwrap_or_default()
            .into(),
        comments,
    })
}

impl Host {
    /// `ticket-get {key, issue, worktree} → TicketView`: the ticket named by
    /// `issue` (the reference the tracker's `key` pattern picked out of the
    /// agent's ticket key), read from the repository in `dir`.
    pub async fn ticket_get(
        &self,
        dir: &Path,
        key: &str,
        issue: &str,
    ) -> Result<TicketView, PluginError> {
        let name = "ticket-get";
        if !self.plugin.builtin() {
            let v = run_command(
                &self.plugin,
                name,
                json!({"key": key, "issue": issue, "worktree": dir}),
                &self.ctx,
            )
            .await?;
            return serde_json::from_value(v).map_err(|e| self.shape(name, &e.to_string()));
        }
        // The reference goes onto a command line or into an API path.
        if issue.is_empty()
            || !issue
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(self.shape(name, &format!("{issue:?} is not an issue reference")));
        }
        match self.plugin.name() {
            GITHUB_ISSUES => {
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
                            "issue",
                            "view",
                            issue,
                            "--json",
                            "title,body,state,url,author,comments",
                        ],
                    )
                    .await?;
                github_view(&self.json("issue view", &out)?).map_err(|e| self.shape(name, &e))
            }
            GITLAB_ISSUES => {
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
                let (host, project) = remote_project(
                    String::from_utf8_lossy(&remote.stdout).trim(),
                )
                .ok_or_else(|| self.shape(name, "the worktree's origin is not a GitLab project"))?;
                let base = format!("projects/{}/issues/{issue}", project.replace('/', "%2F"));
                let issue_json = self.glab_get(dir, &host, &base).await?;
                let notes = self
                    .glab_list(
                        dir,
                        &host,
                        &format!("{base}/notes?sort=asc&order_by=created_at&per_page=100"),
                    )
                    .await?;
                gitlab_view(&issue_json, &notes).map_err(|e| self.shape(name, &e))
            }
            other => Err(self.shape(
                name,
                &format!("{other} can link tickets but cannot read them yet"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_github_issue_and_its_comments() {
        let v = json!({
            "title": "Crash on save", "body": "It **crashes**.", "state": "OPEN",
            "url": "https://github.com/a/b/issues/12", "author": {"login": "ann"},
            "comments": [
                {"id": "IC_1", "author": {"login": "bob"}, "body": "Same here", "createdAt": "2026-01-02T00:00:00Z", "url": "https://github.com/a/b/issues/12#issuecomment-1"},
                {"id": "IC_2", "author": {"login": "bot"}, "body": "  ", "createdAt": "2026-01-03T00:00:00Z"}
            ]
        });
        let t = github_view(&v).unwrap();
        assert_eq!(
            (t.title.as_str(), t.state.as_str(), t.author.as_str()),
            ("Crash on save", "open", "ann")
        );
        assert_eq!(t.comments.len(), 1, "an empty comment is left out");
        assert_eq!(t.comments[0].id, "issue:IC_1");
        assert_eq!(t.comments[0].author, "bob");
        assert!(github_view(&json!({})).is_err());
    }

    #[test]
    fn a_gitlab_issue_and_its_notes() {
        let issue = json!({"title": "Slow", "description": "d", "state": "opened",
            "web_url": "https://gl.test/g/p/-/issues/3", "author": {"username": "cy"}});
        let notes = [
            json!({"id": 9, "body": "later", "created_at": "2026-02-02", "author": {"username": "bob"}}),
            json!({"id": 8, "body": "changed the description", "system": true, "created_at": "2026-02-01", "author": {"username": "x"}}),
            json!({"id": 7, "body": "first", "created_at": "2026-02-01", "author": {"username": "ann"}}),
        ];
        let t = gitlab_view(&issue, &notes).unwrap();
        assert_eq!((t.state.as_str(), t.author.as_str()), ("open", "cy"));
        let ids: Vec<_> = t.comments.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            ["issue:7", "issue:9"],
            "system notes dropped, oldest first"
        );
        assert_eq!(t.comments[0].url, "https://gl.test/g/p/-/issues/3#note_7");
    }

    #[cfg(unix)]
    fn fake(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join(name);
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho \"$@\" >> {}/calls\n{body}\n",
                dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    #[cfg(unix)]
    fn tracker_host(
        dir: &std::path::Path,
        plugin: &str,
        setting: &str,
        bin: &std::path::Path,
    ) -> Host {
        let reg = crate::Registry::load(crate::builtins(), dir);
        let mut ctx = crate::Context::default();
        ctx.settings
            .insert(setting.into(), bin.to_str().unwrap().into());
        Host {
            plugin: reg.get(plugin).unwrap().clone(),
            ctx,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_cli_is_asked_for_the_ticket_and_never_for_more() {
        let d = tempfile::tempdir().unwrap();
        let gh = fake(
            d.path(),
            "gh",
            r#"echo '{"title":"T","body":"b","state":"OPEN","url":"u","author":{"login":"a"},"comments":[{"id":"1","author":{"login":"c"},"body":"hi","createdAt":"t"}]}'"#,
        );
        let host = tracker_host(d.path(), "github-issues", "gh", &gh);
        let t = host.ticket_get(d.path(), "GH-12", "12").await.unwrap();
        assert_eq!((t.title.as_str(), t.comments.len()), ("T", 1));
        let calls = std::fs::read_to_string(d.path().join("calls")).unwrap();
        assert_eq!(
            calls.trim(),
            "issue view 12 --json title,body,state,url,author,comments"
        );
        // A reference that is not a plain name never reaches the CLI.
        assert!(
            host.ticket_get(d.path(), "k", "12; rm -rf /")
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(d.path().join("calls"))
                .unwrap()
                .lines()
                .count(),
            1
        );

        // GitLab: the project comes from the worktree's origin.
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
        let glab = fake(
            d.path(),
            "glab",
            r#"case "$*" in
  *"/notes"*) echo '[{"id":5,"body":"note","created_at":"2026","author":{"username":"u"}}]' ;;
  *) echo '{"title":"G","description":"d","state":"opened","web_url":"https://gl.test/g/p/-/issues/3","author":{"username":"a"}}' ;;
esac"#,
        );
        let host = tracker_host(d.path(), "gitlab-issues", "glab", &glab);
        let t = host.ticket_get(&repo, "GL-3", "3").await.unwrap();
        assert_eq!(
            (t.title.as_str(), t.state.as_str(), t.comments.len()),
            ("G", "open", 1)
        );
        let calls = std::fs::read_to_string(d.path().join("calls")).unwrap();
        assert!(
            calls.contains("api --hostname gl.test projects/g%2Fp/issues/3"),
            "{calls}"
        );
        assert!(
            calls.contains("projects/g%2Fp/issues/3/notes?sort=asc"),
            "{calls}"
        );
    }

    #[tokio::test]
    async fn a_link_only_tracker_says_it_cannot_read() {
        let d = tempfile::tempdir().unwrap();
        let reg = crate::Registry::load(crate::builtins(), d.path());
        let host = Host {
            plugin: reg.get("jira").unwrap().clone(),
            ctx: crate::Context::default(),
        };
        let e = host
            .ticket_get(d.path(), "PAY-1", "PAY-1")
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("cannot read them yet"), "{e}");
    }
}
