//! PTY bridge for browser terminals (`/ws/term/:id`, design §13.1): runs a
//! tmux client (`tmux attach`) inside a pseudo-terminal. Output arrives on a
//! channel; input and resizes go back to the PTY. Dropping the bridge kills
//! the client, never the agent's session.
//!
//! This is the one interactive process xherd runs, so it does not go through
//! `exec::run` (which captures output and times out); the environment is
//! still an allowlist.

use std::io::{Read, Write};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::mpsc;

/// Variables passed to the tmux client: enough to find the server and draw.
const ENV_KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMUX_TMPDIR",
];

pub struct PtyBridge {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyBridge {
    /// Start `argv` in a new PTY of `cols`×`rows`; output chunks go to the
    /// returned receiver, which closes when the process exits.
    pub fn spawn(
        argv: &[String],
        cols: u16,
        rows: u16,
    ) -> std::io::Result<(PtyBridge, mpsc::Receiver<Vec<u8>>)> {
        let pair = native_pty_system().openpty(size(cols, rows)).map_err(io)?;
        let mut cmd = CommandBuilder::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.env_clear();
        for k in ENV_KEEP {
            if let Ok(v) = std::env::var(k) {
                cmd.env(k, v);
            }
        }
        cmd.env("TERM", "xterm-256color");
        let child = pair.slave.spawn_command(cmd).map_err(io)?;
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().map_err(io)?;
        let writer = pair.master.take_writer().map_err(io)?;
        let (tx, rx) = mpsc::channel(64);
        std::thread::Builder::new()
            .name("xherd-pty".into())
            .spawn(move || {
                let mut buf = [0u8; 16 * 1024];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => return,
                        Ok(n) => {
                            if tx.blocking_send(buf[..n].to_vec()).is_err() {
                                return;
                            }
                        }
                    }
                }
            })?;
        Ok((
            PtyBridge {
                master: pair.master,
                writer,
                child,
            },
            rx,
        ))
    }

    /// Keystrokes from the browser. Small writes; a PTY buffer absorbs them.
    pub fn input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()
    }

    pub fn resize(&self, cols: u16, rows: u16) -> std::io::Result<()> {
        self.master.resize(size(cols, rows)).map_err(io)
    }
}

impl Drop for PtyBridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.clamp(2, 500),
        cols: cols.clamp(10, 1000),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// portable-pty returns `anyhow::Error`; keep that crate out of our API.
fn io(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn until(rx: &mut mpsc::Receiver<Vec<u8>>, want: &str) -> String {
        let mut seen = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(b) = rx.recv().await {
                seen.push_str(&String::from_utf8_lossy(&b));
                if seen.contains(want) {
                    return;
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no {want:?} in {seen:?}"));
        seen
    }

    #[tokio::test]
    async fn output_input_and_env_allowlist() {
        // The environment is an allowlist: TERM is ours, TMUX is never passed.
        let argv: Vec<String> = ["sh", "-c", "echo term=$TERM tmux=${TMUX:-none}; cat"]
            .map(String::from)
            .to_vec();
        let (mut p, mut rx) = PtyBridge::spawn(&argv, 80, 24).unwrap();
        let out = until(&mut rx, "tmux=none").await;
        assert!(out.contains("term=xterm-256color"), "{out}");
        p.resize(100, 30).unwrap();
        p.input(b"ping\n").unwrap();
        until(&mut rx, "ping").await;
        drop(p);
    }
}
