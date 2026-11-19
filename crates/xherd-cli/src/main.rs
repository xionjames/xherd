//! The `xherd` binary.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use serde_json::Value;
use xherd_core::config::Config;
use xherd_core::paths::Paths;

mod agent;
mod backup;
mod docker;
mod flow;
mod git;
mod guard;
mod init;
mod open_in;
mod plugin;
mod report;
mod toolchain;
mod xap;

#[derive(Parser)]
#[command(
    name = "xherd",
    version = xherd_core::BUILD_VERSION,
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
    /// Run the daemon in the foreground (`restart`: restart a running one).
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonCmd>,
    },
    /// Container entrypoint (root): prepare the user, home, SSH and git, then run the daemon.
    Init,
    /// Run commands for the daemon as this user (container, R-1203): listens on a unix socket.
    Runner {
        /// Socket path.
        #[arg(long)]
        socket: std::path::PathBuf,
        /// Socket file mode, octal (the daemon's group must reach it).
        #[arg(long, default_value = "660")]
        mode: String,
        /// Also keep a tmux server on this socket path (the agents' sessions).
        #[arg(long)]
        tmux: Option<std::path::PathBuf>,
        /// With --tmux: the user who may use that server (the daemon's).
        #[arg(long, requires = "tmux")]
        tmux_allow: Option<String>,
    },
    /// Run xherd in Docker: write the compose file, check the setup, update.
    #[command(subcommand)]
    Docker(DockerCmd),
    /// Is the daemon up?
    Status,
    /// Create a job: worktree, branch and `.xherd/`.
    New {
        /// Job name: lowercase letters, digits and `-` (also the tmux session and worktree folder).
        name: String,
        /// Ticket: an issue URL or a key (PAY-412, #12). Its key prefixes the branch.
        #[arg(long)]
        ticket: Option<String>,
        /// Flow of the built-in pack, or `none` (default `feature`).
        #[arg(long)]
        flow: Option<String>,
        /// Local repository, or a remote URL cloned into the workspace. Default: the current
        /// directory when it is inside a git repository, else the repository of an https
        /// ticket URL that a tracker reads.
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        base: Option<String>,
        /// What xherd may approve by itself: ask (default) or important.
        #[arg(long)]
        autonomy: Option<xherd_core::autonomy::Autonomy>,
        /// Tool permissions: ask, edits, auto or all (container only); default: the profile's, else Settings.
        #[arg(long)]
        mode: Option<xherd_core::permission::PermissionMode>,
        /// Runtime plugin (claude-code, opencode, codex); default: the profile's, else Settings.
        #[arg(long)]
        runtime: Option<String>,
        /// Model for the runtime (opus, sonnet…); default: the profile's, else Settings, else the runtime's own.
        #[arg(long)]
        model: Option<String>,
        /// Start from a saved agent (`xherd agent ls`): its settings are the defaults for the flags above.
        #[arg(long)]
        agent: Option<String>,
        /// Give one stage of the workflow its own agent: `design=architect` (repeatable; over the workflow's assignments).
        #[arg(long = "stage-agent", value_name = "STAGE=AGENT", value_parser = parse_stage_agent)]
        stage_agent: Vec<(String, String)>,
        prompt: Option<String>,
    },
    /// Saved agents: a named, versioned setup that jobs start from.
    #[command(subcommand)]
    Agent(agent::AgentCmd),
    /// Which saved agent runs which stage of a workflow.
    #[command(subcommand)]
    Flow(flow::FlowCmd),
    /// External plugins: install from a git repository, update, remove, list.
    #[command(subcommand)]
    Plugin(plugin::PluginCmd),
    /// List jobs.
    Ls,
    /// Install a pack's commands for job runtimes (default: xherd, every runtime switched on).
    Install {
        #[arg(long)]
        runtime: Option<String>,
        #[arg(long)]
        pack: Option<String>,
        /// Delete the command files the pack no longer ships.
        #[arg(long)]
        remove_obsolete: bool,
    },
    /// Change what xherd may approve by itself: ask or important.
    Autonomy {
        job: String,
        level: xherd_core::autonomy::Autonomy,
    },
    /// Change a job's tool permissions (ask, edits, auto, all); used from its next session start.
    Mode {
        job: String,
        mode: xherd_core::permission::PermissionMode,
    },
    /// Change a job's model ("" = the runtime's default); used from its next session start.
    Model { job: String, model: String },
    /// Pause a job (it writes a handoff note first), or with --all every live job at once, with no note (kill switch).
    Pause {
        job: Option<String>,
        #[arg(long, conflicts_with = "job")]
        all: bool,
    },
    /// resume a paused, cancelled or failed job.
    Resume { job: String },
    /// Cancel a job (ends its session, keeps the worktree unless --remove-worktree).
    Cancel {
        job: String,
        /// Also remove the worktree folder (the branch and the job's record stay).
        #[arg(long)]
        remove_worktree: bool,
    },
    /// Remove a done or cancelled job's worktree folder (the branch and the record stay).
    RemoveWorktree { job: String },
    /// Type a message into a job's session.
    Send { job: String, text: String },
    /// Ask a paused, parked, watching or failed job something: starts a session with the text and holds it there (no auto-approval, no move to watching) until `xherd finish` or new flow work.
    Talk { job: String, text: String },
    /// End a `talk`: back to watching its PR when it was watching, else paused.
    Finish { job: String },
    /// Mark a job with no workflow done (it has no PR to merge); offers to save its setup as an agent.
    Done { job: String },
    /// Have the job merge its PR's base branch into its branch and resolve the conflicts (a new session; the merge commit waits for your approval).
    ResolveConflicts { job: String },
    /// Approve the job's open gate (documents or commit) as they are now.
    Approve {
        job: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Ask the job to revise its open gate.
    Changes { job: String, comment: String },
    /// Show the open commit proposal's diff, the branch at a ship gate, else every uncommitted change.
    Diff { job: String },
    /// git credential helper: answers `get` with the token of Settings → Git access.
    Credential { op: String },
    /// Git access.
    #[command(subcommand)]
    Git(GitCmd),
    /// Security overview.
    #[command(subcommand)]
    Security(SecurityCmd),
    /// Back up and restore xherd's data and the jobs' unfinished work.
    #[command(subcommand)]
    Backup(BackupCmd),
    /// Checks behind xherd's git hooks.
    #[command(subcommand)]
    Guard(GuardCmd),
    /// Open a job's worktree in an app on this computer (apps: ~/.config/xherd/open.json, else editors on PATH).
    Open {
        job: String,
        #[arg(long)]
        app: Option<String>,
    },
    /// Attach this terminal to a job's session (in the container when xherd runs in Docker).
    Attach {
        job: String,
        /// Attach to this action's session instead (it must be running).
        #[arg(long)]
        action: Option<String>,
    },
    /// List a job's actions and whether they run.
    Actions { job: String },
    /// Run one of a job's actions (a changed or new command needs --approve).
    Run {
        job: String,
        action: String,
        /// Approve the command as it stands now.
        #[arg(long)]
        approve: bool,
    },
    /// Stop a running action (Ctrl-C, killed after 5 s).
    Stop { job: String, action: String },
    /// A shell in a job's worktree (in the container when xherd runs in Docker).
    Shell { job: String },
    /// Register the xherd-term:// link that opens a terminal in a worktree.
    #[command(subcommand)]
    TerminalHandler(TerminalHandlerCmd),
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
    /// XAP: propose an update of the repo's shared memory (AGENTS.md).
    #[command(subcommand)]
    Memory(xap::MemoryCmd),
    /// XAP: register actions the user can run (dev server, tests…).
    #[command(subcommand)]
    Action(xap::ActionCmd),
}

