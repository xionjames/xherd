//! How xherd talks to a runtime session (design §4.1). v1 has the terminal
//! driver (tmux); the ACP driver comes later behind the same trait.

use std::path::PathBuf;

use async_trait::async_trait;
use xherd_term::{PaneStatus, Tmux};

/// Everything a driver needs to know about the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobCtx {
    pub id: String,
    /// Session name (the agent name).
    pub name: String,
    pub workdir: PathBuf,
    /// Session environment (`XHERD_*`).
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct DriverError(pub String);

pub type Result<T> = std::result::Result<T, DriverError>;

#[async_trait]
pub trait Driver: Send + Sync + 'static {
    /// Start a new session running `argv`. Fails if one already exists.
    async fn start(&self, a: &JobCtx, argv: &[String]) -> Result<()>;
    /// Replace the agent's process with a fresh one, keeping other windows (R-023).
    async fn respawn(&self, a: &JobCtx, argv: &[String]) -> Result<()>;
    /// Type text and press Enter `enters` times (R-020).
    async fn send(&self, a: &JobCtx, text: &str, enters: u32) -> Result<()>;
    async fn interrupt(&self, a: &JobCtx) -> Result<()>;
    /// Press each key in order in the agent's window (tmux key names: `1`,
    /// `Enter`, `Escape`). Used to answer a question on the screen (R22).
    async fn send_keys(&self, a: &JobCtx, keys: &[String]) -> Result<()>;
    /// End the session. Not an error when it is already gone.
    async fn stop(&self, a: &JobCtx) -> Result<()>;
    /// Names of sessions that are alive (reconcile, R-011).
    async fn sessions(&self) -> Result<Vec<String>>;
    /// Last lines of the agent's screen (screen rules, R-018).
    async fn capture(&self, a: &JobCtx, lines: u32) -> Result<String>;
    /// Start the action session `session` running `argv` in `a`'s worktree
    /// with `a.env`; its pane survives the command's exit, with `history`
    /// lines of scrollback (R9, R13).
    async fn start_action(
        &self,
        a: &JobCtx,
        session: &str,
        argv: &[String],
        history: u32,
    ) -> Result<()>;
    /// Pane state of an action session; `None` when it does not exist (R11).
    async fn action_status(&self, session: &str) -> Result<Option<PaneStatus>>;
    /// The scrollback of an action session, cut to its last `max_bytes` (R13).
    async fn action_history(&self, session: &str, max_bytes: usize) -> Result<String>;
    /// Press Ctrl-C in an action session (the first step of Stop, R11).
    async fn interrupt_session(&self, session: &str) -> Result<()>;
    /// End an action session by name. Not an error when it is already gone.
    async fn stop_session(&self, session: &str) -> Result<()>;
}

pub const AGENT_WINDOW: &str = "agent";

pub struct TerminalDriver {
    pub tmux: Tmux,
}

fn err(e: impl std::fmt::Display) -> DriverError {
    DriverError(e.to_string())
}

#[async_trait]
impl Driver for TerminalDriver {
    async fn start(&self, a: &JobCtx, argv: &[String]) -> Result<()> {
        if self.tmux.has_session(&a.name).await {
            return Err(DriverError(format!(
                "a tmux session named {} already exists",
                a.name
            )));
        }
        self.tmux
            .new_session(&a.name, &a.workdir, &a.env, argv)
            .await
            .map_err(err)
    }

    async fn respawn(&self, a: &JobCtx, argv: &[String]) -> Result<()> {
        self.tmux
            .respawn_window(&a.name, Some(AGENT_WINDOW), &a.workdir, argv)
            .await
            .map_err(err)
    }

    async fn send(&self, a: &JobCtx, text: &str, enters: u32) -> Result<()> {
        self.tmux
            .send_text(&a.name, Some(AGENT_WINDOW), text, enters)
            .await
            .map_err(err)
    }

    async fn interrupt(&self, a: &JobCtx) -> Result<()> {
        self.tmux
            .send_key(&a.name, Some(AGENT_WINDOW), "C-c")
            .await
            .map_err(err)
    }

