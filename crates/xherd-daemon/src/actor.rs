//! One actor per agent (design §3.2). The actor owns the `Job` value, applies
//! messages one at a time with the pure functions of `xherd-core`, persists,
//! and publishes. Slow work runs in spawned tasks and comes back as `WorkDone`
//! tagged with the agent's generation; stale results are dropped.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use xherd_core::config::Config;
use xherd_core::domain::{Commit, CommitStatus, Gate, TaskStatus};
use xherd_core::report::{Effect, Origin, Outcome, Report, ReportEvent, apply_report};
use xherd_core::status::next_status;
use xherd_core::xap::{Refusal, XapCall, XapCmd, refuse as refusal};
use xherd_core::{Job, Status, Trigger};
use xherd_flow::Pack;
use xherd_flow::xap::{CommitFact, DocFact, DocState, Observed, VerifyFact};
use xherd_store::{Event, Store};
use xherd_term::ScreenRules;

use crate::driver::{Driver, JobCtx};
use crate::support::{
    Change, Clock, Hub, SCOPE_CREDENTIAL, SCOPE_HOOKS, SCOPE_XAP, TOKEN_AGENT, TOKEN_JOB,
    mint_token,
};

#[derive(Debug)]
pub enum Msg {
    Report(Report, Origin, Option<oneshot::Sender<ReportResult>>),
    User(UserAction, oneshot::Sender<Result<Job, ActionError>>),
    Xap(XapCall, oneshot::Sender<Result<Value, Refusal>>),
    Tick(TickKind),
    /// At the daemon's start: the terminal sessions that are not any job's
    /// own, so the job can kill the action sessions it does not know (R12).
    Sessions(Arc<Vec<String>>),
    WorkDone {
        generation: u64,
        result: WorkResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    /// Supervisor only: a slot is free.
    Start {
        port_base: u16,
    },
    Pause,
    /// The kill switch (R-032): pause now, with no handoff note and ending a
    /// pause that still waits for one.
    PauseNow,
    Resume,
    /// `remove_worktree`: once cancelled, also remove the worktree (R12).
    Cancel {
        remove_worktree: bool,
    },
    /// Remove a `done` or `cancelled` agent's worktree (R13, R14).
    RemoveWorktree,
    Send {
        text: String,
    },
    /// Resume a session-less agent with a question (#121, R5–R7): the
    /// session is held open until the agent does flow work or the user
    /// finishes it.
    Ask {
        text: String,
    },
    /// End an Ask session (R9): back to `watching`, else `paused`.
    Finish,
    /// Approve the open gate and hand on the next command (R-104).
    Approve {
        reason: Option<String>,
        /// Document → hash the user read (UI): refused when the file changed
        /// since, so the approval covers exactly what was shown.
        expect: std::collections::BTreeMap<String, String>,
        /// Commit gate: the commit message as the user edited it (else the
        /// agent's proposal is used).
        message: Option<String>,
    },
    /// Send the agent a change request for the open gate (R-105).
    RequestChanges {
        comment: String,
    },
    /// Change what xherd may approve by itself (R-109).
    SetAutonomy {
        level: xherd_core::autonomy::Autonomy,
    },
    /// Change the tool-permission mode (R-002a) with the runtime's argv for
    /// it, built by `Daemon::set_permission_mode`; used from the next start.
    SetMode {
        mode: xherd_core::permission::PermissionMode,
        launch: Vec<String>,
    },
    /// Change the model (issue #112), with the launch argv rebuilt by
    /// `Daemon::set_model`; used from the next start.
    SetModel {
        model: Option<String>,
        launch: Vec<String>,
    },
    /// Shared memory (spec §16.6): commit the proposed `AGENTS.md` (`text` =
    /// the user's edit, else the agent's file) on the agent's branch.
    MemoryAccept {
        text: Option<String>,
    },
    MemoryDismiss,
    /// Drop the pending agent-memory proposal (R29): after an accept, which
    /// `Daemon::agent_memory_accept` wrote, or a dismiss. Memory is untouched.
    AgentMemoryClear,
    /// End a job without a workflow by hand (R30): it has no PR to merge.
    MarkDone,
    /// The job now belongs to this saved agent (R32): accepting a
    /// suggestion or "Save as agent". Only `Job.agent` changes, to the
    /// agent's version 1 snapshot; the job keeps its own settings.
    AttachAgent {
        snapshot: Box<xherd_core::agent_def::JobAgent>,
    },
    /// Remember that the user does not want this job's suggestion (R32).
    SuggestionDismiss,
    /// Check the agent's pull request now (issue #131): the automatic poll
    /// without its guards, refused `no_pr` without a PR URL.
    CheckPr,
    /// Merge the PR's base into its branch (#139, R14): a new session with
    /// the pack's `conflicts` command.
    ResolveConflicts,
    /// A job that stopped without a remote has one now (R9): a new session
    /// with the ship command. `Daemon::set_origin` added and pushed it first.
    ShipAgain,
    /// End a job that stopped without a remote (R10). `Daemon::finish_local`
    /// merged its branch into the base checkout first.
    FinishLocal,
    /// Run an action of the job (R9, R14): `approve` is the user's allowance
    /// of its exact command; `port_base` is filled in by the supervisor.
    ActionRun {
        id: String,
        approve: bool,
        port_base: Option<u16>,
    },
    /// Stop a running action: Ctrl-C, then a kill after `STOP_GRACE` (R11).
    ActionStop {
        id: String,
    },
    /// Stop it, then run it again; `approve` as for `ActionRun` (R11).
    ActionRestart {
        id: String,
        approve: bool,
    },
    /// Remove an action of the job; a running one is stopped first (R17).
    ActionRemove {
        id: String,
    },
    /// The user allowed the command whose hash this is, by saving it to the
    /// repo profile `profile` (R22); refused `action_changed` when the
    /// action's command moved on since.
    ActionApprove {
        id: String,
        hash: String,
        profile: String,
    },
    /// Hide the "Save actions to repo profile" card for this job (R23).
    ActionsCardDismiss,
    Get,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickKind {
    ScreenScan,
    IdleCheck,
    ParkCheck,
    /// `maxSessionMinutes` (R-1206).
    SessionLimit,
    /// `recover`: the daemon just started, so a missing session means the
    /// terminal server went away with a container restart, not that the
    /// agent died; the agent is parked or resumed instead of failed.
    SessionCheck {
        alive: bool,
        recover: bool,
    },
    /// TODO checks and PR polling arrive with those features (v1.x / M4).
    TodoTick,
    PrPoll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkResult {
    Screen(String),
    DeliveryTimeout {
        delivery: u64,
    },
    /// The host plugin's answer to `pr-for-branch` (R-302).
    PrLookup(Result<Option<String>, String>),
    /// The host's answer to a poll of the pull request (R-303).
    /// The host's answer to `pr-template` (R-153).
    PrTemplates(Result<Vec<xherd_plugin::host::Template>, String>),
    /// `None` comments: they could not be read (the status still counts).
    PrPoll(
        Result<
            (
                xherd_core::pr::Snapshot,
                Option<Vec<xherd_core::pr::Comment>>,
            ),
            String,
        >,
    ),
    /// Tokens of a runtime session, read from its transcript (R-1205).
    Usage {
        session: String,
        tokens: u64,
    },
    /// Every sha on the branch since the base, and the commits among them
    /// no recorded commit matched, oldest first, with their files (R4).
    BranchCommits {
        on_branch: Vec<String>,
        found: Vec<xherd_core::domain::BranchCommit>,
    },
    /// The pane of a running action is dead: its exit status (`None` when a
    /// signal killed it) and what it printed (R11, R13).
    /// `stopped`: the user (or the job's end) stopped it; `restart` carries
    /// the approval flag of a Restart, which runs it again.
    ActionEnded {
        id: String,
        exit: Option<i32>,
        output: String,
        stopped: bool,
        restart: Option<bool>,
    },
    /// The action's pane still runs (a service counts as succeeded, R23).
    ActionAlive {
        id: String,
    },
    /// The action's session is gone with no exit status.
    ActionGone {
        id: String,
    },
}

impl WorkResult {
    /// Results that stay valid when the agent's generation moved on: results
    /// about actions (hand-on, resume), since an action outlives the session,
    /// and the branch check (R4), which the stop that starts it often ends.
    fn about_action(&self) -> bool {
        matches!(
            self,
            WorkResult::ActionEnded { .. }
                | WorkResult::ActionAlive { .. }
                | WorkResult::ActionGone { .. }
                | WorkResult::BranchCommits { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReportResult {
    Applied,
    Stale,
    Dormant,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct ActionError {
    pub code: &'static str,
    pub message: String,
}

/// Lines of scrollback an action's pane keeps for its log (R13).
const ACTION_HISTORY_LINES: u32 = 20_000;
/// How long Stop waits after Ctrl-C before killing the session (R11).
const STOP_GRACE: Duration = Duration::from_secs(5);

/// R3: an action's folder, once symlinks are followed, stays inside the
/// worktree. A folder that does not exist yet is judged by its nearest
/// existing parent (the lexical check already ran).
fn cwd_inside(wd: &Path, cwd: &str) -> Result<(), Refusal> {
    let root = wd
        .canonicalize()
        .map_err(|e| refusal("no_workspace", format!("worktree: {e}")))?;
    let mut p = wd.join(cwd);
    let real = loop {
        match p.canonicalize() {
            Ok(r) => break r,
            Err(_) if p.pop() => {}
            Err(_) => return Ok(()),
        }
    };
    if real.starts_with(&root) {
        Ok(())
    } else {
        Err(refusal(
            "path_outside_workspace",
            "cwd leaves the worktree through a link",
        ))
    }
}

/// Ctrl-C an action's session, wait up to `STOP_GRACE` for its pane to die,
/// and take what it printed: its exit status (`None` when killed by a signal
/// or still alive) and its output. The caller ends the session (R11).
async fn stop_action_session(driver: &dyn Driver, session: &str) -> (Option<i32>, String) {
    if let Err(e) = driver.interrupt_session(session).await {
        tracing::warn!(%session, "{}", e);
    }
    let deadline = std::time::Instant::now() + STOP_GRACE;
    let mut exit = None;
    loop {
        match driver.action_status(session).await {
            Ok(Some(st)) if st.dead => {
                exit = st.exit;
                break;
            }
            Ok(None) => break,
            _ => {}
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let output = driver
        .action_history(session, crate::actions::MAX_LOG_BYTES)
        .await
        .unwrap_or_default();
    (exit, output)
}

/// A job that shipped without a remote and now waits for the user (R9, R10).
pub(crate) fn stopped_without_origin(job: &Job) -> bool {
    job.no_origin && job.gate.is_none() && matches!(job.status, Status::Idle | Status::Paused)
}

fn refuse(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

/// Sent by actors to the supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupEvent {
    StatusChanged {
        id: String,
        to: Status,
    },
    /// The job's current agent changed (D26, R40): the supervisor counts
    /// its live jobs against the new one from now on.
    AgentChanged {
        id: String,
        agent: Option<String>,
    },
    /// Whether an action of the job runs: the supervisor keeps the job's
    /// port block while one does (R12a).
    ActionsRunning {
        id: String,
        running: bool,
    },
}

/// The settings actors read, replaced as a whole when the config is saved
/// (`Daemon::save_config`), so most settings apply without a restart.
#[derive(Clone)]
pub struct SettingsHandle(Arc<std::sync::RwLock<Arc<ActorSettings>>>);

impl SettingsHandle {
    pub fn new(s: ActorSettings) -> SettingsHandle {
        SettingsHandle(Arc::new(std::sync::RwLock::new(Arc::new(s))))
    }
    pub fn get(&self) -> Arc<ActorSettings> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn set(&self, s: ActorSettings) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(s);
    }
}

/// Settings an actor reads (snapshot of config).
#[derive(Debug, Clone)]
pub struct ActorSettings {
    /// Git hosts with their providers (`gitHosts`): which host plugin serves a remote.
    pub git_hosts: Vec<xherd_core::config::GitHost>,
    pub idle_alert_minutes: u64,
    pub gate_park_minutes: u64,
    pub fresh_sessions: bool,
    /// Longest a pause waits for the handoff note (`pauseTimeoutSec`, R-026).
    pub pause_timeout: Duration,
    pub delivery_ack: Duration,
    pub xherd_bin: String,
    pub xherd_url: String,
    pub screen_rules: Option<ScreenRules>,
    /// Install missing toolchain versions before the runtime starts
    /// (container); native installs only resolve (R-653).
    pub toolchain_install: bool,
    /// Default versions for tools without a version file (`toolchainDefaults`).
    pub toolchain_defaults: std::collections::BTreeMap<String, String>,
    /// Longest a task's `verify:` command may run.
    pub verify_timeout: Duration,
    /// Time between polls of a watched PR (R-303); zero = never.
    pub pr_poll: Duration,
    /// Code tripwires (R-115), from the config at daemon start.
    pub max_session_minutes: u64,
    pub max_review_loops: u32,
    /// Consecutive automatic approvals allowed (R-120); 0 = no limit.
    pub max_auto_approvals_in_row: u32,
    pub job_token_budget: u64,
    pub daily_token_budget: u64,
    pub max_diff_lines: usize,
    pub sensitive_paths: Vec<String>,
    /// Plugins switched off (`disabledPlugins`): the host, the trust writer
    /// and the toolchain plugins skip them while running.
    pub disabled_plugins: Vec<String>,
    /// Notification kinds that are not published (`mutedNotifications`).
    /// Offer to save a job's setup as an agent (`suggestAgents`, R30).
    pub suggest_agents: bool,
    pub muted_notifications: Vec<String>,
    /// Job actions are on (`jobActions`, R6) and how many a job holds
    /// (`maxActionsPerJob`).
    pub job_actions: bool,
    pub max_actions_per_job: usize,
}

impl ActorSettings {
    /// The config-derived fields over `base` (the parts that do not come
    /// from the config: binary, URL, runtime options). Called at start and
    /// whenever the config is saved, so these settings are live.
    pub fn from_config(
        c: &Config,
        base: ActorSettings,
        pr_poll_override: Option<Duration>,
    ) -> ActorSettings {
        ActorSettings {
            idle_alert_minutes: c.idle_alert_minutes,
            gate_park_minutes: c.gate_park_minutes,
            fresh_sessions: c.fresh_sessions,
            pause_timeout: Duration::from_secs(c.pause_timeout_sec),
            toolchain_defaults: c.toolchain_defaults.clone(),
            pr_poll: pr_poll_override.unwrap_or(Duration::from_secs(c.pr_poll_minutes * 60)),
            max_session_minutes: c.max_session_minutes,
            max_review_loops: c.max_review_loops,
            max_auto_approvals_in_row: c.max_auto_approvals_in_row,
            job_token_budget: c.job_token_budget,
            daily_token_budget: c.daily_token_budget,
            max_diff_lines: c.max_diff_lines,
            sensitive_paths: c.sensitive_paths.clone(),
            disabled_plugins: c.disabled_plugins.clone(),
            git_hosts: c.git_hosts.clone(),
            muted_notifications: c.muted_notifications.clone(),
            suggest_agents: c.suggest_agents,
            job_actions: c.job_actions,
            max_actions_per_job: c.max_actions_per_job,
            ..base
        }
    }
}

/// `host` with its secrets as they are on disk now, and, when it declares a
/// `token` secret but has none of its own, the one kept for `remote_host` in
/// Settings → Git access (read on each use, so a new token applies at once).
///
/// The plugin's own token is kept only for a host the user named: one with a
/// row in Settings → Git access (`git_hosts`) or a plain, wildcard-free name in
/// the plugin's `data.git.hosts`. The remote host comes from the agent's PR URL
/// or its `origin`, both of which the agent can change; a pattern such as
/// `*gitlab*` would otherwise hand the token to `gitlab.attacker.io`.
pub(crate) fn with_git_token(
    host: Arc<xherd_plugin::host::Host>,
    remote_host: Option<&str>,
    git_hosts: &[xherd_core::config::GitHost],
    secrets: &std::path::Path,
) -> Arc<xherd_plugin::host::Host> {
    let Ok(all) = xherd_core::config::Secrets::load(secrets) else {
        return host;
    };
    let mut h = (*host).clone();
    // The plugin's own secrets as they are now (saved in the UI, no restart).
    h.ctx.secrets =
        xherd_plugin::resolve_secrets(&h.plugin.manifest, &all.for_plugin(h.plugin.name()));
    let named = remote_host.is_some_and(|r| {
        git_hosts.iter().any(|g| g.host.eq_ignore_ascii_case(r))
            || xherd_plugin::git::Git::of(&h.plugin.manifest)
                .ok()
                .flatten()
                .is_some_and(|g| {
                    g.hosts
                        .iter()
                        .any(|p| !p.contains('*') && p.eq_ignore_ascii_case(r))
                })
    });
    if !named {
        h.ctx.secrets.remove("token");
    }
    let own = h
        .ctx
        .secrets
        .get("token")
        .is_some_and(|t| t.as_str().is_some_and(|t| !t.is_empty()));
    // Only a plugin that asks for a `token` gets the host's one: it is never
    // handed to a plugin that did not declare it.
    let declares = h
        .plugin
        .manifest
        .settings
        .iter()
        .any(|s| s.secret && s.key == "token");
    if declares
        && !own
        && let Some(t) = remote_host.and_then(|r| all.git_token(r))
    {
        h.ctx.secrets.insert("token".into(), t.into());
    }
    Arc::new(h)
}

/// The host name of the worktree's `origin` remote.
pub(crate) async fn origin_host(wd: &std::path::Path) -> Option<String> {
    let out = xherd_core::exec::run(
        xherd_core::exec::Cmd::new([
            "git",
            "-C",
            &wd.display().to_string(),
            "config",
            "--get",
            "remote.origin.url",
        ])
        .as_agent(),
    )
    .await
    .ok()?;
    crate::git_access::remote_host(String::from_utf8_lossy(&out.stdout).trim())
}

/// Which host plugin serves a remote on `remote_host` (D24): the provider
/// named on its row in Settings → Git access, else the plugin whose patterns
/// claim the name, else `github` (what every remote used before providers).
/// A switched-off plugin is never chosen, and none stands in for it: `None`.
pub fn pick_host(
    hosts: &[Arc<xherd_plugin::host::Host>],
    git_hosts: &[xherd_core::config::GitHost],
    disabled: &[String],
    remote_host: Option<&str>,
) -> Option<Arc<xherd_plugin::host::Host>> {
    let on = |h: &Arc<xherd_plugin::host::Host>| !disabled.iter().any(|n| n == h.plugin.name());
    let named = |n: &str| hosts.iter().find(|h| h.plugin.name() == n);
    let by_row = remote_host.and_then(|r| {
        git_hosts
            .iter()
            .find(|g| g.host == r)
            .and_then(|g| g.provider.as_deref())
    });
    let claimed = || {
        let r = remote_host?;
        hosts.iter().find(|h| {
            xherd_plugin::git::Git::of(&h.plugin.manifest)
                .ok()
                .flatten()
                .is_some_and(|g| g.claims(r))
        })
    };
    // The remote's own provider, even when it is switched off: another
    // plugin (`gh` on a GitLab merge request) would only fail on every poll.
    by_row
        .and_then(named)
        .or_else(claimed)
        .or_else(|| named(xherd_plugin::host::GITHUB))
        .filter(|h| on(h))
        .cloned()
}

impl Default for ActorSettings {
    fn default() -> ActorSettings {
        ActorSettings {
            git_hosts: Vec::new(),
            idle_alert_minutes: 15,
            gate_park_minutes: 30,
            fresh_sessions: true,
            pause_timeout: Duration::from_secs(120),
            delivery_ack: Duration::from_secs(10),
            xherd_bin: "xherd".into(),
            xherd_url: "http://127.0.0.1:4740".into(),
            screen_rules: None,
            toolchain_install: false,
            toolchain_defaults: Default::default(),
            verify_timeout: VERIFY_TIMEOUT,
            pr_poll: Duration::from_secs(300),
            max_session_minutes: 0,
            max_review_loops: 3,
            max_auto_approvals_in_row: 10,
            job_token_budget: 0,
            daily_token_budget: 0,
            max_diff_lines: xherd_core::tripwire::MAX_DIFF_LINES,
            sensitive_paths: Vec::new(),
            disabled_plugins: Vec::new(),
            muted_notifications: Vec::new(),
            suggest_agents: true,
            job_actions: true,
            max_actions_per_job: 12,
        }
    }
}

#[derive(Clone)]
pub struct Env {
    pub store: Store,
    pub driver: Arc<dyn Driver>,
    pub hub: Hub,
    pub clock: Arc<dyn Clock>,
    pub settings: SettingsHandle,
    pub sup: mpsc::UnboundedSender<SupEvent>,
    /// Flow packs by name.
    pub packs: Arc<BTreeMap<String, Pack>>,
    pub git: xherd_git::Git,
    /// Host plugins for pull requests (R-302, D24); `pick_host` chooses one
    /// by the host of the agent's remote.
    pub hosts: Vec<Arc<xherd_plugin::host::Host>>,
    /// `secrets.json`: a host plugin without a token of its own uses the
    /// one kept for the remote's host in Settings → Git access (R-685).
    pub secrets: std::path::PathBuf,
    /// Toolchain plugins in registry order (R-652).
    pub toolchains: Arc<Vec<(String, xherd_plugin::toolchain::Toolchain)>>,
    /// Per runtime plugin: the trust writer (the worktree is marked trusted
    /// before each session: the runtime rewrites that file itself, and may
    /// drop the entry), how to continue its last conversation, and how to
    /// read token usage from its transcript (R-1205).
    pub runtimes: Arc<BTreeMap<String, crate::RuntimeKit>>,
    /// The skills library (`<data>/skills`), copied from at a switch of agent.
    pub skills: std::path::PathBuf,
    /// The data root: action logs live under it (R13).
    pub paths: xherd_core::paths::Paths,
}

/// Default limit for a task's `verify:` command.
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(600);

/// Session losses the daemon answers with a resume by itself, per window.
const AUTO_RESUMES: usize = 2;
const AUTO_RESUME_WINDOW_MS: i64 = 3_600_000;

pub type AgentTx = mpsc::UnboundedSender<Msg>;

pub fn spawn(agent: Job, env: Env) -> AgentTx {
    let (tx, rx) = mpsc::unbounded_channel();
    // Earlier builds also scanned a stopped agent's screen and could block
    // one waiting at a gate on its own words; the scan clears such a block
    // once no failure is shown.
    let auth_screen_block = agent.status == Status::Blocked
        && agent.gate.is_some()
        && agent.block_reason.as_deref() == Some("git access");
    let actions_running = xherd_core::action::running(&agent);
    let actor = Actor {
        job: agent,
        env,
        tx: tx.clone(),
        delivery: None,
        auth_screen_line: None,
        auth_screen_block,
        next_delivery: 0,
        first_unacked: false,
        prompt_on_launch: false,
        resume_session: None,
        resume_unconfirmed: false,
        notified_gate: None,
        auto_resumes: Vec::new(),
        actions_running,
        stopping: BTreeSet::new(),
    };
    tokio::spawn(actor.run(rx));
    tx
}

/// What autonomy would do with the open gate (`Actor::auto_check`).
enum AutoCheck {
    /// Not autonomy's to approve: level `ask`, an Ask hold, no gate, a class
    /// the level does not allow.
    No,
    /// A tripwire or open questions hold it: (gate label, why).
    Held(String, String),
    /// The run limit is reached: (gate label, why).
    Limit(String, String),
    Approve {
        label: String,
        class: xherd_core::domain::Class,
    },
}

struct Actor {
    job: Job,
    env: Env,
    /// Own mailbox, for spawned work to report back.
    tx: AgentTx,
    /// Delivery waiting for acknowledgement: (id, retried).
    delivery: Option<(u64, bool)>,
    /// The auth-failure line last seen on the screen (R-688), so a failure
    /// that stays visible blocks once.
    auth_screen_line: Option<String>,
    /// A git-access block at a gate kept from an earlier build: it ends when
    /// no failure is on the screen.
    auth_screen_block: bool,
    next_delivery: u64,
    /// The first command was typed but no prompt was accepted yet. Claude Code
    /// can start a second session right after the first `session-start` and
    /// lose typed input, so the command stays in `next_prompt` and is typed
    /// again at the next `session-start` until a prompt is submitted.
    first_unacked: bool,
    /// The session was started with `next_prompt` on its launch line
    /// (`promptArgs`): its `session-start` waits for the ack, types nothing.
    prompt_on_launch: bool,
    /// Launch the next session with the runtime's own resume of this session id.
    resume_session: Option<String>,
    /// The last start used a runtime resume and no `session-start` came yet:
    /// if the session dies again, the next resume starts a clean session.
    resume_unconfirmed: bool,
    /// The gate the last `gate` notice was about (once per gate). In memory
    /// only: after a restart the same gate may notify again.
    notified_gate: Option<Gate>,
    /// When the daemon resumed this agent by itself (bounded, see above).
    auto_resumes: Vec<i64>,
    /// What the supervisor was last told about running actions.
    actions_running: bool,
    /// Actions with a stop under way: the screen tick leaves them to it.
    stopping: BTreeSet<String>,
}

impl Actor {
    /// Agents from before `visited_stages`: rebuild it once from their
    /// `stage.enter` / `gate.open` events, oldest first.
    async fn backfill_stages(&mut self) {
        if !self.job.visited_stages.is_empty() || self.job.stage.is_none() {
            return;
        }
        let Ok(mut events) = self.env.store.events(&self.job.id, 100_000).await else {
            return;
        };
        events.reverse();
        for e in events {
            if !matches!(e.kind.as_str(), "xap.stage.enter" | "xap.gate.open") {
                continue;
            }
            let stage = e.payload.as_ref().and_then(|p| p["args"]["stage"].as_str());
            if let Some(s) = stage
                && !self.job.visited_stages.iter().any(|v| v == s)
            {
                self.job.visited_stages.push(s.to_string());
            }
        }
        self.job.note_stage();
        if let Err(err) = self.env.store.put_job(&self.job).await {
            tracing::error!(agent = %self.job.name, %err, "agent not stored");
        }
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Msg>) {
        self.backfill_stages().await;
        while let Some(msg) = rx.recv().await {
            match msg {
                Msg::Report(r, origin, reply) => {
                    let res = self.on_report(r, origin).await;
                    if let Some(reply) = reply {
                        let _ = reply.send(res);
                    }
                }
                Msg::User(action, reply) => {
                    let res = self.on_user(action).await.map(|()| self.job.clone());
                    let _ = reply.send(res);
                }
                Msg::Xap(call, reply) => {
                    let _ = reply.send(self.on_xap(call).await);
                }
                Msg::Tick(t) => self.on_tick(t).await,
                Msg::Sessions(alive) => self.kill_unknown_actions(&alive),
                Msg::WorkDone { generation, result } => {
                    if generation != self.job.generation && !result.about_action() {
                        tracing::debug!(agent = %self.job.name, "dropping stale async result");
                        continue;
                    }
                    self.on_work(result).await;
                }
            }
        }
    }

    fn ctx(&self) -> JobCtx {
        JobCtx {
            id: self.job.id.clone(),
            name: self.job.name.clone(),
            workdir: PathBuf::from(self.job.workdir.clone().unwrap_or_else(|| ".".into())),
            env: Vec::new(),
        }
    }

    fn now(&self) -> i64 {
        self.env.clock.now()
    }

    async fn log(&self, id: Option<String>, kind: &str, payload: Option<Value>) {
        let e = Event {
            id: id.unwrap_or_else(xherd_core::new_id),
            job_id: self.job.id.clone(),
            kind: kind.to_string(),
            payload,
            at: self.now(),
        };
        if let Err(err) = self.env.store.insert_event(e).await {
            tracing::error!(agent = %self.job.name, %err, "event not stored");
        }
    }

    /// R-688: an authentication failure (a host command's error or the
    /// terminal screen) blocks a live agent with the reason `git access`;
    /// others are only notified. Returns whether `text` was one.
    async fn git_access_failed(&mut self, source: &str, text: &str) -> bool {
        let Some(line) = xherd_core::git_auth::failing_line(text) else {
            return false;
        };
        let host = self
            .job
            .pr_url
            .as_deref()
            .and_then(|u| u.split("//").nth(1))
            .and_then(|r| r.split('/').next())
            .unwrap_or("<host>")
            .to_string();
        self.log(
            None,
            "git-access-failed",
            Some(json!({"source": source, "line": line})),
        )
        .await;
        let msg = format!(
            "{}: git access failed ({line}). Run `xherd git doctor {host}` or open Settings → Git access",
            self.job.name
        );
        let before = self.job.status;
        if self.job.fire(Trigger::Blocked, self.now()).is_some() {
            self.job.screen_blocked = false;
            self.job.block_reason = Some("git access".into());
            self.commit(before).await;
        }
        self.notify("blocked", msg);
        true
    }

    /// Clears a git-access block once a host command works again, or the
    /// failure has left the screen. An agent waiting at a gate goes back to
    /// waiting, not to running.
    async fn git_access_ok(&mut self) {
        self.auth_screen_block = false;
        if self.job.block_reason.as_deref() == Some("git access") {
            let before = self.job.status;
            let t = if self.job.gate.is_some() {
                Trigger::StoppedAtGate
            } else {
                Trigger::Working
            };
            self.job.fire(t, self.now());
            self.commit(before).await;
        }
    }

    /// The host plugin for a remote on `remote_host` (`None` = unknown),
    /// unless it is switched off.
    fn host_for(&self, remote_host: Option<&str>) -> Option<Arc<xherd_plugin::host::Host>> {
        let settings = self.env.settings.get();
        pick_host(
            &self.env.hosts,
            &settings.git_hosts,
            &settings.disabled_plugins,
            remote_host,
        )
        .map(|h| with_git_token(h, remote_host, &settings.git_hosts, &self.env.secrets))
    }

    /// The host plugin for the agent's pull request.
    fn host(&self) -> Option<Arc<xherd_plugin::host::Host>> {
        let url = self.job.pr_url.as_deref();
        self.host_for(url.and_then(crate::git_access::remote_host).as_deref())
    }

    fn notify(&self, kind: &str, text: String) {
        if self
            .env
            .settings
            .get()
            .muted_notifications
            .iter()
            .any(|k| k == kind)
        {
            return;
        }
        let _ = self.env.hub.send(Change::Notify {
            job_id: self.job.id.clone(),
            kind: kind.into(),
            text,
        });
    }

    /// Tell the user once that the agent waits at its gate (#133). Called
    /// after a stop's auto-approval, so a gate autonomy approved is gone by
    /// then and sends nothing. Never document or question text.
    fn notify_gate(&mut self) {
        if !matches!(self.job.status, Status::WaitingInput | Status::Idle) {
            return;
        }
        let Some(gate) = &self.job.gate else {
            return;
        };
        if self.notified_gate.as_ref() == Some(gate) {
            return;
        }
        let mut text = format!("{}: {} waits for you", self.job.name, gate.label());
        if let Some(hold) = gate.hold() {
            let hold = hold.split_whitespace().collect::<Vec<_>>().join(" ");
            text.push_str(": ");
            text.extend(hold.chars().take(160));
        }
        self.notified_gate = Some(gate.clone());
        self.notify("gate", text);
    }

    /// Offer to save this job's setup as an agent (R30): once, for a job with
    /// no agent, while `suggestAgents` is on. One notice, never a modal.
    fn offer_agent(&mut self, reason: &str) {
        if !self.job.agents.is_empty()
            || self.job.suggestion.is_some()
            || !self.env.settings.get().suggest_agents
        {
            return;
        }
        self.job.suggestion = Some(xherd_core::job::Suggestion {
            reason: reason.into(),
            at: self.now(),
            dismissed: false,
        });
        self.notify(
            "suggestion",
            format!("{}: save this setup as an agent?", self.job.name),
        );
    }

    /// Keep a text the user wrote to this job, for the agent draft (R31).
    /// Only jobs without an agent: those are the ones that may become one.
    async fn keep_note(&self, text: &str) {
        let text = text.trim();
        if !self.job.agents.is_empty() || text.is_empty() {
            return;
        }
        if let Err(e) = self
            .env
            .store
            .add_note(&self.job.id, text, self.now())
            .await
        {
            tracing::warn!(job = %self.job.name, error = %e, "note not stored");
        }
    }

    /// `.xherd/spec/user-notes.md` (R31): what the user told this job, for
    /// the ship prompt's agent draft. Only the user's own texts; ticket and
    /// PR text never go in.
    async fn write_user_notes(&self) {
        if !self.job.agents.is_empty() {
            return;
        }
        let Ok(wd) = self.workdir() else {
            return;
        };
        let notes = self.env.store.notes(&self.job.id).await.unwrap_or_default();
        let mut text = String::from(
            "# What the user told this job\n\nOne item per message, oldest first.\n\n",
        );
        for (n, _) in &notes {
            let one: String = n.split_whitespace().collect::<Vec<_>>().join(" ");
            text.push_str("- ");
            text.extend(one.chars().take(500));
            text.push('\n');
        }
        if notes.is_empty() {
            text.push_str("None.\n");
        }
        let dir = wd.join(xherd_flow::xap::SPEC_DIR);
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            xherd_core::paths::write_atomic(&dir.join("user-notes.md"), text.as_bytes(), None)
        });
        if let Err(e) = written {
            tracing::warn!(job = %self.job.name, error = %e, "user-notes.md not written");
        }
    }

    /// Persist and publish; tell the supervisor when the status moved.
    async fn commit(&mut self, before: Status) {
        self.job.note_stage();
        if !self.job.status.is_live() {
            self.job.pause_requested_at = None;
        }
        // R12a: the port block stays with the job while an action runs.
        let running = xherd_core::action::running(&self.job);
        if !self.job.status.is_live() && !running {
            self.job.port_base = None;
        }
        // Every end of the session ends an Ask hold (queued = still starting).
        if !self.job.status.is_live() && self.job.status != Status::Queued {
            self.job.ask_hold = None;
        }
        // R4: a gate autonomy will approve at the stop does not wait for the
        // user while the agent still runs.
        self.job.auto_approve_pending = self.job.status == Status::Running
            && matches!(self.auto_check(), AutoCheck::Approve { .. });
        if self.job.gate.is_none() {
            self.notified_gate = None;
        }
        // R30: a job without an agent that is done (merged, or marked done)
        // offers to save its setup. Cancelled and failed jobs never do.
        if self.job.status == Status::Done && before != Status::Done {
            let reason = if self.job.flow.is_none() {
                "marked-done"
            } else {
                "done"
            };
            self.offer_agent(reason);
        }
        if let Err(err) = self.env.store.put_job(&self.job).await {
            tracing::error!(agent = %self.job.name, %err, "agent not stored");
        }
        let _ = self.env.hub.send(Change::Job {
            job: Box::new(self.job.clone()),
        });
        if running != self.actions_running {
            self.actions_running = running;
            let _ = self.env.sup.send(SupEvent::ActionsRunning {
                id: self.job.id.clone(),
                running,
            });
        }
        // R12: a paused, cancelled or done job leaves no action running.
        if matches!(
            self.job.status,
            Status::Paused | Status::Cancelled | Status::Done
        ) {
            for id in self.running_action_ids() {
                self.begin_stop(&id, None).await;
            }
        }
        if self.job.status != before {
            tracing::info!(agent = %self.job.name, from = %before, to = %self.job.status, "status");
            let _ = self.env.sup.send(SupEvent::StatusChanged {
                id: self.job.id.clone(),
                to: self.job.status,
            });
        }
    }

    async fn end_session(&mut self) {
        if let Err(e) = self.env.driver.stop(&self.ctx()).await {
            tracing::warn!(agent = %self.job.name, error = %e, "stopping session failed");
        }
        self.revoke_tokens().await;
        self.delivery = None;
    }

    async fn revoke_tokens(&self) {
        for kind in [TOKEN_JOB, TOKEN_AGENT] {
            if let Err(e) = self
                .env
                .store
                .revoke_tokens(kind, &self.job.id, self.now())
                .await
            {
                tracing::error!(job = %self.job.name, error = %e, "revoking job tokens failed");
            }
        }
    }

    // ---- reports -------------------------------------------------------------

    async fn on_report(&mut self, r: Report, origin: Origin) -> ReportResult {
        if origin == Origin::Job && !r.event.is_frequent() {
            match self.env.store.event_exists(&r.id).await {
                Ok(true) => return ReportResult::Duplicate,
                Ok(false) => {}
                Err(e) => tracing::error!(%e, "event lookup failed"),
            }
        }
        if origin == Origin::Job
            && matches!(r.event, ReportEvent::Stop | ReportEvent::Idle)
            && self.job.status.is_live()
            && self.job.gate.is_none()
            && r.seq > self.job.last_seq
        {
            self.infer_gate().await;
        }
        let before = self.job.status;
        let now = self.now();
        let outcome = apply_report(&mut self.job, &r, origin, now);
        let (result, effects) = match outcome {
            Outcome::Stale => (ReportResult::Stale, Vec::new()),
            Outcome::Dormant => (ReportResult::Dormant, Vec::new()),
            Outcome::Applied { effects, .. } => (ReportResult::Applied, effects),
        };
        if result != ReportResult::Applied || !r.event.is_frequent() {
            let kind = match origin {
                Origin::Job => format!("report.{}", r.event.as_str()),
                Origin::Daemon => format!("screen.{}", r.event.as_str()),
            };
            let id = (origin == Origin::Job).then(|| r.id.clone());
            let payload = json!({"seq": r.seq, "result": result, "payload": r.payload});
            self.log(id, &kind, Some(payload)).await;
        }
        // Stale reports change nothing but the sequence bookkeeping is untouched;
        // dormant ones may still have updated last_seq or released the session.
        if result != ReportResult::Stale {
            self.commit(before).await;
        }
        for e in effects {
            match e {
                Effect::Notify(text) => {
                    self.log(None, "notify", Some(json!({"text": text}))).await;
                    self.notify("blocked", text);
                }
                Effect::DeliveryAck => {
                    self.delivery = None;
                    if self.first_unacked {
                        self.first_unacked = false;
                        self.job.next_prompt = None;
                        self.commit(self.job.status).await;
                    }
                }
                Effect::UserPrompted => self.log(None, "user-prompted", None).await,
                Effect::AfterStop => {
                    self.measure_usage();
                    self.check_branch_commits();
                    // A pause waits for the handoff note; the stop after it
                    // ends the session. Earlier stops (the turn that was
                    // running) change nothing, and no approval happens now.
                    if let Some(asked) = self.job.pause_requested_at {
                        if self.job.handoff_at.is_some_and(|h| h >= asked) {
                            self.finish_pause(false).await;
                        }
                        continue;
                    }
                    // R-024: a background task (a subagent, say) still runs,
                    // and a fresh session or the move to watching would kill
                    // it. Its result starts another turn; that stop decides.
                    let running = xherd_core::report::background_running(&r.payload);
                    if running > 0 {
                        self.log(None, "background-running", Some(json!({"tasks": running})))
                            .await;
                        continue;
                    }
                    // R7: an Ask session stays open while the agent only
                    // answers; flow work ends the hold.
                    if self.job.ask_hold.is_some() {
                        continue;
                    }
                    self.try_auto_approve().await;
                    self.notify_gate();
                    self.after_ship().await;
                }
            }
        }
        if result == ReportResult::Applied && r.event == ReportEvent::SessionStart {
            self.resume_unconfirmed = false;
        }
        if result == ReportResult::Applied
            && r.event == ReportEvent::SessionStart
            && self.job.next_prompt.is_some()
        {
            if std::mem::take(&mut self.prompt_on_launch) {
                // Submitted by the runtime; the ack clears `next_prompt`.
                self.first_unacked = true;
                let text = self.job.next_prompt.clone();
                self.log(
                    None,
                    "first-prompt",
                    Some(json!({"text": text, "onLaunch": true})),
                )
                .await;
            } else {
                self.send_first_prompt().await;
            }
        }
        result
    }

    /// Read the session's tokens from the runtime's transcript (slow: spawned,
    /// back as `WorkDone`). Cumulative, so a dropped result is made up by the
    /// next stop. Without a session id or a runtime that has transcripts there
    /// is nothing to measure.
    fn measure_usage(&self) {
        let kit = self.runtime_kit();
        let (Some(session), Some(r), Some(usage)) = (
            self.job.session.clone(),
            kit.and_then(|k| k.resume.clone()),
            kit.and_then(|k| k.usage.clone()),
        ) else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.job.generation);
        let scratch = self.workdir().ok().map(|w| w.join(".xherd"));
        tokio::spawn(async move {
            // The transcript is in the agent user's home (R-1203).
            let env = xherd_core::runner::agent_env;
            let Some(text) = r.read(&session, &env, scratch.as_deref()).await else {
                return;
            };
            let tokens = usage.totals(&text).budget_tokens();
            let _ = tx.send(Msg::WorkDone {
                generation,
                result: WorkResult::Usage { session, tokens },
            });
        });
    }

    /// List the branch's commits since the job's base (slow: spawned, back
    /// as `WorkResult::BranchCommits`); with the files of those no recorded
    /// commit matches, so `on_branch_commits` can list them (R4). No base, no
    /// workdir or a git error: nothing is checked.
    fn check_branch_commits(&self) {
        let base = self
            .job
            .base
            .clone()
            .filter(|b| !b.is_empty() && !b.starts_with('-') && !b.contains(".."));
        let (Some(base), Ok(wd)) = (base, self.workdir()) else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.job.generation);
        let (git, recorded) = (self.env.git.clone(), self.job.commits.clone());
        tokio::spawn(async move {
            let branch = match git.branch_commits(&wd, &base).await {
                Ok(b) => b,
                Err(e) => {
                    tracing::debug!(error = %e, "branch commits not listed");
                    return;
                }
            };
            let mut found = Vec::new();
            for (sha, subject) in xherd_core::domain::unrecorded_commits(&recorded, &branch) {
                match git.commit_files(&wd, &sha).await {
                    Ok(files) => found.push(xherd_core::domain::BranchCommit {
                        sha,
                        subject,
                        files,
                    }),
                    Err(e) => {
                        tracing::debug!(error = %e, "files of a branch commit not read");
                        return;
                    }
                }
            }
            if found.is_empty() {
                return;
            }
            let on_branch = branch.into_iter().map(|(sha, _)| sha).collect();
            let _ = tx.send(Msg::WorkDone {
                generation,
                result: WorkResult::BranchCommits { on_branch, found },
            });
        });
    }

    /// Record the branch's commits that xherd did not make or hear of
    /// (`domain::settle_branch_commits`): xherd's own shared-memory commits
    /// from before `Mnn` rows (found by their `memory-accepted` events) as
    /// `Mnn`, rebased or amended ones under their row with one
    /// `commit-rewritten` event each, the rest as held `Xnn` rows with one
    /// `outside-commit` event each and one notification (count and short
    /// shas only) (R4, R5, R6).
    async fn on_branch_commits(
        &mut self,
        on_branch: Vec<String>,
        found: Vec<xherd_core::domain::BranchCommit>,
    ) {
        use xherd_core::domain::Settled;
        let memory: Vec<String> = match self.env.store.events_of_kind("memory-accepted").await {
            Ok(events) => events
                .into_iter()
                .filter(|e| e.job_id == self.job.id)
                .filter_map(|e| Some(e.payload?["sha"].as_str()?.to_string()))
                .collect(),
            Err(e) => {
                tracing::warn!(agent = %self.job.name, error = %e, "memory commits not read");
                vec![]
            }
        };
        let settled = xherd_core::domain::settle_branch_commits(
            &mut self.job.commits,
            &on_branch,
            &found,
            &memory,
        );
        if settled.is_empty() {
            return;
        }
        let mut shas = Vec::new();
        for s in settled {
            match s {
                Settled::Outside { id, sha } => {
                    self.log(None, "outside-commit", Some(json!({"id": id, "sha": sha})))
                        .await;
                    shas.push(sha.chars().take(7).collect::<String>());
                }
                Settled::Rewritten { id, from, sha } => {
                    self.log(
                        None,
                        "commit-rewritten",
                        Some(json!({"id": id, "from": from, "sha": sha})),
                    )
                    .await;
                }
                Settled::Memory { .. } => {}
            }
        }
        let n = shas.len();
        if n > 0 {
            let (s, was) = if n == 1 { ("", "was") } else { ("s", "were") };
            self.notify(
                "outside-commit",
                format!(
                    "{}: {n} commit{s} {was} made outside xherd ({})",
                    self.job.name,
                    shas.join(", ")
                ),
            );
        }
        self.commit(self.job.status).await;
    }

    /// Poll the pull request of a watching agent through the host plugin when
    /// `prPollMinutes` have passed (R-303; slow: spawned, back as `WorkDone`).
    /// Comments come with the rest of PR follow-up.
    fn poll_pr(&mut self) {
        let every = self.env.settings.get().pr_poll;
        let now = self.now();
        let due = self
            .job
            .pr_polled_at
            .is_none_or(|t| now - t >= every.as_millis() as i64);
        let (Some(host), Ok(wd), Some(url), true) = (
            self.host(),
            self.workdir(),
            self.job.pr_url.clone(),
            self.job.status == Status::Watching && !every.is_zero() && due,
        ) else {
            return;
        };
        // Set here so a slow host is not asked again by the next tick.
        self.job.pr_polled_at = Some(now);
        self.spawn_pr_poll(host, wd, url);
    }

    /// The user's check-now button (issue #131): the same snapshot, local
    /// mergeability, comments and `on_pr_poll` handling as the automatic
    /// poll (R2), with only `pr_url`, a workdir and a host required (R3).
    fn check_pr(&mut self) -> Result<(), ActionError> {
        let Some(url) = self.job.pr_url.clone() else {
            return Err(refuse(
                "no_pr",
                format!("{} has no pull request to check", self.job.name),
            ));
        };
        let (Some(host), Ok(wd)) = (self.host(), self.workdir()) else {
            return Err(refuse(
                "no_pr",
                format!(
                    "no host plugin to check the pull request of {}",
                    self.job.name
                ),
            ));
        };
        // Set here so a slow host is not asked twice and the next scheduled
        // poll waits a full interval from it.
        self.job.pr_polled_at = Some(self.now());
        self.spawn_pr_poll(host, wd, url);
        Ok(())
    }

    /// Ask the host for the PR snapshot and comments (slow: spawned, back as
    /// `WorkResult::PrPoll`, which `on_pr_poll` applies).
    fn spawn_pr_poll(&mut self, host: Arc<xherd_plugin::host::Host>, wd: PathBuf, url: String) {
        let (tx, generation) = (self.tx.clone(), self.job.generation);
        let (git, agent_base) = (self.env.git.clone(), self.job.base.clone());
        tokio::spawn(async move {
            let result = match host.pr_snapshot(&wd, &url).await {
                Ok(mut snap) => {
                    if snap.state == xherd_core::pr::PrState::Open
                        && snap.mergeable == xherd_core::pr::Mergeable::Unknown
                    {
                        snap.mergeable =
                            local_mergeable(&git, &wd, snap.base.as_deref(), agent_base.as_deref())
                                .await;
                    }
                    let comments = host
                        .pr_comments(&wd, &url)
                        .await
                        .map_err(|e| tracing::warn!(error = %e, "PR comments not read"))
                        .ok();
                    Ok((snap, comments))
                }
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(Msg::WorkDone {
                generation,
                result: WorkResult::PrPoll(result),
            });
        });
    }

    /// What a poll found: tell the user what is new (R-305) and finish the
    /// agent when the PR was merged or closed (R-307).
    async fn on_pr_poll(
        &mut self,
        snap: xherd_core::pr::Snapshot,
        comments: Option<Vec<xherd_core::pr::Comment>>,
    ) {
        use xherd_core::pr::{Event, PrState};
        let before = self.job.status;
        let name = self.job.name.clone();
        for e in self.job.apply_pr(&snap) {
            let text = match e {
                Event::Finished(state) => {
                    self.job.fire(Trigger::PrFinished, self.now());
                    let what = if state == PrState::Merged {
                        "merged"
                    } else {
                        "closed"
                    };
                    self.log(None, "pr-finished", Some(json!({"state": what})))
                        .await;
                    format!("{name}: the pull request was {what}")
                }
                Event::CiFailing(names) => {
                    format!("{name}: checks are failing: {}", names.join(", "))
                }
                Event::CiPassingAgain => format!("{name}: all checks pass again"),
                Event::Conflict => format!("{name}: the pull request has a merge conflict"),
            };
            self.notify("pr", text);
        }
        if self.job.status == Status::Watching
            && let Some(all) = comments
        {
            let fresh = self.job.new_comments(&all);
            if !fresh.is_empty() {
                self.on_new_comments(fresh).await;
            }
        }
        self.commit(before).await;
    }

    /// New comments on the pull request (R-304): save them for the agent in
    /// `pr-comments.json` (each body in an untrusted block, R-1201) and queue
    /// it with the pack's PR review command, in a fresh session. Without a
    /// flow (flow `none`) the user is only told.
    async fn on_new_comments(&mut self, fresh: Vec<xherd_core::pr::Comment>) {
        let n = fresh.len();
        let name = self.job.name.clone();
        let written = self.write_pr_comments(&fresh);
        let command = self.pack_command("prReview");
        if let Err(e) = &written {
            // Not handled: they are new again at the next poll.
            self.job
                .pr_comments_seen
                .retain(|id| !fresh.iter().any(|c| &c.id == id));
            tracing::warn!(agent = %name, error = %e, "pr-comments.json not written");
            return;
        }
        self.log(None, "pr-comments", Some(json!({"count": n})))
            .await;
        let s = if n == 1 { "" } else { "s" };
        match command {
            Some(cmd) => {
                self.job.next_prompt = Some(cmd);
                self.job.fire(Trigger::PrComments, self.now());
                self.notify(
                    "pr-comments",
                    format!("{name}: {n} new PR comment{s}; the agent will triage them"),
                );
            }
            None => self.notify("pr-comments", format!("{name}: {n} new PR comment{s}")),
        }
    }

    /// `.xherd/spec/pr-comments.json`: `{"comments": [...]}` with every body
    /// wrapped as untrusted data. Replaced each time: the agent is queued
    /// right after, and polling resumes only when it is watching again.
    fn write_pr_comments(&self, fresh: &[xherd_core::pr::Comment]) -> Result<(), String> {
        let dir = self
            .workdir()
            .map_err(|e| e.message)?
            .join(xherd_flow::xap::SPEC_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        crate::pr_round::start_round(self.workdir().map_err(|e| e.message)?.as_path(), fresh)?;
        let comments: Vec<_> = fresh
            .iter()
            .map(|c| {
                let mut c = c.clone();
                c.body =
                    xherd_core::untrusted::wrap(&format!("PR comment by {}", c.author), &c.body);
                c
            })
            .collect();
        let text = serde_json::to_string_pretty(&json!({"comments": comments}))
            .map_err(|e| e.to_string())?;
        xherd_core::paths::write_atomic(&dir.join("pr-comments.json"), text.as_bytes(), None)
            .map_err(|e| e.to_string())
    }

    /// Add a measurement to the agent and the day's total, then check the
    /// budgets (R-1205): a notification at 80 %, a pause at 100 % (this is the
    /// agent's stop, so "at its next stop" is now).
    async fn on_usage(&mut self, session: &str, tokens: u64) {
        use xherd_core::usage::{Level, day, level};
        let delta = self.job.record_usage(session, tokens);
        let today = day(self.now());
        if delta > 0
            && let Err(e) = self.env.store.add_usage("all", &today, delta).await
        {
            tracing::warn!(agent = %self.job.name, %e, "usage not stored");
        }
        // Recorded per saved agent (R24), shown but not enforced.
        if delta > 0
            && let Some(a) = self.job.agent()
            && let Err(e) = self
                .env
                .store
                .add_usage(&format!("agent:{}", a.id), &today, delta)
                .await
        {
            tracing::warn!(agent = %self.job.name, %e, "agent usage not stored");
        }
        let daily = self.env.store.usage("all", &today).await.unwrap_or(0);
        let s = self.env.settings.get();
        let agent_budget = self.guardrails(&s).await.job_token_budget;
        let checks = [
            ("agent", self.job.tokens_total(), agent_budget),
            ("daily", daily, s.daily_token_budget),
        ];
        let before = self.job.status;
        let hit = checks
            .iter()
            .find(|(_, used, budget)| level(*used, *budget) == Level::Exceeded);
        if let Some((scope, used, budget)) = hit {
            if next_status(self.job.status, Trigger::Pause).is_some() {
                self.end_session().await;
                self.job.fire(Trigger::Pause, self.now());
                self.log(
                    None,
                    "budget-exhausted",
                    Some(json!({"scope": scope, "used": used, "budget": budget})),
                )
                .await;
                self.notify(
                    "budget",
                    format!(
                        "{} paused: the {scope} token budget is used up ({used} of {budget})",
                        self.job.name
                    ),
                );
            }
        } else if !self.job.budget_warned
            && let Some((scope, used, budget)) = checks
                .iter()
                .find(|(_, used, budget)| level(*used, *budget) == Level::Warn)
        {
            self.job.budget_warned = true;
            self.notify(
                "budget",
                format!(
                    "{}: 80% of the {scope} token budget is used ({used} of {budget})",
                    self.job.name
                ),
            );
        }
        self.commit(before).await;
    }

    /// After ship (R-301): an agent following a flow that stopped with its PR
    /// open and nothing waiting on the user ends its session and watches the
    /// PR. Without a declared PR, a stop in the ship stage asks the host
    /// plugin for the branch's PR (R-302). Flow `none` keeps its session (no
    /// ship stage).
    ///
    /// Only a stop where the flow's work is over counts: the ship or a PR
    /// stage, or the implement stage with every task done (the end of
    /// `pr-apply`). An agent sent back to an earlier stage after the PR
    /// opened (a new review, say) keeps its session: a stop there means
    /// the turn ended, not the work (it may wait on a background task).
    async fn after_ship(&mut self) {
        if self.job.status != Status::Idle || self.job.gate.is_some() || self.job.pack.is_none() {
            return;
        }
        use xherd_core::domain::Role;
        let pack = self.job.pack.as_ref().and_then(|p| self.env.packs.get(p));
        let role = self
            .job
            .stage
            .as_deref()
            .and_then(|s| self.stage_role(pack, s));
        if self.job.pr_url.is_none() {
            // R9: no `origin` means no PR to look for, and the job page
            // offers a remote or a local finish instead.
            if role == Some(Role::Ship) && !self.job.no_origin {
                self.find_pr();
            }
            return;
        }
        let over = match role {
            Some(Role::Ship | Role::Pr) => true,
            Some(Role::Implement) => self.job.tasks.iter().all(|t| t.status == TaskStatus::Done),
            _ => false,
        };
        if !over {
            return;
        }
        let before = self.job.status;
        self.end_session().await;
        self.job.fire(Trigger::Watch, self.now());
        self.log(None, "watching", Some(json!({"pr": self.job.pr_url})))
            .await;
        self.commit(before).await;
    }

    /// Look the branch's PR up through the host plugin (slow: spawned, back
    /// as `WorkDone`). Only in the ship stage, where the agent opens it
    /// (`after_ship` checks).
    fn find_pr(&self) {
        let Ok(wd) = self.workdir() else {
            return;
        };
        let (tx, generation, branch) = (
            self.tx.clone(),
            self.job.generation,
            self.job.branch.clone(),
        );
        let settings = self.env.settings.get();
        let hosts = self.env.hosts.clone();
        let secrets = self.env.secrets.clone();
        tokio::spawn(async move {
            let remote = origin_host(&wd).await;
            let Some(host) = pick_host(
                &hosts,
                &settings.git_hosts,
                &settings.disabled_plugins,
                remote.as_deref(),
            )
            .map(|h| with_git_token(h, remote.as_deref(), &settings.git_hosts, &secrets)) else {
                return;
            };
            let result = host
                .pr_for_branch(&wd, &branch)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::WorkDone {
                generation,
                result: WorkResult::PrLookup(result),
            });
        });
    }

    /// The session is ready: type the command it was started for (the flow's
    /// start command, or the next command after approving a parked agent;
    /// R-005d, R-028). It is cleared once Claude accepts a prompt (the
    /// `user-prompt-submit` ack), so later session starts (`/clear`, resume)
    /// do not repeat it, while a start that loses the typed text does.
    /// Enters needed to submit `cmd`: the pack's `slashEnters` for its own
    /// commands, else one.
    fn enters_for(&self, cmd: &str) -> u32 {
        let runtime = self.runtime_name();
        self.job
            .pack
            .as_deref()
            .and_then(|p| self.env.packs.get(p))
            .and_then(|p| p.install.get(runtime))
            .filter(|i| cmd.starts_with(&i.prefix))
            .map_or(1, |i| i.slash_enters)
    }

    async fn send_first_prompt(&mut self) {
        let Some(cmd) = self.job.next_prompt.clone() else {
            return;
        };
        let enters = self.enters_for(&cmd);
        let before = self.job.status;
        match self.send(&cmd, enters).await {
            Ok(()) => {
                self.first_unacked = true;
                self.log(None, "first-prompt", Some(json!({"text": cmd})))
                    .await;
                self.commit(before).await;
            }
            Err(e) => {
                self.log(
                    None,
                    "first-prompt-failed",
                    Some(json!({"text": cmd, "error": e.message})),
                )
                .await;
                self.notify(
                    "delivery",
                    format!("{}: could not send {cmd:?}: {}", self.job.name, e.message),
                );
            }
        }
    }

    // ---- XAP -------------------------------------------------------------------

    /// Validate and apply one XAP call (design §6.5). Accepted calls are logged
    /// as `xap.<cmd>` under the call id, which also de-duplicates replays.
    async fn on_xap(&mut self, mut call: XapCall) -> Result<Value, Refusal> {
        if !self.job.status.is_live() {
            return Err(refusal(
                "not_live",
                format!("{} is {}", self.job.name, self.job.status),
            ));
        }
        if self.env.store.event_exists(&call.id).await.unwrap_or(false) {
            return Ok(json!({
                "changed": false,
                "duplicate": true,
                "reason": "this call was already applied",
            }));
        }
        // Actions are not flow work (R7): handled before the flow checks.
        if matches!(
            call.cmd,
            XapCmd::ActionAdd { .. }
                | XapCmd::ActionRm { .. }
                | XapCmd::ActionLs { .. }
                | XapCmd::ActionLog { .. }
        ) {
            return self.on_action_xap(&call).await;
        }
        self.commit_tripwires(&mut call.cmd).await;
        let packs = self.env.packs.clone();
        let pack = self.job.pack.as_deref().and_then(|p| packs.get(p));
        self.review_hold(&mut call.cmd, pack)?;
        self.pr_review_check(&call.cmd)?;
        // A sync carrying edits to an approved task list re-opens its gate at
        // once (R-107), so a wider scope never reaches a commit unapproved
        // (R-116); with another gate open it waits for that one.
        let reopen =
            matches!(call.cmd, XapCmd::TasksSync { .. }) && self.approved_tasks_edited(pack).await;
        if reopen && self.job.gate.is_some() {
            return Err(refusal(
                "tasks_approved",
                "tasks.md was approved and has changed, so it needs the user's approval again; \
                 wait until the open gate is decided, then sync",
            ));
        }
        let obs = self.observe(pack, &call.cmd).await?;
        let (before, now) = (self.job.status, self.now());
        let mut changed = xherd_flow::xap::apply(&mut self.job, pack, &call.cmd, obs, now)?;
        if reopen {
            self.infer_gate().await;
            changed |= self.job.gate.is_some();
        }
        if let XapCmd::IssueAdd { .. } = &call.cmd {
            self.append_issue(&call)
                .map_err(|e| refusal("internal", format!("issue ledger: {e}")))?;
        }
        let args = serde_json::to_value(&call.cmd)
            .map(|v| v["args"].clone())
            .unwrap_or(Value::Null);
        self.log(
            Some(call.id.clone()),
            &format!("xap.{}", call.cmd.name()),
            Some(json!({"args": args, "changed": changed})),
        )
        .await;
        self.write_progress(&call.cmd);
        if let XapCmd::StageEnter { stage, .. } = &call.cmd
            && self.stage_role(pack, stage) == Some(xherd_core::domain::Role::Ship)
        {
            self.fetch_templates();
            self.write_user_notes().await;
        }
        // R8: flow work after an Ask ends its hold.
        let hold_ended = call.cmd.is_flow_work() && self.job.ask_hold.take().is_some();
        if hold_ended {
            self.log(None, "ask-hold-ended", Some(json!({"by": call.cmd.name()})))
                .await;
        }
        if changed || hold_ended {
            self.commit(before).await;
        }
        let reason = (!changed)
            .then(|| xherd_flow::xap::no_change_reason(&call.cmd))
            .flatten();
        let mut answer =
            json!({"changed": changed, "stage": self.job.stage, "gate": self.job.gate});
        if let Some(r) = reason {
            answer["reason"] = r.into();
        }
        Ok(answer)
    }

    /// `action.add|rm|ls|log` (R1–R7, R17): the job's own list of actions.
    /// Running, stopping and approving are the user's, never an XAP call.
    async fn on_action_xap(&mut self, call: &XapCall) -> Result<Value, Refusal> {
        use xherd_core::action::{self, ActionDef, ActionState};
        let settings = self.env.settings.get();
        if !settings.job_actions {
            return Err(refusal("actions_off", "job actions are switched off"));
        }
        let before = self.job.status;
        let (changed, text) = match &call.cmd {
            XapCmd::ActionAdd {
                id,
                label,
                run,
                kind,
                cwd,
                description,
                icon,
            } => {
                let wd = self.workdir()?;
                let mut def = ActionDef {
                    id: id.clone(),
                    label: label.clone(),
                    description: description.clone().unwrap_or_default(),
                    run: run.clone(),
                    cwd: cwd.clone().unwrap_or_default(),
                    kind: kind.unwrap_or_default(),
                    icon: icon.clone().unwrap_or_default(),
                };
                action::check_def(&mut def)?;
                cwd_inside(&wd, &def.cwd)?;
                let replaced = self.job.actions.iter().any(|a| a.def.id == def.id);
                let changed =
                    action::add(&mut self.job, def.clone(), settings.max_actions_per_job)?;
                if changed {
                    let p = json!({"id": def.id, "label": def.label, "run": def.run,
                        "cwd": def.cwd, "kind": def.kind, "replaced": replaced});
                    self.log(Some(call.id.clone()), "xap.action.add", Some(p))
                        .await;
                }
                (changed, None)
            }
            XapCmd::ActionRm { id } => {
                let gone = action::remove(&mut self.job, id)?;
                if matches!(&gone.last, Some(r) if r.state == ActionState::Running) {
                    let session = action::session_name(&self.job.name, id);
                    if let Err(e) = self.env.driver.stop_session(&session).await {
                        tracing::warn!(job = %self.job.name, %session, "{}", e);
                    }
                }
                self.log(
                    Some(call.id.clone()),
                    "xap.action.rm",
                    Some(json!({"id": id})),
                )
                .await;
                (true, None)
            }
            XapCmd::ActionLs {} => (false, Some(action::ls_text(&self.job))),
            XapCmd::ActionLog { id } => {
                let a = self
                    .job
                    .actions
                    .iter()
                    .find(|a| a.def.id == *id)
                    .ok_or_else(|| refusal("action_unknown", format!("no action `{id}`")))?;
                let state = match &a.last {
                    None => "never run".to_string(),
                    Some(r) => {
                        let s = serde_json::to_value(r.state)
                            .ok()
                            .and_then(|v| v.as_str().map(String::from))
                            .unwrap_or_default();
                        match r.exit {
                            Some(c) => format!("{s}, exit code {c}"),
                            None => s,
                        }
                    }
                };
                let body = crate::actions::read_log(
                    &self.env.paths,
                    &self.job.id,
                    id,
                    crate::actions::MAX_READ_BYTES,
                )
                .unwrap_or_default();
                let wrapped = xherd_core::untrusted::wrap(&format!("action {id} output"), &body);
                (false, Some(format!("{id}: {state}\n{wrapped}")))
            }
            _ => return Err(refusal("internal", "not an action call")),
        };
        if changed {
            self.commit(before).await;
        }
        let reason = (!changed && text.is_none())
            .then(|| xherd_flow::xap::no_change_reason(&call.cmd))
            .flatten();
        let mut answer = json!({"changed": changed, "text": text});
        if let Some(r) = reason {
            answer["reason"] = r.into();
        }
        Ok(answer)
    }

    /// Approving the `pr-review` gate (R-306): settle one decision per comment
    /// (the user's over the agent's), post the replies decided `reply` through
    /// the host with xherd's marker, remembering each so a retry never posts
    /// twice, and write the final decisions for `pr-apply`. A failed post
    /// refuses the approval: nothing else changes. Inline, like
    /// `commit_for_user`: the user waits for the answer.
    async fn publish_pr_decisions(&self, wd: &Path) -> Result<(), ActionError> {
        use xherd_core::pr::{Decision, MARKER};
        let comments = crate::pr_round::load_comments(wd);
        if comments.is_empty() {
            // No round of comments (an agent started the stage by itself).
            return Ok(());
        }
        let proposals = crate::pr_round::load_proposals(wd).map_err(|e| refuse("doc_schema", e))?;
        let mut state = crate::pr_round::load_overrides(wd);
        let verdicts = xherd_core::pr::decide(&comments, &proposals, &state.overrides)
            .map_err(|e| refuse("pr_decisions", e))?;
        for v in verdicts.iter().filter(|v| v.decision == Decision::Reply) {
            if state.posted.contains(&v.id) {
                continue;
            }
            let (Some(host), Some(url)) = (self.host(), self.job.pr_url.clone()) else {
                return Err(refuse(
                    "no_host",
                    "no host plugin or pull request to reply on",
                ));
            };
            let body = format!(
                "{}\n\n{MARKER}",
                v.reply.as_deref().unwrap_or_default().trim()
            );
            host.pr_reply(wd, &url, &v.id, &body)
                .await
                .map_err(|e| refuse("reply_failed", format!("reply to {}: {e}", v.id)))?;
            state.posted.push(v.id.clone());
            crate::pr_round::save_overrides(wd, &state).map_err(|e| refuse("io", e))?;
        }
        crate::pr_round::write_decisions(wd, &verdicts, &state.posted).map_err(|e| refuse("io", e))
    }

    /// `gate open pr-review` (R-306): `pr-review.md` must decide every comment
    /// of the round, in the format, with a text for each reply (`doc_schema`
    /// otherwise), so the user is never shown an incomplete plan.
    fn pr_review_check(&self, cmd: &XapCmd) -> Result<(), Refusal> {
        let XapCmd::GateOpen { stage, .. } = cmd else {
            return Ok(());
        };
        if stage != xherd_flow::pr_review::STAGE {
            return Ok(());
        }
        let wd = self.workdir()?;
        let proposals = crate::pr_round::load_proposals(&wd)
            .map_err(|e| xherd_core::xap::refuse("doc_schema", e))?;
        let comments = crate::pr_round::load_comments(&wd);
        if comments.is_empty() {
            return Ok(());
        }
        xherd_core::pr::decide(&comments, &proposals, &Default::default())
            .map(|_| ())
            .map_err(|e| xherd_core::xap::refuse("doc_schema", format!("pr-review.md: {e}")))
    }

    fn stage_role(
        &self,
        pack: Option<&xherd_flow::Pack>,
        stage: &str,
    ) -> Option<xherd_core::domain::Role> {
        pack.zip(self.job.flow.as_deref())
            .and_then(|(p, f)| p.flow(f))
            .and_then(|f| f.stage(stage))
            .map(|s| s.role)
    }

    /// The repository's pull-request templates through the host plugin
    /// (R-153; spawned, back as `WorkDone`), for the `ship` prompt.
    fn fetch_templates(&self) {
        let Ok(wd) = self.workdir() else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.job.generation);
        let settings = self.env.settings.get();
        let hosts = self.env.hosts.clone();
        let secrets = self.env.secrets.clone();
        tokio::spawn(async move {
            let remote = origin_host(&wd).await;
            let Some(host) = pick_host(
                &hosts,
                &settings.git_hosts,
                &settings.disabled_plugins,
                remote.as_deref(),
            )
            .map(|h| with_git_token(h, remote.as_deref(), &settings.git_hosts, &secrets)) else {
                return;
            };
            let result = host.pr_template(&wd).await.map_err(|e| e.to_string());
            let _ = tx.send(Msg::WorkDone {
                generation,
                result: WorkResult::PrTemplates(result),
            });
        });
    }

    /// Review gate (R-114, R-120): `review.md` must have well-formed findings
    /// (`doc_schema` otherwise); the gate is held for a human while major or
    /// blocker findings are not fixed, review-sourced tasks are open, or the
    /// review→fix loop ran past `maxReviewLoops`.
    fn review_hold(
        &self,
        cmd: &mut XapCmd,
        pack: Option<&xherd_flow::Pack>,
    ) -> Result<(), Refusal> {
        use xherd_core::domain::{Role, TaskSource, TaskStatus};
        let XapCmd::GateOpen {
            stage, needs_human, ..
        } = cmd
        else {
            return Ok(());
        };
        let is_review = pack
            .zip(self.job.flow.as_deref())
            .and_then(|(p, f)| p.flow(f))
            .and_then(|f| f.stage(stage))
            .is_some_and(|s| s.role == Role::Review);
        if !is_review {
            return Ok(());
        }
        // Without a review.md the gate refuses on its own (`doc_missing`).
        let path = self
            .workdir()?
            .join(xherd_flow::xap::SPEC_DIR)
            .join("review.md");
        let findings = match std::fs::read_to_string(path) {
            Ok(text) => xherd_flow::review::parse(&text)
                .map_err(|e| xherd_core::xap::refuse("doc_schema", e))?,
            Err(_) => Vec::new(),
        };
        let open_findings: Vec<String> =
            xherd_flow::review::open_findings(&findings, &self.job.tasks)
                .iter()
                .map(|f| format!("{} ({})", f.id, f.severity.label()))
                .collect();
        let open: Vec<&str> = self
            .job
            .tasks
            .iter()
            .filter(|t| t.source == TaskSource::Review && t.status != TaskStatus::Done)
            .map(|t| t.id.as_str())
            .collect();
        let max = self.env.settings.get().max_review_loops;
        if let Some(why) =
            xherd_core::tripwire::review_summary(&open_findings, &open, self.job.review_rounds, max)
        {
            *needs_human = Some(match needs_human.take() {
                Some(own) => format!("{own}; {why}"),
                None => why,
            });
        }
        Ok(())
    }

    /// Tripwires on a commit proposal: secret (R-117), sensitive paths and diff size (R-115). Any hit holds it for a human (`needs_human`,
    /// masked). Reads the diff only; a failing git skips the diff checks.
    async fn commit_tripwires(&self, cmd: &mut XapCmd) {
        let XapCmd::CommitPropose {
            files, needs_human, ..
        } = cmd
        else {
            return;
        };
        use xherd_core::tripwire as tw;
        let mut hits = Vec::new();
        let st = self.env.settings.get();
        // The profile's values replace the global ones (R-115).
        let profile = self.profile().await;
        let guard = self.guardrails(&st).await;
        let (sensitive, max_lines) = (&guard.sensitive_paths, guard.max_diff_lines);
        hits.extend(tw::sensitive_summary(files, sensitive));
        if let Ok(wd) = self.workdir() {
            // A merge commit (#139, R16): a human checks how the conflicts
            // were resolved, so autonomy never approves it.
            if self.env.git.merge_in_progress(&wd).await {
                hits.push("merges the base branch: check the conflict resolution".to_string());
            }
            match self.env.git.diff_proposed(&wd, files).await {
                Ok(diff) => {
                    hits.extend(tw::summary(&tw::scan_diff(&diff)));
                    hits.extend(tw::size_summary(&diff, max_lines));
                    // A file too large to read was not scanned: a human looks.
                    hits.extend(
                        diff.lines()
                            .filter_map(|l| l.strip_prefix(xherd_git::TOO_LARGE))
                            .map(|t| format!("not checked for secrets: {t}")),
                    );
                }
                Err(e) => {
                    tracing::debug!(agent = %self.job.name, %e, "commit tripwires: no diff");
                }
            }
            // Lint first: it is the faster of the two.
            for (label, cmd) in [
                (
                    "lint",
                    profile.as_ref().and_then(|p| p.lint_command.as_deref()),
                ),
                (
                    "test",
                    profile.as_ref().and_then(|p| p.test_command.as_deref()),
                ),
            ] {
                if let Some(cmd) = cmd {
                    hits.extend(self.run_command_tripwire(&wd, label, cmd).await);
                }
            }
        }
        if !hits.is_empty() {
            let why = hits.join("; ");
            *needs_human = Some(match needs_human.take() {
                Some(own) => format!("{own}; {why}"),
                None => why,
            });
        }
    }

    /// The limits this job runs under: the profile's (else the global
    /// ones), tightened by its saved agent's (R13).
    async fn guardrails(&self, st: &ActorSettings) -> xherd_core::resolve::Guardrails {
        xherd_core::resolve::guardrails(
            &xherd_core::resolve::Guardrails {
                sensitive_paths: st.sensitive_paths.clone(),
                max_diff_lines: st.max_diff_lines,
                job_token_budget: st.job_token_budget,
            },
            self.profile().await.as_ref(),
            self.job.agent(),
            &self.job.agents,
        )
    }

    /// The profile the agent was created on, read at each use so a change in
    /// Settings applies at once; `None` when unset or deleted.
    async fn profile(&self) -> Option<xherd_core::profile::Profile> {
        let id = self.job.profile.as_deref()?;
        self.env
            .store
            .list_profiles()
            .await
            .ok()?
            .into_iter()
            .find(|p| p.id == id)
    }

    /// The profile's test or lint command (R-115), run in the worktree like a task's
    /// `verify:` (inline, as the agents' user): a failure or timeout holds the
    /// commit. Only the exit status is reported, never the output.
    async fn run_command_tripwire(&self, wd: &Path, label: &str, command: &str) -> Option<String> {
        let out = xherd_core::exec::run(
            xherd_core::exec::Cmd::new(["sh", "-c", command])
                .cwd(wd)
                .timeout(self.env.settings.get().verify_timeout)
                .as_agent(),
        )
        .await;
        match out {
            Ok(o) if o.timed_out => Some(format!("{label} command `{command}` timed out")),
            Ok(o) if o.code != Some(0) => Some(format!(
                "{label} command `{command}` failed (exit {})",
                o.code.map_or("?".into(), |c| c.to_string())
            )),
            Ok(_) => None,
            Err(e) => Some(format!("{label} command `{command}` did not run: {e}")),
        }
    }

    /// Write the progress lines xherd owns (D11) into `tasks.md` / `commits.md`
    /// after a task or commit call succeeded. A missing file or entry is
    /// skipped; this never fails the call.
    fn write_progress(&self, cmd: &XapCmd) {
        match cmd {
            XapCmd::TaskStart { id }
            | XapCmd::TaskDone { id, .. }
            | XapCmd::TaskBlock { id, .. } => {
                let Some(t) = self.job.tasks.iter().find(|t| &t.id == id) else {
                    return;
                };
                let evidence = (t.status == TaskStatus::Done)
                    .then_some(t.evidence.as_deref())
                    .flatten();
                self.edit_spec_file("tasks.md", |text| {
                    xherd_flow::progress::set_task(text, id, t.status, evidence)
                });
            }
            XapCmd::CommitPropose { id, .. }
            | XapCmd::CommitDone { id, .. }
            | XapCmd::CommitDrop { id, .. } => self.write_commit_progress(id),
            _ => {}
        }
    }

    /// `status:` and `sha:` of a commit entry in `commits.md`.
    fn write_commit_progress(&self, id: &str) {
        let Some(c) = self.job.commits.iter().find(|c| c.id == id) else {
            return;
        };
        let status = match c.status {
            CommitStatus::Proposed | CommitStatus::Approved => "proposed",
            CommitStatus::Done => "done",
            CommitStatus::Dropped => "dropped",
        };
        let sha = c.sha.as_deref().unwrap_or("");
        self.edit_spec_file("commits.md", |text| {
            xherd_flow::progress::set_commit(text, id, status, sha)
        });
    }

    fn edit_spec_file(&self, name: &str, edit: impl FnOnce(&str) -> Option<String>) {
        let Ok(wd) = self.workdir() else { return };
        let path = wd.join(xherd_flow::xap::SPEC_DIR).join(name);
        let Ok(text) = std::fs::read_to_string(&path) else {
            tracing::debug!(agent = %self.job.name, name, "no file for progress lines");
            return;
        };
        match edit(&text) {
            Some(new) if new != text => {
                if let Err(e) = xherd_core::paths::write_atomic(&path, new.as_bytes(), None) {
                    tracing::warn!(agent = %self.job.name, name, %e, "progress lines not written");
                }
            }
            Some(_) => {}
            None => {
                tracing::debug!(agent = %self.job.name, name, "entry not found for progress lines")
            }
        }
    }

    fn workdir(&self) -> Result<PathBuf, Refusal> {
        if self.job.worktree_removed_at.is_some() {
            return Err(refusal(
                "worktree_removed",
                format!("{}'s worktree was removed", self.job.name),
            ));
        }
        self.job.workdir.as_ref().map(PathBuf::from).ok_or_else(|| {
            refusal(
                "no_workspace",
                format!("{} has no workspace", self.job.name),
            )
        })
    }

    /// Look at the files and git for calls that need it. Small local reads,
    /// done inline: the agent waits for the answer anyway.
    async fn observe(&self, pack: Option<&Pack>, cmd: &XapCmd) -> Result<Observed, Refusal> {
        use xherd_flow::xap::{HANDOFF, SPEC_DIR, doc_path, gate_docs};
        // Every gate and commit proposal of an xherd flow leaves a valid memory.md.
        if matches!(cmd, XapCmd::GateOpen { .. } | XapCmd::CommitPropose { .. })
            && pack.is_some_and(|p| p.name == xherd_flow::BUILTIN)
            && xherd_flow::xap::flow_of(&self.job, pack).is_ok()
        {
            let file = self.workdir()?.join(SPEC_DIR).join("memory.md");
            xherd_flow::memory::check(std::fs::read_to_string(file).ok().as_deref())?;
        }
        // AGENTS.md and CLAUDE.md at the root are shared memory: they change
        // through `xherd memory propose`, not in a task's commit, except while
        // merging the base branch (conflicts in them need resolving).
        if let XapCmd::CommitPropose { files, .. } = cmd
            && files
                .iter()
                .any(|f| matches!(f.trim_start_matches("./"), "AGENTS.md" | "CLAUDE.md"))
            && !self.env.git.merge_in_progress(&self.workdir()?).await
        {
            return Err(refusal(
                "memory_file",
                "AGENTS.md and CLAUDE.md are shared memory: do not commit them; propose changes with `xherd memory propose`",
            ));
        }
        Ok(match cmd {
            XapCmd::GateOpen { .. } => {
                let wd = self.workdir()?;
                let mut facts = Vec::new();
                for d in gate_docs(&self.job, pack, cmd, &wd)? {
                    facts.push(self.doc_fact(&wd, d).await);
                }
                Observed::Docs(facts)
            }
            XapCmd::TasksSync { file } => {
                let wd = self.workdir()?;
                let d = doc_path(file.as_deref().unwrap_or("tasks.md"), &wd)?;
                let text = inside(&wd, &d).and_then(|p| std::fs::read_to_string(p).ok());
                // A `verify:` that is not a shell command is refused now, before
                // the tasks are approved (editing it later re-opens the gate).
                if let Some(Ok(tasks)) = text.as_deref().map(xherd_flow::tasks::parse) {
                    for t in tasks {
                        if let Some(v) = &t.verify {
                            self.verify_syntax(&t.id, v).await?;
                        }
                    }
                }
                Observed::Text(text)
            }
            XapCmd::TaskDone { id, .. } => {
                // Unknown task or none to run: `apply` decides.
                let verify = self
                    .job
                    .tasks
                    .iter()
                    .find(|t| &t.id == id && t.status != TaskStatus::Done)
                    .and_then(|t| t.verify.clone());
                match verify {
                    Some(command) => {
                        self.verify_syntax(id, &command).await?;
                        Observed::Verify(Some(self.run_verify(command).await?))
                    }
                    None => Observed::Verify(None),
                }
            }
            XapCmd::MemoryPropose { file, .. } => {
                let wd = self.workdir()?;
                if !wd.join("AGENTS.md").exists()
                    && let Some(repo) = &self.job.repo
                    && self
                        .env
                        .store
                        .memory_declined(repo, "AGENTS.md")
                        .await
                        .unwrap_or(false)
                {
                    return Err(refusal(
                        "memory_declined",
                        "the user declined a starter AGENTS.md for this repository; do not propose one",
                    ));
                }
                let f = file.as_deref().unwrap_or(xherd_flow::xap::MEMORY_PROPOSAL);
                let d = doc_path(f, &wd)?;
                Observed::Text(inside(&wd, &d).and_then(|p| std::fs::read_to_string(p).ok()))
            }
            XapCmd::AgentMemoryPropose { file, .. } => {
                let wd = self.workdir()?;
                let f = file
                    .as_deref()
                    .unwrap_or(xherd_flow::xap::AGENT_MEMORY_PROPOSAL);
                let d = doc_path(f, &wd)?;
                Observed::Text(inside(&wd, &d).and_then(|p| std::fs::read_to_string(p).ok()))
            }
            XapCmd::AgentPropose {
                instructions_file: Some(f),
                ..
            } => {
                let wd = self.workdir()?;
                let d = doc_path(f, &wd)?;
                Observed::Text(inside(&wd, &d).and_then(|p| std::fs::read_to_string(p).ok()))
            }
            XapCmd::HandoffWritten { file } => {
                let wd = self.workdir()?;
                let d = doc_path(file.as_deref().unwrap_or(HANDOFF), &wd)?;
                Observed::Exists(inside(&wd, &d).is_some())
            }
            XapCmd::CommitDone { sha, .. } => {
                let wd = self.workdir()?;
                let git = &self.env.git;
                let full = git.rev_parse(&wd, sha).await.ok();
                let (on_branch, files) = match &full {
                    Some(f) => (
                        git.is_ancestor(&wd, f, "HEAD").await,
                        git.commit_files(&wd, f).await.unwrap_or_default(),
                    ),
                    None => (false, Vec::new()),
                };
                Observed::Commit(CommitFact {
                    sha: full,
                    on_branch,
                    files,
                })
            }
            _ => Observed::Nothing,
        })
    }

    /// `verify:` must be one shell command: `sh -n` parses it without running
    /// it. Prose with a stray backtick or quote does not parse.
    async fn verify_syntax(&self, task: &str, command: &str) -> Result<(), Refusal> {
        let out = xherd_core::exec::run(
            xherd_core::exec::Cmd::new(["sh", "-n", "-c", command])
                .timeout(std::time::Duration::from_secs(5)),
        )
        .await;
        match out {
            Ok(o) if o.success() => Ok(()),
            Ok(o) => Err(refusal(
                "verify_invalid",
                format!(
                    "{task}: `verify:` is not one shell command ({}); write a single runnable \
                     command, or none and keep the notes in `contract`",
                    o.stderr.lines().next().unwrap_or("syntax error").trim()
                ),
            )),
            // No shell to check with: the run decides.
            Err(_) => Ok(()),
        }
    }

    /// Run a task's `verify:` command in the worktree (R-118). A deliberate
    /// inline exception to "slow work returns as `WorkDone`", like
    /// `commit_for_user`: the agent waits for the answer and the actor holds
    /// no other work meanwhile. `sh -c` with the agent user's allowlisted env.
    async fn run_verify(&self, command: String) -> Result<VerifyFact, Refusal> {
        let wd = self.workdir()?;
        // With the job's resolved toolchain, run through the same PATH the
        // agent's shells get (#180). `./`: dash's `.` searches PATH for bare names.
        let script = if wd.join(xherd_plugin::toolchain::ENV_FILE).is_file() {
            format!(". ./{}\n{command}", xherd_plugin::toolchain::ENV_FILE)
        } else {
            command.clone()
        };
        let out = xherd_core::exec::run(
            xherd_core::exec::Cmd::new(["sh", "-c", script.as_str()])
                .cwd(wd)
                .timeout(self.env.settings.get().verify_timeout)
                // As the user the agent is (R-1203): it can run the same.
                .as_agent(),
        )
        .await
        .map_err(|e| refusal("verify_failed", format!("verify `{command}`: {e}")))?;
        let mut text = out.stdout_str();
        text.push_str(&out.stderr);
        let lines: Vec<&str> = text.lines().collect();
        Ok(VerifyFact {
            command,
            code: out.code,
            timed_out: out.timed_out,
            tail: lines[lines.len().saturating_sub(20)..].join("\n"),
        })
    }

    /// Hash of a gate document as approvals see it (task progress in the
    /// tasks document does not count, R-107).
    fn doc_sha256(&self, wd: &Path, path: &str, bytes: &[u8]) -> String {
        let flow = self
            .job
            .pack
            .as_deref()
            .and_then(|p| self.env.packs.get(p))
            .and_then(|p| p.flow(self.job.flow.as_deref().unwrap_or_default()));
        xherd_flow::approve::doc_sha256(flow, wd, path, bytes)
    }

    async fn doc_fact(&self, wd: &Path, path: String) -> DocFact {
        let rel = xherd_flow::xap::worktree_rel(&path);
        let state = match (wd.join(&rel).canonicalize(), wd.canonicalize()) {
            (Ok(abs), Ok(root)) if !abs.starts_with(&root) => DocState::Outside,
            (Ok(abs), Ok(_)) if abs.is_file() => match std::fs::read(&abs) {
                Ok(bytes) => {
                    let sha256 = self.doc_sha256(wd, &path, &bytes);
                    let text = String::from_utf8_lossy(&bytes);
                    let md = path.ends_with(".md");
                    let frontmatter_error = md
                        .then(|| xherd_flow::frontmatter::parse(&text).err())
                        .flatten();
                    let questions = if md {
                        xherd_flow::questions::parse(&text)
                    } else {
                        Vec::new()
                    };
                    let approved = self
                        .env
                        .store
                        .approved(&self.job.id, &path, &sha256)
                        .await
                        .unwrap_or(false);
                    DocState::Present {
                        sha256,
                        frontmatter_error,
                        approved,
                        questions,
                    }
                }
                Err(_) => DocState::Missing,
            },
            _ => DocState::Missing,
        };
        DocFact { path, state }
    }

    /// `.xherd/ledger/issues.jsonl` (R-131).
    fn append_issue(&self, call: &XapCall) -> std::io::Result<()> {
        use std::io::Write;
        let XapCmd::IssueAdd {
            title,
            severity,
            about,
            body,
        } = &call.cmd
        else {
            return Ok(());
        };
        let dir = self
            .workdir()
            .map_err(|e| std::io::Error::other(e.message))?
            .join(".xherd/ledger");
        std::fs::create_dir_all(&dir)?;
        let mut line = serde_json::to_vec(&json!({
            "id": call.id, "title": title, "severity": severity,
            "about": about, "body": body, "at": call.at,
        }))?;
        line.push(b'\n');
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("issues.jsonl"))?
            .write_all(&line)
    }

    // ---- user actions --------------------------------------------------------

    async fn on_user(&mut self, action: UserAction) -> Result<(), ActionError> {
        let before = self.job.status;
        let now = self.now();
        let mut payload = None;
        let kind = match &action {
            UserAction::Get => return Ok(()),
            UserAction::Start { port_base } => {
                self.start(*port_base).await?;
                return Ok(());
            }
            UserAction::Pause => {
                if next_status(self.job.status, Trigger::Pause).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.job.name, self.job.status),
                    ));
                }
                if self.begin_pause().await {
                    "user.pause-requested"
                } else {
                    "user.pause"
                }
            }
            UserAction::PauseNow => {
                if next_status(self.job.status, Trigger::Pause).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.job.name, self.job.status),
                    ));
                }
                if self.job.status.is_live() {
                    self.end_session().await;
                }
                self.job.pause_requested_at = None;
                self.job.fire(Trigger::Pause, now);
                "user.pause-all"
            }
            UserAction::Resume => {
                if self.job.worktree_removed_at.is_some() {
                    return Err(refuse(
                        "worktree_removed",
                        format!("{}'s worktree was removed", self.job.name),
                    ));
                }
                if self.job.fire(Trigger::Resume, now).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.job.name, self.job.status),
                    ));
                }
                self.prepare_resume(true).await;
                "user.resume"
            }
            UserAction::Cancel { remove_worktree } => {
                if next_status(self.job.status, Trigger::Cancel).is_some() {
                    if self.job.status.is_live() {
                        self.end_session().await;
                    }
                    self.job.pause_requested_at = None;
                    self.job.fire(Trigger::Cancel, now);
                } else {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.job.name, self.job.status),
                    ));
                }
                if *remove_worktree {
                    // The cancel stands whatever the removal does.
                    self.log(None, "user.cancel", None).await;
                    self.commit(before).await;
                    let before = self.job.status;
                    if let Err(e) = self.remove_worktree().await {
                        self.log(
                            None,
                            "worktree-remove-failed",
                            Some(json!({"error": e.message})),
                        )
                        .await;
                        self.notify(
                            "error",
                            format!(
                                "{} was cancelled, but its worktree could not be removed: {}",
                                self.job.name, e.message
                            ),
                        );
                    }
                    self.commit(before).await;
                    return Ok(());
                }
                "user.cancel"
            }
            UserAction::RemoveWorktree => {
                if !matches!(self.job.status, Status::Done | Status::Cancelled) {
                    return Err(refuse(
                        "not_allowed",
                        format!(
                            "{} is {}: only a done or cancelled agent's worktree can be removed",
                            self.job.name, self.job.status
                        ),
                    ));
                }
                if self.job.worktree_removed_at.is_some() {
                    return Ok(());
                }
                self.remove_worktree().await?;
                "user.remove-worktree"
            }
            UserAction::Send { text } => {
                if self.job.status == Status::Parked {
                    self.unpark_with(text.clone()).await;
                } else {
                    self.send(text, 1).await?;
                }
                self.keep_note(text).await;
                // A message while a question is open is its answer (xap.md `ask`).
                if matches!(self.job.gate, Some(Gate::Question { .. })) {
                    self.job.gate = None;
                    self.job.generation += 1;
                    // No-op for a parked agent (already queued).
                    self.job.fire(Trigger::UserInput, self.now());
                    "user.answer"
                } else {
                    "user.send"
                }
            }
            UserAction::Ask { text } => {
                self.ask(text).await?;
                self.keep_note(text).await;
                payload = Some(json!({"chars": text.trim().chars().count()}));
                "user.ask"
            }
            UserAction::Finish => {
                let Some(hold) = self.job.ask_hold.take() else {
                    return Err(refuse(
                        "not_asked",
                        format!("{} has no Ask session to finish", self.job.name),
                    ));
                };
                if self.job.status.is_live() {
                    self.end_session().await;
                }
                self.job.pause_requested_at = None;
                // Still queued, the Ask text was never delivered: drop it so
                // a later Resume types the pack's `resume` command and a new
                // Ask is not refused `prompt_pending`.
                self.job.next_prompt = None;
                let trigger = if hold.was_watching && self.job.pr_url.is_some() {
                    Trigger::Finish
                } else {
                    Trigger::Pause
                };
                self.job.fire(trigger, now);
                "user.finish"
            }
            UserAction::Approve {
                reason,
                expect,
                message,
            } => {
                self.approve(reason.clone(), expect, message.clone(), false)
                    .await?;
                // The user moved the flow on: an Ask session ends (R8).
                self.job.ask_hold = None;
                "user.approve"
            }
            UserAction::RequestChanges { comment } => {
                self.request_changes(comment).await?;
                self.keep_note(comment).await;
                self.job.ask_hold = None;
                "user.request-changes"
            }
            UserAction::SetAutonomy { level } => {
                self.set_autonomy(*level).await?;
                "user.autonomy"
            }
            UserAction::MemoryAccept { text } => {
                self.memory_accept(text.clone()).await?;
                "user.memory-accept"
            }
            UserAction::MemoryDismiss => {
                if self.job.memory_proposal.take().is_none() {
                    return Ok(());
                }
                // A starter AGENTS.md declined: no more proposals for the repo.
                if let (Ok(wd), Some(repo)) = (self.workdir(), self.job.repo.clone())
                    && !wd.join("AGENTS.md").exists()
                {
                    let _ = self.env.store.put_memory_declined(&repo, "AGENTS.md").await;
                }
                "user.memory-dismiss"
            }
            UserAction::AgentMemoryClear => {
                if self.job.agent_memory_proposal.take().is_none() {
                    return Ok(());
                }
                "user.agent-memory-clear"
            }
            UserAction::MarkDone => {
                if self.job.flow.is_some() {
                    return Err(refuse(
                        "not_allowed",
                        format!(
                            "{} has a workflow: it is done when its PR is merged",
                            self.job.name
                        ),
                    ));
                }
                if next_status(self.job.status, Trigger::MarkDone).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!(
                            "{} is {}: only an idle, waiting, paused or parked job can be marked done",
                            self.job.name, self.job.status
                        ),
                    ));
                }
                if self.job.status.is_live() {
                    self.end_session().await;
                }
                self.job.pause_requested_at = None;
                self.job.fire(Trigger::MarkDone, now);
                "user.mark-done"
            }
            UserAction::AttachAgent { snapshot } => {
                if !self.job.agents.is_empty() {
                    return Err(refuse(
                        "has_agent",
                        format!("{} already belongs to an agent", self.job.name),
                    ));
                }
                payload = Some(json!({"agent": snapshot.id, "version": snapshot.version}));
                self.job.set_agent((**snapshot).clone());
                self.job.pinned = true;
                // The supervisor counts a running job against its agent's
                // `maxLiveJobs` from now on, not only from its next spawn.
                let _ = self.env.sup.send(SupEvent::AgentChanged {
                    id: self.job.id.clone(),
                    agent: self.job.current_agent.clone(),
                });
                self.job.suggestion = None;
                self.job.agent_draft = None;
                "user.attach-agent"
            }
            UserAction::SuggestionDismiss => {
                match self.job.suggestion.as_mut() {
                    Some(s) if !s.dismissed => s.dismissed = true,
                    _ => return Ok(()),
                }
                "user.suggestion-dismiss"
            }
            UserAction::CheckPr => {
                self.check_pr()?;
                "user.pr-check"
            }
            UserAction::ResolveConflicts => {
                self.resolve_conflicts().await?;
                "user.resolve-conflicts"
            }
            UserAction::ShipAgain => {
                self.ship_again().await?;
                "user.ship-again"
            }
            UserAction::FinishLocal => {
                self.require_stopped_without_origin()?;
                if self.job.status.is_live() {
                    self.end_session().await;
                }
                self.job.pause_requested_at = None;
                self.job.fire(Trigger::MarkDone, now);
                "user.finish-local"
            }
            UserAction::ActionRun {
                id,
                approve,
                port_base,
            } => {
                self.run_action(id, *approve, *port_base).await?;
                return Ok(());
            }
            UserAction::ActionStop { id } => {
                self.require_running_action(id)?;
                self.begin_stop(id, None).await;
                return Ok(());
            }
            UserAction::ActionRestart { id, approve } => {
                self.require_running_action(id)?;
                if self.stopping.contains(id) {
                    return Err(refuse("action_stopping", format!("`{id}` is stopping")));
                }
                self.begin_stop(id, Some(*approve)).await;
                return Ok(());
            }
            UserAction::ActionRemove { id } => {
                let gone = xherd_core::action::remove(&mut self.job, id)?;
                if matches!(&gone.last, Some(r) if r.state == xherd_core::action::ActionState::Running)
                {
                    let session = xherd_core::action::session_name(&self.job.name, id);
                    if let Err(e) = self.env.driver.stop_session(&session).await {
                        tracing::warn!(job = %self.job.name, %session, "{}", e);
                    }
                }
                self.stopping.remove(id);
                let _ = std::fs::remove_file(self.env.paths.action_log(&self.job.id, id));
                self.log(None, "action-removed", Some(json!({"id": id})))
                    .await;
                self.commit(before).await;
                return Ok(());
            }
            UserAction::ActionApprove { id, hash, profile } => {
                if !self.job.actions.iter().any(|a| a.def.id == *id) {
                    return Err(refuse("action_unknown", format!("no action `{id}`")));
                }
                if !xherd_core::action::approve(&mut self.job, id, hash) {
                    return Err(refuse(
                        "action_changed",
                        format!("the command of `{id}` changed since it was shown"),
                    ));
                }
                self.log(
                    None,
                    "action-saved",
                    Some(json!({"id": id, "profile": profile})),
                )
                .await;
                self.commit(before).await;
                return Ok(());
            }
            UserAction::ActionsCardDismiss => {
                if self.job.actions_card_dismissed {
                    return Ok(());
                }
                self.job.actions_card_dismissed = true;
                "user.actions-card-dismiss"
            }
            UserAction::SetMode { mode, launch } => {
                if self.job.permission_mode == Some(*mode) {
                    return Ok(());
                }
                if let Some(p) = self.profile().await
                    && let Some(cap) = p.permission_mode
                    && *mode > cap
                {
                    return Err(above_profile(
                        "tool permission mode",
                        &mode.to_string(),
                        &cap.to_string(),
                        &p.name,
                    ));
                }
                self.job.lowered.retain(|l| l.field != "permissionMode");
                self.job.permission_mode = Some(*mode);
                self.job.pinned = true;
                self.job.launch = Some(launch.clone());
                "user.mode"
            }
            UserAction::SetModel { model, launch } => {
                if self.job.model == *model {
                    return Ok(());
                }
                self.job.model = model.clone();
                self.job.pinned = true;
                self.job.launch = Some(launch.clone());
                "user.model"
            }
        };
        self.log(None, kind, payload).await;
        self.commit(before).await;
        Ok(())
    }

    /// Ask (#121, R5–R7): resume an agent that has no session with the
    /// user's text (through the pack's `ask` command) as its first prompt, continuing the last conversation, and
    /// hold the session open (no auto-approval, no move to watching) until the
    /// agent does flow work, the user decides a gate, or finishes it.
    async fn ask(&mut self, text: &str) -> Result<(), ActionError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(refuse("usage", "say what to ask"));
        }
        if self.job.worktree_removed_at.is_some() {
            return Err(refuse(
                "worktree_removed",
                format!("{}'s worktree was removed", self.job.name),
            ));
        }
        let status = self.job.status;
        if !matches!(
            status,
            Status::Paused | Status::Parked | Status::Watching | Status::Error
        ) {
            return Err(refuse(
                "not_allowed",
                format!(
                    "{} is {status}: Ask needs an agent with no session",
                    self.job.name
                ),
            ));
        }
        // A command waits (after an approval, say): Resume delivers it; Ask
        // would replace it.
        if self.job.next_prompt.is_some() {
            return Err(refuse(
                "prompt_pending",
                format!("{} has a command waiting: resume it first", self.job.name),
            ));
        }
        // R1–R3 (missing commits): through the pack's `ask` command, which
        // turns a requested change into tasks and commit proposals; the bare
        // text when the job has no pack or its pack has no such command.
        let prompt = self
            .pack_command_with("ask", &[("prompt", text)])
            .unwrap_or_else(|| text.to_string());
        let now = self.now();
        self.job.next_prompt = Some(prompt);
        self.job.fire(Trigger::Resume, now);
        self.prepare_resume(false).await;
        self.job.ask_hold = Some(xherd_core::job::AskHold {
            at: now,
            was_watching: status == Status::Watching,
        });
        Ok(())
    }

    /// Resolve conflicts (#139, R14): a clean session with the pack's
    /// `conflicts` command, which merges the PR's base into the branch. The
    /// base goes through `pr::base_ref`, since it is typed into the terminal.
    async fn resolve_conflicts(&mut self) -> Result<(), ActionError> {
        let name = self.job.name.clone();
        if self.job.pr_url.is_none() {
            return Err(refuse(
                "no_pr",
                format!("{name} has no pull request to update"),
            ));
        }
        if self.job.worktree_removed_at.is_some() {
            return Err(refuse(
                "worktree_removed",
                format!("{name}'s worktree was removed"),
            ));
        }
        let no_command = || {
            refuse(
                "not_allowed",
                format!("{name}'s flow has no command to resolve conflicts"),
            )
        };
        if !self
            .job
            .pack
            .as_deref()
            .and_then(|p| self.env.packs.get(p))
            .is_some_and(|p| p.commands.contains_key("conflicts"))
        {
            return Err(no_command());
        }
        let status = self.job.status;
        if next_status(status, Trigger::ResolveConflicts).is_none() {
            return Err(refuse(
                "not_allowed",
                format!(
                    "{name} is {status}: resolving conflicts needs an agent with no turn under way"
                ),
            ));
        }
        if self.job.gate.is_some() {
            return Err(refuse(
                "not_allowed",
                format!("{name} waits at a gate: decide the open gate first"),
            ));
        }
        if self.job.next_prompt.is_some() {
            return Err(refuse(
                "prompt_pending",
                format!("{name} has a command waiting: resume it first"),
            ));
        }
        let Some(base) =
            xherd_core::pr::base_ref(self.job.pr_base.as_deref(), self.job.base.as_deref())
        else {
            return Err(refuse(
                "not_allowed",
                format!("the base branch of {name}'s pull request is unknown"),
            ));
        };
        let Some(command) = self.pack_command_with("conflicts", &[("base", &base)]) else {
            return Err(no_command());
        };
        if status == Status::Idle {
            self.end_session().await;
        }
        self.job.pause_requested_at = None;
        self.job.next_prompt = Some(command);
        // A clean session: the merge needs no earlier conversation.
        self.resume_session = None;
        self.job.ask_hold = None;
        self.job.fire(Trigger::ResolveConflicts, self.now());
        Ok(())
    }

    /// R9, R10: only a job that shipped without a remote and stopped can be
    /// shipped again or finished locally.
    fn require_stopped_without_origin(&self) -> Result<(), ActionError> {
        if stopped_without_origin(&self.job) {
            return Ok(());
        }
        Err(refuse(
            "not_stopped",
            format!(
                "{} is not stopped after a ship without a remote",
                self.job.name
            ),
        ))
    }

    /// Ship again (R9): a clean session with the ship stage's command.
    async fn ship_again(&mut self) -> Result<(), ActionError> {
        self.require_stopped_without_origin()?;
        let name = self.job.name.clone();
        if self.job.worktree_removed_at.is_some() {
            return Err(refuse(
                "worktree_removed",
                format!("{name}'s worktree was removed"),
            ));
        }
        let pack = self.job.pack.as_deref().and_then(|p| self.env.packs.get(p));
        let template = pack
            .zip(self.job.flow.as_deref())
            .and_then(|(p, f)| p.flow(f))
            .and_then(|f| {
                f.stages
                    .iter()
                    .find(|s| s.role == xherd_core::domain::Role::Ship)
            })
            .and_then(|s| s.command.clone());
        let Some(command) = template.and_then(|t| {
            pack?
                .command(self.runtime_name(), &t, &[])
                .map_err(|e| tracing::warn!(agent = %name, error = %e, "no ship command"))
                .ok()
        }) else {
            return Err(refuse(
                "not_allowed",
                format!("{name}'s flow has no ship command"),
            ));
        };
        if self.job.status == Status::Idle {
            self.end_session().await;
        }
        self.job.pause_requested_at = None;
        self.job.next_prompt = Some(command);
        self.resume_session = None;
        self.job.ask_hold = None;
        self.job.no_origin = false;
        self.job.fire(Trigger::ShipAgain, self.now());
        Ok(())
    }

    /// Remove the agent's worktree (R14): `git worktree remove --force` as the
    /// agents' user, or `git worktree prune` when the folder is already gone.
    /// The branch, `workdir` and the record stay. Inline: the user waits.
    async fn remove_worktree(&mut self) -> Result<(), ActionError> {
        let (Some(wd), Some(repo)) = (self.job.workdir.clone(), self.job.repo.clone()) else {
            return Err(refuse(
                "no_workspace",
                format!("{} has no worktree", self.job.name),
            ));
        };
        let (wd, repo) = (PathBuf::from(wd), PathBuf::from(repo));
        // R12: nothing runs in a worktree that goes away.
        for id in self.running_action_ids() {
            self.stopping.insert(id.clone());
            let session = xherd_core::action::session_name(&self.job.name, &id);
            let (exit, output) = stop_action_session(self.env.driver.as_ref(), &session).await;
            self.action_ended(&id, exit, &output, true, None).await;
        }
        let res = if wd.exists() {
            self.env.git.worktree_remove(&repo, &wd, true).await
        } else {
            self.env
                .git
                .run(&repo, &["worktree", "prune"])
                .await
                .map(drop)
        };
        res.map_err(|e| refuse("git", e.to_string()))?;
        self.job.worktree_removed_at = Some(self.now());
        self.log(
            None,
            "worktree-removed",
            Some(json!({"path": wd.to_string_lossy()})),
        )
        .await;
        Ok(())
    }

    async fn start(&mut self, port_base: u16) -> Result<(), ActionError> {
        let before = self.job.status;
        let now = self.now();
        if self.job.fire(Trigger::SlotFree, now).is_none() {
            return Err(refuse(
                "not_allowed",
                format!("{} is {}", self.job.name, self.job.status),
            ));
        }
        self.job.port_base = Some(port_base);
        let res = self.launch(port_base).await;
        if let Err(e) = &res {
            self.revoke_tokens().await;
            self.job.fire(Trigger::SessionLost, self.now());
            self.log(None, "start-failed", Some(json!({"error": e.message})))
                .await;
            self.notify(
                "error",
                format!("{} could not start: {}", self.job.name, e.message),
            );
        } else {
            self.log(
                None,
                "session-started",
                Some(json!({"portBase": port_base})),
            )
            .await;
        }
        self.commit(before).await;
        res
    }

    async fn launch(&mut self, port_base: u16) -> Result<(), ActionError> {
        // A session that starts on a stage other than its agent's (the job
        // moved on in a running session) starts on the stage's agent.
        if let Some(stage) = self.job.stage.clone() {
            self.switch_agent(&stage).await;
        }
        let argv = self
            .job
            .launch
            .clone()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| refuse("no_launch", "no runtime command to start"))?;
        let resume = self.runtime_kit().and_then(|k| k.resume.clone());
        let argv = match (self.resume_session.take(), &resume) {
            (Some(id), Some(r)) => match r.argv(&id, &xherd_core::runner::agent_env).await {
                Some(extra) => {
                    self.resume_unconfirmed = true;
                    // A continued session takes its prompt from `promptEnv`
                    // or the resume `promptArgs` when the runtime has them,
                    // else it is typed.
                    let prompt = self.job.next_prompt.as_deref();
                    let env = prompt.and_then(|p| r.0.prompt_env_argv(&id, p));
                    let args = prompt.and_then(|p| r.0.prompt_argv(p));
                    self.prompt_on_launch = env.is_some() || args.is_some();
                    env.unwrap_or_default()
                        .into_iter()
                        .chain(argv)
                        .chain(extra)
                        .chain(args.unwrap_or_default())
                        .collect()
                }
                None => self.with_launch_prompt(argv),
            },
            _ => self.with_launch_prompt(argv),
        };
        self.revoke_tokens().await;
        let token = mint_token(
            &self.env.store,
            TOKEN_JOB,
            Some(&self.job.id),
            &[SCOPE_HOOKS, SCOPE_XAP, SCOPE_CREDENTIAL],
            self.now(),
        )
        .await
        .map_err(|e| refuse("store", e.to_string()))?;
        let mut ctx = self.ctx();
        ctx.env = self.session_env(port_base, Some(token));
        self.env
            .driver
            .start(&ctx, &self.session_argv(argv).await)
            .await
            .map_err(|e| refuse("driver", e.0))?;
        self.job.session_since = Some(self.now());
        Ok(())
    }

    fn action_runs(&self, id: &str) -> bool {
        use xherd_core::action::ActionState;
        self.job.actions.iter().any(|a| {
            a.def.id == id && matches!(&a.last, Some(r) if r.state == ActionState::Running)
        })
    }

    /// Run an action for the user (R9–R11, R14, R17): its own tmux session in
    /// the action's folder, with the job's environment but no token (R10).
    async fn run_action(
        &mut self,
        id: &str,
        approve: bool,
        port_base: Option<u16>,
    ) -> Result<(), ActionError> {
        use xherd_core::action;
        if !self.env.settings.get().job_actions {
            return Err(refuse("actions_off", "job actions are switched off"));
        }
        let before = self.job.status;
        let plan = action::may_run(&self.job, id, approve)?;
        let wd = self.workdir()?;
        cwd_inside(&wd, &plan.def.cwd)?;
        let cwd = wd.join(&plan.def.cwd);
        if !cwd.is_dir() {
            return Err(refuse(
                "cwd_missing",
                format!("the folder `{}` does not exist", plan.def.cwd),
            ));
        }
        let base = self
            .job
            .port_base
            .or(port_base)
            .ok_or_else(|| refuse("no_ports", "the job holds no port block"))?;
        let session = action::session_name(&self.job.name, id);
        // A dead pane left by an earlier run that was never captured.
        if let Err(e) = self.env.driver.stop_session(&session).await {
            tracing::warn!(job = %self.job.name, %session, "{}", e);
        }
        let mut ctx = self.ctx();
        ctx.workdir = cwd;
        ctx.env = self.session_env(base, None);
        let argv = self
            .session_argv(vec!["sh".into(), "-lc".into(), plan.def.run.clone()])
            .await;
        self.env
            .driver
            .start_action(&ctx, &session, &argv, ACTION_HISTORY_LINES)
            .await
            .map_err(|e| refuse("driver", e.0))?;
        self.job.port_base = Some(base);
        let now = self.now();
        action::on_started(&mut self.job, id, now, plan.store_approval.clone());
        if plan.store_approval.is_some() {
            self.log(None, "action-approved", Some(json!({"id": id})))
                .await;
        }
        self.log(
            None,
            "action-run",
            Some(json!({"id": id, "run": plan.def.run, "cwd": plan.def.cwd})),
        )
        .await;
        self.commit(before).await;
        Ok(())
    }

    /// The sessions of this job's actions that its state does not list as
    /// running (a removed action, a stale run) are killed at start. A session
    /// whose id part holds `-act-` may belong to a job named like that, so it
    /// is left alone (R12).
    fn kill_unknown_actions(&self, alive: &[String]) {
        let known: BTreeSet<String> = self
            .job
            .actions
            .iter()
            .filter(|a| matches!(&a.last, Some(r) if r.state == xherd_core::action::ActionState::Running))
            .map(|a| xherd_core::action::session_name(&self.job.name, &a.def.id))
            .collect();
        let prefix = xherd_core::action::session_name(&self.job.name, "");
        for session in alive {
            let Some(id) = session.strip_prefix(&prefix) else {
                continue;
            };
            if id.is_empty() || id.contains("-act-") || known.contains(session) {
                continue;
            }
            let (driver, session) = (self.env.driver.clone(), session.clone());
            tokio::spawn(async move {
                if let Err(e) = driver.stop_session(&session).await {
                    tracing::warn!(session, error = %e, "unknown action session not killed");
                }
            });
        }
    }

    /// The tick of the running actions: a dead pane, a living one, or a session
    /// that is gone, each answered as a `WorkResult` (R11, R13).
    fn check_actions(&self) {
        use xherd_core::action::ActionState;
        let running = self
            .job
            .actions
            .iter()
            .filter(|a| matches!(&a.last, Some(r) if r.state == ActionState::Running))
            .filter(|a| !self.stopping.contains(&a.def.id));
        for a in running {
            let id = a.def.id.clone();
            let session = xherd_core::action::session_name(&self.job.name, &id);
            let (driver, tx, generation) = (
                self.env.driver.clone(),
                self.tx.clone(),
                self.job.generation,
            );
            tokio::spawn(async move {
                let result = match driver.action_status(&session).await {
                    Ok(Some(st)) if st.dead => WorkResult::ActionEnded {
                        id,
                        exit: st.exit,
                        output: driver
                            .action_history(&session, crate::actions::MAX_LOG_BYTES)
                            .await
                            .unwrap_or_default(),
                        stopped: false,
                        restart: None,
                    },
                    Ok(Some(_)) => WorkResult::ActionAlive { id },
                    Ok(None) => WorkResult::ActionGone { id },
                    Err(_) => return,
                };
                let _ = tx.send(Msg::WorkDone { generation, result });
            });
        }
    }

    /// An action's command ended by itself: keep its output (R13), record the
    /// exit (R11, R17) and free the dead session. A second report of the same
    /// end (two ticks in flight) is ignored.
    async fn action_ended(
        &mut self,
        id: &str,
        exit: Option<i32>,
        output: &str,
        stopped: bool,
        restart: Option<bool>,
    ) {
        use xherd_core::action;
        self.stopping.remove(id);
        if !self.action_runs(id) {
            return;
        }
        let before = self.job.status;
        if let Err(e) = crate::actions::write_log(&self.env.paths, &self.job.id, id, output) {
            tracing::warn!(job = %self.job.name, action = id, error = %e, "action log not written");
        }
        let session = action::session_name(&self.job.name, id);
        if let Err(e) = self.env.driver.stop_session(&session).await {
            tracing::warn!(job = %self.job.name, %session, "{}", e);
        }
        let now = self.now();
        match exit {
            Some(code) if !stopped => action::on_exited(&mut self.job, id, now, code),
            _ => action::on_stopped(&mut self.job, id, now, exit),
        }
        self.log(None, "action-exited", Some(json!({"id": id, "code": exit})))
            .await;
        self.commit(before).await;
        if let Some(approve) = restart
            && let Err(e) = self.run_action(id, approve, None).await
        {
            self.log(
                None,
                "action-restart-failed",
                Some(json!({"id": id, "error": e.message})),
            )
            .await;
        }
    }

    fn running_action_ids(&self) -> Vec<String> {
        self.job
            .actions
            .iter()
            .filter(|a| self.action_runs(&a.def.id))
            .map(|a| a.def.id.clone())
            .collect()
    }

    /// Stop and Restart apply to a running action only.
    fn require_running_action(&self, id: &str) -> Result<(), ActionError> {
        if !self.job.actions.iter().any(|a| a.def.id == id) {
            return Err(refuse("action_unknown", format!("no action `{id}`")));
        }
        if !self.action_runs(id) {
            return Err(refuse("action_not_running", format!("`{id}` does not run")));
        }
        Ok(())
    }

    /// Start stopping a running action (R11): Ctrl-C now, the rest (wait,
    /// capture, kill) in the background; the result arrives as `ActionEnded`.
    /// Nothing happens when a stop is already under way.
    async fn begin_stop(&mut self, id: &str, restart: Option<bool>) {
        if !self.stopping.insert(id.to_string()) {
            return;
        }
        self.log(None, "action-stop", Some(json!({"id": id}))).await;
        let session = xherd_core::action::session_name(&self.job.name, id);
        let (driver, tx, generation) = (
            self.env.driver.clone(),
            self.tx.clone(),
            self.job.generation,
        );
        let id = id.to_string();
        tokio::spawn(async move {
            let (exit, output) = stop_action_session(driver.as_ref(), &session).await;
            let result = WorkResult::ActionEnded {
                id,
                exit,
                output,
                stopped: true,
                restart,
            };
            let _ = tx.send(Msg::WorkDone { generation, result });
        });
    }

    /// The environment of a session of this job: `XHERD_*` (the job token only
    /// when given) and the agents' `PATH` with xherd's directory in front.
    fn session_env(&self, port_base: u16, token: Option<String>) -> Vec<(String, String)> {
        let s = self.env.settings.get();
        let mut env = vec![
            ("XHERD_ENV".into(), "1".into()),
            ("XHERD_BIN".into(), s.xherd_bin.clone()),
            ("XHERD_URL".into(), s.xherd_url.clone()),
            ("XHERD_JOB_ID".into(), self.job.id.clone()),
        ];
        if let Some(token) = token {
            env.push(("XHERD_JOB_TOKEN".into(), token));
        }
        env.push(("XHERD_PORT_BASE".into(), port_base.to_string()));
        // The prompts call plain `xherd …`: a command with a variable in it
        // cannot be checked by the runtime and asks every time.
        if let Some(path) = agent_path(&s.xherd_bin) {
            env.push(("PATH".into(), path));
        }
        env
    }

    /// `argv` with the pending `next_prompt` on it, for a runtime that takes
    /// its first prompt at launch (`promptArgs`); else `argv` as it is.
    fn with_launch_prompt(&mut self, argv: Vec<String>) -> Vec<String> {
        let args = self
            .runtime_kit()
            .map(|k| k.prompt_args.clone())
            .unwrap_or_default();
        let extra = self
            .job
            .next_prompt
            .as_deref()
            .and_then(|p| xherd_plugin::runtime::Terminal::prompt_argv(&args, p));
        self.prompt_on_launch = extra.is_some();
        match extra {
            Some(extra) => argv.into_iter().chain(extra).collect(),
            None => argv,
        }
    }

    /// The argv of a new session: the runtime in the agents' PATH (R-1203),
    /// wrapped in `xherd toolchain exec` when the worktree's version files ask
    /// for tools (R-652). The plan is written at every start, so version files
    /// edited since count.
    async fn session_argv(&self, launch: Vec<String>) -> Vec<String> {
        let argv = self.toolchain_argv(launch).await;
        let xherd_bin = self.env.settings.get().xherd_bin.clone();
        agent_pane_argv(agent_path(&xherd_bin).as_deref(), argv)
    }

    async fn toolchain_argv(&self, launch: Vec<String>) -> Vec<String> {
        use xherd_plugin::toolchain::{PLAN_FILE, plan};
        let Some(wd) = self.job.workdir.as_deref().map(Path::new) else {
            return launch;
        };
        let settings = self.env.settings.get();
        let off = |n: &str| settings.disabled_plugins.iter().any(|d| d == n);
        if let Some(t) = self.runtime_kit().and_then(|k| k.trust.as_ref())
            && !off(self.runtime_name())
            && let Err(e) = t.ensure(wd).await
        {
            tracing::warn!(agent = %self.job.name, error = %e, "runtime trust not written");
        }
        let on: Vec<_> = self
            .env
            .toolchains
            .iter()
            .filter(|(n, _)| !off(n))
            .cloned()
            .collect();
        let p = plan(
            wd,
            &on,
            settings.toolchain_install,
            &settings.toolchain_defaults,
        );
        let file = wd.join(PLAN_FILE);
        if p.steps.is_empty() {
            let _ = std::fs::remove_file(&file);
            // A stale env file would keep a tool dir on `verify:`'s PATH (R5).
            let _ = std::fs::remove_file(wd.join(xherd_plugin::toolchain::ENV_FILE));
            return launch;
        }
        let json = serde_json::to_vec_pretty(&p).expect("json");
        if let Err(e) = xherd_core::paths::write_atomic(&file, &json, None) {
            tracing::warn!(agent = %self.job.name, error = %e, "toolchain plan not written; starting without it");
            return launch;
        }
        let mut argv = vec![
            self.env.settings.get().xherd_bin.clone(),
            "toolchain".into(),
            "exec".into(),
            "--".into(),
        ];
        argv.extend(launch);
        argv
    }

    async fn send(&mut self, text: &str, enters: u32) -> Result<(), ActionError> {
        match self.job.status {
            Status::Blocked => {
                return Err(refuse(
                    "agent_blocked",
                    "the agent shows a dialog; answer it in the terminal first",
                ));
            }
            s if !s.is_live() => {
                return Err(refuse("not_live", format!("{} is {s}", self.job.name)));
            }
            _ => {}
        }
        self.env
            .driver
            .send(&self.ctx(), text, enters)
            .await
            .map_err(|e| refuse("driver", e.0))?;
        self.next_delivery += 1;
        self.delivery = Some((self.next_delivery, false));
        self.watch_delivery(self.next_delivery);
        Ok(())
    }

    fn watch_delivery(&self, delivery: u64) {
        let (tx, wait, generation) = (
            self.tx.clone(),
            self.env.settings.get().delivery_ack,
            self.job.generation,
        );
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = tx.send(Msg::WorkDone {
                generation,
                result: WorkResult::DeliveryTimeout { delivery },
            });
        });
    }

    // ---- approvals ---------------------------------------------------------------

    /// The open gate, if it is one the user can approve or change, and the
    /// agent stopped for it.
    fn gate_for_user(&self, parked_ok: bool) -> Result<Gate, ActionError> {
        let gate = self
            .job
            .gate
            .clone()
            .ok_or_else(|| refuse("no_gate", format!("{} has no open gate", self.job.name)))?;
        if let Gate::Question { .. } = gate {
            return Err(refuse(
                "no_gate",
                "the agent asked a question; answer it with `xherd send`",
            ));
        }
        match self.job.status {
            Status::WaitingInput | Status::Idle => Ok(gate),
            Status::Parked if parked_ok => Ok(gate),
            s => Err(refuse(
                "not_waiting",
                format!(
                    "{} is {s}; act on the gate once it stops there",
                    self.job.name
                ),
            )),
        }
    }

    /// Autonomy class of the open gate's stage (spec §6.3). `None`: unknown
    /// (flow `none`, a stage the pack does not declare), so a human decides.
    /// Pull-request rounds are always `human-only` (T1), whatever the pack
    /// says.
    fn gate_class(&self, gate: &Gate) -> Option<xherd_core::domain::Class> {
        use xherd_core::domain::{Class, Role};
        let pack = self.env.packs.get(self.job.pack.as_deref()?)?;
        let flow = pack.flow(self.job.flow.as_deref()?)?;
        let stage = match gate {
            Gate::Docs { stage, .. } => flow.stage(stage)?,
            Gate::Commit(_) => flow.commit_stage(self.job.stage.as_deref())?,
            _ => return None,
        };
        // Human-only by the pack (role `pr`). A stage the user marked human
        // comes back with stage assignments (D26).
        Some(if stage.role == Role::Pr {
            Class::HumanOnly
        } else {
            stage.class
        })
    }

    /// What autonomy would do with the open gate at a stop (R-110),
    /// whatever the status: callers check it (`try_auto_approve` at a stop,
    /// the mark in `commit()` while running). Reads the agent, the pack and
    /// the settings snapshot only.
    fn auto_check(&self) -> AutoCheck {
        use xherd_core::autonomy::{may_auto_approve, run_limit};
        if self.job.autonomy == xherd_core::autonomy::Autonomy::Ask || self.job.ask_hold.is_some() {
            return AutoCheck::No;
        }
        let Some(gate) = &self.job.gate else {
            return AutoCheck::No;
        };
        let Some(class) = self.gate_class(gate) else {
            return AutoCheck::No;
        };
        if !may_auto_approve(self.job.autonomy, class) {
            return AutoCheck::No;
        }
        let (label, held) = match gate {
            Gate::Docs {
                stage,
                needs_human,
                questions,
                asked,
                ..
            } => (
                stage.clone(),
                needs_human.clone().or_else(|| {
                    (!questions.is_empty() || !asked.is_empty())
                        .then(|| "the document has open questions".to_string())
                }),
            ),
            Gate::Commit(c) => (format!("commit {}", c.id), c.needs_human.clone()),
            _ => return AutoCheck::No,
        };
        if let Some(held) = held {
            return AutoCheck::Held(label, held);
        }
        if let Some(why) = run_limit(
            self.job.auto_in_row,
            self.env.settings.get().max_auto_approvals_in_row,
        ) {
            return AutoCheck::Limit(label, why);
        }
        AutoCheck::Approve { label, class }
    }

    /// Auto-approval (R-110, R-111): when the agent stops at a gate its class
    /// allows at the agent's autonomy level, approve it unless a tripwire
    /// holds it. The tripwires that look at the work (secrets, sensitive
    /// paths, diff size, scope, `verify:`, review findings) ran when the gate
    /// was opened and left `needs_human`; the run limit is checked here. This
    /// runs inline in the actor on the stop itself, so the gate cannot change
    /// meanwhile and a user message queued behind it is handled afterwards.
    async fn try_auto_approve(&mut self) {
        if !matches!(self.job.status, Status::WaitingInput | Status::Idle) {
            return;
        }
        let level = self.job.autonomy;
        let (label, class) = match self.auto_check() {
            AutoCheck::No => return,
            AutoCheck::Held(label, held) => {
                // Already shown as "Held:"; nothing more to say.
                self.log(None, "auto-held", Some(json!({"gate": label, "why": held})))
                    .await;
                return;
            }
            AutoCheck::Limit(label, why) => {
                // Make the reason visible on the gate, and say so once.
                match &mut self.job.gate {
                    Some(Gate::Docs { needs_human, .. }) => *needs_human = Some(why.clone()),
                    Some(Gate::Commit(c)) => c.needs_human = Some(why.clone()),
                    _ => {}
                }
                if let Some(Gate::Commit(c)) = &self.job.gate
                    && let Some(mine) = self.job.commits.iter_mut().find(|x| x.id == c.id)
                {
                    mine.needs_human = Some(why.clone());
                }
                self.job.generation += 1;
                // The caller's `notify_gate` says so, with `why` as the hold.
                self.log(None, "auto-held", Some(json!({"gate": label, "why": why})))
                    .await;
                let status = self.job.status;
                self.commit(status).await;
                return;
            }
            AutoCheck::Approve { label, class } => (label, class),
        };
        let before = self.job.status;
        let reason =
            format!("autonomy {level}: {label} is a {class:?} gate and no tripwire held it");
        // No notice on success (#135): Needs attention lists it (R-111).
        if let Err(e) = self
            .approve(Some(reason), &BTreeMap::new(), None, true)
            .await
        {
            self.log(
                None,
                "auto-approval-failed",
                Some(json!({"gate": label, "error": e.message})),
            )
            .await;
            self.notify(
                "error",
                format!(
                    "{}: could not approve {label} automatically: {}",
                    self.job.name, e.message
                ),
            );
            // That notice covers this gate: no `gate` notice for it too.
            self.notified_gate = self.job.gate.clone();
        }
        self.commit(before).await;
    }

    /// Change the autonomy level (R-109): rewrite `instructions.md`, and
    /// approve at once a gate the agent is already waiting at that the new
    /// level allows (no further stop would come).
    /// Rewrite `.xherd/instructions.md` from the job as it is now.
    async fn write_instructions(&self) {
        let Ok(wd) = self.workdir() else { return };
        let file = wd.join(".xherd").join("instructions.md");
        let extra = self.profile().await.and_then(|p| p.instructions);
        if let Err(e) = xherd_core::paths::write_atomic(
            &file,
            xherd_core::new::instructions_md(&self.job, extra.as_deref()).as_bytes(),
            None,
        ) {
            tracing::warn!(agent = %self.job.name, %e, "instructions.md not rewritten");
        }
    }

    /// The agent of `stage` takes over when the next session starts (D26,
    /// R39, R40): instructions and memory, skills, launch line, autonomy,
    /// mode and model become its own (the profile's ceilings still apply),
    /// and limits, budgets and the work log follow `current_agent`. A job the
    /// user attached an agent to by hand (`pinned`), a stage with no agent
    /// and the agent already in charge change nothing. Never called for a
    /// running session: only where a new one is about to start.
    async fn switch_agent(&mut self, stage: &str) {
        if self.job.pinned {
            return;
        }
        let Some(to) = self.job.stage_agents.get(stage).map(|s| s.agent.clone()) else {
            return;
        };
        if self.job.current_agent.as_deref() == Some(to.as_str()) {
            return;
        }
        let Some(new) = self.job.agents.get(&to).cloned() else {
            return;
        };
        let from = self.job.current_agent.clone();
        let profile = self.profile().await;
        let target = self
            .runtime_kit()
            .and_then(|k| k.terminal.as_ref())
            .cloned();
        // Skills: those of the agent leaving go, those of the one arriving
        // come (a repository's own folder is left alone).
        if let Some(wd) = self.job.workdir.clone().map(std::path::PathBuf::from) {
            let skills = target.as_ref().and_then(|t| t.skills.as_ref());
            if let Some(old) = from.as_ref().and_then(|f| self.job.agents.get(f)) {
                crate::skills::remove(&wd, skills, &old.skills).await;
            }
            let wanted: Vec<(String, Option<String>)> =
                new.skills.iter().map(|s| (s.name.clone(), None)).collect();
            let rt = self.runtime_name().to_string();
            match crate::skills::install(&self.env.skills, &wd, &rt, skills, &wanted).await {
                Ok(done) => {
                    if let Some(g) = self.job.agents.get_mut(&to) {
                        g.skills = done;
                    }
                }
                Err(e) => {
                    tracing::warn!(job = %self.job.name, error = %e.message, "skills not installed");
                    self.log(
                        None,
                        "skills-failed",
                        Some(json!({"agent": to, "error": e.message})),
                    )
                    .await;
                }
            }
        }
        // Autonomy, mode, model and launch: the agent's baked values under
        // the profile's ceilings as they are now.
        self.job
            .lowered
            .retain(|l| l.field != "autonomy" && l.field != "permissionMode");
        let want_autonomy = new.autonomy.unwrap_or(self.job.autonomy);
        let cap = profile.as_ref().and_then(|p| p.autonomy);
        let (autonomy, low) = xherd_core::resolve::ceiling(want_autonomy, cap);
        if let (true, Some(p)) = (low, &profile) {
            self.job.lowered.push(xherd_core::resolve::Lowered {
                field: "autonomy".into(),
                from: want_autonomy.to_string(),
                to: autonomy.to_string(),
                profile: p.name.clone(),
            });
        }
        if autonomy != self.job.autonomy {
            self.job.auto_in_row = 0;
        }
        self.job.autonomy = autonomy;
        let want_mode = new.permission_mode.or(self.job.permission_mode);
        self.job.model = new.model.clone();
        if let Some(want) = want_mode {
            let cap = profile.as_ref().and_then(|p| p.permission_mode);
            let (mode, low) = xherd_core::resolve::ceiling(want, cap);
            if let (true, Some(p)) = (low, &profile) {
                self.job.lowered.push(xherd_core::resolve::Lowered {
                    field: "permissionMode".into(),
                    from: want.to_string(),
                    to: mode.to_string(),
                    profile: p.name.clone(),
                });
            }
            self.job.permission_mode = Some(mode);
            let rebuilt = match (&target, low) {
                (Some(t), true) => crate::create::mode_launch(
                    t,
                    mode,
                    new.model.as_deref(),
                    self.env.settings.get().toolchain_install, // = running in the container
                )
                .ok(),
                _ => None,
            };
            match rebuilt {
                Some(l) => self.job.launch = Some(l),
                None if !new.launch.is_empty() => self.job.launch = Some(new.launch.clone()),
                None => {}
            }
        } else if !new.launch.is_empty() {
            self.job.launch = Some(new.launch.clone());
        }
        self.job.current_agent = Some(to.clone());
        self.write_instructions().await;
        let _ = self.env.sup.send(SupEvent::AgentChanged {
            id: self.job.id.clone(),
            agent: Some(to.clone()),
        });
        self.log(
            None,
            "agent-switch",
            Some(json!({"stage": stage, "from": from, "to": to})),
        )
        .await;
    }

    /// The stage the approved gate's next command starts, if it starts one.
    fn next_stage_of(&self, wd: &Path, gate: &Gate) -> Option<String> {
        let pack = self
            .job
            .pack
            .as_deref()
            .and_then(|p| self.env.packs.get(p))?;
        let facts = xherd_flow::approve::DirFacts {
            spec_dir: xherd_flow::approve::spec_dir(wd),
            tasks: &self.job.tasks,
        };
        xherd_flow::approve::next_stage(pack, &self.job, gate, &facts, false)
    }

    async fn set_autonomy(
        &mut self,
        level: xherd_core::autonomy::Autonomy,
    ) -> Result<(), ActionError> {
        if !level.selectable() {
            return Err(refuse(
                "autonomy_unavailable",
                "autonomy never is not available yet (v1.x, R-1209)",
            ));
        }
        if self.job.autonomy == level {
            return Ok(());
        }
        if let Some(p) = self.profile().await
            && let Some(cap) = p.autonomy
            && level > cap
        {
            return Err(above_profile(
                "autonomy",
                &level.to_string(),
                &cap.to_string(),
                &p.name,
            ));
        }
        self.job.lowered.retain(|l| l.field != "autonomy");
        self.job.autonomy = level;
        self.job.pinned = true;
        self.job.auto_in_row = 0;
        self.job.generation += 1;
        self.write_instructions().await;
        self.log(None, "autonomy", Some(json!({"level": level})))
            .await;
        let before = self.job.status;
        self.commit(before).await;
        // The agent learns the new level when its next session starts and
        // reads instructions.md; a running session keeps what it was told.
        self.try_auto_approve().await;
        self.notify_gate();
        Ok(())
    }

    /// Approve by content hash (R-104, R-106): ledger file first (fsync),
    /// then the database, then hand on the next command.
    async fn approve(
        &mut self,
        reason: Option<String>,
        expect: &std::collections::BTreeMap<String, String>,
        message: Option<String>,
        auto: bool,
    ) -> Result<(), ActionError> {
        let gate = self.gate_for_user(true)?;
        let wd = self.workdir().map_err(|e| refuse(e.code, e.message))?;
        let (label, files): (String, Vec<String>) = match &gate {
            Gate::Docs { stage, files, .. } => (
                stage.clone(),
                files.iter().map(|f| f.path.clone()).collect(),
            ),
            Gate::Commit(c) => (
                format!("commit {}", c.id),
                c.files.iter().map(|f| format!("./{f}")).collect(),
            ),
            _ => unreachable!("checked in gate_for_user"),
        };
        // Hash what is on disk now: that is what the user is approving.
        let mut approved = Vec::new();
        for f in &files {
            match inside(&wd, f).map(std::fs::read) {
                Some(Ok(bytes)) => approved.push((
                    f.clone(),
                    self.doc_sha256(&wd, f, &bytes),
                    Some(String::from_utf8_lossy(&bytes).into_owned()),
                )),
                // A commit may delete files.
                _ if matches!(gate, Gate::Commit(_)) => {
                    approved.push((f.clone(), "deleted".into(), None))
                }
                _ => {
                    return Err(refuse(
                        "doc_missing",
                        format!("{f} does not exist any more"),
                    ));
                }
            }
        }
        if let Some((f, _, _)) = approved
            .iter()
            .find(|(f, sha, _)| expect.get(f).is_some_and(|e| e != sha))
        {
            return Err(refuse(
                "changed_since_read",
                format!("{f} changed since you read it; read it again before approving"),
            ));
        }
        if let Gate::Docs { stage, .. } = &gate
            && stage == xherd_flow::pr_review::STAGE
        {
            self.publish_pr_decisions(&wd).await?;
        }
        let next = self.next_command(&wd, &gate)?;
        let now = self.now();
        let head = self.env.git.rev_parse(&wd, "HEAD").await.ok();
        let actor = if auto { "auto" } else { "user" };
        let lines: Vec<Value> = approved
            .iter()
            .map(|(file, sha, _)| {
                json!({"gate": label, "file": file, "sha256": sha, "actor": actor, "auto": auto,
                       "reason": reason, "at": now, "commit": head})
            })
            .collect();
        append_ledger(&wd, &lines).map_err(|e| refuse("ledger", e.to_string()))?;
        for (file, sha, text) in &approved {
            let row = xherd_store::Approval {
                job_id: self.job.id.clone(),
                file: file.clone(),
                sha256: sha.clone(),
                at: now,
                actor: actor.into(),
                auto,
                reason: reason.clone(),
            };
            let stored = self.env.store.insert_approval(row).await;
            let reviewed = match text {
                Some(t) if matches!(gate, Gate::Docs { .. }) => {
                    self.env.store.put_reviewed(&self.job.id, file, t).await
                }
                _ => Ok(()),
            };
            stored
                .and(reviewed)
                .map_err(|e| refuse("store", e.to_string()))?;
        }

        if let Gate::Commit(c) = &gate
            && let Some(mine) = self.job.commits.iter_mut().find(|x| x.id == c.id)
        {
            mine.status = CommitStatus::Approved;
        }
        // R9: a ship approved in a repository with no `origin` stops without
        // a push or a PR; the job page then offers a remote or a local finish.
        if let Gate::Docs { stage, .. } = &gate {
            let pack = self.job.pack.as_ref().and_then(|p| self.env.packs.get(p));
            if self.stage_role(pack, stage) == Some(xherd_core::domain::Role::Ship) {
                self.job.no_origin = !self.env.git.has_remote(&wd, "origin").await;
            }
        }
        self.job.gate = None;
        self.job.generation += 1;
        // A human approval restarts the run-limit count (R-120).
        self.job.auto_in_row = if auto { self.job.auto_in_row + 1 } else { 0 };
        self.log(
            None,
            "approval",
            Some(json!({"gate": label, "files": files, "reason": reason, "auto": auto, "next": next.as_ref().map(|n| &n.0)})),
        )
        .await;
        // xherd makes the approved commit itself; the agent only records it
        // (and fixes it when git refuses).
        if let Gate::Commit(c) = &gate {
            self.commit_for_user(&wd, &c.id, message).await;
        }
        // The next session starts on the next stage's agent (D26, R39).
        if (self.fresh_sessions_on() || self.job.status == Status::Parked)
            && next.is_some()
            && let Some(stage) = self.next_stage_of(&wd, &gate)
        {
            self.switch_agent(&stage).await;
        }
        match (self.job.status, next) {
            (Status::Parked, next) => {
                // The next session starts with it once a slot is free (R-028).
                self.job.next_prompt = next.map(|n| n.0);
                self.job.fire(Trigger::Unpark, now);
            }
            (_, Some((cmd, _))) if self.fresh_sessions_on() => {
                match self.fresh_session(&cmd).await {
                    Ok(()) => {
                        self.job.fire(Trigger::UserInput, now);
                    }
                    Err(e) => {
                        self.log(
                            None,
                            "hand-on-failed",
                            Some(json!({"next": cmd, "error": e.message})),
                        )
                        .await;
                        self.notify(
                            "delivery",
                            format!(
                                "{}: approved, but could not start a fresh session for {cmd:?}: {}",
                                self.job.name, e.message
                            ),
                        );
                    }
                }
            }
            // The approval stands even if typing fails; the user is told the command.
            (_, Some((cmd, enters))) => match self.send(&cmd, enters).await {
                Ok(()) => {
                    self.job.fire(Trigger::UserInput, now);
                }
                Err(e) => {
                    self.log(
                        None,
                        "hand-on-failed",
                        Some(json!({"next": cmd, "error": e.message})),
                    )
                    .await;
                    self.notify(
                        "delivery",
                        format!(
                            "{}: approved, but could not send {cmd:?}: {}",
                            self.job.name, e.message
                        ),
                    );
                }
            },
            (_, None) => {}
        }
        Ok(())
    }

    /// Commit the approved proposal with git: exactly its files and its
    /// message (as edited by the user), hooks included. On success the commit
    /// is `done` with its sha; on failure git's output goes to
    /// `.xherd/commit-error.txt` and the agent, which gets `/xherd:commit`
    /// next, fixes it. The approval stands either way.
    /// Accept the shared-memory proposal (spec §16.6): write `AGENTS.md` (and
    /// a `CLAUDE.md` that imports it) and commit both on the agent's branch,
    /// so they ship with its PR. The user's accept is the approval of exactly
    /// these files, as at a commit gate.
    async fn memory_accept(&mut self, edited: Option<String>) -> Result<(), ActionError> {
        let name = self.job.name.clone();
        let Some(p) = self.job.memory_proposal.clone() else {
            return Err(refuse(
                "no_proposal",
                format!("{name} has no memory proposal"),
            ));
        };
        if self.job.status == Status::Running {
            return Err(refuse(
                "not_allowed",
                format!("{name} is working; accept when it stops"),
            ));
        }
        // An open pull request does not block it: the commit goes on the
        // branch, and the user pushes it (xherd never pushes).
        let wd = self.workdir()?;
        let mut text = match edited.filter(|t| !t.trim().is_empty()) {
            Some(t) => t,
            None => {
                let d = xherd_flow::xap::doc_path(&p.file, &wd)?;
                inside(&wd, &d)
                    .and_then(|f| std::fs::read_to_string(f).ok())
                    .ok_or_else(|| refuse("doc_missing", format!("{} is gone", p.file)))?
            }
        };
        if text.len() > xherd_flow::xap::MEMORY_MAX_BYTES {
            return Err(refuse("doc_schema", "AGENTS.md is over 64 KiB"));
        }
        if !text.ends_with('\n') {
            text.push('\n');
        }
        let mut writes = vec![("AGENTS.md", text)];
        let claude = std::fs::read_to_string(wd.join("CLAUDE.md")).ok();
        if let Some(c) = xherd_flow::memory::claude_md(claude.as_deref()) {
            writes.push(("CLAUDE.md", c));
        }
        let mut files = Vec::new();
        for (f, t) in writes {
            xherd_core::runner::write_file(&wd.join(f), t.as_bytes(), None)
                .await
                .map_err(|e| refuse("internal", format!("{f}: {e}")))?;
            files.push(f.to_string());
        }
        let message = format!(
            "{}\n\n{}",
            xherd_core::domain::MEMORY_COMMIT_SUBJECT,
            p.reason
        );
        let git = self.env.git.clone().with_timeout(Duration::from_secs(300));
        let sha = git.commit_paths(&wd, &files, &message).await.map_err(|e| {
            refuse(
                "commit_failed",
                format!("git refused the commit ({}): {e}", files.join(", ")),
            )
        })?;
        self.job.memory_proposal = None;
        // R6: xherd's own commit is listed (`Mnn`), so it is never taken for
        // one made outside xherd.
        let n = self
            .job
            .commits
            .iter()
            .filter_map(|c| c.id.strip_prefix('M')?.parse::<u32>().ok())
            .max()
            .unwrap_or(0);
        self.job.commits.push(Commit {
            id: format!("M{:02}", n + 1),
            subject: xherd_core::domain::MEMORY_COMMIT_SUBJECT.into(),
            message: Some(message.clone()),
            files: files.clone(),
            status: CommitStatus::Done,
            tasks: vec![],
            sha: Some(sha.clone()),
            needs_human: None,
        });
        self.log(
            None,
            "memory-accepted",
            Some(json!({"sha": sha, "files": files})),
        )
        .await;
        Ok(())
    }

    async fn commit_for_user(&mut self, wd: &Path, id: &str, edited: Option<String>) {
        let Some(i) = self.job.commits.iter().position(|c| c.id == id) else {
            return;
        };
        if let Some(m) = edited
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
        {
            self.job.commits[i].message = Some(m);
        }
        let c = self.job.commits[i].clone();
        let message = c.message.clone().unwrap_or_else(|| c.subject.clone());
        // Hooks (lint, tests) can take a while.
        let git = self.env.git.clone().with_timeout(Duration::from_secs(300));
        let error_file = wd.join(".xherd/commit-error.txt");
        match git.commit_paths(wd, &c.files, &message).await {
            Ok(sha) => {
                let _ = std::fs::remove_file(&error_file);
                self.job.commits[i].sha = Some(sha.clone());
                self.job.commits[i].status = CommitStatus::Done;
                self.write_commit_progress(id);
                self.log(None, "commit-made", Some(json!({"id": id, "sha": sha})))
                    .await;
            }
            Err(e) => {
                let text = e.to_string();
                let _ = xherd_core::paths::write_atomic(&error_file, text.as_bytes(), None);
                self.log(
                    None,
                    "commit-failed",
                    Some(json!({"id": id, "error": text})),
                )
                .await;
                self.notify(
                    "error",
                    format!(
                        "{}: git refused commit {id}; the agent will fix it",
                        self.job.name
                    ),
                );
            }
        }
    }

    /// Whether an approval replaces the session (R-023): fresh sessions are
    /// on and the agent follows a flow (R-024; flow `none` has no gates).
    /// Background tasks (also R-024) are not tracked yet.
    fn fresh_sessions_on(&self) -> bool {
        self.env.settings.get().fresh_sessions && self.job.pack.is_some()
    }

    /// Replace the runtime process in the agent window by a new session
    /// (`respawn-window`; other windows keep running, R-023). The command is
    /// typed on the new session's `session-start`, like a first prompt. The
    /// session keeps its environment and agent token.
    async fn fresh_session(&mut self, cmd: &str) -> Result<(), ActionError> {
        let argv = self
            .job
            .launch
            .clone()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| refuse("no_launch", "no runtime command to start"))?;
        self.job.next_prompt = Some(cmd.to_string());
        let argv = self.with_launch_prompt(argv);
        self.env
            .driver
            .respawn(&self.ctx(), &self.session_argv(argv).await)
            .await
            .map_err(|e| refuse("driver", e.0))?;
        self.delivery = None;
        self.job.session = None;
        self.job.session_since = Some(self.now());
        self.log(None, "fresh-session", Some(json!({"next": cmd})))
            .await;
        Ok(())
    }

    /// The pack's `resume` command for the agent's runtime; `None` for flow
    /// `none` or a pack without one.
    /// A resumed agent gets the pack's `resume` command (a pending next prompt
    /// wins, R-027) and, when the runtime still has the last conversation, a
    /// session that continues it. If the last resume of that kind never
    /// reached `session-start`, this one is a clean session instead.
    /// Pause (R-026). A live flow agent is asked for a handoff note with the
    /// pack's `handoff` command and paused at the stop that follows
    /// `handoff written` (or after `pauseTimeoutSec`); anything else, or a
    /// command that cannot be sent, pauses at once. Returns whether it waits.
    async fn begin_pause(&mut self) -> bool {
        let now = self.now();
        if self.job.pause_requested_at.is_some() {
            return true;
        }
        if matches!(
            self.job.status,
            Status::Running | Status::Idle | Status::WaitingInput
        ) && let Some(cmd) = self.pack_command("handoff")
        {
            let enters = self.enters_for(&cmd);
            match self.send(&cmd, enters).await {
                Ok(()) => {
                    self.job.pause_requested_at = Some(now);
                    return true;
                }
                Err(e) => {
                    self.notify(
                        "error",
                        format!(
                            "{}: could not ask for a handoff ({}); paused without one",
                            self.job.name, e.message
                        ),
                    );
                }
            }
        }
        if self.job.status.is_live() {
            self.end_session().await;
        }
        self.job.fire(Trigger::Pause, now);
        false
    }

    /// End the session of a pause that waited for a handoff.
    async fn finish_pause(&mut self, timed_out: bool) {
        let before = self.job.status;
        self.end_session().await;
        self.job.pause_requested_at = None;
        self.job.fire(Trigger::Pause, self.now());
        self.log(
            None,
            "user.pause",
            Some(json!({"handoff": !timed_out, "timedOut": timed_out})),
        )
        .await;
        if timed_out {
            let s = self.env.settings.get().pause_timeout.as_secs();
            self.notify(
                "limit",
                format!(
                    "{}: paused without a handoff note (none within {s} s)",
                    self.job.name
                ),
            );
        }
        self.commit(before).await;
    }

    /// `after_handoff_fresh`: a handoff note starts a fresh session from it
    /// (R-027), using the note up; Ask passes false, keeping the note and
    /// continuing the last conversation, since its question needs the context.
    async fn prepare_resume(&mut self, after_handoff_fresh: bool) {
        if self.job.next_prompt.is_none() {
            self.job.next_prompt = self.resume_command();
        }
        // After a handoff the next session starts fresh from the note (R-027).
        let handed_off = after_handoff_fresh
            && self.job.handoff_at.take().is_some()
            && self.env.settings.get().fresh_sessions;
        let resume = self.runtime_kit().and_then(|k| k.resume.clone());
        self.resume_session = match (&resume, &self.job.session) {
            (Some(r), Some(id))
                if !handed_off
                    && !self.resume_unconfirmed
                    && r.argv(id, &xherd_core::runner::agent_env).await.is_some() =>
            {
                Some(id.clone())
            }
            _ => None,
        };
    }

    /// The agent's runtime plugin (agents from before the choice: Claude Code).
    fn runtime_name(&self) -> &str {
        self.job
            .runtime
            .as_deref()
            .unwrap_or(xherd_flow::approve::DEFAULT_RUNTIME)
    }

    fn runtime_kit(&self) -> Option<&crate::RuntimeKit> {
        self.env.runtimes.get(self.runtime_name())
    }

    fn resume_command(&self) -> Option<String> {
        self.pack_command("resume")
    }

    /// The pack's command `key` (`resume`, `prReview`…) for the agent's
    /// runtime; `None` for flow `none` or a pack without it.
    fn pack_command(&self, key: &str) -> Option<String> {
        self.pack_command_with(key, &[])
    }

    /// `pack_command` with template variables (`{base}`…) besides `{prefix}`.
    fn pack_command_with(&self, key: &str, vars: &[(&str, &str)]) -> Option<String> {
        let pack = self.env.packs.get(self.job.pack.as_deref()?)?;
        let runtime = self.runtime_name();
        pack.command(runtime, pack.commands.get(key)?, vars)
            .map_err(|e| tracing::warn!(agent = %self.job.name, key, error = %e, "no pack command"))
            .ok()
    }

    fn next_command(&self, wd: &Path, gate: &Gate) -> Result<Option<(String, u32)>, ActionError> {
        let Some(pack) = self.job.pack.as_deref().and_then(|p| self.env.packs.get(p)) else {
            return Ok(None);
        };
        let facts = xherd_flow::approve::DirFacts {
            spec_dir: xherd_flow::approve::spec_dir(wd),
            tasks: &self.job.tasks,
        };
        xherd_flow::approve::next_command(pack, &self.job, gate, &facts, false)
            .map_err(|e| refuse("flow", e))
    }

    async fn request_changes(&mut self, comment: &str) -> Result<(), ActionError> {
        if comment.trim().is_empty() {
            return Err(refuse("usage", "say what to change"));
        }
        let gate = self.gate_for_user(true)?;
        // Remember the documents as the user read them, so the UI can show
        // what the agent changed in answer (no version history is kept).
        if let (Gate::Docs { files, .. }, Ok(wd)) = (&gate, self.workdir()) {
            for f in files {
                if let Some(Ok(bytes)) = inside(&wd, &f.path).map(std::fs::read) {
                    let text = String::from_utf8_lossy(&bytes);
                    self.env
                        .store
                        .put_reviewed(&self.job.id, &f.path, &text)
                        .await
                        .map_err(|e| refuse("store", e.to_string()))?;
                }
            }
        }
        let text = xherd_flow::approve::changes_message(&gate, comment);
        // The revised gate may notify again, even if it comes back unchanged.
        self.notified_gate = None;
        let parked = self.job.status == Status::Parked;
        if parked {
            self.unpark_with(text).await;
        } else {
            self.send(&text, 1).await?;
        }
        // For the work log's rework count (spec §16.4): which gate it was.
        let label = match &gate {
            Gate::Docs { stage, .. } => stage.clone(),
            Gate::Commit(c) => format!("commit {}", c.id),
            _ => String::new(),
        };
        self.log(None, "change-request", Some(json!({"gate": label})))
            .await;
        if !parked {
            self.job.fire(Trigger::UserInput, self.now());
        }
        Ok(())
    }

    /// A message or change request for a parked agent (R-028): it gets a slot
    /// again and the text is typed when its next session starts, which
    /// continues the last conversation when the runtime can find it.
    async fn unpark_with(&mut self, text: String) {
        self.job.next_prompt = Some(text);
        self.job.fire(Trigger::Unpark, self.now());
        self.prepare_resume(true).await;
    }

    /// The approved task list (`task_docs`: the tasks stage's documents, or
    /// the `tasks.md` of the bugfix analysis, not its `spec.md`) has changed
    /// since (task progress and fix tasks aside, `doc_sha256`), while the
    /// agent is past that stage. In that stage the agent rewrites it on
    /// purpose and declares the gate itself.
    async fn approved_tasks_edited(&self, pack: Option<&Pack>) -> bool {
        let Some(flow) = pack.and_then(|p| p.flow(self.job.flow.as_deref().unwrap_or_default()))
        else {
            return false;
        };
        let Ok(wd) = self.workdir() else { return false };
        let stage = self.job.stage.as_deref().and_then(|s| flow.stage(s));
        if stage.is_some_and(|s| s.gates_tasks()) {
            return false;
        }
        let ever = self
            .env
            .store
            .approved_files(&self.job.id)
            .await
            .unwrap_or_default();
        for s in &flow.stages {
            for d in s.task_docs() {
                let Ok(path) = xherd_flow::xap::doc_path(d, &wd) else {
                    continue;
                };
                if !ever.contains(&path) {
                    continue;
                }
                let Some(bytes) = inside(&wd, &path).and_then(|p| std::fs::read(p).ok()) else {
                    continue;
                };
                let sha = xherd_flow::approve::doc_sha256(Some(flow), &wd, &path, &bytes);
                let approved = self.env.store.approved(&self.job.id, &path, &sha).await;
                if !approved.unwrap_or(true) {
                    return true;
                }
            }
        }
        false
    }

    /// The agent stopped without a declared gate: open one from the flow's
    /// documents if some exist unapproved (R-102) or changed since approved (R-107).
    async fn infer_gate(&mut self) {
        let packs = self.env.packs.clone();
        let Some(flow) = self
            .job
            .pack
            .as_deref()
            .and_then(|p| packs.get(p))
            .and_then(|p| p.flow(self.job.flow.as_deref().unwrap_or_default()))
        else {
            return;
        };
        let Ok(wd) = self.workdir() else { return };
        let ever = self
            .env
            .store
            .approved_files(&self.job.id)
            .await
            .unwrap_or_default();
        let mut seen = Vec::new();
        for s in &flow.stages {
            for d in s.docs() {
                let Ok(path) = xherd_flow::xap::doc_path(d, &wd) else {
                    continue;
                };
                let bytes = inside(&wd, &path).and_then(|p| std::fs::read(p).ok());
                let sha = bytes
                    .as_ref()
                    .map(|b| xherd_flow::approve::doc_sha256(Some(flow), &wd, &path, b));
                let questions = match &bytes {
                    Some(b) if path.ends_with(".md") => {
                        xherd_flow::questions::parse(&String::from_utf8_lossy(b))
                    }
                    _ => Vec::new(),
                };
                let approved = match &sha {
                    Some(h) => self
                        .env
                        .store
                        .approved(&self.job.id, &path, h)
                        .await
                        .unwrap_or(false),
                    None => false,
                };
                seen.push(xherd_flow::approve::DocSeen {
                    stage: s.id.clone(),
                    ever_approved: ever.contains(&path),
                    path,
                    sha256: sha,
                    approved,
                    questions,
                });
            }
        }
        if let Some((mut gate, reopened)) = xherd_flow::approve::infer_gate(flow, &seen) {
            // The agent did not declare this gate, so it could not say whether
            // a human is needed: autonomy never approves it (R-112).
            if let Gate::Docs { needs_human, .. } = &mut gate {
                *needs_human = Some(if reopened {
                    "an approved document was edited".into()
                } else {
                    "the agent did not declare this gate".into()
                });
            }
            let kind = if reopened {
                "gate-reopened"
            } else {
                "gate-inferred"
            };
            if let Gate::Docs { stage, .. } = &gate {
                self.job.stage = Some(stage.clone());
            }
            self.log(None, kind, Some(json!({"gate": gate}))).await;
            self.job.gate = Some(gate);
        }
    }

    // ---- ticks and async work --------------------------------------------------

    async fn on_tick(&mut self, t: TickKind) {
        let before = self.job.status;
        let now = self.now();
        let minutes = |m: u64| (m as i64) * 60_000;
        match t {
            TickKind::SessionCheck { alive, recover } => {
                if self.job.status.is_live() && !alive {
                    self.revoke_tokens().await;
                    self.delivery = None;
                    // Waiting at a gate you can approve: park, which is what
                    // a long wait does (approving starts a fresh session).
                    let at_gate = self.job.status == Status::WaitingInput
                        && matches!(self.job.gate, Some(Gate::Docs { .. } | Gate::Commit(_)));
                    if recover && at_gate {
                        self.job.fire(Trigger::Park, now);
                        self.log(None, "parked", Some(json!({"why": "interrupted"})))
                            .await;
                        self.notify(
                            "recovery",
                            format!(
                                "{}: xherd restarted; approve the gate to continue",
                                self.job.name
                            ),
                        );
                    } else {
                        self.job.fire(Trigger::SessionLost, now);
                        // Every end of the session ends an Ask hold, also
                        // when it is resumed at once below (no `commit()`
                        // sees the error status in between).
                        self.job.ask_hold = None;
                        self.log(None, "session-lost", None).await;
                        if recover {
                            // Same as Resume: a new session reloads the state.
                            self.job.fire(Trigger::Resume, now);
                            self.prepare_resume(true).await;
                            self.notify(
                                "recovery",
                                format!("{}: xherd restarted; resuming", self.job.name),
                            );
                        } else {
                            // A session that vanishes while xherd runs is
                            // resumed by itself a couple of times per hour, so
                            // an agent is not stuck in error until someone
                            // notices; after that the Resume button is the way.
                            self.auto_resumes
                                .retain(|t| now - t < AUTO_RESUME_WINDOW_MS);
                            if self.auto_resumes.len() < AUTO_RESUMES {
                                self.auto_resumes.push(now);
                                self.job.fire(Trigger::Resume, now);
                                self.prepare_resume(true).await;
                                self.log(
                                    None,
                                    "auto-resume",
                                    Some(json!({"n": self.auto_resumes.len(), "of": AUTO_RESUMES})),
                                )
                                .await;
                                self.notify(
                                    "recovery",
                                    format!(
                                        "{}: terminal session disappeared; resuming ({}/{})",
                                        self.job.name,
                                        self.auto_resumes.len(),
                                        AUTO_RESUMES
                                    ),
                                );
                            } else {
                                self.notify(
                                    "recovery",
                                    format!(
                                        "{}: terminal session disappeared again; press Resume",
                                        self.job.name
                                    ),
                                );
                            }
                        }
                    }
                    self.commit(before).await;
                }
                // Actions outlive the agent's session: after a restart each
                // running one is adopted (alive or dead pane) or lost (R12).
                self.check_actions();
            }
            TickKind::IdleCheck => {
                let m = self.env.settings.get().idle_alert_minutes;
                if self.job.status == Status::Idle
                    && m > 0
                    && !self.job.idle_alerted
                    && now - self.job.status_since >= minutes(m)
                {
                    self.job.idle_alerted = true;
                    self.log(None, "idle-alert", None).await;
                    self.notify(
                        "idle",
                        format!("{} has been idle for {m} min", self.job.name),
                    );
                    self.commit(before).await;
                }
            }
            TickKind::SessionLimit => {
                let m = self.env.settings.get().max_session_minutes;
                if m > 0
                    && self.job.status.is_live()
                    && self.job.pause_requested_at.is_none()
                    && self
                        .job
                        .session_since
                        .is_some_and(|s| now - s >= minutes(m))
                {
                    self.log(None, "session-limit", Some(json!({"minutes": m})))
                        .await;
                    let waits = self.begin_pause().await;
                    self.notify(
                        "limit",
                        format!(
                            "{} {}: its session ran longer than {m} min",
                            self.job.name,
                            if waits {
                                "is being paused after a handoff"
                            } else {
                                "paused"
                            }
                        ),
                    );
                    self.commit(before).await;
                }
            }
            TickKind::ParkCheck => {
                let m = self.env.settings.get().gate_park_minutes;
                if self.job.status == Status::WaitingInput
                    && m > 0
                    && now - self.job.status_since >= minutes(m)
                {
                    self.end_session().await;
                    self.job.fire(Trigger::Park, now);
                    self.log(None, "parked", None).await;
                    self.commit(before).await;
                }
            }
            TickKind::ScreenScan => {
                if let Some(asked) = self.job.pause_requested_at
                    && self.job.status.is_live()
                    && now - asked >= self.env.settings.get().pause_timeout.as_millis() as i64
                {
                    self.finish_pause(true).await;
                }
                if self.job.status.is_live() {
                    let (driver, ctx, tx, generation) = (
                        self.env.driver.clone(),
                        self.ctx(),
                        self.tx.clone(),
                        self.job.generation,
                    );
                    let lines = self
                        .env
                        .settings
                        .get()
                        .screen_rules
                        .as_ref()
                        .map_or(40, |r| (r.last_lines as u32 * 4).max(40));
                    tokio::spawn(async move {
                        if let Ok(screen) = driver.capture(&ctx, lines).await {
                            let _ = tx.send(Msg::WorkDone {
                                generation,
                                result: WorkResult::Screen(screen),
                            });
                        }
                    });
                }
                self.check_actions();
            }
            TickKind::PrPoll => self.poll_pr(),
            TickKind::TodoTick => {}
        }
    }

    async fn on_work(&mut self, w: WorkResult) {
        match w {
            WorkResult::ActionEnded {
                id,
                exit,
                output,
                stopped,
                restart,
            } => {
                self.action_ended(&id, exit, &output, stopped, restart)
                    .await;
            }
            WorkResult::ActionAlive { id } => {
                let (before, now) = (self.job.status, self.now());
                if self.action_runs(&id) && xherd_core::action::on_alive(&mut self.job, &id, now) {
                    self.commit(before).await;
                }
            }
            WorkResult::ActionGone { id } => {
                if self.action_runs(&id) {
                    let (before, now) = (self.job.status, self.now());
                    xherd_core::action::on_lost(&mut self.job, &id, now);
                    self.log(None, "action-lost", Some(json!({"id": id}))).await;
                    self.commit(before).await;
                }
            }
            WorkResult::Screen(screen) => {
                // Only a running agent can be stuck on a failing command; a
                // stopped one's screen is its own text about the work.
                let scan = self.job.status == Status::Running || self.auth_screen_block;
                let line = scan
                    .then(|| xherd_core::git_auth::failing_screen_line(&screen))
                    .flatten();
                match line {
                    None => {
                        self.auth_screen_line = None;
                        if self.auth_screen_block {
                            self.git_access_ok().await;
                        }
                    }
                    Some(l) if self.auth_screen_line.as_deref() != Some(&l) => {
                        self.auth_screen_line = Some(l.clone());
                        if self.git_access_failed("screen", &l).await {
                            return;
                        }
                    }
                    Some(_) => {}
                }
                let settings = self.env.settings.get();
                let Some(rules) = settings.screen_rules.as_ref() else {
                    return;
                };
                let shown = rules.matching(&screen).is_some();
                let event = match (shown, self.job.status == Status::Blocked) {
                    (true, false) => ReportEvent::Blocked,
                    (false, true) if self.job.screen_blocked => ReportEvent::Working,
                    _ => return,
                };
                let r = Report {
                    id: xherd_core::new_id(),
                    job_id: self.job.id.clone(),
                    event,
                    payload: json!({"message": "dialog on screen"}),
                    seq: self.job.last_daemon_seq + 1,
                    at: self.now(),
                };
                self.on_report(r, Origin::Daemon).await;
            }
            WorkResult::PrTemplates(Ok(list)) => {
                // Always written, empty included: "none" is an answer.
                let written = self.workdir().map_err(|e| e.message).and_then(|wd| {
                    let text = serde_json::to_string_pretty(&json!({"templates": list}))
                        .map_err(|e| e.to_string())?;
                    let dir = wd.join(xherd_flow::xap::SPEC_DIR);
                    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                    xherd_core::paths::write_atomic(
                        &dir.join("pr-templates.json"),
                        text.as_bytes(),
                        None,
                    )
                    .map_err(|e| e.to_string())
                });
                if let Err(e) = written {
                    tracing::warn!(agent = %self.job.name, error = %e, "pr-templates.json not written");
                }
            }
            WorkResult::PrTemplates(Err(e)) => {
                tracing::warn!(agent = %self.job.name, error = %e, "PR templates lookup failed");
            }
            WorkResult::PrPoll(Ok((snap, comments))) => self.on_pr_poll(snap, comments).await,
            WorkResult::PrPoll(Err(e)) => {
                tracing::warn!(agent = %self.job.name, error = %e, "PR poll failed");
                self.log(None, "pr-poll-failed", Some(json!({"error": e})))
                    .await;
                self.git_access_failed("pr-poll", &e).await;
            }
            WorkResult::Usage { session, tokens } => self.on_usage(&session, tokens).await,
            WorkResult::BranchCommits { on_branch, found } => {
                self.on_branch_commits(on_branch, found).await
            }
            WorkResult::PrLookup(Ok(None)) => self.git_access_ok().await,
            WorkResult::PrLookup(Ok(Some(url))) => {
                self.git_access_ok().await;
                let before = self.job.status;
                self.job.pr_url = Some(url.clone());
                self.log(None, "pr-found", Some(json!({"url": url}))).await;
                self.commit(before).await;
                self.after_ship().await;
            }
            WorkResult::PrLookup(Err(e)) => {
                tracing::warn!(agent = %self.job.name, error = %e, "PR lookup failed");
                self.log(None, "pr-lookup-failed", Some(json!({"error": e})))
                    .await;
                self.git_access_failed("pr-lookup", &e).await;
            }
            WorkResult::DeliveryTimeout { delivery } => match self.delivery {
                Some((d, false))
                    if d == delivery
                        && self.job.status.is_live()
                        && self.job.status != Status::Blocked =>
                {
                    tracing::info!(agent = %self.job.name, "delivery not acknowledged; pressing Enter again");
                    let _ = self.env.driver.send(&self.ctx(), "", 1).await;
                    self.delivery = Some((d, true));
                    self.log(None, "delivery-retry", None).await;
                    self.notify(
                        "delivery",
                        format!(
                            "{}: message not acknowledged; pressed Enter again",
                            self.job.name
                        ),
                    );
                }
                _ => {}
            },
        }
    }
}

