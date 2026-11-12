//! Open in… (spec §12.6, design §8.7): editor URLs that work because the
//! workspace has the same path on the host (R-701), the Dev Containers entry
//! (R-702), and requests for the host companion (R-704). The companion polls
//! the daemon as the local user, so nothing listens on the host. It runs its
//! own apps, the built-in terminal (R-707), and user commands from Settings
//! only after the user confirmed each argv on the host (R-706).

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
    /// The agent's name, for the terminal and `{agent}`.
    pub agent: String,
    /// `app: "terminal"`: which terminal app (unset = the host's default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    /// `app: "cmd:<id>"`: the user command, run only once confirmed on the host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argv: Option<Vec<String>>,
}

/// Request kinds a companion understands beyond its own apps.
pub const TERMINAL: &str = "terminal";
pub const COMMANDS: &str = "commands";

/// What a polling companion offers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Offer {
    pub apps: Vec<String>,
    pub features: Vec<String>,
    /// Terminal apps installed on the companion's computer, its default first
    /// (R-707); empty from an older companion.
    pub terminals: Vec<String>,
}

#[derive(Default)]
struct State {
    offer: Offer,
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
    /// What the companion offers, `None` when none is polling.
    pub fn offer(&self) -> Option<Offer> {
        let s = self.state.lock().unwrap();
        match s.seen {
            Some(t) if t.elapsed() < COMPANION_TTL => Some(s.offer.clone()),
            _ => None,
        }
    }

    /// The companion's long poll: records its offer, returns the next request
    /// or `None` after `wait`.
    pub async fn next(&self, offer: Offer, wait: Duration) -> Option<OpenRequest> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let notified = self.wake.notified();
            {
                let mut s = self.state.lock().unwrap();
                s.offer = offer.clone();
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
    /// local browser (R-671), minus what Settings hides (R-706).
    pub async fn open_targets(&self, who: &str) -> Result<Vec<Target>, ActionError> {
        let a = self.action(who, UserAction::Get).await?;
        let dir = a.workdir.ok_or_else(|| ActionError {
            code: "no_workspace",
            message: format!("{} has no worktree", a.name),
        })?;
        let cfg = self.live_config().open_in;
        let mut out = url_targets(&dir, self.container().as_deref());
        out.extend(companion_targets(
            self.companion.offer().as_ref(),
            &cfg.commands,
        ));
        out.retain(|t| !cfg.hidden.contains(&t.id));
        Ok(out)
    }

    /// The container id (its hostname), in the container only (R-702).
    fn container(&self) -> Option<String> {
        self.sandbox
            .then(|| std::fs::read_to_string("/etc/hostname").ok())
            .flatten()
            .map(|h| h.trim().to_string())
    }

    /// `GET /open-in`, for Settings → Open in: the link entries this daemon
    /// offers and what the companion offers right now.
    pub fn open_in_view(&self) -> serde_json::Value {
        let links: Vec<_> = url_targets("/", self.container().as_deref())
            .into_iter()
            .map(|t| serde_json::json!({"id": t.id, "label": t.label}))
            .collect();
        serde_json::json!({"links": links, "companion": self.companion.offer()})
    }

    /// `POST /agents/:id/open`: hand the folder to the companion (R-704,
    /// R-706, R-707).
    pub async fn open_with_companion(&self, who: &str, app: &str) -> Result<(), ActionError> {
        let none = |what: &str| ActionError {
            code: "no_companion",
            message: format!(
                "no companion offers {what}; run `xherd companion` on this computer (an up-to-date one for the terminal and your commands)"
            ),
        };
        let offer = self.companion.offer().ok_or_else(|| none(app))?;
        let has = |f: &str| offer.features.iter().any(|x| x == f);
        let cfg = self.live_config().open_in;
        let (terminal, argv) = if app == TERMINAL {
            if !has(TERMINAL) {
                return Err(none("the terminal"));
            }
            (cfg.terminal, None)
        } else if let Some(id) = app.strip_prefix("cmd:") {
            let c = cfg
                .commands
                .iter()
                .find(|c| c.id == id)
                .ok_or_else(|| ActionError {
                    code: "not_found",
                    message: format!("no Open-in command {id}"),
                })?;
            if !has(COMMANDS) {
                return Err(none(&c.label));
            }
            (None, Some(c.argv.clone()))
        } else if let Some(name) = app.strip_prefix("companion:")
            && offer.apps.iter().any(|a| a == name)
        {
            (None, None)
        } else {
            return Err(none(app));
        };
        // The companion knows its own apps by their plain name.
        let app = app.strip_prefix("companion:").unwrap_or(app);
        let a = self.action(who, UserAction::Get).await?;
        let dir = a.workdir.ok_or_else(|| ActionError {
            code: "no_workspace",
            message: format!("{} has no worktree", a.name),
        })?;
        self.companion.push(OpenRequest {
            app: app.into(),
            dir,
            agent: a.name,
            terminal,
            argv,
        });
        Ok(())
    }
}

/// The companion's entries: the terminal, its own apps and the user commands
/// it can run (an older companion offers only its apps).
fn companion_targets(
    offer: Option<&Offer>,
    commands: &[xherd_core::config::OpenCommand],
) -> Vec<Target> {
    let Some(o) = offer else { return Vec::new() };
    let entry = |id: String, label: String| Target {
        id,
        label,
        kind: "companion",
        url: None,
    };
    let has = |f: &str| o.features.iter().any(|x| x == f);
    let mut out = Vec::new();
    if has(TERMINAL) {
        out.push(entry(
            TERMINAL.into(),
            "Terminal (shell in the worktree)".into(),
        ));
    }
    out.extend(o.apps.iter().map(|app| {
        entry(
            format!("companion:{app}"),
            format!("{app} (on this computer)"),
        )
    }));
    if has(COMMANDS) {
        out.extend(
            commands
                .iter()
                .map(|c| entry(format!("cmd:{}", c.id), c.label.clone())),
        );
    }
    out
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

    #[test]
    fn companion_entries_follow_what_it_offers() {
        let cmds = [xherd_core::config::OpenCommand {
            id: "idea".into(),
            label: "IDEA".into(),
            argv: vec!["idea".into(), "{dir}".into()],
        }];
        assert!(companion_targets(None, &cmds).is_empty());
        let old = Offer {
            apps: vec!["code".into()],
            features: vec![],
            ..Default::default()
        };
        let ids = |o: &Offer| -> Vec<String> {
            companion_targets(Some(o), &cmds)
                .into_iter()
                .map(|t| t.id)
                .collect()
        };
        assert_eq!(ids(&old), ["companion:code"]);
        let new = Offer {
            apps: vec!["code".into()],
            features: vec![TERMINAL.into(), COMMANDS.into()],
            ..Default::default()
        };
        assert_eq!(ids(&new), ["terminal", "companion:code", "cmd:idea"]);
    }

    #[tokio::test]
    async fn companion_queue() {
        let c = Companion::default();
        assert!(c.offer().is_none());
        let offer = Offer {
            apps: vec!["idea".into()],
            features: vec![],
            ..Default::default()
        };
        assert_eq!(c.next(offer.clone(), Duration::from_millis(10)).await, None);
        assert_eq!(c.offer(), Some(offer.clone()));
        let waiter = {
            let c = c.clone();
            tokio::spawn(async move { c.next(offer, Duration::from_secs(5)).await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        let r = OpenRequest {
            app: "idea".into(),
            dir: "/w".into(),
            agent: "SC-1".into(),
            terminal: None,
            argv: None,
        };
        c.push(r.clone());
        assert_eq!(waiter.await.unwrap(), Some(r));
    }
}
