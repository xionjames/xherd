//! The `xherd` binary.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::Value;
use xherd_core::config::Config;
use xherd_core::paths::Paths;

mod docker;
mod git;
mod guard;
mod init;
mod open_in;
mod report;
mod toolchain;
mod xap;

#[derive(Parser)]
#[command(
    name = "xherd",
    version,
    about = "Run, watch and steer a herd of coding agents"
)]
struct Cli {
    /// Machine-readable output (XAP commands).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the daemon in the foreground.
    Daemon,
    /// Container entrypoint (root): prepare the user, home, SSH and git, then run the daemon.
    Init,
    /// Run xherd in Docker: write the compose file, check the setup, update.
    #[command(subcommand)]
    Docker(DockerCmd),
    /// Is the daemon up?
    Status,
    /// Create an agent: worktree, branch and `.xherd/` for a ticket.
    New {
        /// Ticket key (upper-cased).
        key: String,
        #[arg(long)]
        slug: Option<String>,
        /// Flow of the built-in pack, or `none` (default `feature`).
        #[arg(long)]
        flow: Option<String>,
        /// Local repository, or a remote URL cloned into the workspace (default: the current directory).
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        base: Option<String>,
        prompt: Option<String>,
    },
    /// List agents.
    Ls,
    /// Install a pack's commands for agent runtimes (default: xherd, every runtime).
    Install {
        #[arg(long)]
        runtime: Option<String>,
        #[arg(long)]
        pack: Option<String>,
    },
    /// Pause an agent (ends its session).
    Pause { agent: String },
    /// Resume a paused, cancelled or failed agent.
    Resume { agent: String },
    /// Cancel an agent (ends its session, keeps the worktree).
    Cancel { agent: String },
    /// Type a message into an agent's session.
    Send { agent: String, text: String },
    /// Approve the agent's open gate (documents or commit) as they are now.
    Approve {
        agent: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Ask the agent to revise its open gate.
    Changes { agent: String, comment: String },
    /// Show the open commit proposal's diff, the branch at a ship gate, else every uncommitted change.
    Diff { agent: String },
    /// git credential helper: answers `get` with the token of Settings → Git access.
    Credential { op: String },
    /// Git access.
    #[command(subcommand)]
    Git(GitCmd),
    /// Checks behind xherd's git hooks.
    #[command(subcommand)]
    Guard(GuardCmd),
    /// Open an agent's worktree in an app on this computer (apps: ~/.config/xherd/open.json, else editors on PATH).
    Open {
        agent: String,
        #[arg(long)]
        app: Option<String>,
    },
    /// Attach this terminal to an agent's session (in the container when xherd runs in Docker).
    Attach { agent: String },
    /// A shell in an agent's worktree (in the container when xherd runs in Docker).
    Shell { agent: String },
    /// Run Open-in requests from the browser with this computer's apps.
    Companion,
    /// Toolchains of the current worktree (R-652).
    #[command(subcommand)]
    Toolchain(ToolchainCmd),
    /// Runtime hook (reads the hook JSON on stdin). Silent; always exits 0.
    Hook { event: String },
    /// Push status for agents or wrappers without hooks.
    Report {
        #[arg(long, value_parser = ["working", "idle", "blocked"])]
        state: Option<String>,
        #[arg(long)]
        release: bool,
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        resume_cmd: Option<String>,
        #[arg(long)]
        seq: Option<u64>,
    },
    /// XAP: mark the current stage.
    #[command(subcommand)]
    Stage(xap::StageCmd),
    /// XAP: declare an approval gate.
    #[command(subcommand)]
    Gate(xap::GateCmd),
    /// XAP: ask the user a question and wait.
    Ask { text: String },
    /// XAP: task list.
    #[command(subcommand)]
    Tasks(xap::TasksCmd),
    /// XAP: task progress.
    #[command(subcommand)]
    Task(xap::TaskCmd),
    /// XAP: commit gates.
    #[command(subcommand)]
    Commit(xap::CommitCmd),
    /// XAP: pull request opened.
    #[command(subcommand)]
    Pr(xap::PrCmd),
    /// XAP: handoff note written.
    #[command(subcommand)]
    Handoff(xap::HandoffCmd),
    /// XAP: record an out-of-scope finding.
    #[command(subcommand)]
    Issue(xap::IssueCmd),
}

#[derive(Subcommand)]
enum GitCmd {
    /// Check SSH and HTTPS access to a host, in order, with the fix for the first failure.
    Doctor {
        /// Host (default: the --repo host, else github.com).
        host: Option<String>,
        /// Also check API and git access to this repository.
        #[arg(long)]
        repo: Option<String>,
    },
}

#[derive(Subcommand)]
enum DockerCmd {
    /// Write ~/.config/xherd/compose.yaml and .env for the detected engine.
    Init {
        /// Workspace folder, mounted at the same path (default ~/xherd).
        #[arg(long)]
        workspace: Option<std::path::PathBuf>,
        #[arg(long)]
        image: Option<String>,
        #[arg(long, default_value_t = 4740)]
        port: u16,
        /// Another host folder with repositories, mounted at the same path (repeatable).
        #[arg(long = "extra-path")]
        extra_paths: Vec<std::path::PathBuf>,
        /// Rewrite an existing compose file.
        #[arg(long)]
        force: bool,
    },
    /// Check engine, mounts, ownership, SSH agent and port binding, each with a fix.
    Doctor,
    /// Pause live agents, pull and restart the container, resume them.
    Update {
        /// Update even when the daemon does not answer.
        #[arg(long)]
        force: bool,
        /// Restart with the image already here (a local build) instead of pulling.
        #[arg(long)]
        no_pull: bool,
    },
}

#[derive(Subcommand)]
enum GuardCmd {
    /// The `pre-push` check (git passes the remote and its URL; ref lines arrive on stdin).
    PrePush {
        /// The only branch this agent may push.
        #[arg(long)]
        branch: String,
        #[arg(trailing_var_arg = true)]
        _git: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ToolchainCmd {
    /// Provide the tools `.xherd/toolchain.json` lists, then run the command
    /// with them on PATH (the daemon starts agents this way).
    Exec {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        argv: Vec<String>,
    },
}

fn main() -> ExitCode {
    // Hot path first: no argument parser errors, no panics, nothing on stdout (R-012).
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("hook") {
        std::panic::set_hook(Box::new(|_| {}));
        let event = args.get(2).cloned().unwrap_or_default();
        let _ = std::panic::catch_unwind(|| report::hook(&event));
        return ExitCode::SUCCESS;
    }

    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let code = if e.use_stderr() { 64 } else { 0 };
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    let json = cli.json;
    let code = match cli.cmd {
        Cmd::Daemon => run_daemon(),
        Cmd::Init => init::init(),
        Cmd::Docker(DockerCmd::Init {
            workspace,
            image,
            port,
            extra_paths,
            force,
        }) => docker::init(docker::InitArgs {
            workspace,
            image,
            port,
            extra_paths,
            force,
        }),
        Cmd::Docker(DockerCmd::Doctor) => docker::doctor(),
        Cmd::Docker(DockerCmd::Update { force, no_pull }) => {
            with_client(|c| docker::update(c, force, !no_pull))
        }
        Cmd::Status => status(),
        Cmd::Ls => ls(),
        Cmd::Install { runtime, pack } => install(runtime, pack),
        Cmd::New {
            key,
            slug,
            flow,
            repo,
            base,
            prompt,
        } => new_agent(key, slug, flow, repo, base, prompt),
        Cmd::Pause { agent } => action(&agent, "pause", None),
        Cmd::Resume { agent } => action(&agent, "resume", None),
        Cmd::Cancel { agent } => action(&agent, "cancel", None),
        Cmd::Send { agent, text } => {
            action(&agent, "send", Some(serde_json::json!({"text": text})))
        }
        Cmd::Approve { agent, reason } => action(
            &agent,
            "approve",
            Some(serde_json::json!({"reason": reason})),
        ),
        Cmd::Changes { agent, comment } => action(
            &agent,
            "request-changes",
            Some(serde_json::json!({"comment": comment})),
        ),
        Cmd::Diff { agent } => diff(&agent, json),
        Cmd::Toolchain(ToolchainCmd::Exec { argv }) => toolchain::exec(argv),
        Cmd::Credential { op } => git::credential(&op),
        Cmd::Guard(GuardCmd::PrePush { branch, .. }) => guard::pre_push(&branch),
        Cmd::Open { agent, app } => with_client(|c| open_in::open(c, &agent, app)),
        Cmd::Attach { agent } => with_client(|c| open_in::attach(c, &agent, false)),
        Cmd::Shell { agent } => with_client(|c| open_in::attach(c, &agent, true)),
        Cmd::Companion => with_client(open_in::companion),
        Cmd::Git(GitCmd::Doctor { host, repo }) => with_client(|c| git::doctor(c, host, repo)),
        Cmd::Hook { .. } => 0,
        Cmd::Report {
            state,
            release,
            message,
            session,
            resume_cmd,
            seq,
        } => report::report(report::ReportArgs {
            state,
            release,
            message,
            session,
            resume_cmd,
            seq,
        }),
        Cmd::Stage(c) => xap::run(c.into(), json),
        Cmd::Gate(c) => xap::run(c.into(), json),
        Cmd::Ask { text } => xap::run(xherd_core::xap::XapCmd::Ask { text }, json),
        Cmd::Tasks(c) => xap::run(c.into(), json),
        Cmd::Task(c) => xap::run(c.into(), json),
        Cmd::Commit(c) => xap::run(c.into(), json),
        Cmd::Pr(c) => xap::run(c.into(), json),
        Cmd::Handoff(c) => xap::run(c.into(), json),
        Cmd::Issue(c) => xap::run(c.into(), json),
    };
    ExitCode::from(code as u8)
}

// ---- daemon -------------------------------------------------------------------

fn run_daemon() -> i32 {
    let filter = tracing_subscriber::EnvFilter::try_from_env("XHERD_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("xherd daemon: {e}");
            return 1;
        }
    };
    match rt.block_on(daemon_main()) {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!(error = %e, "daemon stopped");
            eprintln!("xherd daemon: {e}");
            1
        }
    }
}

async fn daemon_main() -> Result<(), Box<dyn std::error::Error>> {
    let paths = Paths::from_env();
    let (cfg, _) = Config::load(&paths.config())?;
    // In the container the daemon binds every interface (`XHERD_BIND`), and
    // the published port's gateway counts as the local host (`XHERD_LOCAL_PEERS`).
    let bind = std::env::var("XHERD_BIND")
        .ok()
        .filter(|b| !b.is_empty())
        .unwrap_or_else(|| cfg.bind.clone());
    let addr = format!("{bind}:{}", cfg.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("cannot listen on {addr}: {e} (is another daemon running?)"))?;

    let tmux = xherd_term::Tmux::new(cfg.tmux_socket.clone());
    let mut opts = xherd_daemon::Options::new(
        Arc::new(xherd_daemon::TerminalDriver { tmux }),
        Arc::new(xherd_daemon::SystemClock),
    );
    opts.xherd_bin = std::env::current_exe()?.to_string_lossy().into_owned();
    opts.scheduler = Some(Default::default());
    opts.sandbox = std::env::var("XHERD_SANDBOX").is_ok_and(|v| v == "1");
    opts.local_peers = std::env::var("XHERD_LOCAL_PEERS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let d = xherd_daemon::Daemon::start(paths.clone(), opts).await?;
    tracing::info!(url = %format!("http://{addr}"), data = %paths.root.display(), version = xherd_core::VERSION, "xherd daemon listening");
    xherd_api::serve(d, listener, shutdown_signal()).await?;
    tracing::info!("xherd daemon stopped; agent sessions keep running");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        if let Ok(mut s) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
}

// ---- user client ----------------------------------------------------------------

fn with_client(f: impl FnOnce(&Client) -> i32) -> i32 {
    match Client::new() {
        Ok(c) => f(&c),
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

struct Client {
    url: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl Client {
    fn new() -> Result<Client, String> {
        let (cfg, _) = Config::load(&Paths::from_env().config()).map_err(|e| e.to_string())?;
        let url = std::env::var("XHERD_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| cfg.url());
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(1)))
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .new_agent();
        Ok(Client {
            url,
            token: cfg.user_token,
            agent,
        })
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let url = format!("{}/api/v1{path}", self.url.trim_end_matches('/'));
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let res = if method == "GET" {
            let mut r = self.agent.get(&url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.call()
        } else {
            let mut r = self.agent.post(&url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            match body {
                Some(b) => r.send_json(b),
                None => r.send_empty(),
            }
        };
        let resp = res.map_err(|e| format!("daemon not reachable at {}: {e}", self.url))?;
        let ok = resp.status().is_success();
        let v: Value = resp.into_body().read_json().unwrap_or(Value::Null);
        if ok {
            Ok(v)
        } else {
            Err(v["error"].as_str().unwrap_or("request failed").to_string())
        }
    }
}

fn status() -> i32 {
    match Client::new().and_then(|c| c.call("GET", "/healthz", None).map(|v| (c.url, v))) {
        Ok((url, v)) => {
            println!(
                "xherd {} running at {url}",
                v["version"].as_str().unwrap_or("?")
            );
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn ls() -> i32 {
    let v = match Client::new().and_then(|c| c.call("GET", "/agents", None)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let agents = v["agents"].as_array().cloned().unwrap_or_default();
    if agents.is_empty() {
        println!("no agents");
        return 0;
    }
    println!(
        "{:<24} {:<14} {:<16} GATE / PR",
        "NAME", "STATUS", "SESSION"
    );
    for a in agents {
        println!(
            "{:<24} {:<14} {:<16} {}",
            a["name"].as_str().unwrap_or("?"),
            a["status"].as_str().unwrap_or("?"),
            a["session"].as_str().unwrap_or("-"),
            gate_or_pr(&a)
        );
    }
    0
}

/// The open gate, else the pull request once there is one.
fn gate_or_pr(a: &Value) -> String {
    match (a["gate"].is_object(), a["prUrl"].as_str()) {
        (false, Some(url)) => format!("pr: {url}"),
        _ => gate_summary(&a["gate"]),
    }
}

/// `spec: spec.md`, `commit C01: a.rs, b.rs`, `question: …`, or `-`.
fn gate_summary(g: &Value) -> String {
    let list = |v: &Value, key: Option<&str>| {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| key.map_or(x.as_str(), |k| x[k].as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };
    match g["kind"].as_str() {
        Some("docs") => format!(
            "{}: {}",
            g["stage"].as_str().unwrap_or("?"),
            list(&g["files"], Some("path"))
        ),
        Some("commit") => format!(
            "commit {}: {}",
            g["id"].as_str().unwrap_or("?"),
            list(&g["files"], None)
        ),
        Some("question") => format!("question: {}", g["text"].as_str().unwrap_or("")),
        Some(k) => k.to_string(),
        None => "-".into(),
    }
}

fn install(runtime: Option<String>, pack: Option<String>) -> i32 {
    let body = serde_json::json!({"runtime": runtime, "pack": pack});
    match Client::new().and_then(|c| c.call("POST", "/install", Some(body))) {
        Ok(v) => {
            for f in v["installed"].as_array().into_iter().flatten() {
                let path = f["path"].as_str().unwrap_or("?");
                match f["action"].as_str() {
                    Some("kept") => {
                        println!("kept     {path} (edited; new version in {path}.xherd-new)")
                    }
                    Some(a) => println!("{a:<8} {path}"),
                    None => {}
                }
            }
            0
        }
        Err(e) => {
            eprintln!("xherd install: {e}");
            2
        }
    }
}

fn new_agent(
    key: String,
    slug: Option<String>,
    flow: Option<String>,
    repo: Option<String>,
    base: Option<String>,
    prompt: Option<String>,
) -> i32 {
    // The daemon has its own working directory: send an absolute path.
    let repo = match repo {
        Some(r) if r.contains("://") || r.starts_with("git@") => r,
        r => {
            let p = r.map_or_else(std::env::current_dir, |r| std::fs::canonicalize(&r));
            match p {
                Ok(p) => p.to_string_lossy().into_owned(),
                Err(e) => {
                    eprintln!("xherd new: repository: {e}");
                    return 2;
                }
            }
        }
    };
    let body = serde_json::json!({
        "ticketKey": key, "slug": slug, "repo": repo, "base": base, "flow": flow, "prompt": prompt,
    });
    match Client::new().and_then(|c| c.call("POST", "/agents", Some(body))) {
        Ok(v) => {
            let a = &v["agent"];
            println!(
                "{} {} in {}",
                a["name"].as_str().unwrap_or("?"),
                a["status"].as_str().unwrap_or("?"),
                a["workdir"].as_str().unwrap_or("?")
            );
            0
        }
        Err(e) => {
            eprintln!("xherd new: {e}");
            2
        }
    }
}

/// The diff on stdout (pipe it to a pager); what it covers on stderr.
fn diff(agent: &str, json: bool) -> i32 {
    let v = match Client::new().and_then(|c| c.call("GET", &format!("/agents/{agent}/diff"), None))
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    if json {
        println!("{v}");
        return 0;
    }
    let names = |k: &str| {
        v[k].as_array()
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default()
    };
    let c = &v["commit"];
    if let Some(range) = v["branch"].as_str() {
        eprintln!("ship: branch {range}: {}", names("files"));
        let others = names("others");
        if !others.is_empty() {
            eprintln!("uncommitted, not pushed: {others}");
        }
    } else if c.is_object() {
        eprintln!(
            "commit {} \"{}\": {}",
            c["id"].as_str().unwrap_or("?"),
            c["subject"].as_str().unwrap_or(""),
            names("files")
        );
        if let Some(r) = c["needsHuman"].as_str() {
            eprintln!("needs a human: {r}");
        }
        let others = names("others");
        if !others.is_empty() {
            eprintln!("not in this commit: {others}");
        }
    } else {
        eprintln!(
            "no commit proposed; uncommitted changes: {}",
            names("files")
        );
    }
    let d = v["diff"].as_str().unwrap_or("");
    if !d.is_empty() {
        println!("{d}");
    }
    0
}

fn action(agent: &str, what: &str, body: Option<Value>) -> i32 {
    match Client::new().and_then(|c| c.call("POST", &format!("/agents/{agent}/{what}"), body)) {
        Ok(v) => {
            println!(
                "{} is {}",
                v["agent"]["name"].as_str().unwrap_or(agent),
                v["agent"]["status"].as_str().unwrap_or("?")
            );
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shell words of a one-line command (double and single quotes only).
    fn words(s: &str) -> Vec<String> {
        let (mut out, mut cur, mut quote, mut any) = (Vec::new(), String::new(), None, false);
        for c in s.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => cur.push(c),
                (None, '"' | '\'') => (quote, any) = (Some(c), true),
                (None, c) if c.is_whitespace() => {
                    if any || !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                    any = false;
                }
                (None, c) => cur.push(c),
            }
        }
        if any || !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    /// Every `` `xherd …` `` call the xherd prompts teach is a valid
    /// command line for this CLI (a call ends at the closing backtick).
    #[test]
    fn prompt_calls_parse() {
        let env = |k: &str| (k == "HOME").then(|| "/home/u".to_string());
        let plan = xherd_flow::builtin()
            .plan_install("claude-code", xherd_flow::install::BUILTIN_FILES, &env)
            .unwrap();
        let mut seen = 0;
        for f in &plan {
            for line in f.text.lines() {
                for (i, _) in line.match_indices("`xherd ") {
                    let rest = &line[i + "`xherd ".len()..];
                    let call = rest.split('`').next().unwrap();
                    let argv = std::iter::once("xherd".to_string()).chain(words(call));
                    if let Err(e) = Cli::try_parse_from(argv) {
                        panic!("{}: `{call}`: {e}", f.path.display());
                    }
                    seen += 1;
                }
            }
        }
        assert!(seen > 30, "only {seen} calls found");
        assert_eq!(words(r#"a "b c" 'd' """#), ["a", "b c", "d", ""]);
    }

    #[test]
    fn ls_shows_the_gate_else_the_pr() {
        let url = "https://github.com/acme/app/pull/42";
        let gate =
            serde_json::json!({"kind": "docs", "stage": "ship", "files": [{"path": "pr.md"}]});
        let a = serde_json::json!({"gate": gate, "prUrl": url});
        assert_eq!(gate_or_pr(&a), "ship: pr.md");
        assert_eq!(
            gate_or_pr(&serde_json::json!({"prUrl": url})),
            format!("pr: {url}")
        );
        assert_eq!(gate_or_pr(&serde_json::json!({})), "-");
    }
}
