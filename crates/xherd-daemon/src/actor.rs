//! One actor per agent (design §3.2). The actor owns the `Agent` value, applies
//! messages one at a time with the pure functions of `xherd-core`, persists,
//! and publishes. Slow work runs in spawned tasks and comes back as `WorkDone`
//! tagged with the agent's generation; stale results are dropped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use xherd_core::config::Config;
use xherd_core::domain::{CommitStatus, Gate, TaskStatus};
use xherd_core::report::{Effect, Origin, Outcome, Report, ReportEvent, apply_report};
use xherd_core::status::next_status;
use xherd_core::xap::{Refusal, XapCall, XapCmd, refuse as refusal};
use xherd_core::{Agent, Status, Trigger};
use xherd_flow::Pack;
use xherd_flow::xap::{CommitFact, DocFact, DocState, Observed, VerifyFact};
use xherd_store::{Event, Store};
use xherd_term::ScreenRules;

use crate::driver::{AgentCtx, Driver};
use crate::support::{
    Change, Clock, Hub, SCOPE_CREDENTIAL, SCOPE_HOOKS, SCOPE_XAP, TOKEN_AGENT, mint_token,
};

#[derive(Debug)]
pub enum Msg {
    Report(Report, Origin, Option<oneshot::Sender<ReportResult>>),
    User(UserAction, oneshot::Sender<Result<Agent, ActionError>>),
    Xap(XapCall, oneshot::Sender<Result<Value, Refusal>>),
    Tick(TickKind),
    WorkDone { generation: u64, result: WorkResult },
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
    Cancel,
    Send {
        text: String,
    },
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

fn refuse(code: &'static str, message: impl Into<String>) -> ActionError {
    ActionError {
        code,
        message: message.into(),
    }
}

/// Sent by actors to the supervisor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupEvent {
    StatusChanged { id: String, to: Status },
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
    pub agent_token_budget: u64,
    pub daily_token_budget: u64,
    pub max_diff_lines: usize,
    pub sensitive_paths: Vec<String>,
    /// Plugins switched off (`disabledPlugins`): the host, the trust writer
    /// and the toolchain plugins skip them while running.
    pub disabled_plugins: Vec<String>,
    /// Notification kinds that are not published (`mutedNotifications`).
    pub muted_notifications: Vec<String>,
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
            agent_token_budget: c.agent_token_budget,
            daily_token_budget: c.daily_token_budget,
            max_diff_lines: c.max_diff_lines,
            sensitive_paths: c.sensitive_paths.clone(),
            disabled_plugins: c.disabled_plugins.clone(),
            muted_notifications: c.muted_notifications.clone(),
            ..base
        }
    }
}

impl Default for ActorSettings {
    fn default() -> ActorSettings {
        ActorSettings {
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
            agent_token_budget: 0,
            daily_token_budget: 0,
            max_diff_lines: xherd_core::tripwire::MAX_DIFF_LINES,
            sensitive_paths: Vec::new(),
            disabled_plugins: Vec::new(),
            muted_notifications: Vec::new(),
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
    /// Host plugin for pull-request lookups (R-302); `None` when not installed.
    pub host: Option<Arc<xherd_plugin::host::Host>>,
    /// Toolchain plugins in registry order (R-652).
    pub toolchains: Arc<Vec<(String, xherd_plugin::toolchain::Toolchain)>>,
    /// Marks the worktree trusted in the runtime's file before each session
    /// (the runtime rewrites that file itself, and may drop the entry).
    pub trust: Option<Arc<crate::TrustWriter>>,
    /// How to continue the runtime's last conversation, when it can.
    pub resume: Option<Arc<crate::Resumer>>,
    /// How to read token usage from the runtime's transcript (R-1205).
    pub usage: Option<Arc<xherd_plugin::runtime::Usage>>,
}

/// Default limit for a task's `verify:` command.
pub const VERIFY_TIMEOUT: Duration = Duration::from_secs(600);

/// Session losses the daemon answers with a resume by itself, per window.
const AUTO_RESUMES: usize = 2;
const AUTO_RESUME_WINDOW_MS: i64 = 3_600_000;

pub type AgentTx = mpsc::UnboundedSender<Msg>;

pub fn spawn(agent: Agent, env: Env) -> AgentTx {
    let (tx, rx) = mpsc::unbounded_channel();
    let actor = Actor {
        agent,
        env,
        tx: tx.clone(),
        delivery: None,
        auth_screen_line: None,
        next_delivery: 0,
        first_unacked: false,
        resume_session: None,
        resume_unconfirmed: false,
        auto_resumes: Vec::new(),
    };
    tokio::spawn(actor.run(rx));
    tx
}

struct Actor {
    agent: Agent,
    env: Env,
    /// Own mailbox, for spawned work to report back.
    tx: AgentTx,
    /// Delivery waiting for acknowledgement: (id, retried).
    delivery: Option<(u64, bool)>,
    /// The auth-failure line last seen on the screen (R-688), so a failure
    /// that stays visible blocks once.
    auth_screen_line: Option<String>,
    next_delivery: u64,
    /// The first command was typed but no prompt was accepted yet. Claude Code
    /// can start a second session right after the first `session-start` and
    /// lose typed input, so the command stays in `next_prompt` and is typed
    /// again at the next `session-start` until a prompt is submitted.
    first_unacked: bool,
    /// Launch the next session with the runtime's own resume of this session id.
    resume_session: Option<String>,
    /// The last start used a runtime resume and no `session-start` came yet:
    /// if the session dies again, the next resume starts a clean session.
    resume_unconfirmed: bool,
    /// When the daemon resumed this agent by itself (bounded, see above).
    auto_resumes: Vec<i64>,
}

impl Actor {
    /// Agents from before `visited_stages`: rebuild it once from their
    /// `stage.enter` / `gate.open` events, oldest first.
    async fn backfill_stages(&mut self) {
        if !self.agent.visited_stages.is_empty() || self.agent.stage.is_none() {
            return;
        }
        let Ok(mut events) = self.env.store.events(&self.agent.id, 100_000).await else {
            return;
        };
        events.reverse();
        for e in events {
            if !matches!(e.kind.as_str(), "xap.stage.enter" | "xap.gate.open") {
                continue;
            }
            let stage = e.payload.as_ref().and_then(|p| p["args"]["stage"].as_str());
            if let Some(s) = stage
                && !self.agent.visited_stages.iter().any(|v| v == s)
            {
                self.agent.visited_stages.push(s.to_string());
            }
        }
        self.agent.note_stage();
        if let Err(err) = self.env.store.put_agent(&self.agent).await {
            tracing::error!(agent = %self.agent.name, %err, "agent not stored");
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
                    let res = self.on_user(action).await.map(|()| self.agent.clone());
                    let _ = reply.send(res);
                }
                Msg::Xap(call, reply) => {
                    let _ = reply.send(self.on_xap(call).await);
                }
                Msg::Tick(t) => self.on_tick(t).await,
                Msg::WorkDone { generation, result } => {
                    if generation != self.agent.generation {
                        tracing::debug!(agent = %self.agent.name, "dropping stale async result");
                        continue;
                    }
                    self.on_work(result).await;
                }
            }
        }
    }

