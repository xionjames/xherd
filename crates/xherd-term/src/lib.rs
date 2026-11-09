//! tmux wrapper, screen rules, and the PTY bridge for browser terminals.

pub mod pty;

use std::path::Path;
use std::time::Duration;

use regex::Regex;
use xherd_core::exec::{Cmd, ExecError, run};

/// Pause between typed text and each Enter.
const ENTER_PAUSE: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Debug, thiserror::Error)]
pub enum TermError {
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error("tmux {args} failed: {stderr}")]
    Failed { args: String, stderr: String },
    #[error("invalid screen rule {pattern:?}: {source}")]
    Rule {
        pattern: String,
        source: regex::Error,
    },
}

pub type Result<T> = std::result::Result<T, TermError>;

/// tmux client bound to one server: `-L <name>`, `-S <path>` when the socket
/// has a `/` (the container: the server runs as another user, R-1203), or the
/// default server.
#[derive(Debug, Clone, Default)]
pub struct Tmux {
    socket: Option<String>,
}

/// Exact session target (`=name`), so `SC-1` never matches `SC-10`.
fn session(name: &str) -> String {
    format!("={name}")
}

/// Exact window target in a session; `None` = the session's current window.
fn window(name: &str, win: Option<&str>) -> String {
    format!("={name}:{}", win.unwrap_or(""))
}

impl Tmux {
    pub fn new(socket: Option<String>) -> Tmux {
        Tmux { socket }
    }

    /// The flags that select the server.
    fn server_args(&self) -> Vec<String> {
        match &self.socket {
            Some(s) if s.contains('/') => vec!["-S".into(), s.clone()],
            Some(s) => vec!["-L".into(), s.clone()],
            None => Vec::new(),
        }
    }

    /// argv of a tmux client attached to the session (browser terminal).
    pub fn attach_argv(&self, name: &str) -> Vec<String> {
        let mut argv = vec!["tmux".to_string()];
        argv.extend(self.server_args());
        argv.extend(["attach-session".into(), "-t".into(), session(name)]);
        argv
    }

    pub async fn run(&self, args: &[&str]) -> Result<String> {
        let mut argv = vec!["tmux".to_string()];
        argv.extend(self.server_args());
        argv.extend(args.iter().map(|s| s.to_string()));
        let out = run(Cmd::new(argv).timeout(Duration::from_secs(10))).await?;
        if !out.success() {
            return Err(TermError::Failed {
                args: args.join(" "),
                stderr: out.stderr.trim().to_string(),
            });
        }
        Ok(out.stdout_str())
    }

