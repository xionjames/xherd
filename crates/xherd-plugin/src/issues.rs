//! Reading a ticket and its comments through a tracker plugin (`ticket-get`).
//! The built-in `github-issues` asks `gh issue view`, `gitlab-issues` asks
//! `glab api`; an external tracker answers the command through the protocol.
//! Reading changes nothing; `ticket-comment` posts only text the user has
//! accepted (R-308).
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
pub const JIRA: &str = "jira";

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

/// Atlassian Document Format (Jira's rich text) as Markdown. Unknown nodes
/// keep their text; nothing here can produce HTML, and the UI sanitises the
/// result anyway.
pub fn adf_to_markdown(v: &Value) -> String {
    fn inline(n: &Value) -> String {
        match n["type"].as_str() {
            Some("text") => {
                let mut t = n["text"].as_str().unwrap_or_default().to_string();
                for m in n["marks"].as_array().into_iter().flatten() {
                    t = match m["type"].as_str() {
                        Some("code") => format!("`{t}`"),
                        Some("strong") => format!("**{t}**"),
                        Some("em") => format!("*{t}*"),
                        Some("strike") => format!("~~{t}~~"),
                        Some("link") => match m["attrs"]["href"].as_str() {
                            Some(h) => format!("[{t}]({h})"),
                            None => t,
                        },
                        _ => t,
                    };
                }
                t
            }
            Some("hardBreak") => "\n".into(),
            Some("mention") => n["attrs"]["text"].as_str().unwrap_or("@someone").into(),
            Some("emoji") => n["attrs"]["text"]
                .as_str()
                .or_else(|| n["attrs"]["shortName"].as_str())
                .unwrap_or_default()
                .into(),
            Some("inlineCard" | "blockCard" | "embedCard") => {
                n["attrs"]["url"].as_str().unwrap_or_default().into()
            }
            _ => children(n, ""),
        }
    }
    fn children(n: &Value, sep: &str) -> String {
        n["content"]
            .as_array()
            .into_iter()
            .flatten()
            .map(inline)
            .collect::<Vec<_>>()
            .join(sep)
    }
    fn block(n: &Value, out: &mut Vec<String>, indent: &str) {
        match n["type"].as_str() {
            Some("paragraph") => out.push(format!("{indent}{}", children(n, ""))),
            Some("heading") => {
                let level = n["attrs"]["level"].as_u64().unwrap_or(1).clamp(1, 6) as usize;
                out.push(format!("{} {}", "#".repeat(level), children(n, "")));
            }
            Some("codeBlock") => out.push(format!("```\n{}\n```", children(n, ""))),
            Some("rule") => out.push("---".into()),
            Some("blockquote" | "panel" | "expand") => {
                let mut inner = Vec::new();
                for c in n["content"].as_array().into_iter().flatten() {
                    block(c, &mut inner, "");
                }
                out.push(
                    inner
                        .join("\n\n")
                        .lines()
                        .map(|l| format!("> {l}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            Some(kind @ ("bulletList" | "orderedList")) => {
                let mut items = Vec::new();
                for (i, item) in n["content"].as_array().into_iter().flatten().enumerate() {
                    let bullet = if kind == "bulletList" {
                        "-".to_string()
                    } else {
                        format!("{}.", i + 1)
                    };
                    let mut inner = Vec::new();
                    for c in item["content"].as_array().into_iter().flatten() {
                        block(c, &mut inner, "");
                    }
                    let text = inner.join("\n");
                    let pad = " ".repeat(bullet.len() + 1);
                    let mut lines = text.lines();
                    let first = lines.next().unwrap_or_default();
                    let rest: String = lines.map(|l| format!("\n{indent}{pad}{l}")).collect();
                    items.push(format!("{indent}{bullet} {first}{rest}"));
                }
                out.push(items.join("\n"));
            }
            Some("table") => {
                for row in n["content"].as_array().into_iter().flatten() {
                    let cells: Vec<String> = row["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|c| {
                            let mut inner = Vec::new();
                            for b in c["content"].as_array().into_iter().flatten() {
                                block(b, &mut inner, "");
                            }
                            inner.join(" ").replace('|', "\\|").replace('\n', " ")
                        })
                        .collect();
                    out.push(format!("| {} |", cells.join(" | ")));
                }
            }
            _ => {
                let t = children(n, "");
                if !t.is_empty() {
                    out.push(t);
                }
            }
        }
    }
    match v {
        Value::String(s) => s.clone(),
        Value::Object(_) => {
            let mut out = Vec::new();
            for c in v["content"].as_array().into_iter().flatten() {
                block(c, &mut out, "");
            }
            out.join("\n\n")
        }
        _ => String::new(),
    }
}

/// `acli ... --paginate --json` prints one JSON document per page, back to
/// back (`{"comments": [...], "isLast": false, ...}{"comments": [], ...}`):
/// the comments of all of them as one list. A bare list is read as a page.
pub fn merge_comment_pages(bytes: &[u8]) -> Result<Value, String> {
    let mut all = Vec::new();
    for page in serde_json::Deserializer::from_slice(bytes).into_iter::<Value>() {
        match page.map_err(|e| e.to_string())? {
            Value::Array(a) => all.extend(a),
            Value::Object(o) => {
                for k in ["comments", "values"] {
                    if let Some(Value::Array(a)) = o.get(k) {
                        all.extend(a.iter().cloned());
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(Value::Array(all))
}

/// `acli jira workitem view KEY --json` and `comment list --json` as a view.
/// The output is Jira's REST shape (`fields.summary`, `fields.status.name`,
/// ADF for `description` and comment bodies, checked against real `acli`
/// output for `view`); a flat object (`summary`, `status` at the top) is read
/// too. `base` (the Jira site, https) builds the ticket's link.
pub fn jira_view(
    key: &str,
    view: &Value,
    comments: &Value,
    base: Option<&str>,
) -> Result<TicketView, String> {
    let f = if view["fields"].is_object() {
        &view["fields"]
    } else {
        view
    };
    let title = f["summary"]
        .as_str()
        .ok_or("the work item has no summary")?;
    let name = |v: &Value| {
        v["displayName"]
            .as_str()
            .or_else(|| v["name"].as_str())
            .or_else(|| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    // `self` is the REST URL on an internal Atlassian host, not the site you
    // browse, so the link comes from the plugin's `baseUrl` setting only.
    let url = base
        .map(|b| format!("{}/browse/{key}", b.trim_end_matches('/')))
        .unwrap_or_default();
    let list = match comments {
        Value::Array(a) => a.as_slice(),
        Value::Object(_) => comments["comments"]
            .as_array()
            .or_else(|| comments["values"].as_array())
            .map(Vec::as_slice)
            .unwrap_or_default(),
        _ => &[],
    };
    let mut out: Vec<Comment> = list
        .iter()
        .filter_map(|c| {
            let body = adf_to_markdown(&c["body"]);
            let id = c["id"]
                .as_str()
                .map(str::to_string)
                .or_else(|| c["id"].as_u64().map(|n| n.to_string()))?;
            (!body.trim().is_empty()).then(|| Comment {
                id: format!("issue:{id}"),
                kind: "issue".into(),
                author: name(&c["author"]),
                body,
                url: if url.is_empty() {
                    String::new()
                } else {
                    format!("{url}?focusedCommentId={id}")
                },
                created_at: c["created"].as_str().unwrap_or_default().into(),
                path: None,
                line: None,
                in_reply_to: None,
            })
        })
        .collect();
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(TicketView {
        title: title.into(),
        body: adf_to_markdown(&f["description"]),
        state: f["status"]["name"]
            .as_str()
            .or_else(|| f["status"].as_str())
            .unwrap_or_default()
            .to_ascii_lowercase(),
        url,
        author: name(&f["reporter"]),
        comments: out,
    })
}

/// An issue reference that can go onto a command line or into an API path:
/// a plain name that cannot be read as an option (no leading `-`).
fn plain_ref(issue: &str) -> bool {
    !issue.is_empty()
        && !issue.starts_with('-')
        && issue
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Longest comment xherd posts: a summary, not a document.
pub const MAX_COMMENT: usize = 10_000;

impl Host {
    /// `ticket-comment {key, issue, body, worktree}`: add a comment to the
    /// ticket. Only ever called with text the user has read and accepted
    /// (R-308). `body` is passed as one argument, never through a shell; the
    /// GitHub and GitLab comments carry xherd's hidden marker.
    pub async fn ticket_comment(
        &self,
        dir: &Path,
        key: &str,
        issue: &str,
        body: &str,
    ) -> Result<(), PluginError> {
        let name = "ticket-comment";
        if body.trim().is_empty() || body.chars().count() > MAX_COMMENT {
            return Err(self.shape(name, &format!("a comment is 1 to {MAX_COMMENT} characters")));
        }
        if body
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r'))
        {
            return Err(self.shape(name, "a comment cannot hold control characters"));
        }
        if !self.plugin.builtin() {
            run_command(
                &self.plugin,
                name,
                json!({"key": key, "issue": issue, "body": body, "worktree": dir}),
                &self.ctx,
            )
            .await?;
            return Ok(());
        }
        if !plain_ref(issue) {
            return Err(self.shape(name, &format!("{issue:?} is not an issue reference")));
        }
        let marked = format!("{body}\n\n{}", xherd_core::pr::MARKER);
        match self.plugin.name() {
            GITHUB_ISSUES => {
                let gh = self
                    .ctx
                    .settings
                    .get("gh")
                    .and_then(Value::as_str)
                    .unwrap_or("gh");
                self.gh(gh, dir, &["issue", "comment", issue, "--body", &marked])
                    .await?;
                Ok(())
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
                let path = format!(
                    "projects/{}/issues/{issue}/notes",
                    project.replace('/', "%2F")
                );
                let field = format!("body={marked}");
                let args = ["-X", "POST", path.as_str(), "-f", &field];
                let out = self.glab_api(dir, &host, &args).await?;
                self.ok(out, &args)?;
                Ok(())
            }
            JIRA => {
                let acli = self
                    .ctx
                    .settings
                    .get("acli")
                    .and_then(Value::as_str)
                    .unwrap_or("acli");
                self.acli(
                    dir,
                    acli,
                    &[
                        "jira", "workitem", "comment", "create", "--key", issue, "--body", body,
                    ],
                )
                .await?;
                Ok(())
            }
            other => Err(self.shape(name, &format!("{other} cannot post comments yet"))),
        }
    }

    /// One `acli` run in `dir`; the CLI keeps its own login.
    async fn acli(&self, dir: &Path, bin: &str, args: &[&str]) -> Result<Vec<u8>, PluginError> {
        let out = self
            .cli_raw(bin, "XHERD_NO_PROMPT", "ACLI_TOKEN", dir, args)
            .await?;
        self.ok(out, args)
    }

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
        if !plain_ref(issue) {
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
            JIRA => {
                let acli = self
                    .ctx
                    .settings
                    .get("acli")
                    .and_then(Value::as_str)
                    .unwrap_or("acli");
                let view = self
                    .acli(
                        dir,
                        acli,
                        &[
                            "jira",
                            "workitem",
                            "view",
                            issue,
                            "--json",
                            "--fields",
                            "summary,status,description,reporter",
                        ],
                    )
                    .await?;
                let comments = self
                    .acli(
                        dir,
                        acli,
                        &[
                            "jira",
                            "workitem",
                            "comment",
                            "list",
                            "--key",
                            issue,
                            "--json",
                            "--paginate",
                        ],
                    )
                    .await?;
                let base = self
                    .ctx
                    .settings
                    .get("baseUrl")
                    .and_then(Value::as_str)
                    .filter(|b| b.starts_with("https://"));
                jira_view(
                    issue,
                    &self.json("workitem view", &view)?,
                    &merge_comment_pages(&comments).map_err(|e| self.shape(name, &e))?,
                    base,
                )
                .map_err(|e| self.shape(name, &e))
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
        // Nor one that the CLI would read as an option.
        assert!(host.ticket_get(d.path(), "k", "--web").await.is_err());
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
    async fn a_tracker_without_reading_says_so() {
        let d = tempfile::tempdir().unwrap();
        let reg = crate::Registry::load(crate::builtins(), d.path());
        let host = Host {
            plugin: reg.get("bitbucket").unwrap().clone(),
            ctx: crate::Context::default(),
        };
        let e = host
            .ticket_get(d.path(), "PAY-1", "PAY-1")
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("cannot read them yet"), "{e}");
    }

    fn adf(content: Value) -> Value {
        json!({"type": "doc", "version": 1, "content": content})
    }

    #[test]
    fn adf_becomes_markdown() {
        let doc = adf(json!([
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Steps"}]},
            {"type": "paragraph", "content": [
                {"type": "text", "text": "Open "},
                {"type": "text", "text": "the app", "marks": [{"type": "strong"}]},
                {"type": "hardBreak"},
                {"type": "text", "text": "see "},
                {"type": "text", "text": "docs", "marks": [{"type": "link", "attrs": {"href": "https://x.test/d"}}]},
                {"type": "mention", "attrs": {"text": "@ann"}}
            ]},
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "one"}]}]},
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "two"}]}]}
            ]},
            {"type": "orderedList", "content": [
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "a"}]}]}
            ]},
            {"type": "codeBlock", "content": [{"type": "text", "text": "let x = 1;"}]},
            {"type": "somethingNew", "content": [{"type": "text", "text": "kept"}]}
        ]));
        let md = adf_to_markdown(&doc);
        assert!(
            md.starts_with("## Steps\n\nOpen **the app**\nsee [docs](https://x.test/d)@ann"),
            "{md}"
        );
        assert!(md.contains("- one\n- two"), "{md}");
        assert!(md.contains("1. a"), "{md}");
        assert!(md.contains("```\nlet x = 1;\n```"), "{md}");
        assert!(md.ends_with("kept"), "an unknown node keeps its text: {md}");
        // Plain strings pass through; nothing becomes HTML.
        assert_eq!(adf_to_markdown(&json!("plain")), "plain");
        assert_eq!(adf_to_markdown(&Value::Null), "");
    }

    #[test]
    fn a_jira_work_item_and_its_comments() {
        let view = json!({"key": "PAY-1",
            "fields": {"summary": "Pay twice", "status": {"name": "In Progress"},
                "reporter": {"displayName": "Ann"},
                "description": adf(json!([{"type": "paragraph", "content": [{"type": "text", "text": "It charges twice."}]}]))}});
        let comments = json!({"comments": [
            {"id": "11", "author": {"displayName": "Bob"}, "created": "2026-03-02T10:00:00.000+0000",
             "body": adf(json!([{"type": "paragraph", "content": [{"type": "text", "text": "later"}]}]))},
            {"id": "10", "author": {"displayName": "Cy"}, "created": "2026-03-01T10:00:00.000+0000",
             "body": adf(json!([{"type": "paragraph", "content": [{"type": "text", "text": "first"}]}]))},
            {"id": "12", "author": {"displayName": "Dee"}, "body": adf(json!([]))}
        ]});
        let t = jira_view(
            "PAY-1",
            &view,
            &comments,
            Some("https://acme.atlassian.net"),
        )
        .unwrap();
        assert_eq!(
            (t.title.as_str(), t.state.as_str(), t.author.as_str()),
            ("Pay twice", "in progress", "Ann")
        );
        assert_eq!(t.body, "It charges twice.");
        assert_eq!(t.url, "https://acme.atlassian.net/browse/PAY-1");
        let ids: Vec<_> = t.comments.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            ["issue:10", "issue:11"],
            "oldest first, an empty one dropped"
        );
        assert_eq!(
            t.comments[0].url,
            "https://acme.atlassian.net/browse/PAY-1?focusedCommentId=10"
        );

        // A flat object, a bare list of comments and a configured site work too.
        let flat = json!({"summary": "S", "status": "Done", "description": "text"});
        let list = json!([{"id": 1, "author": {"displayName": "X"}, "body": "hi", "created": "t"}]);
        let t = jira_view("K-1", &flat, &list, Some("https://s.atlassian.net/")).unwrap();
        assert_eq!(
            (t.state.as_str(), t.body.as_str(), t.url.as_str()),
            ("done", "text", "https://s.atlassian.net/browse/K-1")
        );
        assert_eq!(t.comments.len(), 1);
        assert!(jira_view("K-1", &json!({}), &json!([]), None).is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn jira_is_read_through_acli() {
        let d = tempfile::tempdir().unwrap();
        let acli = fake(
            d.path(),
            "acli",
            r#"case "$*" in
  *"comment list"*) echo '{"comments":[{"id":"5","author":{"displayName":"B"},"body":"hi","created":"t"}]}' ;;
  *) echo '{"fields":{"summary":"S","status":{"name":"Open"},"description":"d"}}' ;;
esac"#,
        );
        let host = tracker_host(d.path(), "jira", "acli", &acli);
        let t = host.ticket_get(d.path(), "PAY-1", "PAY-1").await.unwrap();
        assert_eq!((t.title.as_str(), t.comments.len()), ("S", 1));
        let calls = std::fs::read_to_string(d.path().join("calls")).unwrap();
        assert!(
            calls.contains(
                "jira workitem view PAY-1 --json --fields summary,status,description,reporter"
            ),
            "{calls}"
        );
        assert!(
            calls.contains("jira workitem comment list --key PAY-1 --json --paginate"),
            "{calls}"
        );
        assert!(host.ticket_get(d.path(), "k", "A B").await.is_err());
    }

    /// The shape `acli jira workitem view --json` really prints (a sample
    /// with the same structure and invented values): REST fields, ADF with
    /// `attrs`, many null extras, and a `self` on an internal host.
    #[test]
    fn the_real_acli_view_shape() {
        let real = json!({
            "changelog": null, "editmeta": null,
            "expand": "renderedFields,names,schema,operations,editmeta,changelog,versionedRepresentations",
            "fields": {
                "description": {"type": "doc", "version": 1, "content": [
                    {"type": "paragraph", "attrs": {"localId": "aa11"}, "content": [{"type": "text", "text": "First “quoted” line."}]},
                    {"type": "paragraph", "attrs": {"localId": "bb22"}, "content": [{"type": "text", "text": "Second line."}]}
                ]},
                "reporter": {"accountId": "x", "accountType": "atlassian", "active": true,
                    "avatarUrls": {"16x16": "https://a.test/16"}, "displayName": "Ann Example",
                    "emailAddress": "ann@example.test", "self": "https://internal.test/rest/api/3/user?accountId=x",
                    "timeZone": "Europe/Paris"},
                "status": {"description": "", "id": "10003", "name": "To Do",
                    "statusCategory": {"colorName": "blue-gray", "id": 2, "key": "new", "name": "To Do"}},
                "summary": "Store preview image per flow"
            },
            "fieldsToInclude": null, "id": "126677", "key": "SC-1",
            "names": null, "operations": null, "properties": null, "renderedFields": null,
            "schema": null, "self": "https://internal.test/rest/api/3/issue/126677",
            "transitions": null, "versionedRepresentations": null
        });
        let t = jira_view(
            "SC-1",
            &real,
            &json!([]),
            Some("https://acme.atlassian.net/"),
        )
        .unwrap();
        assert_eq!(t.title, "Store preview image per flow");
        assert_eq!(
            (t.state.as_str(), t.author.as_str()),
            ("to do", "Ann Example")
        );
        assert_eq!(t.body, "First “quoted” line.\n\nSecond line.");
        // The link is the configured site, never the internal REST host.
        assert_eq!(t.url, "https://acme.atlassian.net/browse/SC-1");
        // Without a configured site there is no link rather than a wrong one.
        assert_eq!(jira_view("SC-1", &real, &json!([]), None).unwrap().url, "");
    }

    /// What `acli jira workitem comment list --json --paginate` really
    /// prints (invented text, same structure): comment authors and bodies are
    /// plain strings, there is no timestamp, and each page is its own JSON
    /// document.
    #[test]
    fn the_real_acli_comment_pages() {
        let out = br#"{
  "comments": [
    {"author": "Ann Example", "body": "Done in https://example.test/pull/1 (draft).\nSecond line.",
     "id": "177175", "visibility": "public"},
    {"author": "Bob Example", "body": "Thanks", "id": "177176", "visibility": "public"}
  ],
  "isLast": false, "maxResults": 50, "startAt": 0, "total": 2
}
{
  "comments": [],
  "isLast": true, "maxResults": 50, "startAt": 50, "total": 2
}
"#;
        let merged = merge_comment_pages(out).unwrap();
        assert_eq!(merged.as_array().unwrap().len(), 2);
        let view = json!({"fields": {"summary": "S"}});
        let t = jira_view("SC-1", &view, &merged, Some("https://acme.atlassian.net")).unwrap();
        let ids: Vec<_> = t.comments.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            ["issue:177175", "issue:177176"],
            "the CLI's order is kept"
        );
        assert_eq!(t.comments[0].author, "Ann Example");
        assert!(
            t.comments[0]
                .body
                .starts_with("Done in https://example.test/pull/1")
        );
        assert_eq!(t.comments[0].created_at, "");
        // A single page, a bare list and nothing at all.
        assert_eq!(
            merge_comment_pages(br#"{"comments": [{"id": 1}]}"#)
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            merge_comment_pages(b"[{\"id\": 1}, {\"id\": 2}]")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert!(
            merge_comment_pages(b"")
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(merge_comment_pages(b"{not json").is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_comment_goes_through_the_cli_as_one_argument() {
        let d = tempfile::tempdir().unwrap();
        let body = "Update:\n- done $(rm -rf /) `x` \"q\"";

        let gh = fake(d.path(), "gh", "true");
        let host = tracker_host(d.path(), "github-issues", "gh", &gh);
        host.ticket_comment(d.path(), "GH-12", "12", body)
            .await
            .unwrap();
        let calls = std::fs::read_to_string(d.path().join("calls")).unwrap();
        // The text is one argument (the fake echoes them joined) and carries the marker.
        assert!(
            calls.starts_with(
                "issue comment 12 --body Update:\n- done $(rm -rf /) `x` \"q\"\n\n<!-- xherd -->"
            ),
            "{calls}"
        );

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
        let glab = fake(d.path(), "glab", "true");
        let host = tracker_host(d.path(), "gitlab-issues", "glab", &glab);
        host.ticket_comment(&repo, "GL-3", "3", "hello")
            .await
            .unwrap();
        let calls = std::fs::read_to_string(d.path().join("calls")).unwrap();
        assert!(calls.contains("api --hostname gl.test -X POST projects/g%2Fp/issues/3/notes -f body=hello\n\n<!-- xherd -->"), "{calls}");

        let acli = fake(d.path(), "acli", "true");
        let host = tracker_host(d.path(), "jira", "acli", &acli);
        host.ticket_comment(d.path(), "PAY-1", "PAY-1", "plain text")
            .await
            .unwrap();
        let calls = std::fs::read_to_string(d.path().join("calls")).unwrap();
        assert!(
            calls.contains("jira workitem comment create --key PAY-1 --body plain text\n"),
            "{calls}"
        );
        assert!(
            !calls.lines().last().unwrap().contains("xherd"),
            "no marker on Jira"
        );
    }

    #[tokio::test]
    async fn a_comment_that_is_empty_too_long_odd_or_for_a_bad_reference_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let reg = crate::Registry::load(crate::builtins(), d.path());
        let host = Host {
            plugin: reg.get("github-issues").unwrap().clone(),
            ctx: crate::Context::default(),
        };
        for (issue, body) in [
            ("12", "   "),
            ("12", "a\u{7}b"),
            ("12; rm", "ok"),
            ("", "ok"),
        ] {
            assert!(
                host.ticket_comment(d.path(), "k", issue, body)
                    .await
                    .is_err(),
                "{issue:?} {body:?}"
            );
        }
        let long = "x".repeat(MAX_COMMENT + 1);
        assert!(
            host.ticket_comment(d.path(), "k", "12", &long)
                .await
                .is_err()
        );
        // A tracker that only links says it cannot post.
        let bb = Host {
            plugin: reg.get("bitbucket").unwrap().clone(),
            ctx: crate::Context::default(),
        };
        let e = bb
            .ticket_comment(d.path(), "k", "1", "ok")
            .await
            .unwrap_err()
            .to_string();
        assert!(e.contains("cannot post comments yet"), "{e}");
    }
}
