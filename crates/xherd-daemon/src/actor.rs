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
use xherd_core::report::{Effect, Origin, Outcome, Report, ReportEvent, apply_report};
use xherd_core::status::next_status;
use xherd_core::xap::{Refusal, XapCall, XapCmd, refuse as refusal};
use xherd_core::{Agent, Status, Trigger};
use xherd_flow::Pack;
use xherd_flow::xap::{CommitFact, DocFact, DocState, Observed};
use xherd_store::{Event, Store};
use xherd_term::ScreenRules;

use crate::driver::{AgentCtx, Driver};
use crate::support::{Change, Clock, Hub, SCOPE_HOOKS, SCOPE_XAP, TOKEN_AGENT, mint_token};

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
    Resume,
    Cancel,
    Send {
        text: String,
    },
    Get,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickKind {
    ScreenScan,
    IdleCheck,
    ParkCheck,
    SessionCheck {
        alive: bool,
    },
    /// TODO checks and PR polling arrive with those features (v1.x / M4).
    TodoTick,
    PrPoll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkResult {
    Screen(String),
    DeliveryTimeout { delivery: u64 },
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

/// Settings an actor reads (snapshot of config).
#[derive(Debug, Clone)]
pub struct ActorSettings {
    pub idle_alert_minutes: u64,
    pub gate_park_minutes: u64,
    pub delivery_ack: Duration,
    pub xherd_bin: String,
    pub xherd_url: String,
    pub screen_rules: Option<ScreenRules>,
}

impl Default for ActorSettings {
    fn default() -> ActorSettings {
        ActorSettings {
            idle_alert_minutes: 15,
            gate_park_minutes: 30,
            delivery_ack: Duration::from_secs(10),
            xherd_bin: "xherd".into(),
            xherd_url: "http://127.0.0.1:4740".into(),
            screen_rules: None,
        }
    }
}

#[derive(Clone)]
pub struct Env {
    pub store: Store,
    pub driver: Arc<dyn Driver>,
    pub hub: Hub,
    pub clock: Arc<dyn Clock>,
    pub settings: Arc<ActorSettings>,
    pub sup: mpsc::UnboundedSender<SupEvent>,
    /// Flow packs by name.
    pub packs: Arc<BTreeMap<String, Pack>>,
    pub git: xherd_git::Git,
}

pub type AgentTx = mpsc::UnboundedSender<Msg>;

pub fn spawn(agent: Agent, env: Env) -> AgentTx {
    let (tx, rx) = mpsc::unbounded_channel();
    let actor = Actor {
        agent,
        env,
        tx: tx.clone(),
        delivery: None,
        next_delivery: 0,
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
    next_delivery: u64,
}

impl Actor {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Msg>) {
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

    fn notify(&self, text: String) {
        let _ = self.env.hub.send(Change::Notify {
            agent_id: self.agent.id.clone(),
            text,
        });
    }

    /// Persist and publish; tell the supervisor when the status moved.
    async fn commit(&self, before: Status) {
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
                    self.notify(text);
                }
                Effect::DeliveryAck => self.delivery = None,
                Effect::UserPrompted => self.log(None, "user-prompted", None).await,
                // Token usage, auto-approval, PR lookup: M1+.
                Effect::AfterStop => {}
            }
        }
        result
    }

    // ---- XAP -------------------------------------------------------------------

    /// Validate and apply one XAP call (design §6.5). Accepted calls are logged
    /// as `xap.<cmd>` under the call id, which also de-duplicates replays.
    async fn on_xap(&mut self, call: XapCall) -> Result<Value, Refusal> {
        if !self.agent.status.is_live() {
            return Err(refusal(
                "not_live",
                format!("{} is {}", self.agent.name, self.agent.status),
            ));
        }
        if self.env.store.event_exists(&call.id).await.unwrap_or(false) {
            return Ok(json!({"changed": false, "duplicate": true}));
        }
        let packs = self.env.packs.clone();
        let pack = self.agent.pack.as_deref().and_then(|p| packs.get(p));
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
        if changed {
            self.commit(before).await;
        }
        Ok(json!({"changed": changed, "stage": self.agent.stage, "gate": self.agent.gate}))
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
        use xherd_flow::xap::{HANDOFF, doc_path, gate_docs};
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

    async fn doc_fact(&self, wd: &Path, path: String) -> DocFact {
        let rel = xherd_flow::xap::worktree_rel(&path);
        let state = match (wd.join(&rel).canonicalize(), wd.canonicalize()) {
            (Ok(abs), Ok(root)) if !abs.starts_with(&root) => DocState::Outside,
            (Ok(abs), Ok(_)) if abs.is_file() => match std::fs::read(&abs) {
                Ok(bytes) => {
                    let sha256 = xherd_core::hash::content_sha256(&bytes);
                    let frontmatter_error = path
                        .ends_with(".md")
                        .then(|| {
                            xherd_flow::frontmatter::parse(&String::from_utf8_lossy(&bytes)).err()
                        })
                        .flatten();
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
                // M0 implements the flow-`none` pause (R-026): no handoff yet.
                if next_status(self.agent.status, Trigger::Pause).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.agent.name, self.agent.status),
                    ));
                }
                if self.agent.status.is_live() {
                    self.end_session().await;
                }
                self.agent.fire(Trigger::Pause, now);
                "user.pause"
            }
            UserAction::Resume => {
                if self.agent.fire(Trigger::Resume, now).is_none() {
                    return Err(refuse(
                        "not_allowed",
                        format!("{} is {}", self.agent.name, self.agent.status),
                    ));
                }
                "user.resume"
            }
            UserAction::Cancel => {
                if next_status(self.agent.status, Trigger::Cancel).is_some() {
                    if self.agent.status.is_live() {
                        self.end_session().await;
                    }
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
                self.send(text).await?;
                "user.send"
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
            self.notify(format!(
                "{} could not start: {}",
                self.agent.name, e.message
            ));
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
        self.revoke_tokens().await;
        let token = mint_token(
            &self.env.store,
            TOKEN_AGENT,
            Some(&self.agent.id),
            &[SCOPE_HOOKS, SCOPE_XAP],
            self.now(),
        )
        .await
        .map_err(|e| refuse("store", e.to_string()))?;
        let s = &self.env.settings;
        let mut ctx = self.ctx();
        ctx.env = vec![
            ("XHERD_ENV".into(), "1".into()),
            ("XHERD_BIN".into(), s.xherd_bin.clone()),
            ("XHERD_URL".into(), s.xherd_url.clone()),
            ("XHERD_AGENT_ID".into(), self.agent.id.clone()),
            ("XHERD_AGENT_TOKEN".into(), token),
            ("XHERD_PORT_BASE".into(), port_base.to_string()),
        ];
        self.env
            .driver
            .start(&ctx, &argv)
            .await
            .map_err(|e| refuse("driver", e.0))
    }

    async fn send(&mut self, text: &str) -> Result<(), ActionError> {
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
            .send(&self.ctx(), text, 1)
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
            self.env.settings.delivery_ack,
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

    // ---- ticks and async work --------------------------------------------------

    async fn on_tick(&mut self, t: TickKind) {
        let before = self.agent.status;
        let now = self.now();
        let minutes = |m: u64| (m as i64) * 60_000;
        match t {
            TickKind::SessionCheck { alive } => {
                if self.agent.status.is_live() && !alive {
                    self.revoke_tokens().await;
                    self.delivery = None;
                    self.agent.fire(Trigger::SessionLost, now);
                    self.log(None, "session-lost", None).await;
                    self.notify(format!("{}: terminal session disappeared", self.agent.name));
                    self.commit(before).await;
                }
            }
            TickKind::IdleCheck => {
                let m = self.env.settings.idle_alert_minutes;
                if self.agent.status == Status::Idle
                    && m > 0
                    && !self.agent.idle_alerted
                    && now - self.agent.status_since >= minutes(m)
                {
                    self.agent.idle_alerted = true;
                    self.log(None, "idle-alert", None).await;
                    self.notify(format!("{} has been idle for {m} min", self.agent.name));
                    self.commit(before).await;
                }
            }
            TickKind::ParkCheck => {
                let m = self.env.settings.gate_park_minutes;
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
                if self.agent.status.is_live()
                    && self
                        .env
                        .settings
                        .screen_rules
                        .as_ref()
                        .is_some_and(|r| !r.is_empty())
                {
                    let (driver, ctx, tx, generation) = (
                        self.env.driver.clone(),
                        self.ctx(),
                        self.tx.clone(),
                        self.agent.generation,
                    );
                    let lines = self
                        .env
                        .settings
                        .screen_rules
                        .as_ref()
                        .map_or(40, |r| r.last_lines as u32 * 4);
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
            TickKind::TodoTick | TickKind::PrPoll => {}
        }
    }

    async fn on_work(&mut self, w: WorkResult) {
        match w {
            WorkResult::Screen(screen) => {
                let Some(rules) = self.env.settings.screen_rules.as_ref() else {
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
                    self.notify(format!(
                        "{}: message not acknowledged; pressed Enter again",
                        self.agent.name
                    ));
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
