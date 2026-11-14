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

/// What the installer reports (`POST /open-in/terminal-handler`): a record,
/// or `{"installed": false}` after an uninstall.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum HandlerReport {
    Removed {
        installed: bool,
    },
    Installed {
        version: u32,
        xherd: String,
        os: String,
        terminals: Vec<String>,
        root: String,
    },
}

impl HandlerReport {
    /// Checks the report and turns it into the record to keep (`None`:
    /// remove it). `at` is now.
    pub fn into_record(self, at: i64) -> Result<Option<HandlerRecord>, String> {
        match self {
            HandlerReport::Removed { installed: false } => Ok(None),
            HandlerReport::Removed { installed: true } => {
                Err("an installed handler reports its version".into())
            }
            HandlerReport::Installed {
                version,
                xherd,
                os,
                terminals,
                root,
            } => {
                let plain = |what: &str, v: &str| {
                    if v.is_empty() || v.len() > 4096 || v.chars().any(char::is_control) {
                        Err(format!("bad {what}"))
                    } else {
                        Ok(())
                    }
                };
                if version < 1 {
                    return Err("bad version".into());
                }
                plain("xherd", &xherd)?;
                plain("root", &root)?;
                if os != "mac" && os != "linux" {
                    return Err("os must be mac or linux".into());
                }
                if terminals.len() > 16
                    || terminals
                        .iter()
                        .any(|t| !xherd_core::config::TERMINAL_APPS.contains(&t.as_str()))
                {
                    return Err("unknown terminal".into());
                }
                Ok(Some(HandlerRecord {
                    version,
                    xherd,
                    os,
                    terminals,
                    root,
                    at,
                }))
            }
        }
    }
}

/// The link handler (`xherd-open-terminal`), shipped inside the installer.
pub const HANDLER_SH: &str = include_str!("terminal-handler.sh");
/// The installer template that `handler_script` renders.
pub const INSTALL_SH: &str = include_str!("terminal-install.sh");