/// Whether the agent's branch merges into the PR's base, worked out in the
/// worktree when the host cannot say (#75): fetch `origin`, then merge
/// `HEAD` into `origin/<base>` without touching any file. The base is the
/// host's, else the agent's own; `Unknown` when neither is known or git
/// cannot tell (a failed fetch included: stale refs would mislead).
async fn local_mergeable(
    git: &xherd_git::Git,
    wd: &Path,
    host_base: Option<&str>,
    agent_base: Option<&str>,
) -> xherd_core::pr::Mergeable {
    use xherd_core::pr::Mergeable;
    let Some(base) = xherd_core::pr::base_ref(host_base, agent_base) else {
        return Mergeable::Unknown;
    };
    if let Err(e) = git.fetch(wd, "origin").await {
        tracing::warn!(error = %e, "fetch for the merge check failed");
        return Mergeable::Unknown;
    }
    match git.merges_cleanly(wd, &base, "HEAD").await {
        Some(true) => Mergeable::Yes,
        Some(false) => Mergeable::No,
        None => Mergeable::Unknown,
    }
}

/// The file of a canonical document path, if it exists inside the worktree
/// (symlinks resolved).
pub(crate) fn inside(wd: &Path, doc: &str) -> Option<PathBuf> {
    let abs = wd
        .join(xherd_flow::xap::worktree_rel(doc))
        .canonicalize()
        .ok()?;
    (abs.starts_with(wd.canonicalize().ok()?) && abs.is_file()).then_some(abs)
}