#[derive(Subcommand)]
enum BackupCmd {
    /// Write one archive: database, config, plugins, archive, and each live job's .xherd, branch bundle and uncommitted work. No secrets or logins.
    Create {
        /// Folder for the archive (default: the data root's backups/, in the container <workspace>/xherd-backups).
        #[arg(long)]
        to: Option<std::path::PathBuf>,
    },
    /// Check an archive (hashes, database, bundles) without restoring it.
    Verify { archive: std::path::PathBuf },
    /// Restore an archive into the data root (daemon stopped). Jobs come back paused.
    Restore {
        archive: std::path::PathBuf,
        /// Workspace on this machine, when it differs from the one in the backup.
        #[arg(long)]
        workspace: Option<String>,
        /// Also rewrite a path prefix of the jobs' repositories: OLD=NEW (repeatable).
        #[arg(long = "map")]
        maps: Vec<String>,
        /// Replace an existing database (it is kept in backups/).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum SecurityCmd {
    /// Summarise the current exposure: network binding, plugins, limits.
    Check,
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
enum DaemonCmd {
    /// Restart the running daemon in place (same process, new code and plugins); jobs keep running.
    Restart,
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
    /// Pause live jobs, pull and restart the container, resume them.
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
enum TerminalHandlerCmd {
    /// Install (or update) the handler for this user, for the daemon's worktrees.
    Install,
    /// Remove the handler and its registration.
    Uninstall,
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

/// `xherd job new …` is `xherd new …`: the word `job` before a command is dropped.
fn argv_without_job_word(mut args: Vec<String>) -> Vec<String> {
    if args.get(1).map(String::as_str) == Some("job") && args.len() > 2 {
        args.remove(1);
    }
    args
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

    let cli = match Cli::try_parse_from(argv_without_job_word(args)) {
        Ok(c) => c,
        Err(e) => {
            let code = if e.use_stderr() { 64 } else { 0 };
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    let json = cli.json;
    let code = match cli.cmd {
        Cmd::Daemon { action: None } => run_daemon(),
        Cmd::Daemon {
            action: Some(DaemonCmd::Restart),
        } => with_client(daemon_restart),
        Cmd::Init => init::init(),
        Cmd::Runner {
            socket,
            mode,
            tmux,
            tmux_allow,
        } => run_runner(&socket, &mode, tmux, tmux_allow),
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
        Cmd::Agent(c) => agent::run(c, json),
        Cmd::Flow(c) => flow::run(c, json),
        Cmd::Plugin(c) => plugin::run(c, json),
        Cmd::Install {
            runtime,
            pack,
            remove_obsolete,
        } => install(runtime, pack, remove_obsolete),
        Cmd::New {
            name,
            ticket,
            flow,
            repo,
            base,
            autonomy,
            mode,
            runtime,
            model,
            agent,
            stage_agent,
            prompt,
        } => new_job(
            repo,
            xherd_core::new::NewJob {
                name,
                ticket,
                ticket_key: None,
                slug: None,
                repo: String::new(),
                agent,
                agents: stage_agent.into_iter().collect(),
                base,
                flow,
                prompt,
                autonomy,
                permission_mode: mode,
                runtime,
                model,
            },
        ),
        Cmd::Model { job, model } => {
            action(&job, "model", Some(serde_json::json!({"model": model})))
        }
        Cmd::Mode { job, mode } => action(&job, "mode", Some(serde_json::json!({"mode": mode}))),
        Cmd::Autonomy { job, level } => {
            action(&job, "autonomy", Some(serde_json::json!({"level": level})))
        }
        Cmd::Pause { job: Some(job), .. } => action(&job, "pause", None),
        Cmd::Pause { all: true, .. } => with_client(pause_all),
        Cmd::Pause { .. } => {
            eprintln!("xherd pause: give an job name, or --all");
            2
        }
        Cmd::Resume { job } => action(&job, "resume", None),
        Cmd::Cancel {
            job,
            remove_worktree,
        } => action(
            &job,
            "cancel",
            remove_worktree.then(|| serde_json::json!({"removeWorktree": true})),
        ),
        Cmd::RemoveWorktree { job } => action(&job, "remove-worktree", None),
        Cmd::Send { job, text } => action(&job, "send", Some(serde_json::json!({"text": text}))),
        Cmd::Talk { job, text } => action(&job, "ask", Some(serde_json::json!({"text": text}))),
        Cmd::Finish { job } => action(&job, "finish", None),
        Cmd::Done { job } => action(&job, "mark-done", None),
        Cmd::ResolveConflicts { job } => action(&job, "resolve-conflicts", None),
        Cmd::Approve { job, reason } => {
            action(&job, "approve", Some(serde_json::json!({"reason": reason})))
        }
        Cmd::Changes { job, comment } => action(
            &job,
            "request-changes",
            Some(serde_json::json!({"comment": comment})),
        ),
        Cmd::Diff { job } => diff(&job, json),
        Cmd::Toolchain(ToolchainCmd::Exec { argv }) => toolchain::exec(argv),
        Cmd::Credential { op } => git::credential(&op),
        Cmd::Guard(GuardCmd::PrePush { branch, .. }) => guard::pre_push(&branch),
        Cmd::Open { job, app } => with_client(|c| open_in::open(c, &job, app)),
        Cmd::Attach { job, action } => {
            with_client(|c| open_in::attach(c, &job, false, action.as_deref()))
        }
        Cmd::Shell { job } => with_client(|c| open_in::attach(c, &job, true, None)),
        Cmd::Actions { job } => with_client(|c| actions_list(c, &job, json)),
        Cmd::Run {
            job,
            action: id,
            approve,
        } => action_call(
            &job,
            &id,
            "run",
            approve.then(|| serde_json::json!({"approve": true})),
            json,
        ),
        Cmd::Stop { job, action: id } => action_call(&job, &id, "stop", None, json),
        Cmd::TerminalHandler(TerminalHandlerCmd::Install) => open_in::terminal_handler(false),
        Cmd::TerminalHandler(TerminalHandlerCmd::Uninstall) => open_in::terminal_handler(true),
        Cmd::Git(GitCmd::Doctor { host, repo }) => with_client(|c| git::doctor(c, host, repo)),
        Cmd::Security(SecurityCmd::Check) => with_client(security_check),
        Cmd::Backup(BackupCmd::Create { to }) => backup::create(to),
        Cmd::Backup(BackupCmd::Verify { archive }) => backup::verify(&archive),
        Cmd::Backup(BackupCmd::Restore {
            archive,
            workspace,
            maps,
            force,
        }) => backup::restore(&archive, workspace, maps, force),
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
        Cmd::Memory(c) => xap::run(c.into(), json),
        Cmd::Action(c) => xap::run(c.into(), json),
    };
    ExitCode::from(code as u8)
}

// ---- daemon -------------------------------------------------------------------

fn run_runner(
    socket: &std::path::Path,
    mode: &str,
    tmux: Option<std::path::PathBuf>,
    tmux_allow: Option<String>,
) -> i32 {
    let Ok(mode) = u32::from_str_radix(mode, 8) else {
        eprintln!("xherd runner: --mode {mode:?} is not octal");
        return 2;
    };
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("xherd runner: {e}");
            return 1;
        }
    };
    let served = rt.block_on(async {
        // The daemon waits for this socket to answer, then starts sessions
        // at once: serve it only once the daemon may use the tmux server
        // (a resume at start failed with "Permission denied" otherwise).
        if let Some(t) = tmux {
            let (tx, rx) = tokio::sync::oneshot::channel();
            tokio::spawn(xherd_core::runner::supervise_tmux(t, tmux_allow, tx));
            let _ = tokio::time::timeout(std::time::Duration::from_secs(20), rx).await;
        }
        xherd_core::runner::serve(socket, mode).await
    });
    match served {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("xherd runner: {}: {e}", socket.display());
            1
        }
    }
}

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
        Ok(false) => 0,
        // A restart was requested (R7a): the runtime is gone and the listener
        // closed; start again as the same process so new plugins load.
        Ok(true) => {
            drop(rt);
            let e = exec_self();
            tracing::error!(error = %e, "restart failed");
            eprintln!("xherd daemon: cannot restart: {e}");
            1
        }
        Err(e) => {
            tracing::error!(error = %e, "daemon stopped");
            eprintln!("xherd daemon: {e}");
            1
        }
    }
}

/// How long open connections (terminals, websockets) may delay a stop or restart.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// The binary to run again: Linux appends " (deleted)" to `current_exe` when
/// the file was replaced by an update.
fn restart_exe(exe: std::path::PathBuf) -> std::path::PathBuf {
    match exe.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
        Some(s) => s.into(),
        None => exe,
    }
}

