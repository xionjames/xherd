//! The ticket of an agent, read through a tracker plugin (`ticket-get`): the
//! Ticket panel shows its title, state, text and comments. Read-only: xherd
//! posts nothing to a tracker (R-308).

use std::sync::Arc;

use serde_json::{Value, json};
use xherd_plugin::tracker::Tracker;

use crate::{ActionError, Daemon};

/// The tracker plugin that reads `key` for a repository on `remote_host`, and
/// the issue reference it makes of the key: a tracker that serves the host
/// first, then one that serves any host. `None`: no enabled tracker reads it.
pub fn pick_tracker(
    trackers: &[(String, Tracker)],
    remote_host: Option<&str>,
    key: &str,
) -> Option<(String, String)> {
    let by_host = trackers.iter().find_map(|(n, t)| {
        let served = remote_host.is_some_and(|h| t.serves(h));
        served
            .then(|| t.issue_ref(key))
            .flatten()
            .map(|r| (n.clone(), r))
    });
    by_host.or_else(|| {
        trackers
            .iter()
            .filter(|(_, t)| t.hosts.is_empty())
            .find_map(|(n, t)| t.issue_ref(key).map(|r| (n.clone(), r)))
    })
}

/// The tracker that serves an agent's ticket, ready to be asked.
struct Resolved {
    wd: std::path::PathBuf,
    tracker: String,
    issue: String,
    host: Arc<xherd_plugin::host::Host>,
}

fn no_tracker(key: &str) -> ActionError {
    ActionError {
        code: "no_tracker",
        message: format!("no enabled issue tracker reads the ticket key {key}"),
    }
}

impl Daemon {
    /// The agent and, when an enabled tracker reads its ticket key, that
    /// tracker (with the live secrets of its settings).
    async fn resolve_ticket(
        &self,
        who: &str,
    ) -> Result<(xherd_core::agent::Agent, Option<Resolved>), ActionError> {
        let a = self.action(who, crate::actor::UserAction::Get).await?;
        let wd = crate::workdir_of(&a)?.to_path_buf();
        let remote = crate::actor::origin_host(&wd).await;
        let trackers: Vec<(String, Tracker)> = self
            .plugins
            .of_kind(xherd_plugin::Kind::Tracker)
            .filter(|p| self.plugin_enabled(p.name()))
            .filter_map(|p| match Tracker::of(&p.manifest) {
                Ok(Some(t)) if t.key.is_some() => Some((p.name().to_string(), t)),
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(plugin = p.name(), "{e}");
                    None
                }
            })
            .collect();
        let Some((tracker, issue)) = pick_tracker(&trackers, remote.as_deref(), &a.ticket_key)
        else {
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
        let host = crate::actor::with_git_token(host, remote.as_deref(), &self.paths.secrets());
        Ok((
            a,
            Some(Resolved {
                wd,
                tracker,
                issue,
                host,
            }),
        ))
    }

    /// `GET /agents/{id}/ticket`: `{tracker, issue, ticket}`, or
    /// `{tracker: null}` when no enabled tracker reads this ticket key.
    pub async fn agent_ticket(&self, who: &str) -> Result<Value, ActionError> {
        let (a, r) = self.resolve_ticket(who).await?;
        let Some(r) = r else {
            return Ok(json!({"tracker": null, "key": a.ticket_key}));
        };
        let ticket = r
            .host
            .ticket_get(&r.wd, &a.ticket_key, &r.issue)
            .await
            .map_err(|e| ActionError {
                code: "ticket_failed",
                message: e.to_string(),
            })?;
        Ok(json!({"tracker": r.tracker, "issue": r.issue, "ticket": ticket}))
    }

    /// `GET /agents/{id}/ticket-summary`: the draft summary of what the agent
    /// has done, for the user to read and edit. Nothing is posted.
    pub async fn ticket_summary(&self, who: &str) -> Result<Value, ActionError> {
        let (a, r) = self.resolve_ticket(who).await?;
        let r = r.ok_or_else(|| no_tracker(&a.ticket_key))?;
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
        let r = r.ok_or_else(|| no_tracker(&a.ticket_key))?;
        r.host
            .ticket_comment(&r.wd, &a.ticket_key, &r.issue, body)
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

    fn tracker(hosts: &[&str], key: &str) -> Tracker {
        serde_json::from_value(json!({"hosts": hosts, "key": key})).unwrap()
    }

    #[test]
    fn a_ticket_goes_to_the_tracker_of_its_host_then_to_one_for_any_host() {
        let ts = vec![
            (
                "github-issues".to_string(),
                tracker(&["github.com"], r"^(?:GH-)?(\d+)$"),
            ),
            (
                "gitlab-issues".to_string(),
                tracker(&["*gitlab*"], r"^(?:GL-)?(\d+)$"),
            ),
            ("any".to_string(), tracker(&[], r"^([A-Z]+-\d+)$")),
        ];
        let pick = |h, k| pick_tracker(&ts, h, k);
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