/// Append to `.xherd/ledger/approvals.jsonl` and fsync (design §3.4).
fn append_ledger(wd: &Path, lines: &[Value]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = wd.join(".xherd/ledger");
    std::fs::create_dir_all(&dir)?;
    let mut buf = Vec::new();
    for l in lines {
        serde_json::to_writer(&mut buf, l)?;
        buf.push(b'\n');
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("approvals.jsonl"))?;
    f.write_all(&buf)?;
    f.sync_all()
}

/// A live change that would go past the repository profile's ceiling (R12a).
fn above_profile(what: &str, want: &str, cap: &str, profile: &str) -> ActionError {
    refuse(
        "above_profile",
        format!("{what} {want} is above what the profile {profile} allows ({cap})"),
    )
}

/// The PATH of an agent's pane: the folder of `xherd_bin`, then the agents'
/// PATH, not ours: the daemon's is the system's (R-1203). `None` when the
/// binary path is not absolute.
fn agent_path(xherd_bin: &str) -> Option<String> {
    let dir = Path::new(xherd_bin).parent().filter(|d| d.is_absolute())?;
    let old = xherd_core::runner::agent_env("PATH").unwrap_or_default();
    Some(
        format!("{}:{old}", dir.display())
            .trim_end_matches(':')
            .to_string(),
    )
}

