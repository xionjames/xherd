//! Open in… (spec §12.6, design §8.7): editor URLs that work because the
//! workspace has the same path on the host (R-701) and the Dev Containers
//! entry (R-702). Every entry is a link the browser opens; nothing on the
//! host listens for the daemon.

use serde::Serialize;

use crate::{ActionError, Daemon, UserAction};

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
    /// offers.
    pub fn open_in_view(&self) -> serde_json::Value {
        let links: Vec<_> = url_targets("/", self.container().as_deref())
            .into_iter()
            .map(|t| serde_json::json!({"id": t.id, "label": t.label}))
            .collect();
        serde_json::json!({ "links": links })
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
}