    fn ctx(&self) -> AgentCtx {
        AgentCtx {
            id: self.agent.id.clone(),
            name: self.agent.name.clone(),
            workdir: PathBuf::from(self.agent.workdir.clone().unwrap_or_else(|| ".".into())),
            env: Vec::new(),
        }
    }

    fn now(&self) -> i64 {
        self.env.clock.now()
    }

    async fn log(&self, id: Option<String>, kind: &str, payload: Option<Value>) {
        let e = Event {
            id: id.unwrap_or_else(xherd_core::new_id),
            agent_id: self.agent.id.clone(),
            kind: kind.to_string(),
            payload,
            at: self.now(),
        };
        if let Err(err) = self.env.store.insert_event(e).await {
            tracing::error!(agent = %self.agent.name, %err, "event not stored");
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
            .agent
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
            self.agent.name
        );
        let before = self.agent.status;
        if self.agent.fire(Trigger::Blocked, self.now()).is_some() {
            self.agent.screen_blocked = false;
            self.agent.block_reason = Some("git access".into());
            self.commit(before).await;
        }
        self.notify("blocked", msg);
        true
    }

    /// Clears a git-access block once a host command works again.
    async fn git_access_ok(&mut self) {
        if self.agent.block_reason.as_deref() == Some("git access") {
            let before = self.agent.status;
            self.agent.fire(Trigger::Working, self.now());
            self.commit(before).await;
        }
    }