/// Replace this process with a new `xherd` with the same arguments; returns
/// only when that failed.
fn exec_self() -> std::io::Error {
    use std::os::unix::process::CommandExt;
    match std::env::current_exe() {
        Ok(exe) => std::process::Command::new(restart_exe(exe))
            .args(std::env::args_os().skip(1))
            .exec(),
        Err(e) => e,
    }
}

/// `xherd daemon restart`: ask, then wait until the daemon is down and up again.
fn daemon_restart(c: &Client) -> i32 {
    if let Err(e) = c.call("POST", "/admin/restart", None) {
        eprintln!("xherd daemon restart: {e}");
        return 1;
    }
    let up = || c.call("GET", "/healthz", None).is_ok();
    let poll = Duration::from_millis(50);
    let start = std::time::Instant::now();
    let mut seen_down = false;
    while start.elapsed() < Duration::from_secs(60) {
        if up() {
            if seen_down {
                println!("xherd restarted");
                return 0;
            }
            // Still the old process: it stops within the grace period.
            if start.elapsed() > STOP_GRACE + Duration::from_secs(5) {
                break;
            }
        } else {
            seen_down = true;
        }
        std::thread::sleep(poll);
    }
    eprintln!(
        "xherd daemon restart: {}",
        if seen_down {
            "the daemon did not come back within 60 s (see its log)"
        } else {
            "the daemon did not stop"
        }
    );
    1
}

