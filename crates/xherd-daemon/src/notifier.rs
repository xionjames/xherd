//! Notifications to ntfy (spec §16.3): every hub `Notify` is also posted to
//! the ntfy topic in `config.json` (`ntfyServer` + `ntfyTopic`), read live, so
//! nothing leaves the machine until both are set. The text carries agent names
//! and reasons, never PR comment text or document contents.

use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use xherd_core::exec::{Cmd, run};

use crate::Daemon;
use crate::support::Change;

/// `http(s)://host[:port][/path]`, no spaces or control characters.
pub fn valid_server(s: &str) -> bool {
    (s.starts_with("https://") || s.starts_with("http://"))
        && s.len() > 8
        && s.len() <= 200
        && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// ntfy topics are plain names; the topic is part of the URL path.
pub fn valid_topic(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Where a message is posted, when both settings are valid.
pub fn endpoint(server: Option<&str>, topic: Option<&str>) -> Option<String> {
    let (s, t) = (server?.trim_end_matches('/'), topic?);
    (valid_server(s) && valid_topic(t)).then(|| format!("{s}/{t}"))
}

/// The curl command for one message: text on stdin, the kind as a tag.
fn post(url: &str, kind: &str, text: &str) -> Cmd {
    Cmd::new([
        "curl",
        "-sS",
        "--fail",
        "--max-time",
        "10",
        "-H",
        "Title: xherd",
        "-H",
        &format!("Tags: {kind}"),
        "--data-binary",
        "@-",
        url,
    ])
    .stdin(text.to_string())
    .timeout(Duration::from_secs(15))
}

/// An operating system notification through the platform's tool; the text
/// is an argument, never part of a script. `None` where there is no tool.
fn desktop(text: &str) -> Option<Cmd> {
    let argv: Vec<String> = if cfg!(target_os = "macos") {
        [
            "osascript",
            "-e",
            "on run argv",
            "-e",
            "display notification (item 1 of argv) with title \"xherd\"",
            "-e",
            "end run",
            text,
        ]
        .map(String::from)
        .to_vec()
    } else if cfg!(target_os = "linux") {
        ["notify-send", "--app-name=xherd", "--", "xherd", text]
            .map(String::from)
            .to_vec()
    } else {
        return None;
    };
    Some(Cmd::new(argv).timeout(Duration::from_secs(10)))
}

pub fn spawn(d: Daemon) -> tokio::task::JoinHandle<()> {
    let mut rx = d.hub.subscribe();
    tokio::spawn(async move {
        loop {
            let (kind, text) = match rx.recv().await {
                Ok(Change::Notify { kind, text, .. }) => (kind, text),
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return,
            };
            let c = d.live_config();
            if c.desktop_notifications
                && !d.sandbox
                && let Some(cmd) = desktop(&text)
                && let Err(e) = run(cmd).await
            {
                tracing::warn!(err = %e, "desktop notification failed");
            }
            let Some(url) = endpoint(c.ntfy_server.as_deref(), c.ntfy_topic.as_deref()) else {
                continue;
            };
            match run(post(&url, &kind, &text)).await {
                Ok(o) if o.code == Some(0) => {}
                Ok(o) => tracing::warn!(code = ?o.code, err = %o.stderr, "ntfy post failed"),
                Err(e) => tracing::warn!(err = %e, "ntfy post failed"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_needs_both_and_valid_values() {
        let e = |s, t| endpoint(s, t);
        assert_eq!(
            e(Some("https://ntfy.example.com/"), Some("xherd-me")).as_deref(),
            Some("https://ntfy.example.com/xherd-me")
        );
        assert_eq!(e(Some("https://ntfy.example.com"), None), None);
        assert_eq!(e(None, Some("t")), None);
        assert_eq!(e(Some("ftp://x"), Some("t")), None);
        assert_eq!(e(Some("https://x y"), Some("t")), None);
        assert_eq!(e(Some("https://x"), Some("a/b")), None);
        assert_eq!(e(Some("https://x"), Some("")), None);
    }

    #[test]
    fn desktop_text_is_an_argument() {
        if let Some(c) = desktop("SC-1: a \"quoted\" reason") {
            assert!(c.argv.last().unwrap().contains("quoted"));
            assert!(
                !c.argv
                    .iter()
                    .take(c.argv.len() - 1)
                    .any(|a| a.contains("quoted"))
            );
        }
    }

    #[test]
    fn the_text_goes_on_stdin_not_in_argv() {
        let c = post("https://x/t", "gate", "SC-1: secret reason");
        assert!(!c.argv.iter().any(|a| a.contains("secret reason")));
        assert!(c.argv.contains(&"Tags: gate".to_string()));
    }
}