/// `argv` run as `env PATH=<path> …`, outermost. tmux ignores a `-e PATH` for
/// a new pane and takes the daemon's PATH, which lacks the agents' folders
/// (`init.rs`), so the runtime would not be found.
fn agent_pane_argv(path: Option<&str>, argv: Vec<String>) -> Vec<String> {
    match path {
        None => argv,
        Some(p) => {
            let mut out = vec!["/usr/bin/env".to_string(), format!("PATH={p}")];
            out.extend(argv);
            out
        }
    }
}

#[cfg(test)]
mod agent_pane_tests {
    use super::*;

    #[test]
    fn agent_pane_puts_the_agents_path_outermost() {
        let argv = vec!["claude".to_string(), "--version".to_string()];
        let out = agent_pane_argv(Some("/usr/local/bin:/h/.local/bin"), argv.clone());
        assert_eq!(
            out,
            [
                "/usr/bin/env",
                "PATH=/usr/local/bin:/h/.local/bin",
                "claude",
                "--version"
            ]
        );
        assert_eq!(agent_pane_argv(None, argv.clone()), argv);
    }

    #[test]
    fn agent_pane_path_starts_with_the_xherd_folder() {
        let p = agent_path("/usr/local/bin/xherd").unwrap();
        assert!(p.starts_with("/usr/local/bin:"), "{p}");
        assert_eq!(agent_path("xherd"), None);
    }
}

