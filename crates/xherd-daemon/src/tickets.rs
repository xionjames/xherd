//! The ticket of an agent, read through a tracker plugin (`ticket-get`): the
//! Ticket panel shows its title, state, text and comments. The only thing
//! ever posted is a summary the user has read, edited and sent (R-308).

use std::sync::Arc;

use serde_json::{Value, json};
use xherd_core::Ticket;
use xherd_core::new::{TicketInput, key_of_issue};
use xherd_plugin::tracker::Tracker;

use crate::{ActionError, Daemon};

/// An enabled tracker plugin that reads tickets: name, rules, and its
/// `baseUrl` setting (for `{baseUrl}` in its URL pattern).
pub struct Reader {
    pub name: String,
    pub tracker: Tracker,
    pub base_url: Option<String>,
}

/// The tracker plugin that reads `ticket`, and the issue reference it makes
/// of it. A URL goes to the first tracker whose `url` pattern reads it (#114).
/// A key goes to a tracker that serves `remote_host` first, then to one that
/// serves any host. `None`: no enabled tracker reads it.
pub fn pick_tracker(
    trackers: &[Reader],
    remote_host: Option<&str>,
    ticket: &Ticket,
) -> Option<(String, String)> {
    if let Some(url) = &ticket.url {
        return trackers.iter().find_map(|r| {
            r.tracker
                .issue_from_url(url, r.base_url.as_deref())
                .map(|i| (r.name.clone(), i))
        });
    }
    let key = ticket.key.as_deref()?;
    let by_host = trackers.iter().find_map(|r| {
        let served = remote_host.is_some_and(|h| r.tracker.serves(h));
        served
            .then(|| r.tracker.issue_ref(key))
            .flatten()
            .map(|i| (r.name.clone(), i))
    });
    by_host.or_else(|| {
        trackers
            .iter()
            .filter(|r| r.tracker.hosts.is_empty())
            .find_map(|r| r.tracker.issue_ref(key).map(|i| (r.name.clone(), i)))
    })
}

/// The tracker that serves an agent's ticket, ready to be asked.
struct Resolved {
    wd: std::path::PathBuf,
    tracker: String,
    issue: String,
    url: Option<String>,
    key: String,
    host: Arc<xherd_plugin::host::Host>,
}

fn no_tracker(a: &xherd_core::agent::Agent) -> ActionError {
    ActionError {
        code: "no_tracker",
        message: match a.ticket_ref() {
            "" => format!("{} has no ticket", a.name),
            t => format!("no enabled issue tracker reads the ticket {t}"),
        },
    }
}

/// The host of an https URL, lower-cased.
fn url_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
    Some(host.split(':').next()?.to_ascii_lowercase())
}

