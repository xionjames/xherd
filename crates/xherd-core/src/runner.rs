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
    /// Read a file; a missing one is not an error.
    ReadFile {
        path: PathBuf,
    },
    /// Write a file atomically, creating its folder.
    WriteFile {
        path: PathBuf,
        data: String,
        mode: Option<u32>,
    },
    /// Create a folder and its parents; `mode` is set on the last one.
    Mkdir {
        path: PathBuf,
        mode: u32,
    },
    /// The first file `depth` folders below `dir` whose name matches `name`
    /// (see [`find_in_subdirs_as_agent`]).
    Find {
        dir: PathBuf,
        name: String,
        #[serde(default = "one")]
        depth: u8,
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
    /// A file read: `None` when it does not exist.
    File {
        data: Option<String>,
    },
    /// A search: the path found, if any.
    Found {
        path: Option<PathBuf>,
    },
    Done,
    /// A file operation failed.
    IoFailed {
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
        Reply::Refused { message } | Reply::IoFailed { message } => Err(ExecError::Runner(message)),
        Reply::Pong | Reply::File { .. } | Reply::Found { .. } | Reply::Done => Err(
            ExecError::Runner(format!("unexpected answer for {program}")),
        ),
    }
}

fn one() -> u8 {
    1
}

/// Does a file name match `pattern`, which has at most one `*`?
fn name_matches(pattern: &str, name: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == name,
        Some((pre, post)) => {
            name.len() >= pre.len() + post.len() && name.starts_with(pre) && name.ends_with(post)
        }
    }
}

fn find_in_subdirs(dir: &Path, name: &str, depth: u8) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?.flatten();
    if depth == 0 {
        return entries.map(|e| e.path()).find(|p| {
            p.is_file()
                && p.file_name()
                    .is_some_and(|n| name_matches(name, &n.to_string_lossy()))
        });
    }
    entries
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .find_map(|d| find_in_subdirs(&d, name, depth - 1))
}

/// The first file `depth` folders below `dir` (`<dir>/<any folder>/<name>`
/// for 1) whose name matches `name` (at most one `*`), as the agent user (a
/// runtime keeps its session transcripts in its own home).
pub async fn find_in_subdirs_as_agent(dir: &Path, name: &str, depth: u8) -> Option<PathBuf> {
    let Some(sock) = runner_socket() else {
        return find_in_subdirs(dir, name, depth);
    };
    let req = Request::Find {
        dir: dir.into(),
        name: name.into(),
        depth,
    };
    match roundtrip(sock, &req, Duration::from_secs(30)).await {
        Ok(Reply::Found { path }) => path,
        _ => None,
    }
}

/// An environment variable as the agent user sees it: `XHERD_AGENT_<NAME>`
/// when set (`XHERD_AGENT_HOME`, `XHERD_AGENT_PATH`: in the container the
/// daemon's own home and PATH are not the agents', R-1203), else ours.
pub fn agent_env(name: &str) -> Option<String> {
    std::env::var(format!("XHERD_AGENT_{name}"))
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var(name).ok())
}

fn make_dir(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

/// Read a file as the agent user (`None` if it does not exist). Without a
/// runner it is read in place.
pub async fn read_file(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let Some(sock) = runner_socket() else {
        return match std::fs::read(path) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        };
    };
    let req = Request::ReadFile { path: path.into() };
    match roundtrip(sock, &req, Duration::from_secs(30))
        .await
        .map_err(std::io::Error::other)?
    {
        Reply::File { data: None } => Ok(None),
        Reply::File { data: Some(d) } => hex::decode(d).map(Some).map_err(std::io::Error::other),
        other => Err(file_error(other)),
    }
}

