//! The runner (R-1203): in the container the daemon is not the user the agents
//! run as, so it cannot run a git command in a worktree itself. A small
//! process running as the agent user (`xherd runner`) does it for the daemon
//! over a unix socket; `exec::run` sends it every command marked
//! `Cmd::as_agent`. It adds no rights: whoever can reach the socket is the
//! agent user already, or the daemon the runner works for.
//!
//! Wire format: one request per connection, a 4-byte big-endian length and a
//! JSON body, then the same for the reply. Bytes are hex strings.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::exec::{self, Cmd, ExecError, Output};

/// Bigger frames are refused (a command's captured output is capped lower).
const MAX_FRAME: usize = 64 * 1024 * 1024;
/// Time the runner gets on top of the command's own timeout to answer.
const SLACK: Duration = Duration::from_secs(30);

static RUNNER: OnceLock<PathBuf> = OnceLock::new();

/// Send every `as_agent` command of this process to the runner at `socket`.
/// Set once at daemon start in the container; without it such commands run
/// in place (native mode, tests).
pub fn use_runner(socket: PathBuf) {
    let _ = RUNNER.set(socket);
}

pub fn runner_socket() -> Option<&'static Path> {
    RUNNER.get().map(PathBuf::as_path)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
enum Request {
    Ping,
    #[serde(rename_all = "camelCase")]
    Exec {
        argv: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        stdin: Option<String>,
        timeout_ms: u64,
        max_output: usize,
        redact: Vec<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
enum Reply {
    Pong,
    #[serde(rename_all = "camelCase")]
    Output {
        code: Option<i32>,
        stdout: String,
        stderr: String,
        timed_out: bool,
        truncated: bool,
    },
    /// The command could not be run (`ExecError`).
    #[serde(rename_all = "camelCase")]
    Failed {
        kind: FailKind,
        program: String,
        message: String,
    },
    /// The request itself was refused or malformed.
    Refused {
        message: String,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum FailKind {
    EmptyArgv,
    Spawn,
    Io,
}

async fn write_frame(s: &mut UnixStream, body: &[u8]) -> std::io::Result<()> {
    s.write_all(&(body.len() as u32).to_be_bytes()).await?;
    s.write_all(body).await?;
    s.flush().await
}

async fn read_frame(s: &mut UnixStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).await?;
    Ok(body)
}

async fn roundtrip(socket: &Path, req: &Request, limit: Duration) -> Result<Reply, String> {
    let work = async {
        let mut s = UnixStream::connect(socket)
            .await
            .map_err(|e| format!("cannot reach the runner at {}: {e}", socket.display()))?;
        let body = serde_json::to_vec(req).map_err(|e| e.to_string())?;
        write_frame(&mut s, &body)
            .await
            .map_err(|e| e.to_string())?;
        let reply = read_frame(&mut s).await.map_err(|e| e.to_string())?;
        serde_json::from_slice(&reply).map_err(|e| e.to_string())
    };
    tokio::time::timeout(limit, work)
        .await
        .map_err(|_| "the runner did not answer in time".to_string())?
}

/// True when the runner at `socket` answers.
pub async fn ping(socket: &Path) -> bool {
    matches!(
        roundtrip(socket, &Request::Ping, Duration::from_secs(5)).await,
        Ok(Reply::Pong)
    )
}

/// Run `cmd` through the runner at `socket`.
pub async fn exec(socket: &Path, cmd: Cmd) -> Result<Output, ExecError> {
    let program = cmd.argv.first().cloned().unwrap_or_default();
    if cmd.inherit_output {
        return Err(ExecError::Runner(
            "a command with inherited output cannot run through the runner".into(),
        ));
    }
    let limit = cmd.timeout + SLACK;
    let req = Request::Exec {
        argv: cmd.argv,
        cwd: cmd.cwd,
        env: cmd.env,
        stdin: cmd.stdin.map(hex::encode),
        timeout_ms: cmd.timeout.as_millis().min(u64::MAX as u128) as u64,
        max_output: cmd.max_output,
        redact: cmd.redact,
    };
    match roundtrip(socket, &req, limit)
        .await
        .map_err(ExecError::Runner)?
    {
        Reply::Output {
            code,
            stdout,
            stderr,
            timed_out,
            truncated,
        } => Ok(Output {
            code,
            stdout: hex::decode(stdout).map_err(|e| ExecError::Runner(e.to_string()))?,
            stderr,
            timed_out,
            truncated,
        }),
        Reply::Failed {
            kind,
            program,
            message,
        } => Err(match kind {
            FailKind::EmptyArgv => ExecError::EmptyArgv,
            FailKind::Spawn => ExecError::Spawn {
                program,
                source: std::io::Error::other(message),
            },
            FailKind::Io => ExecError::Io {
                program,
                source: std::io::Error::other(message),
            },
        }),
        Reply::Refused { message } => Err(ExecError::Runner(message)),
        Reply::Pong => Err(ExecError::Runner(format!(
            "unexpected answer for {program}"
        ))),
    }
}

async fn handle(req: Request) -> Reply {
    match req {
        Request::Ping => Reply::Pong,
        Request::Exec {
            argv,
            cwd,
            env,
            stdin,
            timeout_ms,
            max_output,
            redact,
        } => {
            let stdin = match stdin.map(hex::decode).transpose() {
                Ok(s) => s,
                Err(e) => {
                    return Reply::Refused {
                        message: format!("stdin: {e}"),
                    };
                }
            };
            let program = argv.first().cloned().unwrap_or_default();
            let mut cmd = Cmd::new(argv);
            cmd.cwd = cwd;
            cmd.env = env;
            cmd.stdin = stdin;
            cmd.timeout = Duration::from_millis(timeout_ms);
            cmd.max_output = max_output;
            cmd.redact = redact;
            match exec::run(cmd).await {
                Ok(o) => Reply::Output {
                    code: o.code,
                    stdout: hex::encode(o.stdout),
                    stderr: o.stderr,
                    timed_out: o.timed_out,
                    truncated: o.truncated,
                },
                Err(ExecError::EmptyArgv) => Reply::Failed {
                    kind: FailKind::EmptyArgv,
                    program,
                    message: "empty argv".into(),
                },
                Err(ExecError::Spawn { program, source }) => Reply::Failed {
                    kind: FailKind::Spawn,
                    program,
                    message: source.to_string(),
                },
                Err(ExecError::Io { program, source }) => Reply::Failed {
                    kind: FailKind::Io,
                    program,
                    message: source.to_string(),
                },
                Err(ExecError::Runner(m)) => Reply::Refused { message: m },
            }
        }
    }
}

async fn serve_one(mut s: UnixStream) {
    let reply = match read_frame(&mut s).await {
        Err(e) => {
            tracing::debug!(error = %e, "runner: bad request frame");
            return;
        }
        Ok(body) => match serde_json::from_slice::<Request>(&body) {
            Ok(req) => handle(req).await,
            Err(e) => Reply::Refused {
                message: format!("bad request: {e}"),
            },
        },
    };
    if let Ok(body) = serde_json::to_vec(&reply) {
        let _ = write_frame(&mut s, &body).await;
    }
}

/// Bind `socket` (a stale file is replaced) with `mode`, then answer
/// requests until the process ends. `xherd runner` calls this.
pub async fn serve(socket: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(mode))?;
    loop {
        let (s, _) = listener.accept().await?;
        tokio::spawn(serve_one(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn start() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("r.sock");
        let s = sock.clone();
        tokio::spawn(async move { serve(&s, 0o660).await });
        for _ in 0..100 {
            if ping(&sock).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        (dir, sock)
    }

    #[tokio::test]
    async fn runs_a_command_with_cwd_env_and_stdin() {
        let (dir, sock) = start().await;
        let o = exec(
            &sock,
            Cmd::new(["sh", "-c", "pwd; echo $X; cat"])
                .cwd(dir.path())
                .env("X", "42")
                .stdin("in"),
        )
        .await
        .unwrap();
        assert!(o.success());
        let out = o.stdout_str();
        let mut lines = out.lines();
        assert_eq!(
            std::fs::canonicalize(lines.next().unwrap()).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
        assert_eq!(lines.next(), Some("42"));
        assert_eq!(lines.next(), Some("in"));
    }

    #[tokio::test]
    async fn exit_code_timeout_redaction_and_cap_survive_the_trip() {
        let (_d, sock) = start().await;
        let o = exec(&sock, Cmd::new(["sh", "-c", "exit 3"])).await.unwrap();
        assert_eq!(o.code, Some(3));

        let o = exec(
            &sock,
            Cmd::new(["sleep", "30"]).timeout(Duration::from_millis(200)),
        )
        .await
        .unwrap();
        assert!(o.timed_out && !o.success());

        let mut c = Cmd::new([
            "sh",
            "-c",
            "echo token=ghp_secret >&2; yes | head -c 100000",
        ])
        .redact(vec!["ghp_secret".into()]);
        c.max_output = 1000;
        let o = exec(&sock, c).await.unwrap();
        assert_eq!(o.stderr.trim(), "token=***");
        assert_eq!(o.stdout.len(), 1000);
        assert!(o.truncated);
    }

    #[tokio::test]
    async fn spawn_errors_keep_their_kind() {
        let (_d, sock) = start().await;
        assert!(matches!(
            exec(&sock, Cmd::new(["/nonexistent/xherd-nope"])).await,
            Err(ExecError::Spawn { .. })
        ));
        assert!(matches!(
            exec(&sock, Cmd::new(Vec::<String>::new())).await,
            Err(ExecError::EmptyArgv) | Err(ExecError::Runner(_))
        ));
    }

    #[tokio::test]
    async fn binary_stdout_is_exact_and_inherit_is_refused() {
        let (_d, sock) = start().await;
        let o = exec(&sock, Cmd::new(["printf", r"\000\377a"]))
            .await
            .unwrap();
        assert_eq!(o.stdout, vec![0, 255, b'a']);
        assert!(matches!(
            exec(&sock, Cmd::new(["true"]).inherit_output()).await,
            Err(ExecError::Runner(_))
        ));
    }

    #[tokio::test]
    async fn no_runner_is_a_clear_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("none.sock");
        assert!(!ping(&sock).await);
        let e = exec(&sock, Cmd::new(["true"])).await.unwrap_err();
        assert!(e.to_string().contains("cannot reach the runner"));
    }
}