#[cfg(test)]
mod pick_host_tests {
    use super::*;
    use xherd_core::config::{Credential, GitHost};

    fn hosts() -> Vec<Arc<xherd_plugin::host::Host>> {
        let d = tempfile::tempdir().unwrap();
        let reg = xherd_plugin::Registry::load(xherd_plugin::builtins(), d.path());
        ["github", "gitlab", "bitbucket"]
            .iter()
            .map(|n| {
                Arc::new(xherd_plugin::host::Host {
                    plugin: reg.get(n).unwrap().clone(),
                    ctx: xherd_plugin::Context::default(),
                })
            })
            .collect()
    }

    fn row(host: &str, provider: Option<&str>) -> GitHost {
        GitHost {
            host: host.into(),
            protocols: vec!["https".into()],
            credential: Credential::None,
            provider: provider.map(str::to_string),
            username: None,
            ca_file: None,
            prefer_https: false,
            prefer_ssh: false,
        }
    }

    #[test]
    fn a_host_plugin_without_a_token_uses_the_one_kept_for_the_remote() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("secrets.json");
        std::fs::write(&f, r#"{"git": {"gitlab.example.com": "glpat-1"}}"#).unwrap();
        let gl = hosts().remove(1);
        let tok = |h: &Arc<xherd_plugin::host::Host>| h.ctx.secrets.get("token").cloned();
        let with = with_git_token(gl.clone(), Some("gitlab.example.com"), &[], &f);
        assert_eq!(tok(&with), Some("glpat-1".into()));
        assert_eq!(
            tok(&with_git_token(
                gl.clone(),
                Some("other.example.com"),
                &[],
                &f
            )),
            None
        );
        assert_eq!(tok(&with_git_token(gl.clone(), None, &[], &f)), None);
        // The plugin's own token (saved in the UI) wins, and applies without a restart.
        std::fs::write(
            &f,
            r#"{"git": {"gitlab.example.com": "glpat-1"}, "gitlab": {"token": "mine"}}"#,
        )
        .unwrap();
        let rows = [row("gitlab.example.com", None)];
        assert_eq!(
            tok(&with_git_token(gl, Some("gitlab.example.com"), &rows, &f)),
            Some("mine".into())
        );
    }