    /// Start a detached session running `argv` directly (no shell) in `cwd`,
    /// with `env` set in the session environment.
    pub async fn new_session(
        &self,
        name: &str,
        cwd: &Path,
        env: &[(String, String)],
        argv: &[String],
    ) -> Result<()> {
        let c = cwd.to_string_lossy();
        let mut args: Vec<String> =
            vec!["new-session".into(), "-d".into(), "-s".into(), name.into()];
        args.extend(["-n".into(), "agent".into(), "-c".into(), c.into_owned()]);
        args.extend(["-x".into(), "200".into(), "-y".into(), "50".into()]);
        for (k, v) in env {
            args.extend(["-e".into(), format!("{k}={v}")]);
        }
        args.extend(argv.iter().cloned());
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&refs).await.map(drop)
    }

    /// Add a named window (e.g. toolchain setup, dev server) to a session.
    pub async fn new_window(
        &self,
        name: &str,
        win: &str,
        cwd: &Path,
        argv: &[String],
    ) -> Result<()> {
        let c = cwd.to_string_lossy();
        let target = session(name);
        let mut args = vec!["new-window", "-d", "-t", &target, "-n", win, "-c", &c];
        args.extend(argv.iter().map(String::as_str));
        self.run(&args).await.map(drop)
    }

    pub async fn has_session(&self, name: &str) -> bool {
        self.run(&["has-session", "-t", &session(name)])
            .await
            .is_ok()
    }

    /// Session names; empty when no server is running.
    pub async fn list_sessions(&self) -> Result<Vec<String>> {
        match self.run(&["list-sessions", "-F", "#{session_name}"]).await {
            Ok(out) => Ok(out.lines().map(str::to_string).collect()),
            Err(TermError::Failed { stderr, .. })
                if stderr.contains("no server running") || stderr.contains("error connecting") =>
            {
                Ok(Vec::new())
            }
            Err(e) => Err(e),
        }
    }

    pub async fn kill_session(&self, name: &str) -> Result<()> {
        self.run(&["kill-session", "-t", &session(name)])
            .await
            .map(drop)
    }

    /// Type `text` literally, then press Enter `enters` times (R-020).
    pub async fn send_text(
        &self,
        name: &str,
        win: Option<&str>,
        text: &str,
        enters: u32,
    ) -> Result<()> {
        let t = window(name, win);
        if !text.is_empty() {
            self.run(&["send-keys", "-t", &t, "-l", "--", text]).await?;
        }
        for _ in 0..enters {
            // Without a pause a TUI can take the keys as one paste and drop
            // the Enter (seen with Claude Code 2.1.319).
            tokio::time::sleep(ENTER_PAUSE).await;
            self.run(&["send-keys", "-t", &t, "Enter"]).await?;
        }
        Ok(())
    }

    /// Send a key name such as `C-c` or `Escape`.
    pub async fn send_key(&self, name: &str, win: Option<&str>, key: &str) -> Result<()> {
        self.run(&["send-keys", "-t", &window(name, win), key])
            .await
            .map(drop)
    }

    /// The last `lines` lines of the visible pane plus scrollback.
    pub async fn capture(&self, name: &str, win: Option<&str>, lines: u32) -> Result<String> {
        let start = format!("-{lines}");
        self.run(&[
            "capture-pane",
            "-p",
            "-J",
            "-t",
            &window(name, win),
            "-S",
            &start,
        ])
        .await
    }

    /// Replace a window's process with `argv`, keeping other windows (R-023).
    pub async fn respawn_window(
        &self,
        name: &str,
        win: Option<&str>,
        cwd: &Path,
        argv: &[String],
    ) -> Result<()> {
        let c = cwd.to_string_lossy();
        let t = window(name, win);
        let mut args = vec!["respawn-window", "-k", "-t", &t, "-c", &c];
        args.extend(argv.iter().map(String::as_str));
        self.run(&args).await.map(drop)
    }

    pub async fn set_environment(&self, name: &str, key: &str, value: &str) -> Result<()> {
        self.run(&["set-environment", "-t", &session(name), key, value])
            .await
            .map(drop)
    }

    pub async fn kill_server(&self) -> Result<()> {
        self.run(&["kill-server"]).await.map(drop)
    }
}

/// Screen rules of a runtime (R-018): patterns that mean "a dialog is shown".
#[derive(Debug, Clone, Default)]
pub struct ScreenRules {
    rules: Vec<Regex>,
    /// How many trailing non-empty lines are matched.
    pub last_lines: usize,
}

impl ScreenRules {
    pub fn new<S: AsRef<str>>(patterns: &[S], last_lines: usize) -> Result<ScreenRules> {
        let rules = patterns
            .iter()
            .map(|p| {
                Regex::new(p.as_ref()).map_err(|source| TermError::Rule {
                    pattern: p.as_ref().to_string(),
                    source,
                })
            })
            .collect::<Result<_>>()?;
        Ok(ScreenRules { rules, last_lines })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The first pattern that matches the tail of `screen`.
    pub fn matching(&self, screen: &str) -> Option<&str> {
        let lines: Vec<&str> = screen.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(self.last_lines)..].join("\n");
        self.rules
            .iter()
            .find(|r| r.is_match(&tail))
            .map(Regex::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_only_look_at_the_tail() {
        let r = ScreenRules::new(&[r"Do you want to proceed\?"], 3).unwrap();
        assert!(
            r.matching("x\n\nDo you want to proceed?\n 1. Yes\n 2. No\n\n")
                .is_some()
        );
        assert!(
            r.matching("Do you want to proceed?\na\nb\nc\nd\n")
                .is_none()
        );
        assert!(ScreenRules::new(&["("], 3).is_err());
    }

    #[test]
    fn targets_are_exact() {
        assert_eq!(session("SC-1"), "=SC-1");
        assert_eq!(window("SC-1", None), "=SC-1:");
        assert_eq!(window("SC-1", Some("agent")), "=SC-1:agent");
    }
}
