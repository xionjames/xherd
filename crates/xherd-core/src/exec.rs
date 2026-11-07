//! The one runner for every external command (design §3.6): argv only (no
//! shell), working directory, environment allowlist plus injected variables,
//! stdin bytes, capped stdout/stderr, timeout that kills the whole process
//! group, secret redaction of captured stderr, and a tracing span.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tracing::Instrument;

/// Variables passed through from the daemon's environment.
pub const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "TZ",
    "TMPDIR",
    "SSH_AUTH_SOCK",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_RUNTIME_DIR",
];

pub const DEFAULT_MAX_OUTPUT: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Cmd {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    /// Injected variables (e.g. `XHERD_*`), on top of the allowlist.
    pub env: Vec<(String, String)>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
    pub max_output: usize,
    /// Strings replaced by `***` in captured stderr.
    pub redact: Vec<String>,
    /// Stdout and stderr go to ours instead of being captured (a live log in
    /// a terminal); `Output` then has them empty.
    pub inherit_output: bool,
}

impl Cmd {
    pub fn new<I, S>(argv: I) -> Cmd
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Cmd {
            argv: argv.into_iter().map(Into::into).collect(),
            cwd: None,
            env: Vec::new(),
            stdin: None,
            timeout: Duration::from_secs(60),
            max_output: DEFAULT_MAX_OUTPUT,
            redact: Vec::new(),
            inherit_output: false,
        }
    }
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Cmd {
        self.cwd = Some(dir.into());
        self
    }
    pub fn env(mut self, k: impl Into<String>, v: impl Into<String>) -> Cmd {
        self.env.push((k.into(), v.into()));
        self
    }
    pub fn stdin(mut self, bytes: impl Into<Vec<u8>>) -> Cmd {
        self.stdin = Some(bytes.into());
        self
    }
    pub fn timeout(mut self, t: Duration) -> Cmd {
        self.timeout = t;
        self
    }
    pub fn redact(mut self, secrets: Vec<String>) -> Cmd {
        self.redact = secrets;
        self
    }
    pub fn inherit_output(mut self) -> Cmd {
        self.inherit_output = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Exit code; `None` when killed by a signal (including timeout).
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    /// Lossy UTF-8, redacted.
    pub stderr: String,
    pub timed_out: bool,
    pub truncated: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("empty argv")]
    EmptyArgv,
    #[error("could not start {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("{program} failed: {source}")]
    Io {
        program: String,
        source: std::io::Error,
    },
}

pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        out = out.replace(s.as_str(), "***");
    }
    out
}

async fn read_capped<R: AsyncRead + Unpin>(
    mut r: R,
    cap: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    loop {
        let n = r.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        let room = cap.saturating_sub(buf.len());
        if room < n {
            truncated = true;
        }
        buf.extend_from_slice(&chunk[..n.min(room)]);
    }
    Ok((buf, truncated))
}

pub async fn run(cmd: Cmd) -> Result<Output, ExecError> {
    let program = cmd.argv.first().cloned().ok_or(ExecError::EmptyArgv)?;
    let span = tracing::debug_span!("exec", program = %program, args = cmd.argv.len() - 1);
    run_inner(cmd, program).instrument(span).await
}

async fn run_inner(cmd: Cmd, program: String) -> Result<Output, ExecError> {
    let mut c = tokio::process::Command::new(&program);
    c.args(&cmd.argv[1..])
        .env_clear()
        .stdin(if cmd.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .kill_on_drop(true);
    if cmd.inherit_output {
        c.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    } else {
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    #[cfg(unix)]
    c.process_group(0);
    for k in ENV_ALLOWLIST {
        if let Some(v) = std::env::var_os(k) {
            c.env(k, v);
        }
    }
    for (k, v) in &cmd.env {
        c.env(k, v);
    }
    if let Some(d) = &cmd.cwd {
        c.current_dir(d);
    }
    let mut child = c.spawn().map_err(|source| ExecError::Spawn {
        program: program.clone(),
        source,
    })?;
    let pid = child.id();

    if let Some(bytes) = cmd.stdin {
        let mut stdin = child.stdin.take().expect("stdin piped");
        // A child that never reads stdin must not block us: write in the background.
        tokio::spawn(async move {
            let _ = stdin.write_all(&bytes).await;
        });
    }
    let out = child
        .stdout
        .take()
        .map(|o| tokio::spawn(read_capped(o, cmd.max_output)));
    let err = child
        .stderr
        .take()
        .map(|e| tokio::spawn(read_capped(e, cmd.max_output)));

    let (code, timed_out) = match tokio::time::timeout(cmd.timeout, child.wait()).await {
        Ok(status) => {
            let status = status.map_err(|source| ExecError::Io {
                program: program.clone(),
                source,
            })?;
            (status.code(), false)
        }
        Err(_) => {
            kill_group(pid);
            let _ = child.wait().await;
            (None, true)
        }
    };
    // After a timeout, grandchildren holding the pipes were killed with the group.
    type Reader = tokio::task::JoinHandle<std::io::Result<(Vec<u8>, bool)>>;
    async fn join(h: Option<Reader>) -> (Vec<u8>, bool) {
        match h {
            Some(h) => h.await.ok().and_then(Result::ok).unwrap_or_default(),
            None => Default::default(),
        }
    }
    let (stdout, t1) = join(out).await;
    let (stderr, t2) = join(err).await;
    let stderr = redact(&String::from_utf8_lossy(&stderr), &cmd.redact);
    if timed_out {
        tracing::warn!(program = %program, "command timed out; process group killed");
    }
    Ok(Output {
        code,
        stdout,
        stderr,
        timed_out,
        truncated: t1 || t2,
    })
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        let _ = nix::sys::signal::killpg(
            nix::unistd::Pid::from_raw(pid as i32),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: Option<u32>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_stdout_and_stdin() {
        let o = run(Cmd::new(["cat"]).stdin("hello")).await.unwrap();
        assert!(o.success());
        assert_eq!(o.stdout, b"hello");
    }

    #[tokio::test]
    async fn environment_is_allowlisted() {
        // `cargo test` runs tests with CARGO_* variables set; none may leak.
        let o = run(Cmd::new(["env"]).env("XHERD_AGENT_ID", "a1"))
            .await
            .unwrap();
        let s = o.stdout_str();
        assert!(s.contains("XHERD_AGENT_ID=a1"));
        assert!(!s.contains("CARGO_"));
    }

    #[tokio::test]
    async fn timeout_kills_process_tree() {
        let start = std::time::Instant::now();
        // The shell's child `sleep` holds the pipes; killpg must take it down too.
        let o =
            run(Cmd::new(["sh", "-c", "sleep 30 & sleep 30"]).timeout(Duration::from_millis(200)))
                .await
                .unwrap();
        assert!(o.timed_out);
        assert!(!o.success());
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn stderr_is_redacted_and_output_capped() {
        let mut c = Cmd::new([
            "sh",
            "-c",
            "echo token=ghp_secret >&2; yes | head -c 100000",
        ])
        .redact(vec!["ghp_secret".into()]);
        c.max_output = 1000;
        let o = run(c).await.unwrap();
        assert_eq!(o.stderr.trim(), "token=***");
        assert_eq!(o.stdout.len(), 1000);
        assert!(o.truncated);
    }

    #[tokio::test]
    async fn missing_program_is_a_spawn_error() {
        assert!(matches!(
            run(Cmd::new(["/nonexistent/xherd-nope"])).await,
            Err(ExecError::Spawn { .. })
        ));
    }
}