impl Daemon {
    /// Every enabled tracker plugin that reads tickets (a `key` or `url`
    /// pattern).
    fn ticket_readers(&self) -> Vec<Reader> {
        self.plugins
            .of_kind(xherd_plugin::Kind::Tracker)
            .filter(|p| self.plugin_enabled(p.name()))
            .filter_map(|p| match Tracker::of(&p.manifest) {
                Ok(Some(t)) if t.key.is_some() || t.url.is_some() => {
                    let base_url = self
                        .plugin_context(p)
                        .settings
                        .get("baseUrl")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    Some(Reader {
                        name: p.name().to_string(),
                        tracker: t,
                        base_url,
                    })
                }
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(plugin = p.name(), "{e}");
                    None
                }
            })
            .collect()
    }

    /// A ticket as the user gave it, made into the agent's `Ticket` (#114):
    /// a URL gets its key from the tracker that reads it (none: the URL is
    /// kept as a plain link); a key stays as given. Also the tracker found,
    /// for a repository on `remote_host`.
    pub(crate) fn ticket_of(
        &self,
        input: &TicketInput,
        remote_host: Option<&str>,
    ) -> (Ticket, Option<String>) {
        let ticket = match input {
            TicketInput::Key(k) => Ticket {
                key: Some(k.clone()),
                url: None,
            },
            TicketInput::Url(u) => Ticket {
                key: None,
                url: Some(u.clone()),
            },
        };
        let found = pick_tracker(&self.ticket_readers(), remote_host, &ticket);
        let ticket = match (input, &found) {
            (TicketInput::Url(u), Some((_, issue))) => Ticket {
                key: key_of_issue(issue),
                url: Some(u.clone()),
            },
            _ => ticket,
        };
        (ticket, found.map(|(t, _)| t))
    }

    /// `POST /tickets/resolve {ticket, repo?}`: what the New agent form
    /// shows under the ticket field: the key, the URL and the tracker that
    /// reads it (`null`: none, the URL stays a plain link). `repo` is a
    /// local repository path, for the tracker of its host.
    pub async fn resolve_ticket_input(
        &self,
        ticket: &str,
        repo: Option<&str>,
    ) -> Result<Value, ActionError> {
        let Some(input) = xherd_core::new::parse_ticket(ticket)? else {
            return Ok(json!({"key": null, "url": null, "tracker": null}));
        };
        let remote = match repo.map(std::path::Path::new).filter(|p| p.is_absolute()) {
            Some(p) => crate::actor::origin_host(p).await,
            None => None,
        };
        let (t, tracker) = self.ticket_of(&input, remote.as_deref());
        Ok(json!({"key": t.key, "url": t.url, "tracker": tracker}))
    }

    /// The agent and, when an enabled tracker reads its ticket key, that
    /// tracker (with the live secrets of its settings).
    async fn resolve_ticket(
        &self,
        who: &str,
    ) -> Result<(xherd_core::agent::Agent, Option<Resolved>), ActionError> {
        let a = self.action(who, crate::actor::UserAction::Get).await?;
        let wd = crate::workdir_of(&a)?.to_path_buf();
        let Some(ticket) = a.ticket.clone() else {
            return Ok((a, None));
        };
        let remote = crate::actor::origin_host(&wd).await;
        let trackers = self.ticket_readers();
        let Some((tracker, issue)) = pick_tracker(&trackers, remote.as_deref(), &ticket) else {
            return Ok((a, None));
        };
        let p = self
            .plugins
            .get(&tracker)
            .ok_or_else(|| ActionError {
                code: "no_plugin",
                message: format!("no plugin named {tracker}"),
            })?
            .clone();
        let host = Arc::new(xherd_plugin::host::Host {
            ctx: self.plugin_context(&p),
            plugin: p,
        });
        // The repository host's token goes only to a tracker that serves that
        // host (github-issues for github.com), never to one for any host, and
        // never for a ticket URL on another host.
        let ticket_host = ticket.url.as_deref().map(url_host);
        let served = remote.as_deref().filter(|r| {
            trackers
                .iter()
                .any(|t| t.name == tracker && t.tracker.serves(r))
                && ticket_host
                    .as_ref()
                    .is_none_or(|h| h.as_deref() == Some(r.to_ascii_lowercase().as_str()))
        });
        let git_hosts = self.live_config().git_hosts;
        let host = crate::actor::with_git_token(host, served, &git_hosts, &self.paths.secrets());
        Ok((
            a,
            Some(Resolved {
                wd,
                tracker,
                issue,
                url: ticket.url,
                key: ticket.key.unwrap_or_default(),
                host,
            }),
        ))
    }

    /// `GET /agents/{id}/ticket`: `{tracker, issue, ticket}`, or
    /// `{tracker: null, ticket}` when the agent has no ticket or no enabled
    /// tracker reads it.
    pub async fn agent_ticket(&self, who: &str) -> Result<Value, ActionError> {
        let (a, r) = self.resolve_ticket(who).await?;
        let Some(r) = r else {
            return Ok(json!({"tracker": null, "ticket": a.ticket}));
        };
        let ticket = r
            .host
            .ticket_get(&r.wd, &r.key, &r.issue, r.url.as_deref())
            .await
            .map_err(|e| ActionError {
                code: "ticket_failed",
                message: e.to_string(),
            })?;
        Ok(json!({"tracker": r.tracker, "issue": r.issue, "ticket": ticket}))
    }

    /// `GET /agents/{id}/ticket-status`: which tracker reads the agent's
    /// ticket and whether a summary was already posted (the event
    /// `ticket-comment`). Cheap: no tracker CLI is run, so the agent page can
    /// ask it every time it opens.
    pub async fn ticket_status(&self, who: &str) -> Result<Value, ActionError> {
        let (a, r) = self.resolve_ticket(who).await?;
        // Kept past the event retention period (`KEPT_EVENTS`).
        let posted = self
            .store
            .agent_has_event(&a.id, "ticket-comment")
            .await
            .unwrap_or(false);
        Ok(match r {
            Some(r) => json!({"tracker": r.tracker, "issue": r.issue, "posted": posted}),
            None => json!({"tracker": null, "posted": posted}),
        })
    }

    /// `GET /agents/{id}/ticket-summary`: the draft summary of what the agent
    /// has done, for the user to read and edit. Nothing is posted.
    pub async fn ticket_summary(&self, who: &str) -> Result<Value, ActionError> {
        let (a, r) = self.resolve_ticket(who).await?;
        let r = r.ok_or_else(|| no_tracker(&a))?;
        Ok(json!({
            "tracker": r.tracker,
            "issue": r.issue,
            "text": xherd_core::ticket::summary(&a),
        }))
    }

    /// `POST /agents/{id}/ticket-comment`: post `body`, which the user has
    /// read and accepted (R-308), as a comment on the ticket. Logged as the
    /// event `ticket-comment` (its length, never its text).
    pub async fn post_ticket_comment(&self, who: &str, body: &str) -> Result<Value, ActionError> {
        let (a, r) = self.resolve_ticket(who).await?;
        let r = r.ok_or_else(|| no_tracker(&a))?;
        r.host
            .ticket_comment(&r.wd, &r.key, &r.issue, r.url.as_deref(), body)
            .await
            .map_err(|e| ActionError {
                code: "comment_failed",
                message: e.to_string(),
            })?;
        let event = xherd_store::Event {
            id: xherd_core::new_id(),
            agent_id: a.id.clone(),
            kind: "ticket-comment".into(),
            payload: Some(json!({
                "tracker": r.tracker,
                "issue": r.issue,
                "chars": body.chars().count(),
            })),
            at: self.clock.now(),
        };
        if let Err(e) = self.store.insert_event(event).await {
            tracing::warn!(error = %e, "ticket comment event not stored");
        }
        tracing::info!(agent = %a.name, tracker = %r.tracker, "comment posted on the ticket");
        Ok(json!({"posted": true, "tracker": r.tracker, "issue": r.issue}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(name: &str, hosts: &[&str], key: &str, url: Option<&str>) -> Reader {
        Reader {
            name: name.into(),
            tracker: serde_json::from_value(json!({"hosts": hosts, "key": key, "url": url}))
                .unwrap(),
            base_url: Some("https://acme.atlassian.net".into()),
        }
    }

    fn key(k: &str) -> Ticket {
        Ticket {
            key: Some(k.into()),
            url: None,
        }
    }

    #[test]
    fn a_ticket_url_goes_to_the_tracker_that_reads_it() {
        let ts = vec![
            reader(
                "github-issues",
                &["github.com"],
                r"^(?:GH-|#)?(\d+)$",
                Some(r"^https://github\.com/[\w.-]+/[\w.-]+/issues/(\d+)$"),
            ),
            reader(
                "jira",
                &[],
                r"^([A-Z]+-\d+)$",
                Some(r"{baseUrl}/browse/([A-Z]+-\d+)$"),
            ),
        ];
        let url = |u: &str| Ticket {
            key: None,
            url: Some(u.into()),
        };
        assert_eq!(
            pick_tracker(
                &ts,
                Some("gitlab.com"),
                &url("https://github.com/a/b/issues/3")
            ),
            Some(("github-issues".into(), "3".into()))
        );
        assert_eq!(
            pick_tracker(&ts, None, &url("https://acme.atlassian.net/browse/PAY-4")),
            Some(("jira".into(), "PAY-4".into()))
        );
        assert_eq!(
            pick_tracker(&ts, None, &url("https://example.org/x/1")),
            None
        );
        assert_eq!(
            pick_tracker(&ts, Some("github.com"), &key("#7")),
            Some(("github-issues".into(), "7".into()))
        );
        assert_eq!(
            url_host("https://Me@GitHub.com:443/a"),
            Some("github.com".into())
        );
    }

    #[test]
    fn a_ticket_goes_to_the_tracker_of_its_host_then_to_one_for_any_host() {
        let ts = vec![
            reader("github-issues", &["github.com"], r"^(?:GH-)?(\d+)$", None),
            reader("gitlab-issues", &["*gitlab*"], r"^(?:GL-)?(\d+)$", None),
            reader("any", &[], r"^([A-Z]+-\d+)$", None),
        ];
        let pick = |h, k| pick_tracker(&ts, h, &key(k));
        assert_eq!(
            pick(Some("github.com"), "GH-12"),
            Some(("github-issues".into(), "12".into()))
        );
        assert_eq!(
            pick(Some("gitlab.com"), "7"),
            Some(("gitlab-issues".into(), "7".into()))
        );
        // GitHub's tracker does not read a number on a GitLab host; a tracker
        // for any host that reads the key does.
        assert_eq!(
            pick(Some("gitlab.com"), "GH-12"),
            Some(("any".into(), "GH-12".into()))
        );
        assert_eq!(
            pick(Some("gitlab.com"), "12"),
            Some(("gitlab-issues".into(), "12".into()))
        );
        // A key no host tracker reads falls to the one that serves any host.
        assert_eq!(
            pick(Some("github.com"), "PAY-9"),
            Some(("any".into(), "PAY-9".into()))
        );
        assert_eq!(pick(None, "PAY-9"), Some(("any".into(), "PAY-9".into())));
        assert_eq!(pick(Some("example.org"), "12"), None);
    }
}
