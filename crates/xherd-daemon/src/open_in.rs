//! Open in… (spec §12.6, design §8.7): editor URLs that work because the
//! workspace has the same path on the host (R-701), the Dev Containers
//! entry (R-702), the user's own links (R-706) and the `xherd-term://` link
//! a handler on the user's computer opens in a terminal (R-707). Every entry
//! is a link the browser opens; nothing on the host listens for the daemon.

use serde::{Deserialize, Serialize};
use xherd_core::config::{OpenIn, OpenLink};
use xherd_core::paths::write_atomic;

use crate::{ActionError, Daemon, UserAction};

/// Version of the link handler and its installer; bumped whenever either
/// script changes, so the UI offers an update.
pub const HANDLER_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub id: String,
    pub label: String,
    /// Always `url` (a link the browser opens); kept for the UI and later kinds.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// What the installed handler reported (`<data>/terminal-handler.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HandlerRecord {
    pub version: u32,
    /// The xherd build that rendered the installer.
    pub xherd: String,
    /// `mac` or `linux`.
    pub os: String,
    /// Installed terminals, the desktop's default first.
    pub terminals: Vec<String>,
    /// The worktrees root the handler accepts.
    pub root: String,
    pub at: i64,
}

/// Percent-encodes every byte but the unreserved ones; `keep_slash` keeps `/`.
fn pct(s: &str, keep_slash: bool) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            b'/' if keep_slash => "/".into(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A path for a URL: each segment encoded, `/` kept.
pub fn pct_path(s: &str) -> String {
    pct(s, true)
}

/// A query value or a single segment: `/` encoded too.
pub fn pct_all(s: &str) -> String {
    pct(s, false)
}

fn link(id: &str, label: &str, u: String) -> Target {
    Target {
        id: id.into(),
        label: label.into(),
        kind: "url",
        url: Some(u),
    }
}

/// URL-scheme entries for a folder (R-701), and Dev Containers in the
/// container (R-702, `container`: its id, the hostname).
pub fn url_targets(dir: &str, container: Option<&str>) -> Vec<Target> {
    let enc = pct_path(dir);
    let mut out = vec![
        link("vscode", "VS Code", format!("vscode://file{enc}")),
        link("cursor", "Cursor", format!("cursor://file{enc}")),
        link("zed", "Zed", format!("zed://file{enc}")),
    ];
    if let Some(c) = container.filter(|c| !c.is_empty()) {
        out.push(link(
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

/// The `xherd-term://` link (R-707), with the chosen terminal when one is set.
pub fn terminal_link(dir: &str, terminal: Option<&str>) -> Target {
    let mut u = format!("xherd-term://open?dir={}", pct_path(dir));
    if let Some(t) = terminal.filter(|t| !t.is_empty()) {
        u.push_str("&terminal=");
        u.push_str(&pct_all(t));
    }
    link("terminal", "Terminal", u)
}

/// The user's links (R-706), `{dir}` and `{agent}` replaced, encoded.
pub fn user_links(dir: &str, agent: &str, links: &[OpenLink]) -> Vec<Target> {
    let (dir, agent) = (pct_path(dir), pct_all(agent));
    links
        .iter()
        .map(|l| {
            link(
                &format!("link:{}", l.id),
                &l.label,
                l.url.replace("{dir}", &dir).replace("{agent}", &agent),
            )
        })
        .collect()
}

/// Every entry for an agent's worktree: editors, the terminal (only once a
/// handler reported, R-707), the user's links, minus the hidden ids.
pub fn agent_targets(
    dir: &str,
    agent: &str,
    container: Option<&str>,
    cfg: &OpenIn,
    handler: bool,
) -> Vec<Target> {
    let mut out = url_targets(dir, container);
    if handler {
        out.push(terminal_link(dir, cfg.terminal.as_deref()));
    }
    out.extend(user_links(dir, agent, &cfg.links));
    out.retain(|t| !cfg.hidden.contains(&t.id));
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
        Ok(agent_targets(
            &dir,
            &a.name,
            self.container().as_deref(),
            &self.live_config().open_in,
            self.handler_record().is_some(),
        ))
    }

    /// The container id (its hostname), in the container only (R-702).
    fn container(&self) -> Option<String> {
        self.sandbox
            .then(|| std::fs::read_to_string("/etc/hostname").ok())
            .flatten()
            .map(|h| h.trim().to_string())
    }

    /// The installed handler's last report, if any (a file that does not
    /// parse counts as none).
    pub fn handler_record(&self) -> Option<HandlerRecord> {
        let bytes = std::fs::read(self.paths.terminal_handler()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Stores the handler's report, or removes it (`None`: uninstalled).
    pub fn record_handler(&self, rec: Option<&HandlerRecord>) -> std::io::Result<()> {
        let path = self.paths.terminal_handler();
        match rec {
            Some(r) => write_atomic(
                &path,
                &serde_json::to_vec_pretty(r).map_err(std::io::Error::other)?,
                Some(0o600),
            ),
            None => match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        }
    }

    /// The worktrees root the handler must accept, from the live config.
    pub fn worktrees_root(&self) -> String {
        self.live_config()
            .workspace_root()
            .join("worktrees")
            .to_string_lossy()
            .into_owned()
    }

    /// `GET /open-in`, for Settings → Open in: the link entries this daemon
    /// offers and the handler's state.
    pub fn open_in_view(&self) -> serde_json::Value {
        let links: Vec<_> = url_targets("/", self.container().as_deref())
            .into_iter()
            .map(|t| serde_json::json!({"id": t.id, "label": t.label}))
            .collect();
        let rec = self.handler_record();
        let update = rec.as_ref().is_some_and(|r| r.version < HANDLER_VERSION);
        let moved = rec
            .as_ref()
            .is_some_and(|r| r.root != self.worktrees_root());
        serde_json::json!({
            "links": links,
            "native": !self.sandbox,
            "handler": {
                "shipped": HANDLER_VERSION,
                "installed": rec,
                "updateAvailable": update,
                "rootChanged": moved,
            },
        })
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

    #[test]
    fn terminal_link_encodes_the_folder() {
        let t = terminal_link("/home/u/x herd/wt/a#b&c/é", Some("kitty"));
        assert_eq!(t.id, "terminal");
        assert_eq!(t.kind, "url");
        assert_eq!(
            t.url.as_deref(),
            Some("xherd-term://open?dir=/home/u/x%20herd/wt/a%23b%26c/%C3%A9&terminal=kitty")
        );
        let t = terminal_link("/w/app/fix", None);
        assert_eq!(t.url.as_deref(), Some("xherd-term://open?dir=/w/app/fix"));
        let t = terminal_link("/w/app/fix", Some(""));
        assert_eq!(t.url.as_deref(), Some("xherd-term://open?dir=/w/app/fix"));
    }

    #[test]
    fn user_links_replace_dir_and_agent() {
        let links = [OpenLink {
            id: "idea".into(),
            label: "IntelliJ IDEA".into(),
            url: "idea://open?file={dir}&name={agent}&again={dir}".into(),
        }];
        let t = user_links("/w/my app/a&b", "x/y z", &links);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].id, "link:idea");
        assert_eq!(t[0].label, "IntelliJ IDEA");
        assert_eq!(
            t[0].url.as_deref(),
            Some("idea://open?file=/w/my%20app/a%26b&name=x%2Fy%20z&again=/w/my%20app/a%26b")
        );
    }

    #[test]
    fn terminal_only_with_a_record_and_hidden_ids_go() {
        let ids = |t: Vec<Target>| t.into_iter().map(|t| t.id).collect::<Vec<_>>();
        let mut cfg = OpenIn {
            links: vec![OpenLink {
                id: "idea".into(),
                label: "IDEA".into(),
                url: "idea://open?file={dir}".into(),
            }],
            ..OpenIn::default()
        };
        assert_eq!(
            ids(agent_targets("/w/a/b", "b", None, &cfg, false)),
            ["vscode", "cursor", "zed", "link:idea"]
        );
        assert_eq!(
            ids(agent_targets("/w/a/b", "b", None, &cfg, true)),
            ["vscode", "cursor", "zed", "terminal", "link:idea"]
        );
        cfg.hidden = vec!["zed".into(), "terminal".into(), "link:idea".into()];
        assert_eq!(
            ids(agent_targets("/w/a/b", "b", None, &cfg, true)),
            ["vscode", "cursor"]
        );
    }

    #[test]
    fn pct_keeps_or_encodes_the_slash() {
        assert_eq!(pct_path("/a b/c"), "/a%20b/c");
        assert_eq!(pct_all("/a b/c"), "%2Fa%20b%2Fc");
        assert_eq!(pct_all("A-z_0.9~"), "A-z_0.9~");
    }

    #[test]
    fn record_round_trips_as_camel_case() {
        let r = HandlerRecord {
            version: 1,
            xherd: "0.3.0".into(),
            os: "linux".into(),
            terminals: vec!["konsole".into()],
            root: "/w/worktrees".into(),
            at: 5,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["terminals"][0], "konsole");
        assert_eq!(serde_json::from_value::<HandlerRecord>(v).unwrap(), r);
    }
}