/// Runs the daemon; `true` when it stopped for a restart request.
async fn daemon_main() -> Result<bool, Box<dyn std::error::Error>> {
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

    let tmux = xherd_term::Tmux::new(cfg.tmux_server());
    let mut opts = xherd_daemon::Options::new(
        Arc::new(xherd_daemon::TerminalDriver { tmux }),
        Arc::new(xherd_daemon::SystemClock),
    );
    opts.xherd_bin = std::env::current_exe()?.to_string_lossy().into_owned();
    opts.scheduler = Some(Default::default());
    opts.sandbox = std::env::var("XHERD_SANDBOX").is_ok_and(|v| v == "1");
    opts.install_env = Some(Arc::new(xherd_core::runner::agent_env));
    opts.runtime_install = opts.sandbox;
    // Commands in worktrees run as the agent user through its runner (R-1203).
    if let Some(sock) = std::env::var_os("XHERD_RUNNER").filter(|s| !s.is_empty()) {
        xherd_core::runner::use_runner(sock.into());
    }
    opts.local_peers = std::env::var("XHERD_LOCAL_PEERS")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    opts.plugin_install = std::env::var("XHERD_PLUGIN_INSTALL").map_or(true, |v| v != "off");
    let d = xherd_daemon::Daemon::start(paths.clone(), opts).await?;
    tracing::info!(url = %format!("http://{addr}"), data = %paths.root.display(), version = xherd_core::BUILD_VERSION, "xherd daemon listening");
    let restarting = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (stopping_tx, stopping_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown = {
        let (d, restarting) = (d.clone(), restarting.clone());
        async move {
            tokio::select! {
                _ = shutdown_signal() => {},
                _ = d.restart_requested() => {
                    restarting.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _ = stopping_tx.send(());
        }
    };
    let serve = xherd_api::serve(d, listener, shutdown);
    tokio::pin!(serve);
    // Graceful stop, but open terminals must not hold it forever.
    tokio::select! {
        r = &mut serve => r?,
        _ = async {
            let _ = stopping_rx.await;
            tokio::time::sleep(STOP_GRACE).await;
        } => tracing::warn!("open connections did not close in time"),
    }
    let restart = restarting.load(std::sync::atomic::Ordering::SeqCst);
    tracing::info!(restart, "xherd daemon stopped; agent sessions keep running");
    Ok(restart)
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

/// `xherd security check` (R-1210): informational, exit 0 when it answered.
fn pause_all(c: &Client) -> i32 {
    match c.call("POST", "/jobs/pause-all", None) {
        Ok(v) => {
            let names: Vec<&str> = v["paused"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|n| n.as_str())
                .collect();
            println!("paused {}: {}", names.len(), names.join(", "));
            0
        }
        Err(e) => {
            eprintln!("xherd pause --all: {e}");
            1
        }
    }
}

fn security_check(c: &Client) -> i32 {
    let v = match c.call("GET", "/security", None) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("xherd security check: {e}");
            return 2;
        }
    };
    for f in v["findings"].as_array().into_iter().flatten() {
        println!(
            "{} {:<18} {}",
            if f["ok"].as_bool().unwrap_or(false) {
                "ok  "
            } else {
                "WARN"
            },
            f["label"].as_str().unwrap_or("?"),
            f["detail"].as_str().unwrap_or("")
        );
    }
    for p in v["pending"].as_array().into_iter().flatten() {
        println!("n/a  {}", p.as_str().unwrap_or(""));
    }
    0
}

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
        self.call_with(&self.agent, method, path, body)
    }

    /// For calls that take minutes (a backup): a 30-minute limit instead of 30 seconds.
    fn call_slow(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let slow = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(1)))
            .timeout_global(Some(Duration::from_secs(30 * 60)))
            .build()
            .new_agent();
        self.call_with(&slow, method, path, body)
    }

    fn call_with(
        &self,
        agent: &ureq::Agent,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}/api/v1{path}", self.url.trim_end_matches('/'));
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let res = if method == "GET" || method == "DELETE" {
            let mut r = if method == "GET" {
                agent.get(&url)
            } else {
                agent.delete(&url)
            };
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.call()
        } else {
            let mut r = if method == "PUT" {
                agent.put(&url)
            } else {
                agent.post(&url)
            };
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
    let v = match Client::new().and_then(|c| c.call("GET", "/jobs", None)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let agents = v["jobs"].as_array().cloned().unwrap_or_default();
    if agents.is_empty() {
        println!("no jobs");
        return 0;
    }
    print!("{}", ls_table(&agents));
    0
}

/// `xherd ls` as text: each column as wide as its longest value, the
/// runtime's session id cut to 8 characters (its full form is a UUID).
/// `--stage-agent design=architect`.
fn parse_stage_agent(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((stage, agent)) if !stage.trim().is_empty() && !agent.trim().is_empty() => {
            Ok((stage.trim().to_string(), agent.trim().to_string()))
        }
        _ => Err("expected STAGE=AGENT, like design=architect".into()),
    }
}

