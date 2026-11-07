//! Open in… (spec §12.6, design §8.7): editor URLs that work because the
//! workspace has the same path on the host (R-701), the Dev Containers entry
//! (R-702), and requests for the host companion (R-704). The companion polls
//! the daemon as the local user, so nothing listens on the host and only
//! commands the companion itself has configured ever run.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Notify;

use crate::{ActionError, Daemon, UserAction};

/// A companion that has not polled for this long is gone.
const COMPANION_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    pub label: String,
    /// `url`: a link the browser opens; `companion`: `POST /agents/:id/open`.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenRequest {
    pub app: String,
    pub dir: String,
}

#[derive(Default)]
struct State {
    apps: Vec<String>,
    seen: Option<Instant>,
    queue: VecDeque<OpenRequest>,
}

/// Shared between the API handlers and the companion's long poll.
#[derive(Clone, Default)]
pub struct Companion {
    state: Arc<Mutex<State>>,
    wake: Arc<Notify>,
}

impl Companion {
    fn apps(&self) -> Vec<String> {
        let s = self.state.lock().unwrap();
        match s.seen {
            Some(t) if t.elapsed() < COMPANION_TTL => s.apps.clone(),
            _ => Vec::new(),
        }
    }

    /// The companion's long poll: records its apps, returns the next request
    /// or `None` after `wait`.
    pub async fn next(&self, apps: Vec<String>, wait: Duration) -> Option<OpenRequest> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let notified = self.wake.notified();
            {
                let mut s = self.state.lock().unwrap();
                s.apps = apps.clone();
                s.seen = Some(Instant::now());
                if let Some(r) = s.queue.pop_front() {
                    return Some(r);
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return None;
            }
        }
    }

    fn push(&self, r: OpenRequest) {
        let mut s = self.state.lock().unwrap();
        // Older requests are stale by the time a companion comes back.
        if s.queue.len() >= 8 {
            s.queue.pop_front();
        }
        s.queue.push_back(r);
        drop(s);
        self.wake.notify_one();
    }
}

/// URL-scheme entries for a folder (R-701), and Dev Containers in the
/// container (R-702, `container`: its id, the hostname).
pub fn url_targets(dir: &str, container: Option<&str>) -> Vec<Target> {
    let enc: String = dir
        .split('/')
        .map(|seg| {
            seg.bytes()
                .map(|b| match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                        (b as char).to_string()
                    }
                    _ => format!("%{b:02X}"),
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("/");
    let url = |id: &str, label: &str, u: String| Target {
        id: id.into(),
        label: label.into(),
        kind: "url",
        url: Some(u),
    };
    let mut out = vec![
        url("vscode", "VS Code", format!("vscode://file{enc}")),
        url("cursor", "Cursor", format!("cursor://file{enc}")),
        url("zed", "Zed", format!("zed://file{enc}")),
    ];
    if let Some(c) = container.filter(|c| !c.is_empty()) {
        out.push(url(
            "devcontainer",
            "VS Code, inside the xherd container",
            format!(
                "vscode://vscode-remote/attached-container+{}{enc}",
                hex::encode(c)
            ),
        ));
    }
    out
}

impl Daemon {
    /// `GET /agents/:id/open-targets`: what works for this agent from a
    /// local browser (R-671; remote clients come with phones).
    pub async fn open_targets(&self, who: &str) -> Result<Vec<Target>, ActionError> {
        let a = self.action(who, UserAction::Get).await?;
        let dir = a.workdir.ok_or_else(|| ActionError {
            code: "no_workspace",
            message: format!("{} has no worktree", a.name),
        })?;
        let container = if self.sandbox {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_string())
        } else {
            None
        };
        let mut out = url_targets(&dir, container.as_deref());
        out.extend(self.companion.apps().into_iter().map(|app| Target {
            id: format!("companion:{app}"),
            label: format!("{app} (on this computer)"),
            kind: "companion",
            url: None,
        }));
        Ok(out)
    }

    /// `POST /agents/:id/open`: hand the folder to the companion, which runs
    /// its own configured command for `app` (R-704).
    pub async fn open_with_companion(&self, who: &str, app: &str) -> Result<(), ActionError> {
        if !self.companion.apps().iter().any(|a| a == app) {
            return Err(ActionError {
                code: "no_companion",
                message: format!(
                    "no companion offers {app}; run `xherd companion` on this computer"
                ),
            });
        }
        let a = self.action(who, UserAction::Get).await?;
        let dir = a.workdir.ok_or_else(|| ActionError {
            code: "no_workspace",
            message: format!("{} has no worktree", a.name),
        })?;
        self.companion.push(OpenRequest {
            app: app.into(),
            dir,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_encoded_and_devcontainer_only_in_the_container() {
        let t = url_targets("/Users/me/x herd/wt/SC-1", None);
        assert_eq!(
            t[0].url.as_deref(),
            Some("vscode://file/Users/me/x%20herd/wt/SC-1")
        );
        assert!(t.iter().all(|t| t.id != "devcontainer"));
        let t = url_targets("/w/SC-1", Some("ab12"));
        assert_eq!(
            t.last().unwrap().url.as_deref(),
            Some("vscode://vscode-remote/attached-container+61623132/w/SC-1")
        );
    }

    #[tokio::test]
    async fn companion_queue() {
        let c = Companion::default();
        assert!(c.apps().is_empty());
        assert_eq!(
            c.next(vec!["idea".into()], Duration::from_millis(10)).await,
            None
        );
        assert_eq!(c.apps(), ["idea"]);
        let waiter = {
            let c = c.clone();
            tokio::spawn(async move { c.next(vec!["idea".into()], Duration::from_secs(5)).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        c.push(OpenRequest {
            app: "idea".into(),
            dir: "/w".into(),
        });
        assert_eq!(
            waiter.await.unwrap(),
            Some(OpenRequest {
                app: "idea".into(),
                dir: "/w".into()
            })
        );
    }
}