    /// The host plugin, unless it is switched off.
    fn host(&self) -> Option<Arc<xherd_plugin::host::Host>> {
        let off = self.env.settings.get().disabled_plugins.clone();
        self.env
            .host
            .clone()
            .filter(|h| !off.iter().any(|n| n == h.plugin.name()))
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
            agent_id: self.agent.id.clone(),
            kind: kind.into(),
            text,
        });
    }

    /// Persist and publish; tell the supervisor when the status moved.
    async fn commit(&mut self, before: Status) {
        self.agent.note_stage();
        if !self.agent.status.is_live() {
            self.agent.pause_requested_at = None;
        }
        if let Err(err) = self.env.store.put_agent(&self.agent).await {
            tracing::error!(agent = %self.agent.name, %err, "agent not stored");
        }
        let _ = self.env.hub.send(Change::Agent {
            agent: Box::new(self.agent.clone()),
        });
        if self.agent.status != before {
            tracing::info!(agent = %self.agent.name, from = %before, to = %self.agent.status, "status");
            let _ = self.env.sup.send(SupEvent::StatusChanged {
                id: self.agent.id.clone(),
                to: self.agent.status,
            });
        }
    }

    async fn end_session(&mut self) {
        if let Err(e) = self.env.driver.stop(&self.ctx()).await {
            tracing::warn!(agent = %self.agent.name, error = %e, "stopping session failed");
        }
        self.revoke_tokens().await;
        self.delivery = None;
    }

    async fn revoke_tokens(&self) {
        if let Err(e) = self
            .env
            .store
            .revoke_tokens(TOKEN_AGENT, &self.agent.id, self.now())
            .await
        {
            tracing::error!(agent = %self.agent.name, error = %e, "revoking agent tokens failed");
        }
    }

    // ---- reports -------------------------------------------------------------

    async fn on_report(&mut self, r: Report, origin: Origin) -> ReportResult {
        if origin == Origin::Agent && !r.event.is_frequent() {
            match self.env.store.event_exists(&r.id).await {
                Ok(true) => return ReportResult::Duplicate,
                Ok(false) => {}
                Err(e) => tracing::error!(%e, "event lookup failed"),
            }
        }
        if origin == Origin::Agent
            && matches!(r.event, ReportEvent::Stop | ReportEvent::Idle)
            && self.agent.status.is_live()
            && self.agent.gate.is_none()
            && r.seq > self.agent.last_seq
        {
            self.infer_gate().await;
        }
        let before = self.agent.status;
        let now = self.now();
        let outcome = apply_report(&mut self.agent, &r, origin, now);
        let (result, effects) = match outcome {
            Outcome::Stale => (ReportResult::Stale, Vec::new()),
            Outcome::Dormant => (ReportResult::Dormant, Vec::new()),
            Outcome::Applied { effects, .. } => (ReportResult::Applied, effects),
        };
        if result != ReportResult::Applied || !r.event.is_frequent() {
            let kind = match origin {
                Origin::Agent => format!("report.{}", r.event.as_str()),
                Origin::Daemon => format!("screen.{}", r.event.as_str()),
            };
            let id = (origin == Origin::Agent).then(|| r.id.clone());
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
                        self.agent.next_prompt = None;
                        self.commit(self.agent.status).await;
                    }
                }
                Effect::UserPrompted => self.log(None, "user-prompted", None).await,
                Effect::AfterStop => {
                    self.measure_usage();
                    // A pause waits for the handoff note; the stop after it
                    // ends the session. Earlier stops (the turn that was
                    // running) change nothing, and no approval happens now.
                    if let Some(asked) = self.agent.pause_requested_at {
                        if self.agent.handoff_at.is_some_and(|h| h >= asked) {
                            self.finish_pause(false).await;
                        }
                        continue;
                    }
                    self.try_auto_approve().await;
                    self.after_ship().await;
                }
            }
        }
        if result == ReportResult::Applied && r.event == ReportEvent::SessionStart {
            self.resume_unconfirmed = false;
        }
        if result == ReportResult::Applied
            && r.event == ReportEvent::SessionStart
            && self.agent.next_prompt.is_some()
        {
            self.send_first_prompt().await;
        }
        result
    }

    /// Read the session's tokens from the runtime's transcript (slow: spawned,
    /// back as `WorkDone`). Cumulative, so a dropped result is made up by the
    /// next stop. Without a session id or a runtime that has transcripts there
    /// is nothing to measure.
    fn measure_usage(&self) {
        let (Some(session), Some(r), Some(usage)) = (
            self.agent.session.clone(),
            self.env.resume.clone(),
            self.env.usage.clone(),
        ) else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.agent.generation);
        tokio::spawn(async move {
            // The transcript is in the agent user's home (R-1203).
            let Some(path) = r.transcript(&session, &xherd_core::runner::agent_env).await else {
                return;
            };
            let tokens = match xherd_core::runner::read_file(&path).await {
                Ok(Some(b)) => Some(usage.totals(&String::from_utf8_lossy(&b)).budget_tokens()),
                _ => None,
            };
            if let Some(tokens) = tokens {
                let _ = tx.send(Msg::WorkDone {
                    generation,
                    result: WorkResult::Usage { session, tokens },
                });
            }
        });
    }

    /// Poll the pull request of a watching agent through the host plugin when
    /// `prPollMinutes` have passed (R-303; slow: spawned, back as `WorkDone`).
    /// Comments come with the rest of PR follow-up.
    fn poll_pr(&mut self) {
        let every = self.env.settings.get().pr_poll;
        let now = self.now();
        let due = self
            .agent
            .pr_polled_at
            .is_none_or(|t| now - t >= every.as_millis() as i64);
        let (Some(host), Ok(wd), Some(url), true) = (
            self.host(),
            self.workdir(),
            self.agent.pr_url.clone(),
            self.agent.status == Status::Watching && !every.is_zero() && due,
        ) else {
            return;
        };
        // Set here so a slow host is not asked again by the next tick.
        self.agent.pr_polled_at = Some(now);
        let (tx, generation) = (self.tx.clone(), self.agent.generation);
        tokio::spawn(async move {
            let result = match host.pr_snapshot(&wd, &url).await {
                Ok(snap) => {
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
        let before = self.agent.status;
        let name = self.agent.name.clone();
        for e in self.agent.apply_pr(&snap) {
            let text = match e {
                Event::Finished(state) => {
                    self.agent.fire(Trigger::PrFinished, self.now());
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
        if self.agent.status == Status::Watching
            && let Some(all) = comments
        {
            let fresh = self.agent.new_comments(&all);
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
        let name = self.agent.name.clone();
        let written = self.write_pr_comments(&fresh);
        let command = self.pack_command("prReview");
        if let Err(e) = &written {
            // Not handled: they are new again at the next poll.
            self.agent
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
                self.agent.next_prompt = Some(cmd);
                self.agent.fire(Trigger::PrComments, self.now());
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
        let delta = self.agent.record_usage(session, tokens);
        let today = day(self.now());
        if delta > 0
            && let Err(e) = self.env.store.add_usage("all", &today, delta).await
        {
            tracing::warn!(agent = %self.agent.name, %e, "usage not stored");
        }
        let daily = self.env.store.usage("all", &today).await.unwrap_or(0);
        let s = self.env.settings.get();
        let agent_budget = self
            .profile()
            .await
            .and_then(|p| p.agent_token_budget)
            .unwrap_or(s.agent_token_budget);
        let checks = [
            ("agent", self.agent.tokens_total(), agent_budget),
            ("daily", daily, s.daily_token_budget),
        ];
        let before = self.agent.status;
        let hit = checks
            .iter()
            .find(|(_, used, budget)| level(*used, *budget) == Level::Exceeded);
        if let Some((scope, used, budget)) = hit {
            if next_status(self.agent.status, Trigger::Pause).is_some() {
                self.end_session().await;
                self.agent.fire(Trigger::Pause, self.now());
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
                        self.agent.name
                    ),
                );
            }
        } else if !self.agent.budget_warned
            && let Some((scope, used, budget)) = checks
                .iter()
                .find(|(_, used, budget)| level(*used, *budget) == Level::Warn)
        {
            self.agent.budget_warned = true;
            self.notify(
                "budget",
                format!(
                    "{}: 80% of the {scope} token budget is used ({used} of {budget})",
                    self.agent.name
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
    async fn after_ship(&mut self) {
        if self.agent.status != Status::Idle
            || self.agent.gate.is_some()
            || self.agent.pack.is_none()
        {
            return;
        }
        if self.agent.pr_url.is_none() {
            self.find_pr();
            return;
        }
        let before = self.agent.status;
        self.end_session().await;
        self.agent.fire(Trigger::Watch, self.now());
        self.log(None, "watching", Some(json!({"pr": self.agent.pr_url})))
            .await;
        self.commit(before).await;
    }

    /// Look the branch's PR up through the host plugin (slow: spawned, back
    /// as `WorkDone`). Only in the ship stage, where the agent opens it.
    fn find_pr(&self) {
        let ship = match (&self.agent.pack, &self.agent.flow, &self.agent.stage) {
            (Some(p), Some(f), Some(stage)) => self
                .env
                .packs
                .get(p)
                .and_then(|p| p.flow(f))
                .is_some_and(|f| {
                    f.stages
                        .iter()
                        .any(|s| &s.id == stage && s.role == xherd_core::domain::Role::Ship)
                }),
            _ => false,
        };
        let (Some(host), Ok(wd), true) = (self.host(), self.workdir(), ship) else {
            return;
        };
        let (tx, generation, branch) = (
            self.tx.clone(),
            self.agent.generation,
            self.agent.name.clone(),
        );
        tokio::spawn(async move {
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
        let runtime = self
            .agent
            .runtime
            .as_deref()
            .unwrap_or(xherd_flow::approve::DEFAULT_RUNTIME);
        self.agent
            .pack
            .as_deref()
            .and_then(|p| self.env.packs.get(p))
            .and_then(|p| p.install.get(runtime))
            .filter(|i| cmd.starts_with(&i.prefix))
            .map_or(1, |i| i.slash_enters)
    }

    async fn send_first_prompt(&mut self) {
        let Some(cmd) = self.agent.next_prompt.clone() else {
            return;
        };
        let enters = self.enters_for(&cmd);
        let before = self.agent.status;
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
                    format!("{}: could not send {cmd:?}: {}", self.agent.name, e.message),
                );
            }
        }
    }

    // ---- XAP -------------------------------------------------------------------

    /// Validate and apply one XAP call (design §6.5). Accepted calls are logged
    /// as `xap.<cmd>` under the call id, which also de-duplicates replays.
    async fn on_xap(&mut self, mut call: XapCall) -> Result<Value, Refusal> {
        if !self.agent.status.is_live() {
            return Err(refusal(
                "not_live",
                format!("{} is {}", self.agent.name, self.agent.status),
            ));
        }
        if self.env.store.event_exists(&call.id).await.unwrap_or(false) {
            return Ok(json!({"changed": false, "duplicate": true}));
        }
        self.commit_tripwires(&mut call.cmd).await;
        let packs = self.env.packs.clone();
        let pack = self.agent.pack.as_deref().and_then(|p| packs.get(p));
        self.review_hold(&mut call.cmd, pack)?;
        self.pr_review_check(&call.cmd)?;
        let obs = self.observe(pack, &call.cmd).await?;
        let (before, now) = (self.agent.status, self.now());
        let changed = xherd_flow::xap::apply(&mut self.agent, pack, &call.cmd, obs, now)?;
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
        }
        if changed {
            self.commit(before).await;
        }
        Ok(json!({"changed": changed, "stage": self.agent.stage, "gate": self.agent.gate}))
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
            let (Some(host), Some(url)) = (self.host(), self.agent.pr_url.clone()) else {
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
        pack.zip(self.agent.flow.as_deref())
            .and_then(|(p, f)| p.flow(f))
            .and_then(|f| f.stage(stage))
            .map(|s| s.role)
    }

    /// The repository's pull-request templates through the host plugin
    /// (R-153; spawned, back as `WorkDone`), for the `ship` prompt.
    fn fetch_templates(&self) {
        let (Some(host), Ok(wd)) = (self.host(), self.workdir()) else {
            return;
        };
        let (tx, generation) = (self.tx.clone(), self.agent.generation);
        tokio::spawn(async move {
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
            .zip(self.agent.flow.as_deref())
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
            xherd_flow::review::open_findings(&findings, &self.agent.tasks)
                .iter()
                .map(|f| format!("{} ({})", f.id, f.severity.label()))
                .collect();
        let open: Vec<&str> = self
            .agent
            .tasks
            .iter()
            .filter(|t| t.source == TaskSource::Review && t.status != TaskStatus::Done)
            .map(|t| t.id.as_str())
            .collect();
        let max = self.env.settings.get().max_review_loops;
        if let Some(why) = xherd_core::tripwire::review_summary(
            &open_findings,
            &open,
            self.agent.review_rounds,
            max,
        ) {
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
        let sensitive = profile
            .as_ref()
            .and_then(|p| p.sensitive_paths.as_ref())
            .unwrap_or(&st.sensitive_paths);
        let max_lines = profile
            .as_ref()
            .and_then(|p| p.max_diff_lines)
            .unwrap_or(st.max_diff_lines);
        hits.extend(tw::sensitive_summary(files, sensitive));
        if let Ok(wd) = self.workdir() {
            match self.env.git.diff_proposed(&wd, files).await {
                Ok(diff) => {
                    hits.extend(tw::summary(&tw::scan_diff(&diff)));
                    hits.extend(tw::size_summary(&diff, max_lines));
                }
                Err(e) => {
                    tracing::debug!(agent = %self.agent.name, %e, "commit tripwires: no diff");
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

    /// The profile the agent was created on, read at each use so a change in
    /// Settings applies at once; `None` when unset or deleted.
    async fn profile(&self) -> Option<xherd_core::profile::Profile> {
        let id = self.agent.profile.as_deref()?;
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
                let Some(t) = self.agent.tasks.iter().find(|t| &t.id == id) else {
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
        let Some(c) = self.agent.commits.iter().find(|c| c.id == id) else {
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
            tracing::debug!(agent = %self.agent.name, name, "no file for progress lines");
            return;
        };
        match edit(&text) {
            Some(new) if new != text => {
                if let Err(e) = xherd_core::paths::write_atomic(&path, new.as_bytes(), None) {
                    tracing::warn!(agent = %self.agent.name, name, %e, "progress lines not written");
                }
            }
            Some(_) => {}
            None => {
                tracing::debug!(agent = %self.agent.name, name, "entry not found for progress lines")
            }
        }
    }

    fn workdir(&self) -> Result<PathBuf, Refusal> {
        self.agent
            .workdir
            .as_ref()
            .map(PathBuf::from)
            .ok_or_else(|| {
                refusal(
                    "no_workspace",
                    format!("{} has no workspace", self.agent.name),
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
            && xherd_flow::xap::flow_of(&self.agent, pack).is_ok()
        {
            let file = self.workdir()?.join(SPEC_DIR).join("memory.md");
            xherd_flow::memory::check(std::fs::read_to_string(file).ok().as_deref())?;
        }
        Ok(match cmd {
            XapCmd::GateOpen { .. } => {
                let wd = self.workdir()?;
                let mut facts = Vec::new();
                for d in gate_docs(&self.agent, pack, cmd, &wd)? {
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
                    .agent
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
        let out = xherd_core::exec::run(
            xherd_core::exec::Cmd::new(["sh", "-c", command.as_str()])
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
            .agent
            .pack
            .as_deref()
            .and_then(|p| self.env.packs.get(p))
            .and_then(|p| p.flow(self.agent.flow.as_deref().unwrap_or_default()));
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
                        .approved(&self.agent.id, &path, &sha256)
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
        let before = self.agent.status;
        let now = self.now();
        let kind = match &action {
            UserAction::Get => return Ok(()),
            UserAction::Start { port_base } => {
                self.start(*port_base).await?;
                return Ok(());
            }
            UserAction::Pause => {
                if next_status(self.agent.status, Trigger::Pause).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.agent.name, self.agent.status),
                    ));
                }
                if self.begin_pause().await {
                    "user.pause-requested"
                } else {
                    "user.pause"
                }
            }
            UserAction::PauseNow => {
                if next_status(self.agent.status, Trigger::Pause).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.agent.name, self.agent.status),
                    ));
                }
                if self.agent.status.is_live() {
                    self.end_session().await;
                }
                self.agent.pause_requested_at = None;
                self.agent.fire(Trigger::Pause, now);
                "user.pause-all"
            }
            UserAction::Resume => {
                if self.agent.fire(Trigger::Resume, now).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.agent.name, self.agent.status),
                    ));
                }
                self.prepare_resume().await;
                "user.resume"
            }
            UserAction::Cancel => {
                if next_status(self.agent.status, Trigger::Cancel).is_some() {
                    if self.agent.status.is_live() {
                        self.end_session().await;
                    }
                    self.agent.pause_requested_at = None;
                    self.agent.fire(Trigger::Cancel, now);
                } else {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.agent.name, self.agent.status),
                    ));
                }
                "user.cancel"
            }
            UserAction::Send { text } => {
                self.send(text, 1).await?;
                // A message while a question is open is its answer (xap.md `ask`).
                if matches!(self.agent.gate, Some(Gate::Question { .. })) {
                    self.agent.gate = None;
                    self.agent.generation += 1;
                    self.agent.fire(Trigger::UserInput, self.now());
                    "user.answer"
                } else {
                    "user.send"
                }
            }
            UserAction::Approve {
                reason,
                expect,
                message,
            } => {
                self.approve(reason.clone(), expect, message.clone(), false)
                    .await?;
                "user.approve"
            }
            UserAction::RequestChanges { comment } => {
                self.request_changes(comment).await?;
                "user.request-changes"
            }
            UserAction::SetAutonomy { level } => {
                self.set_autonomy(*level).await?;
                "user.autonomy"
            }
        };
        self.log(None, kind, None).await;
        self.commit(before).await;
        Ok(())
    }

    async fn start(&mut self, port_base: u16) -> Result<(), ActionError> {
        let before = self.agent.status;
        let now = self.now();
        if self.agent.fire(Trigger::SlotFree, now).is_none() {
            return Err(refuse(
                "not_allowed",
                format!("{} is {}", self.agent.name, self.agent.status),
            ));
        }
        let res = self.launch(port_base).await;
        if let Err(e) = &res {
            self.revoke_tokens().await;
            self.agent.fire(Trigger::SessionLost, self.now());
            self.log(None, "start-failed", Some(json!({"error": e.message})))
                .await;
            self.notify(
                "error",
                format!("{} could not start: {}", self.agent.name, e.message),
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
        let argv = self
            .agent
            .launch
            .clone()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| refuse("no_launch", "no runtime command to start"))?;
        let argv = match (self.resume_session.take(), &self.env.resume) {
            (Some(id), Some(r)) => match r.argv(&id, &xherd_core::runner::agent_env).await {
                Some(extra) => {
                    self.resume_unconfirmed = true;
                    argv.into_iter().chain(extra).collect()
                }
                None => argv,
            },
            _ => argv,
        };
        self.revoke_tokens().await;
        let token = mint_token(
            &self.env.store,
            TOKEN_AGENT,
            Some(&self.agent.id),
            &[SCOPE_HOOKS, SCOPE_XAP, SCOPE_CREDENTIAL],
            self.now(),
        )
        .await
        .map_err(|e| refuse("store", e.to_string()))?;
        let s = self.env.settings.get();
        let mut ctx = self.ctx();
        ctx.env = vec![
            ("XHERD_ENV".into(), "1".into()),
            ("XHERD_BIN".into(), s.xherd_bin.clone()),
            ("XHERD_URL".into(), s.xherd_url.clone()),
            ("XHERD_AGENT_ID".into(), self.agent.id.clone()),
            ("XHERD_AGENT_TOKEN".into(), token),
            ("XHERD_PORT_BASE".into(), port_base.to_string()),
        ];
        // The prompts call plain `xherd …`: a command with a variable in it
        // cannot be checked by the runtime and asks every time.
        if let Some(dir) = Path::new(&s.xherd_bin).parent().filter(|d| d.is_absolute()) {
            // The agents' PATH, not ours: the daemon's is the system's (R-1203).
            let old = xherd_core::runner::agent_env("PATH").unwrap_or_default();
            ctx.env.push((
                "PATH".into(),
                format!("{}:{old}", dir.display())
                    .trim_end_matches(':')
                    .to_string(),
            ));
        }
        self.env
            .driver
            .start(&ctx, &self.session_argv(argv).await)
            .await
            .map_err(|e| refuse("driver", e.0))?;
        self.agent.session_since = Some(self.now());
        Ok(())
    }

    /// The argv of a new session: the runtime, wrapped in `xherd toolchain
    /// exec` when the worktree's version files ask for tools (R-652). The plan
    /// is written at every start, so version files edited since count.
    async fn session_argv(&self, launch: Vec<String>) -> Vec<String> {
        use xherd_plugin::toolchain::{PLAN_FILE, plan};
        let Some(wd) = self.agent.workdir.as_deref().map(Path::new) else {
            return launch;
        };
        let settings = self.env.settings.get();
        let off = |n: &str| settings.disabled_plugins.iter().any(|d| d == n);
        if let Some(t) = &self.env.trust
            && !off(xherd_flow::approve::DEFAULT_RUNTIME)
            && let Err(e) = t.ensure(wd).await
        {
            tracing::warn!(agent = %self.agent.name, error = %e, "runtime trust not written");
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
            return launch;
        }
        let json = serde_json::to_vec_pretty(&p).expect("json");
        if let Err(e) = xherd_core::paths::write_atomic(&file, &json, None) {
            tracing::warn!(agent = %self.agent.name, error = %e, "toolchain plan not written; starting without it");
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
        match self.agent.status {
            Status::Blocked => {
                return Err(refuse(
                    "agent_blocked",
                    "the agent shows a dialog; answer it in the terminal first",
                ));
            }
            s if !s.is_live() => {
                return Err(refuse("not_live", format!("{} is {s}", self.agent.name)));
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
            self.agent.generation,
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
        let gate =
            self.agent.gate.clone().ok_or_else(|| {
                refuse("no_gate", format!("{} has no open gate", self.agent.name))
            })?;
        if let Gate::Question { .. } = gate {
            return Err(refuse(
                "no_gate",
                "the agent asked a question; answer it with `xherd send`",
            ));
        }
        match self.agent.status {
            Status::WaitingInput | Status::Idle => Ok(gate),
            Status::Parked if parked_ok => Ok(gate),
            s => Err(refuse(
                "not_waiting",
                format!(
                    "{} is {s}; act on the gate once it stops there",
                    self.agent.name
                ),
            )),
        }
    }

    /// Autonomy class of the open gate's stage (spec §6.3). `None`: unknown
    /// (flow `none`, a stage the pack does not declare), so a human decides.
    /// Pull-request rounds and the ship stage of a `hotfix` flow are always
    /// `human-only` (R-121, T1), whatever the pack says.
    fn gate_class(&self, gate: &Gate) -> Option<xherd_core::domain::Class> {
        use xherd_core::domain::{Class, Role};
        let pack = self.env.packs.get(self.agent.pack.as_deref()?)?;
        let flow_name = self.agent.flow.as_deref()?;
        let flow = pack.flow(flow_name)?;
        let stage = match gate {
            Gate::Docs { stage, .. } => flow.stage(stage)?,
            Gate::Commit(_) => flow.stages.iter().find(|s| s.role == Role::Implement)?,
            _ => return None,
        };
        let human_only =
            stage.role == Role::Pr || (flow_name == "hotfix" && stage.role == Role::Ship);
        Some(if human_only {
            Class::HumanOnly
        } else {
            stage.class
        })
    }

    /// Auto-approval (R-110, R-111): when the agent stops at a gate its class
    /// allows at the agent's autonomy level, approve it unless a tripwire
    /// holds it. The tripwires that look at the work (secrets, sensitive
    /// paths, diff size, scope, `verify:`, review findings) ran when the gate
    /// was opened and left `needs_human`; the run limit is checked here. This
    /// runs inline in the actor on the stop itself, so the gate cannot change
    /// meanwhile and a user message queued behind it is handled afterwards.
    async fn try_auto_approve(&mut self) {
        use xherd_core::autonomy::{may_auto_approve, run_limit};
        let level = self.agent.autonomy;
        if level == xherd_core::autonomy::Autonomy::Ask
            || !matches!(self.agent.status, Status::WaitingInput | Status::Idle)
        {
            return;
        }
        let Some(gate) = self.agent.gate.clone() else {
            return;
        };
        let Some(class) = self.gate_class(&gate) else {
            return;
        };
        if !may_auto_approve(level, class) {
            return;
        }
        let (label, held) = match &gate {
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
            _ => return,
        };
        if held.is_some() {
            // Already shown as "Held:"; nothing more to say.
            self.log(None, "auto-held", Some(json!({"gate": label, "why": held})))
                .await;
            return;
        }
        if let Some(why) = run_limit(
            self.agent.auto_in_row,
            self.env.settings.get().max_auto_approvals_in_row,
        ) {
            // Make the reason visible on the gate, and say so once.
            match &mut self.agent.gate {
                Some(Gate::Docs { needs_human, .. }) => *needs_human = Some(why.clone()),
                Some(Gate::Commit(c)) => c.needs_human = Some(why.clone()),
                _ => {}
            }
            if let Some(Gate::Commit(c)) = &self.agent.gate
                && let Some(mine) = self.agent.commits.iter_mut().find(|x| x.id == c.id)
            {
                mine.needs_human = Some(why.clone());
            }
            self.agent.generation += 1;
            self.log(None, "auto-held", Some(json!({"gate": label, "why": why})))
                .await;
            self.notify(
                "gate",
                format!("{}: {label} waits for you: {why}", self.agent.name),
            );
            let status = self.agent.status;
            self.commit(status).await;
            return;
        }
        let before = self.agent.status;
        let reason =
            format!("autonomy {level}: {label} is a {class:?} gate and no tripwire held it");
        match self
            .approve(Some(reason), &BTreeMap::new(), None, true)
            .await
        {
            Ok(()) => {
                self.notify(
                    "auto-approved",
                    format!(
                        "{}: approved {label} automatically (autonomy {level}); see the log",
                        self.agent.name
                    ),
                );
            }
            Err(e) => {
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
                        self.agent.name, e.message
                    ),
                );
            }
        }
        self.commit(before).await;
    }

    /// Change the autonomy level (R-109): rewrite `instructions.md`, and
    /// approve at once a gate the agent is already waiting at that the new
    /// level allows (no further stop would come).
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
        if self.agent.autonomy == level {
            return Ok(());
        }
        self.agent.autonomy = level;
        self.agent.auto_in_row = 0;
        self.agent.generation += 1;
        if let Ok(wd) = self.workdir() {
            let file = wd.join(".xherd").join("instructions.md");
            let extra = self.profile().await.and_then(|p| p.instructions);
            if let Err(e) = xherd_core::paths::write_atomic(
                &file,
                xherd_core::new::instructions_md(&self.agent, extra.as_deref()).as_bytes(),
                None,
            ) {
                tracing::warn!(agent = %self.agent.name, %e, "instructions.md not rewritten");
            }
        }
        self.log(None, "autonomy", Some(json!({"level": level})))
            .await;
        let before = self.agent.status;
        self.commit(before).await;
        // The agent learns the new level when its next session starts and
        // reads instructions.md; a running session keeps what it was told.
        self.try_auto_approve().await;
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
                agent_id: self.agent.id.clone(),
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
                    self.env.store.put_reviewed(&self.agent.id, file, t).await
                }
                _ => Ok(()),
            };
            stored
                .and(reviewed)
                .map_err(|e| refuse("store", e.to_string()))?;
        }

        if let Gate::Commit(c) = &gate
            && let Some(mine) = self.agent.commits.iter_mut().find(|x| x.id == c.id)
        {
            mine.status = CommitStatus::Approved;
        }
        self.agent.gate = None;
        self.agent.generation += 1;
        // A human approval restarts the run-limit count (R-120).
        self.agent.auto_in_row = if auto { self.agent.auto_in_row + 1 } else { 0 };
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
        match (self.agent.status, next) {
            (Status::Parked, next) => {
                // The next session starts with it once a slot is free (R-028).
                self.agent.next_prompt = next.map(|n| n.0);
                self.agent.fire(Trigger::Unpark, now);
            }
            (_, Some((cmd, _))) if self.fresh_sessions_on() => {
                match self.fresh_session(&cmd).await {
                    Ok(()) => {
                        self.agent.fire(Trigger::UserInput, now);
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
                                self.agent.name, e.message
                            ),
                        );
                    }
                }
            }
            // The approval stands even if typing fails; the user is told the command.
            (_, Some((cmd, enters))) => match self.send(&cmd, enters).await {
                Ok(()) => {
                    self.agent.fire(Trigger::UserInput, now);
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
                            self.agent.name, e.message
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
    async fn commit_for_user(&mut self, wd: &Path, id: &str, edited: Option<String>) {
        let Some(i) = self.agent.commits.iter().position(|c| c.id == id) else {
            return;
        };
        if let Some(m) = edited
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
        {
            self.agent.commits[i].message = Some(m);
        }
        let c = self.agent.commits[i].clone();
        let message = c.message.clone().unwrap_or_else(|| c.subject.clone());
        // Hooks (lint, tests) can take a while.
        let git = self.env.git.clone().with_timeout(Duration::from_secs(300));
        let error_file = wd.join(".xherd/commit-error.txt");
        match git.commit_paths(wd, &c.files, &message).await {
            Ok(sha) => {
                let _ = std::fs::remove_file(&error_file);
                self.agent.commits[i].sha = Some(sha.clone());
                self.agent.commits[i].status = CommitStatus::Done;
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
                        self.agent.name
                    ),
                );
            }
        }
    }

    /// Whether an approval replaces the session (R-023): fresh sessions are
    /// on and the agent follows a flow (R-024; flow `none` has no gates).
    /// Background tasks (also R-024) are not tracked yet.
    fn fresh_sessions_on(&self) -> bool {
        self.env.settings.get().fresh_sessions && self.agent.pack.is_some()
    }

    /// Replace the runtime process in the agent window by a new session
    /// (`respawn-window`; other windows keep running, R-023). The command is
    /// typed on the new session's `session-start`, like a first prompt. The
    /// session keeps its environment and agent token.
    async fn fresh_session(&mut self, cmd: &str) -> Result<(), ActionError> {
        let argv = self
            .agent
            .launch
            .clone()
            .filter(|a| !a.is_empty())
            .ok_or_else(|| refuse("no_launch", "no runtime command to start"))?;
        self.env
            .driver
            .respawn(&self.ctx(), &self.session_argv(argv).await)
            .await
            .map_err(|e| refuse("driver", e.0))?;
        self.delivery = None;
        self.agent.session = None;
        self.agent.session_since = Some(self.now());
        self.agent.next_prompt = Some(cmd.to_string());
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
        if self.agent.pause_requested_at.is_some() {
            return true;
        }
        if matches!(
            self.agent.status,
            Status::Running | Status::Idle | Status::WaitingInput
        ) && let Some(cmd) = self.pack_command("handoff")
        {
            let enters = self.enters_for(&cmd);
            match self.send(&cmd, enters).await {
                Ok(()) => {
                    self.agent.pause_requested_at = Some(now);
                    return true;
                }
                Err(e) => {
                    self.notify(
                        "error",
                        format!(
                            "{}: could not ask for a handoff ({}); paused without one",
                            self.agent.name, e.message
                        ),
                    );
                }
            }
        }
        if self.agent.status.is_live() {
            self.end_session().await;
        }
        self.agent.fire(Trigger::Pause, now);
        false
    }

    /// End the session of a pause that waited for a handoff.
    async fn finish_pause(&mut self, timed_out: bool) {
        let before = self.agent.status;
        self.end_session().await;
        self.agent.pause_requested_at = None;
        self.agent.fire(Trigger::Pause, self.now());
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
                    self.agent.name
                ),
            );
        }
        self.commit(before).await;
    }

    async fn prepare_resume(&mut self) {
        if self.agent.next_prompt.is_none() {
            self.agent.next_prompt = self.resume_command();
        }
        // After a handoff the next session starts fresh from the note (R-027).
        let handed_off =
            self.agent.handoff_at.take().is_some() && self.env.settings.get().fresh_sessions;
        self.resume_session = match (&self.env.resume, &self.agent.session) {
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

    fn resume_command(&self) -> Option<String> {
        self.pack_command("resume")
    }

    /// The pack's command `key` (`resume`, `prReview`…) for the agent's
    /// runtime; `None` for flow `none` or a pack without it.
    fn pack_command(&self, key: &str) -> Option<String> {
        let pack = self.env.packs.get(self.agent.pack.as_deref()?)?;
        let runtime = self
            .agent
            .runtime
            .as_deref()
            .unwrap_or(xherd_flow::approve::DEFAULT_RUNTIME);
        pack.command(runtime, pack.commands.get(key)?, &[])
            .map_err(
                |e| tracing::warn!(agent = %self.agent.name, key, error = %e, "no pack command"),
            )
            .ok()
    }

    fn next_command(&self, wd: &Path, gate: &Gate) -> Result<Option<(String, u32)>, ActionError> {
        let Some(pack) = self
            .agent
            .pack
            .as_deref()
            .and_then(|p| self.env.packs.get(p))
        else {
            return Ok(None);
        };
        let facts = xherd_flow::approve::DirFacts {
            spec_dir: xherd_flow::approve::spec_dir(wd),
            tasks: &self.agent.tasks,
        };
        xherd_flow::approve::next_command(pack, &self.agent, gate, &facts, false)
            .map_err(|e| refuse("flow", e))
    }

    async fn request_changes(&mut self, comment: &str) -> Result<(), ActionError> {
        if comment.trim().is_empty() {
            return Err(refuse("usage", "say what to change"));
        }
        let gate = self.gate_for_user(false)?;
        // Remember the documents as the user read them, so the UI can show
        // what the agent changed in answer (no version history is kept).
        if let (Gate::Docs { files, .. }, Ok(wd)) = (&gate, self.workdir()) {
            for f in files {
                if let Some(Ok(bytes)) = inside(&wd, &f.path).map(std::fs::read) {
                    let text = String::from_utf8_lossy(&bytes);
                    self.env
                        .store
                        .put_reviewed(&self.agent.id, &f.path, &text)
                        .await
                        .map_err(|e| refuse("store", e.to_string()))?;
                }
            }
        }
        let text = xherd_flow::approve::changes_message(&gate, comment);
        self.send(&text, 1).await?;
        // For the work log's rework count (spec §16.4): which gate it was.
        let label = match &gate {
            Gate::Docs { stage, .. } => stage.clone(),
            Gate::Commit(c) => format!("commit {}", c.id),
            _ => String::new(),
        };
        self.log(None, "change-request", Some(json!({"gate": label})))
            .await;
        self.agent.fire(Trigger::UserInput, self.now());
        Ok(())
    }

    /// The agent stopped without a declared gate: open one from the flow's
    /// documents if some exist unapproved (R-102) or changed since approved (R-107).
    async fn infer_gate(&mut self) {
        let packs = self.env.packs.clone();
        let Some(flow) = self
            .agent
            .pack
            .as_deref()
            .and_then(|p| packs.get(p))
            .and_then(|p| p.flow(self.agent.flow.as_deref().unwrap_or_default()))
        else {
            return;
        };
        let Ok(wd) = self.workdir() else { return };
        let ever = self
            .env
            .store
            .approved_files(&self.agent.id)
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
                        .approved(&self.agent.id, &path, h)
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
                self.agent.stage = Some(stage.clone());
            }
            self.log(None, kind, Some(json!({"gate": gate}))).await;
            self.agent.gate = Some(gate);
        }
    }

    // ---- ticks and async work --------------------------------------------------

    async fn on_tick(&mut self, t: TickKind) {
        let before = self.agent.status;
        let now = self.now();
        let minutes = |m: u64| (m as i64) * 60_000;
        match t {
            TickKind::SessionCheck { alive, recover } => {
                if self.agent.status.is_live() && !alive {
                    self.revoke_tokens().await;
                    self.delivery = None;
                    // Waiting at a gate you can approve: park, which is what
                    // a long wait does (approving starts a fresh session).
                    let at_gate = self.agent.status == Status::WaitingInput
                        && matches!(self.agent.gate, Some(Gate::Docs { .. } | Gate::Commit(_)));
                    if recover && at_gate {
                        self.agent.fire(Trigger::Park, now);
                        self.log(None, "parked", Some(json!({"why": "interrupted"})))
                            .await;
                        self.notify(
                            "recovery",
                            format!(
                                "{}: xherd restarted; approve the gate to continue",
                                self.agent.name
                            ),
                        );
                    } else {
                        self.agent.fire(Trigger::SessionLost, now);
                        self.log(None, "session-lost", None).await;
                        if recover {
                            // Same as Resume: a new session reloads the state.
                            self.agent.fire(Trigger::Resume, now);
                            self.prepare_resume().await;
                            self.notify(
                                "recovery",
                                format!("{}: xherd restarted; resuming", self.agent.name),
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
                                self.agent.fire(Trigger::Resume, now);
                                self.prepare_resume().await;
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
                                        self.agent.name,
                                        self.auto_resumes.len(),
                                        AUTO_RESUMES
                                    ),
                                );
                            } else {
                                self.notify(
                                    "recovery",
                                    format!(
                                        "{}: terminal session disappeared again; press Resume",
                                        self.agent.name
                                    ),
                                );
                            }
                        }
                    }
                    self.commit(before).await;
                }
            }
            TickKind::IdleCheck => {
                let m = self.env.settings.get().idle_alert_minutes;
                if self.agent.status == Status::Idle
                    && m > 0
                    && !self.agent.idle_alerted
                    && now - self.agent.status_since >= minutes(m)
                {
                    self.agent.idle_alerted = true;
                    self.log(None, "idle-alert", None).await;
                    self.notify(
                        "idle",
                        format!("{} has been idle for {m} min", self.agent.name),
                    );
                    self.commit(before).await;
                }
            }
            TickKind::SessionLimit => {
                let m = self.env.settings.get().max_session_minutes;
                if m > 0
                    && self.agent.status.is_live()
                    && self.agent.pause_requested_at.is_none()
                    && self
                        .agent
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
                            self.agent.name,
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
                if self.agent.status == Status::WaitingInput
                    && m > 0
                    && now - self.agent.status_since >= minutes(m)
                {
                    self.end_session().await;
                    self.agent.fire(Trigger::Park, now);
                    self.log(None, "parked", None).await;
                    self.commit(before).await;
                }
            }
            TickKind::ScreenScan => {
                if let Some(asked) = self.agent.pause_requested_at
                    && self.agent.status.is_live()
                    && now - asked >= self.env.settings.get().pause_timeout.as_millis() as i64
                {
                    self.finish_pause(true).await;
                }
                if self.agent.status.is_live() {
                    let (driver, ctx, tx, generation) = (
                        self.env.driver.clone(),
                        self.ctx(),
                        self.tx.clone(),
                        self.agent.generation,
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
            }
            TickKind::PrPoll => self.poll_pr(),
            TickKind::TodoTick => {}
        }
    }

    async fn on_work(&mut self, w: WorkResult) {
        match w {
            WorkResult::Screen(screen) => {
                match xherd_core::git_auth::failing_line(&screen) {
                    None => self.auth_screen_line = None,
                    Some(l) if self.auth_screen_line.as_deref() != Some(&l) => {
                        self.auth_screen_line = Some(l);
                        if self.git_access_failed("screen", &screen).await {
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
                let event = match (shown, self.agent.status == Status::Blocked) {
                    (true, false) => ReportEvent::Blocked,
                    (false, true) if self.agent.screen_blocked => ReportEvent::Working,
                    _ => return,
                };
                let r = Report {
                    id: xherd_core::new_id(),
                    agent_id: self.agent.id.clone(),
                    event,
                    payload: json!({"message": "dialog on screen"}),
                    seq: self.agent.last_daemon_seq + 1,
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
                    tracing::warn!(agent = %self.agent.name, error = %e, "pr-templates.json not written");
                }
            }
            WorkResult::PrTemplates(Err(e)) => {
                tracing::warn!(agent = %self.agent.name, error = %e, "PR templates lookup failed");
            }
            WorkResult::PrPoll(Ok((snap, comments))) => self.on_pr_poll(snap, comments).await,
            WorkResult::PrPoll(Err(e)) => {
                tracing::warn!(agent = %self.agent.name, error = %e, "PR poll failed");
                self.log(None, "pr-poll-failed", Some(json!({"error": e})))
                    .await;
                self.git_access_failed("pr-poll", &e).await;
            }
            WorkResult::Usage { session, tokens } => self.on_usage(&session, tokens).await,
            WorkResult::PrLookup(Ok(None)) => self.git_access_ok().await,
            WorkResult::PrLookup(Ok(Some(url))) => {
                self.git_access_ok().await;
                let before = self.agent.status;
                self.agent.pr_url = Some(url.clone());
                self.log(None, "pr-found", Some(json!({"url": url}))).await;
                self.commit(before).await;
                self.after_ship().await;
            }
            WorkResult::PrLookup(Err(e)) => {
                tracing::warn!(agent = %self.agent.name, error = %e, "PR lookup failed");
                self.log(None, "pr-lookup-failed", Some(json!({"error": e})))
                    .await;
                self.git_access_failed("pr-lookup", &e).await;
            }
            WorkResult::DeliveryTimeout { delivery } => match self.delivery {
                Some((d, false))
                    if d == delivery
                        && self.agent.status.is_live()
                        && self.agent.status != Status::Blocked =>
                {
                    tracing::info!(agent = %self.agent.name, "delivery not acknowledged; pressing Enter again");
                    let _ = self.env.driver.send(&self.ctx(), "", 1).await;
                    self.delivery = Some((d, true));
                    self.log(None, "delivery-retry", None).await;
                    self.notify(
                        "delivery",
                        format!(
                            "{}: message not acknowledged; pressed Enter again",
                            self.agent.name
                        ),
                    );
                }
                _ => {}
            },
        }
    }
}

/// The file of a canonical document path, if it exists inside the worktree
/// (symlinks resolved).
fn inside(wd: &Path, doc: &str) -> Option<PathBuf> {
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