/// The saved agent a job runs with now, by name (`-` when none).
fn current_agent_name(job: &Value) -> String {
    job["currentAgent"]
        .as_str()
        .and_then(|id| job["agents"][id]["name"].as_str())
        .unwrap_or("-")
        .to_string()
}

fn ls_table(agents: &[Value]) -> String {
    let mut rows = vec![["JOB", "AGENT", "STATUS", "SESSION", "GATE / PR"].map(String::from)];
    for a in agents {
        let session = a["session"].as_str().unwrap_or("-");
        rows.push([
            a["name"].as_str().unwrap_or("?").to_string(),
            current_agent_name(a),
            a["status"].as_str().unwrap_or("?").to_string(),
            session.chars().take(8).collect(),
            gate_or_pr(a),
        ]);
    }
    let width = |i: usize| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0);
    let (w0, w1, w2, w3) = (width(0), width(1), width(2), width(3));
    rows.iter()
        .map(|r| {
            format!(
                "{:<w0$}  {:<w1$}  {:<w2$}  {:<w3$}  {}\n",
                r[0], r[1], r[2], r[3], r[4]
            )
        })
        .collect()
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

fn install(runtime: Option<String>, pack: Option<String>, remove_obsolete: bool) -> i32 {
    let body = serde_json::json!({
        "runtime": runtime, "pack": pack, "removeObsolete": remove_obsolete,
    });
    match Client::new().and_then(|c| c.call("POST", "/install", Some(body))) {
        Ok(v) => {
            for l in install_lines(&v) {
                println!("{l}");
            }
            0
        }
        Err(e) => {
            eprintln!("xherd install: {e}");
            2
        }
    }
}