/// Write a file atomically as the agent user. Without a runner it is
/// written in place.
pub async fn write_file(path: &Path, bytes: &[u8], mode: Option<u32>) -> std::io::Result<()> {
    let Some(sock) = runner_socket() else {
        return crate::paths::write_atomic(path, bytes, mode);
    };
    let req = Request::WriteFile {
        path: path.into(),
        data: hex::encode(bytes),
        mode,
    };
    match roundtrip(sock, &req, Duration::from_secs(30))
        .await
        .map_err(std::io::Error::other)?
    {
        Reply::Done => Ok(()),
        other => Err(file_error(other)),
    }
}

/// Create a folder as the agent user and set `mode` on it (e.g. `0o2775`).
pub async fn mkdir(path: &Path, mode: u32) -> std::io::Result<()> {
    let Some(sock) = runner_socket() else {
        return make_dir(path, mode);
    };
    let req = Request::Mkdir {
        path: path.into(),
        mode,
    };
    match roundtrip(sock, &req, Duration::from_secs(30))
        .await
        .map_err(std::io::Error::other)?
    {
        Reply::Done => Ok(()),
        other => Err(file_error(other)),
    }
}

fn file_error(r: Reply) -> std::io::Error {
    match r {
        Reply::IoFailed { message } | Reply::Refused { message } => std::io::Error::other(message),
        _ => std::io::Error::other("unexpected answer from the runner"),
    }
}

