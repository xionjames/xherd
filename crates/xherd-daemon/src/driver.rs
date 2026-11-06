//! How xherd talks to a runtime session (design §4.1). v1 has the terminal
//! driver (tmux); the ACP driver comes later behind the same trait.

use std::path::PathBuf;

use async_trait::async_trait;
use xherd_term::Tmux;

/// Everything a driver needs to know about the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentCtx {
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
    async fn start(&self, a: &AgentCtx, argv: &[String]) -> Result<()>;
    /// Replace the agent's process with a fresh one, keeping other windows (R-023).
    async fn respawn(&self, a: &AgentCtx, argv: &[String]) -> Result<()>;
    /// Type text and press Enter `enters` times (R-020).
    async fn send(&self, a: &AgentCtx, text: &str, enters: u32) -> Result<()>;
    async fn interrupt(&self, a: &AgentCtx) -> Result<()>;
    /// End the session. Not an error when it is already gone.
    async fn stop(&self, a: &AgentCtx) -> Result<()>;
    /// Names of sessions that are alive (reconcile, R-011).
    async fn sessions(&self) -> Result<Vec<String>>;
    /// Last lines of the agent's screen (screen rules, R-018).
    async fn capture(&self, a: &AgentCtx, lines: u32) -> Result<String>;
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
    async fn start(&self, a: &AgentCtx, argv: &[String]) -> Result<()> {
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

    async fn respawn(&self, a: &AgentCtx, argv: &[String]) -> Result<()> {
        self.tmux
            .respawn_window(&a.name, Some(AGENT_WINDOW), &a.workdir, argv)
            .await
            .map_err(err)
    }

    async fn send(&self, a: &AgentCtx, text: &str, enters: u32) -> Result<()> {
        self.tmux
            .send_text(&a.name, Some(AGENT_WINDOW), text, enters)
            .await
            .map_err(err)
    }

    async fn interrupt(&self, a: &AgentCtx) -> Result<()> {
        self.tmux
            .send_key(&a.name, Some(AGENT_WINDOW), "C-c")
            .await
            .map_err(err)
    }

    async fn stop(&self, a: &AgentCtx) -> Result<()> {
        if !self.tmux.has_session(&a.name).await {
            return Ok(());
        }
        self.tmux.kill_session(&a.name).await.map_err(err)
    }

    async fn sessions(&self) -> Result<Vec<String>> {
        self.tmux.list_sessions().await.map_err(err)
    }

    async fn capture(&self, a: &AgentCtx, lines: u32) -> Result<String> {
        self.tmux
            .capture(&a.name, Some(AGENT_WINDOW), lines)
            .await
            .map_err(err)
    }
}

/// Records calls; used by actor tests and by `xherd flow test` later.
#[derive(Default)]
pub struct FakeDriver {
    pub calls: std::sync::Mutex<Vec<String>>,
    pub alive: std::sync::Mutex<std::collections::BTreeSet<String>>,
    pub screen: std::sync::Mutex<String>,
    pub fail_start: std::sync::atomic::AtomicBool,
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
    async fn start(&self, a: &AgentCtx, argv: &[String]) -> Result<()> {
        self.log(format!("start {} {}", a.name, argv.join(" ")));
        if self.fail_start.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(DriverError("boom".into()));
        }
        self.alive.lock().unwrap().insert(a.name.clone());
        Ok(())
    }
    async fn respawn(&self, a: &AgentCtx, argv: &[String]) -> Result<()> {
        self.log(format!("respawn {} {}", a.name, argv.join(" ")));
        Ok(())
    }
    async fn send(&self, a: &AgentCtx, text: &str, enters: u32) -> Result<()> {
        self.log(format!("send {} {text:?} {enters}", a.name));
        Ok(())
    }
    async fn interrupt(&self, a: &AgentCtx) -> Result<()> {
        self.log(format!("interrupt {}", a.name));
        Ok(())
    }
    async fn stop(&self, a: &AgentCtx) -> Result<()> {
        self.log(format!("stop {}", a.name));
        self.alive.lock().unwrap().remove(&a.name);
        Ok(())
    }
    async fn sessions(&self) -> Result<Vec<String>> {
        Ok(self.alive.lock().unwrap().iter().cloned().collect())
    }
    async fn capture(&self, _a: &AgentCtx, _lines: u32) -> Result<String> {
        Ok(self.screen.lock().unwrap().clone())
    }
}
