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

impl Daemon {
    /// `GET /agents/{id}/ticket`: `{tracker, issue, ticket}`, or
    /// `{tracker: null}` when no enabled tracker reads this ticket key.
    pub async fn agent_ticket(&self, who: &str) -> Result<Value, ActionError> {
        let a = self.action(who, crate::actor::UserAction::Get).await?;
        let wd = crate::workdir_of(&a)?;
        let remote = crate::actor::origin_host(wd).await;
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
        let Some((name, issue)) = pick_tracker(&trackers, remote.as_deref(), &a.ticket_key) else {
            return Ok(json!({"tracker": null, "key": a.ticket_key}));
        };
        let p = self
            .plugins
            .get(&name)
            .ok_or_else(|| ActionError {
                code: "no_plugin",
                message: format!("no plugin named {name}"),
            })?
            .clone();
        let host = Arc::new(xherd_plugin::host::Host {
            ctx: self.plugin_context(&p),
            plugin: p,
        });
        let host = crate::actor::with_git_token(host, remote.as_deref(), &self.paths.secrets());
        let ticket = host
            .ticket_get(wd, &a.ticket_key, &issue)
            .await
            .map_err(|e| ActionError {
                code: "ticket_failed",
                message: e.to_string(),
            })?;
        Ok(json!({"tracker": name, "issue": issue, "ticket": ticket}))
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