    async fn send_keys(&self, a: &JobCtx, keys: &[String]) -> Result<()> {
        for key in keys {
            self.tmux
                .send_key(&a.name, Some(AGENT_WINDOW), key)
                .await
                .map_err(err)?;
        }
        Ok(())
    }

    async fn stop(&self, a: &JobCtx) -> Result<()> {
        if !self.tmux.has_session(&a.name).await {
            return Ok(());
        }
        self.tmux.kill_session(&a.name).await.map_err(err)
    }

    async fn sessions(&self) -> Result<Vec<String>> {
        self.tmux.list_sessions().await.map_err(err)
    }

    async fn capture(&self, a: &JobCtx, lines: u32) -> Result<String> {
        self.tmux
            .capture(&a.name, Some(AGENT_WINDOW), lines)
            .await
            .map_err(err)
    }

    async fn start_action(
        &self,
        a: &JobCtx,
        session: &str,
        argv: &[String],
        history: u32,
    ) -> Result<()> {
        if self.tmux.has_session(session).await {
            return Err(DriverError(format!(
                "a tmux session named {session} already exists"
            )));
        }
        self.tmux
            .start_held(session, &a.workdir, &a.env, argv, history)
            .await
            .map_err(err)
    }

    async fn action_status(&self, session: &str) -> Result<Option<PaneStatus>> {
        self.tmux.pane_status(session).await.map_err(err)
    }

    async fn action_history(&self, session: &str, max_bytes: usize) -> Result<String> {
        self.tmux.history(session, max_bytes).await.map_err(err)
    }

    async fn interrupt_session(&self, session: &str) -> Result<()> {
        // A held session has one window, the one that runs the command.
        self.tmux.send_key(session, None, "C-c").await.map_err(err)
    }

    async fn stop_session(&self, session: &str) -> Result<()> {
        if !self.tmux.has_session(session).await {
            return Ok(());
        }
        self.tmux.kill_session(session).await.map_err(err)
    }
}

/// Records calls; used by actor tests and by `xherd flow test` later.
#[derive(Default)]
pub struct FakeDriver {
    pub calls: std::sync::Mutex<Vec<String>>,
    pub alive: std::sync::Mutex<std::collections::BTreeSet<String>>,
    pub screen: std::sync::Mutex<String>,
    pub fail_start: std::sync::atomic::AtomicBool,
    /// Action sessions: pane state and scrollback, set by tests to simulate
    /// an exit (`start_action` inserts a running pane with no history).
    pub held: std::sync::Mutex<std::collections::BTreeMap<String, (PaneStatus, String)>>,
    /// An action that ignores Ctrl-C: `interrupt_session` leaves the pane alive.
    pub stubborn: std::sync::atomic::AtomicBool,
}

impl FakeDriver {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn log(&self, s: String) {
        self.calls.lock().unwrap().push(s);
    }
}