/// The lines `xherd install` prints for the daemon's answer (R7).
fn install_lines(v: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    let (mut pending, mut obsolete) = (false, false);
    for f in v["installed"].as_array().into_iter().flatten() {
        let path = f["path"].as_str().unwrap_or("?");
        match f["action"].as_str() {
            Some("kept") => {
                pending = true;
                out.push(format!(
                    "kept     {path} (edited; new version in {path}.xherd-new)"
                ));
            }
            Some("declined") => out.push(format!("declined {path} (you kept your version)")),
            Some("obsolete") => {
                obsolete = true;
                out.push(format!("obsolete {path} (the pack no longer ships it)"));
            }
            Some("removed") => out.push(format!("removed  {path}")),
            Some(a) => out.push(format!("{a:<8} {path}")),
            None => {}
        }
    }
    if obsolete || pending {
        out.push("Edited and obsolete files stay as they are: decide in Setup, or run `xherd install --remove-obsolete` to delete the obsolete ones.".into());
    }
    out
}

/// Is `dir` inside a git repository (an ancestor has a `.git` file or folder)? No process runs.
fn inside_git(dir: &std::path::Path) -> bool {
    dir.ancestors().any(|d| d.join(".git").exists())
}

fn new_job(repo: Option<String>, mut req: xherd_core::new::NewJob) -> i32 {
    // The daemon has its own working directory: send an absolute path.
    // An agent with a repository of its own needs none from here (R20).
    let agent_has_repo = repo.is_none()
        && req.agent.as_deref().is_some_and(|a| {
            Client::new()
                .and_then(|c| c.call("GET", &format!("/agents/{a}"), None))
                .is_ok_and(|v| v["repo"].as_str().is_some_and(|r| !r.is_empty()))
        });
    let repo = match repo {
        _ if agent_has_repo => String::new(),
        Some(r) if r.contains("://") || r.starts_with("git@") => r,
        None if req
            .ticket
            .as_deref()
            .is_some_and(|t| t.starts_with("https://"))
            && !std::env::current_dir().is_ok_and(|d| inside_git(&d)) =>
        {
            // Not in a repository: the daemon derives it from the ticket URL.
            String::new()
        }
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
    req.repo = repo;
    let body = serde_json::to_value(&req).expect("serialisable");
    match Client::new().and_then(|c| c.call("POST", "/jobs", Some(body))) {
        Ok(v) => {
            let a = &v["job"];
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
    let v = match Client::new().and_then(|c| c.call("GET", &format!("/jobs/{agent}/diff"), None)) {
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

/// `xherd actions <job>` (R29): one line per action, from the job's own
/// listing, with the ports it was given.
fn actions_list(c: &Client, job: &str, json: bool) -> i32 {
    let v = match c.call("GET", &format!("/jobs/{job}/actions"), None) {
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
    let list = v["actions"].as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        println!("No actions registered.");
    }
    for a in &list {
        let d = &a["def"];
        let state = match (a["last"]["state"].as_str(), a["last"]["exit"].as_i64()) {
            (None, _) => "never run".to_string(),
            (Some("exited"), Some(c)) => format!("exited {c}"),
            (Some(s), _) => s.to_string(),
        };
        let approved = if a["approved"].is_string() {
            ""
        } else {
            " (not approved)"
        };
        println!(
            "{} [{}] {} | {} | {state}{approved}",
            d["id"].as_str().unwrap_or("?"),
            d["kind"].as_str().unwrap_or("task"),
            d["label"].as_str().unwrap_or(""),
            d["run"].as_str().unwrap_or("")
        );
    }
    if let Some(p) = v["portBase"].as_u64() {
        println!("ports from {p}");
    }
    0
}

/// `xherd run|stop <job> <action>`.
fn action_call(job: &str, id: &str, what: &str, body: Option<Value>, json: bool) -> i32 {
    let path = format!("/jobs/{job}/actions/{id}/{what}");
    match Client::new().and_then(|c| c.call("POST", &path, body)) {
        Ok(v) => {
            if json {
                println!("{v}");
            } else {
                println!("{what} {id}: ok");
            }
            0
        }
        Err(e) => {
            eprintln!("{e}");
            2
        }
    }
}

fn action(agent: &str, what: &str, body: Option<Value>) -> i32 {
    match Client::new().and_then(|c| c.call("POST", &format!("/jobs/{agent}/{what}"), body)) {
        Ok(v) => {
            println!(
                "{} is {}",
                v["job"]["name"].as_str().unwrap_or(agent),
                v["job"]["status"].as_str().unwrap_or("?")
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
    #[test]
    fn install_lines_report_obsolete_and_hint() {
        let v = serde_json::json!({"installed": [
            {"path": "a.md", "action": "written"},
            {"path": "b.md", "action": "obsolete"},
            {"path": "c.md", "action": "removed"},
            {"path": "d.md", "action": "declined"},
        ]});
        let l = super::install_lines(&v);
        assert_eq!(l[0], "written  a.md");
        assert!(l[1].starts_with("obsolete b.md"));
        assert_eq!(l[2], "removed  c.md");
        assert!(l[3].starts_with("declined d.md"));
        assert!(l[4].contains("--remove-obsolete"));
        let ok = serde_json::json!({"installed": [{"path": "a.md", "action": "unchanged"}]});
        assert_eq!(super::install_lines(&ok), vec!["unchanged a.md"]);
    }

    use super::*;

    #[test]
    fn daemon_restart_is_a_subcommand() {
        let cli = Cli::try_parse_from(["xherd", "daemon"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Daemon { action: None }));
        let cli = Cli::try_parse_from(["xherd", "daemon", "restart"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Cmd::Daemon {
                action: Some(DaemonCmd::Restart)
            }
        ));
    }

    #[test]
    fn action_commands_parse() {
        let cli = Cli::try_parse_from(["xherd", "run", "feat", "dev", "--approve"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Run { approve: true, .. }));
        let cli = Cli::try_parse_from(["xherd", "stop", "feat", "dev"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Stop { .. }));
        let cli = Cli::try_parse_from(["xherd", "actions", "feat"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Actions { .. }));
        let cli = Cli::try_parse_from(["xherd", "attach", "feat", "--action", "dev"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Cmd::Attach {
                action: Some(_),
                ..
            }
        ));
        let cli = Cli::try_parse_from(["xherd", "attach", "feat"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Attach { action: None, .. }));
    }

    #[test]
    fn restart_exe_drops_the_deleted_suffix() {
        let p = |s: &str| std::path::PathBuf::from(s);
        assert_eq!(
            restart_exe(p("/usr/bin/xherd (deleted)")),
            p("/usr/bin/xherd")
        );
        assert_eq!(restart_exe(p("/usr/bin/xherd")), p("/usr/bin/xherd"));
    }

    #[test]
    fn job_word_is_an_alias() {
        let argv = |v: &[&str]| argv_without_job_word(v.iter().map(|s| s.to_string()).collect());
        let cli = Cli::try_parse_from(argv(&["xherd", "job", "new", "shop"])).unwrap();
        assert!(matches!(cli.cmd, Cmd::New { ref name, .. } if name == "shop"));
        let cli = Cli::try_parse_from(argv(&["xherd", "new", "shop"])).unwrap();
        assert!(matches!(cli.cmd, Cmd::New { .. }));
        assert!(Cli::try_parse_from(argv(&["xherd", "job"])).is_err());
    }

    #[test]
    fn inside_git_looks_at_ancestors_for_a_git_file_or_folder() {
        let d = tempfile::tempdir().unwrap();
        let deep = d.path().join("a/b");
        std::fs::create_dir_all(&deep).unwrap();
        assert!(!inside_git(&deep));
        std::fs::write(d.path().join(".git"), "gitdir: x").unwrap();
        assert!(inside_git(&deep));
        let d2 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d2.path().join(".git")).unwrap();
        assert!(inside_git(d2.path()));
    }

    #[test]
    fn ls_columns_fit_long_names_and_sessions() {
        let agents = [
            serde_json::json!({"name": "SHOP-151-order-export-csv", "status": "waiting-input",
                "agents": {"a1": {"id": "a1", "name": "reviewer"}}, "currentAgent": "a1",
                "session": "5c0e9f1a-2b7d-4e8a-9f3c-1d2e3f4a5b6c",
                "gate": {"kind": "commit", "id": "C02", "files": ["a.ts"]}}),
            serde_json::json!({"name": "SHOP-9-x", "status": "paused"}),
        ];
        assert_eq!(
            ls_table(&agents),
            "JOB                        AGENT     STATUS         SESSION   GATE / PR\n\
             SHOP-151-order-export-csv  reviewer  waiting-input  5c0e9f1a  commit C02: a.ts\n\
             SHOP-9-x                   -         paused         -         -\n"
        );
    }

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
    fn talk_and_finish_parse() {
        let cli = Cli::try_parse_from(["xherd", "talk", "shop", "Why the retry?"]).unwrap();
        assert!(
            matches!(cli.cmd, Cmd::Talk { ref job, ref text } if job == "shop" && text == "Why the retry?")
        );
        let cli = Cli::try_parse_from(["xherd", "finish", "shop"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Finish { ref job } if job == "shop"));
        assert!(Cli::try_parse_from(["xherd", "talk", "shop"]).is_err());
    }

    #[test]
    fn done_parses() {
        let cli = Cli::try_parse_from(["xherd", "done", "shop"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::Done { ref job } if job == "shop"));
    }

    #[test]
    fn resolve_conflicts_parses() {
        let cli = Cli::try_parse_from(["xherd", "resolve-conflicts", "shop"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::ResolveConflicts { ref job } if job == "shop"));
        assert!(Cli::try_parse_from(["xherd", "resolve-conflicts"]).is_err());
    }

    #[test]
    fn cancel_and_remove_worktree_parse() {
        let cli = Cli::try_parse_from(["xherd", "cancel", "shop"]).unwrap();
        assert!(
            matches!(cli.cmd, Cmd::Cancel { ref job, remove_worktree: false } if job == "shop")
        );
        let cli = Cli::try_parse_from(["xherd", "cancel", "shop", "--remove-worktree"]).unwrap();
        assert!(matches!(
            cli.cmd,
            Cmd::Cancel {
                remove_worktree: true,
                ..
            }
        ));
        let cli = Cli::try_parse_from(["xherd", "remove-worktree", "shop"]).unwrap();
        assert!(matches!(cli.cmd, Cmd::RemoveWorktree { ref job } if job == "shop"));
        assert!(Cli::try_parse_from(["xherd", "remove-worktree"]).is_err());
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