    /// The remote host is what the agent reports: a pattern such as `*gitlab*`
    /// claims `gitlab.attacker.io`, but the plugin's own token is not sent there.
    #[test]
    fn the_plugins_own_token_goes_only_to_a_host_the_user_named() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("secrets.json");
        std::fs::write(
            &f,
            r#"{"gitlab": {"token": "mine"}, "github": {"token": "gh"}}"#,
        )
        .unwrap();
        let [gh, gl, _] = <[_; 3]>::try_from(hosts()).ok().unwrap();
        let tok = |h: Arc<xherd_plugin::host::Host>, r: Option<&str>, rows: &[GitHost]| {
            with_git_token(h, r, rows, &f)
                .ctx
                .secrets
                .get("token")
                .cloned()
        };
        // Unknown host, claimed only by the wildcard pattern: no token.
        assert_eq!(tok(gl.clone(), Some("gitlab.attacker.io"), &[]), None);
        assert_eq!(tok(gl.clone(), None, &[]), None);
        // A host with a row in Git access (any case) gets it.
        let rows = [row("gitlab.corp.example", None)];
        assert_eq!(
            tok(gl.clone(), Some("gitlab.corp.example"), &rows),
            Some("mine".into())
        );
        assert_eq!(tok(gl, Some("gitlab.attacker.io"), &rows), None);
        // A plain name in the plugin's own patterns needs no row.
        assert_eq!(tok(gh.clone(), Some("github.com"), &[]), Some("gh".into()));
        assert_eq!(tok(gh, Some("github.com.evil.io"), &[]), None);
    }

    /// A plugin that declares no `token` secret (Jira, an external tracker)
    /// never receives the host's token.
    #[test]
    fn the_host_token_goes_only_to_a_plugin_that_declares_one() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("secrets.json");
        std::fs::write(&f, r#"{"git": {"github.com": "ghp-1"}}"#).unwrap();
        let reg = xherd_plugin::Registry::load(xherd_plugin::builtins(), d.path());
        let jira = Arc::new(xherd_plugin::host::Host {
            plugin: reg.get("jira").unwrap().clone(),
            ctx: xherd_plugin::Context::default(),
        });
        let with = with_git_token(jira, Some("github.com"), &[], &f);
        assert_eq!(with.ctx.secrets.get("token"), None);
    }

    #[test]
    fn a_remote_goes_to_its_provider() {
        let hs = hosts();
        let pick = |rows: &[GitHost], off: &[String], r: Option<&str>| {
            pick_host(&hs, rows, off, r).map(|h| h.plugin.name().to_string())
        };
        let none: &[String] = &[];
        assert_eq!(
            pick(&[], none, Some("gitlab.com")).as_deref(),
            Some("gitlab")
        );
        assert_eq!(
            pick(&[], none, Some("github.com")).as_deref(),
            Some("github")
        );
        // Bitbucket is its own provider even without PR commands.
        assert_eq!(
            pick(&[], none, Some("bitbucket.org")).as_deref(),
            Some("bitbucket")
        );
        // Unknown names and no remote keep the old behaviour: GitHub.
        assert_eq!(
            pick(&[], none, Some("git.example.com")).as_deref(),
            Some("github")
        );
        assert_eq!(pick(&[], none, None).as_deref(), Some("github"));
        // The provider named on the host's row wins over its name.
        let rows = [row("git.example.com", Some("gitlab"))];
        assert_eq!(
            pick(&rows, none, Some("git.example.com")).as_deref(),
            Some("gitlab")
        );
        // A switched-off plugin is never used, and no other one stands in
        // for it (gh cannot read a GitLab merge request).
        let off = ["gitlab".to_string()];
        assert_eq!(pick(&rows, &off, Some("gitlab.com")), None);
        assert_eq!(pick(&rows, &off, Some("git.example.com")), None);
        assert_eq!(
            pick(&[], &off, Some("github.com")).as_deref(),
            Some("github")
        );
    }
}