#[async_trait]
impl Driver for FakeDriver {
    async fn start(&self, a: &JobCtx, argv: &[String]) -> Result<()> {
        self.log(format!("start {} {}", a.name, argv.join(" ")));
        if self.fail_start.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(DriverError("boom".into()));
        }
        self.alive.lock().unwrap().insert(a.name.clone());
        Ok(())
    }
    async fn respawn(&self, a: &JobCtx, argv: &[String]) -> Result<()> {
        self.log(format!("respawn {} {}", a.name, argv.join(" ")));
        Ok(())
    }
    async fn send(&self, a: &JobCtx, text: &str, enters: u32) -> Result<()> {
        self.log(format!("send {} {text:?} {enters}", a.name));
        Ok(())
    }
    async fn interrupt(&self, a: &JobCtx) -> Result<()> {
        self.log(format!("interrupt {}", a.name));
        Ok(())
    }
    async fn send_keys(&self, a: &JobCtx, keys: &[String]) -> Result<()> {
        self.log(format!("send-keys {} {}", a.name, keys.join(" ")));
        Ok(())
    }
    async fn stop(&self, a: &JobCtx) -> Result<()> {
        self.log(format!("stop {}", a.name));
        self.alive.lock().unwrap().remove(&a.name);
        Ok(())
    }
    async fn sessions(&self) -> Result<Vec<String>> {
        Ok(self.alive.lock().unwrap().iter().cloned().collect())
    }
    async fn capture(&self, _a: &JobCtx, _lines: u32) -> Result<String> {
        Ok(self.screen.lock().unwrap().clone())
    }
    async fn start_action(
        &self,
        a: &JobCtx,
        session: &str,
        argv: &[String],
        history: u32,
    ) -> Result<()> {
        self.log(format!(
            "start-action {session} {} {history} {}",
            a.workdir.display(),
            argv.join(" ")
        ));
        if self.fail_start.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(DriverError("boom".into()));
        }
        self.alive.lock().unwrap().insert(session.to_string());
        let running = PaneStatus {
            dead: false,
            exit: None,
        };
        self.held
            .lock()
            .unwrap()
            .insert(session.to_string(), (running, String::new()));
        Ok(())
    }
    async fn action_status(&self, session: &str) -> Result<Option<PaneStatus>> {
        Ok(self.held.lock().unwrap().get(session).map(|h| h.0))
    }
    async fn action_history(&self, session: &str, max_bytes: usize) -> Result<String> {
        let held = self.held.lock().unwrap();
        let text = held.get(session).map(|h| h.1.as_str()).unwrap_or_default();
        let mut from = text.len().saturating_sub(max_bytes);
        while !text.is_char_boundary(from) {
            from += 1;
        }
        Ok(text[from..].to_string())
    }
    async fn interrupt_session(&self, session: &str) -> Result<()> {
        self.log(format!("interrupt-session {session}"));
        if !self.stubborn.load(std::sync::atomic::Ordering::SeqCst)
            && let Some(h) = self.held.lock().unwrap().get_mut(session)
        {
            h.0 = PaneStatus {
                dead: true,
                exit: Some(130),
            };
        }
        Ok(())
    }
    async fn stop_session(&self, session: &str) -> Result<()> {
        self.log(format!("stop-session {session}"));
        self.alive.lock().unwrap().remove(session);
        self.held.lock().unwrap().remove(session);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> JobCtx {
        JobCtx {
            id: "j1".into(),
            name: "xherd-a".into(),
            workdir: PathBuf::from("/w"),
            env: vec![],
        }
    }

    #[tokio::test]
    async fn fake_records_the_keys_in_order() {
        let d = FakeDriver::default();
        d.send_keys(&ctx(), &["2".to_string(), "Enter".into()])
            .await
            .unwrap();
        assert_eq!(d.calls(), vec!["send-keys xherd-a 2 Enter".to_string()]);
    }

    #[tokio::test]
    async fn fake_action_sessions_start_exit_and_stop() {
        let d = FakeDriver::default();
        let argv = vec!["sh".to_string(), "-c".into(), "x".into()];
        d.start_action(&ctx(), "xherd-a-act-web", &argv, 5000)
            .await
            .unwrap();
        assert_eq!(
            d.action_status("xherd-a-act-web").await.unwrap(),
            Some(PaneStatus {
                dead: false,
                exit: None
            })
        );
        assert!(
            d.sessions()
                .await
                .unwrap()
                .contains(&"xherd-a-act-web".to_string())
        );
        d.held.lock().unwrap().insert(
            "xherd-a-act-web".into(),
            (
                PaneStatus {
                    dead: true,
                    exit: Some(1),
                },
                "héllo\nbye\n".into(),
            ),
        );
        assert_eq!(
            d.action_history("xherd-a-act-web", 3).await.unwrap(),
            "ye\n"
        );
        d.stop_session("xherd-a-act-web").await.unwrap();
        assert_eq!(d.action_status("xherd-a-act-web").await.unwrap(), None);
        assert!(d.sessions().await.unwrap().is_empty());
        assert!(d.calls()[0].starts_with("start-action xherd-a-act-web /w 5000 sh -c x"));
    }
}