/// The installer for this computer: `root` (the worktrees folder the handler
/// accepts) and `report_url` (where it reports, `http://host:port`) are baked
/// into single-quoted shell literals, the AppleScript and the report's JSON,
/// so a value with a quote, `\`, `$`, a backquote or a control character is
/// refused.
pub fn handler_script(root: &str, report_url: &str) -> Result<String, String> {
    let plain = |what: &str, v: &str| {
        if v.is_empty()
            || v.chars()
                .any(|c| c.is_control() || matches!(c, '\'' | '"' | '\\' | '$' | '`'))
        {
            Err(format!(
                "the {what} has a character the handler cannot carry"
            ))
        } else {
            Ok(())
        }
    };
    plain("worktrees folder", root)?;
    if !root.starts_with('/') {
        return Err("the worktrees folder is not an absolute path".into());
    }
    plain("report address", report_url)?;
    if !(report_url.starts_with("http://") || report_url.starts_with("https://"))
        || report_url.contains(char::is_whitespace)
    {
        return Err("the report address is not an http address".into());
    }
    plain("xherd version", xherd_core::BUILD_VERSION)?;
    let root = root.trim_end_matches('/');
    let root = if root.is_empty() { "/" } else { root };
    // Values first, then the handler, so nothing in it is ever replaced.
    let script = INSTALL_SH
        .replace("@ROOT@", root)
        .replace("@REPORT_URL@", report_url.trim_end_matches('/'))
        .replace("@VERSION@", &HANDLER_VERSION.to_string())
        .replace("@XHERD@", xherd_core::BUILD_VERSION);
    let handler = format!("{}\n", HANDLER_SH.trim_end_matches('\n'));
    Ok(script.replacen("@HANDLER@\n", &handler, 1))
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

    /// `POST /open-in/terminal-handler/install` (native only): runs the
    /// installer as the user the daemon runs as, reporting to this daemon on
    /// loopback. Returns whether it succeeded and the end of its output.
    pub async fn install_terminal_handler(
        &self,
        uninstall: bool,
    ) -> Result<(bool, String), ActionError> {
        if self.sandbox {
            return Err(ActionError {
                code: "native_only",
                message: "xherd runs in a container: run the command from Settings → Open in \
                          on your computer"
                    .into(),
            });
        }
        let report = format!("http://127.0.0.1:{}", self.config.port);
        let script = handler_script(&self.worktrees_root(), &report).map_err(|m| ActionError {
            code: "bad_config",
            message: m,
        })?;
        let op = if uninstall { "uninstall" } else { "install" };
        let mut cmd = xherd_core::exec::Cmd::new(["sh", "-s", op])
            .stdin(script)
            .timeout(std::time::Duration::from_secs(60));
        // Not in the allowlist, but xdg-mime and terminal detection read them.
        for k in [
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "TERMINAL",
            "XDG_CURRENT_DESKTOP",
            "DBUS_SESSION_BUS_ADDRESS",
            "DISPLAY",
            "WAYLAND_DISPLAY",
        ] {
            if let Ok(v) = std::env::var(k) {
                cmd = cmd.env(k, v);
            }
        }
        let out = xherd_core::exec::run(cmd).await.map_err(|e| ActionError {
            code: "io",
            message: format!("could not run the installer: {e}"),
        })?;
        let text = format!("{}{}", out.stdout_str(), out.stderr);
        let lines: Vec<&str> = text.lines().collect();
        let tail = lines[lines.len().saturating_sub(20)..].join("\n");
        Ok((out.success(), tail))
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

    /// A change to either script must bump `HANDLER_VERSION` (so installed
    /// handlers are offered the update) and then update these hashes.
    #[test]
    fn handler_version_is_pinned_to_the_scripts() {
        use sha2::{Digest, Sha256};
        let sha = |s: &str| hex::encode(Sha256::digest(s.as_bytes()));
        assert_eq!(
            (HANDLER_VERSION, sha(HANDLER_SH), sha(INSTALL_SH)),
            (
                1,
                "6235be7359183ae7050b788df7cc07cb1c723fe98b7c0a35c7009287758473cf".to_string(),
                "c827fca1d327bdcd6f3b0d6ba17bb501a77467570441ca9d17efd92ac9d21050".to_string()
            ),
            "a script changed: bump HANDLER_VERSION and update the pinned hashes"
        );
    }

    /// The installer's quoted heredoc ends at this line; the handler must not
    /// contain it.
    const HEREDOC_END: &str = "XHERD_HANDLER_EOF";

    #[test]
    fn the_handler_never_ends_the_heredoc() {
        assert!(!HANDLER_SH.contains(HEREDOC_END));
        assert_eq!(INSTALL_SH.matches(HEREDOC_END).count(), 2);
        assert_eq!(INSTALL_SH.matches("\n@HANDLER@\n").count(), 1);
    }

    #[test]
    fn handler_script_bakes_in_plain_values() {
        let s = handler_script("/home/me/x herd/worktrees/", "http://127.0.0.1:4740").unwrap();
        assert!(s.contains("\nroot='/home/me/x herd/worktrees'\n"));
        assert!(s.contains("\nreport_url='http://127.0.0.1:4740'\n"));
        assert!(s.contains(&format!("\nhandler_version='{HANDLER_VERSION}'\n")));
        assert!(s.contains(HANDLER_SH));
        // The handler's own text is not touched by the replacements.
        let rest = s.replace(HANDLER_SH, "");
        for p in [
            "@ROOT@",
            "@REPORT_URL@",
            "@VERSION@",
            "@XHERD@",
            "@HANDLER@",
        ] {
            assert!(!rest.contains(p), "{p} left");
        }
    }

    #[test]
    fn handler_script_refuses_what_breaks_the_quoting() {
        let url = "http://127.0.0.1:4740";
        for root in [
            "/a'b", "/a\"b", "/a\\b", "/a$b", "/a`b`", "/a\nb", "/a\tb", "/a\u{7f}", "", "rel",
        ] {
            assert!(handler_script(root, url).is_err(), "{root:?}");
        }
        for url in [
            "http://h'",
            "http://h$(x)",
            "http://h\n",
            "http://h x",
            "ftp://h",
            "h:1",
            "",
        ] {
            assert!(handler_script("/w", url).is_err(), "{url:?}");
        }
        assert!(handler_script("/w", "https://[::1]:4740/").is_ok());
    }
}