async fn handle(req: Request) -> Reply {
    match req {
        Request::Ping => Reply::Pong,
        Request::ReadFile { path } => match std::fs::read(&path) {
            Ok(b) => Reply::File {
                data: Some(hex::encode(b)),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Reply::File { data: None },
            Err(e) => Reply::IoFailed {
                message: format!("{}: {e}", path.display()),
            },
        },
        Request::WriteFile { path, data, mode } => {
            let bytes = match hex::decode(data) {
                Ok(b) => b,
                Err(e) => {
                    return Reply::Refused {
                        message: format!("data: {e}"),
                    };
                }
            };
            match crate::paths::write_atomic(&path, &bytes, mode) {
                Ok(()) => Reply::Done,
                Err(e) => Reply::IoFailed {
                    message: format!("{}: {e}", path.display()),
                },
            }
        }
        Request::Find { dir, name, depth } => Reply::Found {
            path: find_in_subdirs(&dir, &name, depth),
        },
        Request::Mkdir { path, mode } => match make_dir(&path, mode) {
            Ok(()) => Reply::Done,
            Err(e) => Reply::IoFailed {
                message: format!("{}: {e}", path.display()),
            },
        },
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

/// Keep a tmux server running on `socket`, as the user this runner is, and
/// let `allow` (the daemon's user) use it: tmux (>= 3.3) `server-access` for
/// the commands, and the socket opened to the group. The agents' sessions
/// live in this server, so they are this user's processes (R-1203). Never
/// returns; a server that exits is started again.
pub async fn supervise_tmux(socket: PathBuf, allow: Option<String>) {
    use std::process::Stdio;
    let conf = socket.with_extension("conf");
    // `-D` already turns exit-empty off; say it anyway. The access rule is
    // not in this file: tmux reads it only when the first client comes.
    if let Err(e) = std::fs::write(&conf, "set -s exit-empty off\n") {
        eprintln!("xherd runner: {}: {e}", conf.display());
        return;
    }
    loop {
        let _ = std::fs::remove_file(&socket);
        let started = std::time::Instant::now();
        match tokio::process::Command::new("tmux")
            .arg("-S")
            .arg(&socket)
            .arg("-f")
            .arg(&conf)
            .arg("-D")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(mut child) => {
                tokio::spawn(open_to_group(socket.clone(), allow.clone()));
                let _ = child.wait().await;
                eprintln!("xherd runner: the tmux server exited; starting it again");
            }
            Err(e) => eprintln!("xherd runner: cannot start tmux: {e}"),
        }
        let pause = if started.elapsed() < Duration::from_secs(5) {
            Duration::from_secs(2)
        } else {
            Duration::from_millis(500)
        };
        tokio::time::sleep(pause).await;
    }
}

/// The server creates its socket for its owner only; the daemon's user is in
/// our group and needs to write to it, and the server has to let that user
/// run commands (`server-access`, asked of the running server, with retries
/// while it starts).
async fn open_to_group(socket: PathBuf, allow: Option<String>) {
    use std::os::unix::fs::PermissionsExt;
    for _ in 0..100 {
        if socket.exists() {
            let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o660));
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let Some(user) = allow else { return };
    for _ in 0..50 {
        let ok = tokio::process::Command::new("tmux")
            .arg("-S")
            .arg(&socket)
            .args(["server-access", "-aw", &user])
            .stdin(std::process::Stdio::null())
            .status()
            .await
            .is_ok_and(|s| s.success());
        if ok {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    eprintln!("xherd runner: could not give {user} access to the tmux server");
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

    #[test]
    fn finds_transcripts_by_depth_and_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("proj")).unwrap();
        std::fs::write(d.join("proj/s-1.jsonl"), "").unwrap();
        std::fs::create_dir_all(d.join("2026/11/12")).unwrap();
        let rollout = d.join("2026/11/12/rollout-2026-11-12T11-58-52-s-2.jsonl");
        std::fs::write(&rollout, "").unwrap();
        assert_eq!(
            find_in_subdirs(d, "s-1.jsonl", 1),
            Some(d.join("proj/s-1.jsonl"))
        );
        assert_eq!(find_in_subdirs(d, "rollout-*-s-2.jsonl", 3), Some(rollout));
        assert_eq!(find_in_subdirs(d, "rollout-*-s-2.jsonl", 1), None);
        assert_eq!(find_in_subdirs(d, "rollout-*-s-3.jsonl", 3), None);
        assert!(!name_matches("a*b", "ab_"));
        assert!(name_matches("a*b", "ab"));
        assert!(!name_matches("aa*aa", "aaa"));
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
    async fn files_are_written_read_and_made_through_the_socket() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, sock) = start().await;
        let f = dir.path().join("a/b/c.json");
        let w = Request::WriteFile {
            path: f.clone(),
            data: hex::encode(b"{}\n"),
            mode: Some(0o640),
        };
        assert!(matches!(
            roundtrip(&sock, &w, Duration::from_secs(5)).await,
            Ok(Reply::Done)
        ));
        assert_eq!(std::fs::read(&f).unwrap(), b"{}\n");
        assert_eq!(
            std::fs::metadata(&f).unwrap().permissions().mode() & 0o777,
            0o640
        );

        let r = Request::ReadFile { path: f.clone() };
        let Ok(Reply::File { data: Some(d) }) = roundtrip(&sock, &r, Duration::from_secs(5)).await
        else {
            panic!("read failed");
        };
        assert_eq!(hex::decode(d).unwrap(), b"{}\n");
        let missing = Request::ReadFile {
            path: dir.path().join("none"),
        };
        assert!(matches!(
            roundtrip(&sock, &missing, Duration::from_secs(5)).await,
            Ok(Reply::File { data: None })
        ));

        let d = dir.path().join("x/y");
        let m = Request::Mkdir {
            path: d.clone(),
            mode: 0o2775,
        };
        assert!(matches!(
            roundtrip(&sock, &m, Duration::from_secs(5)).await,
            Ok(Reply::Done)
        ));
        // The setgid bit may be dropped by the filesystem; the rest must hold.
        assert_eq!(
            std::fs::metadata(&d).unwrap().permissions().mode() & 0o777,
            0o775
        );
    }

    #[tokio::test]
    async fn without_a_runner_files_are_handled_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("n/f.txt");
        assert_eq!(read_file(&f).await.unwrap(), None);
        write_file(&f, b"hi", None).await.unwrap();
        assert_eq!(read_file(&f).await.unwrap().as_deref(), Some(&b"hi"[..]));
        mkdir(&dir.path().join("m/k"), 0o755).await.unwrap();
        assert!(dir.path().join("m/k").is_dir());
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
