//! The xherd daemon core: supervisor, agent actors, drivers, scheduler.
//! HTTP lives in `xherd-api`.

use std::collections::BTreeMap;
use std::sync::Arc;

use xherd_core::config::{Config, ConfigError, Secrets};
use xherd_core::paths::Paths;
use xherd_core::queue::Queued;
use xherd_core::report::{Origin, Report};
use xherd_core::xap::{Refusal, XapCall};
use xherd_core::{Agent, queue};
use xherd_flow::Pack;
use xherd_plugin::Registry;
use xherd_store::Store;

pub mod actor;
mod create;
pub mod driver;
pub mod install;
pub mod scheduler;
pub mod supervisor;
pub mod support;

pub use actor::{ActionError, ActorSettings, ReportResult, TickKind, UserAction};
pub use driver::{AgentCtx, Driver, FakeDriver, TerminalDriver};
pub use supervisor::SupervisorHandle;
pub use support::{Change, Clock, Hub, ManualClock, SystemClock, hash_token};

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Store(#[from] xherd_store::StoreError),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub struct Options {
    pub driver: Arc<dyn Driver>,
    pub clock: Arc<dyn Clock>,
    /// Absolute path of the `xherd` binary given to agents as `XHERD_BIN`.
    pub xherd_bin: String,
    pub builtin_plugins: Vec<xherd_plugin::Manifest>,
    /// `None` = no scheduler (tests drive ticks by hand).
    pub scheduler: Option<scheduler::Intervals>,
    /// How long a delivery may wait for its acknowledgement (R-022).
    pub delivery_ack: std::time::Duration,
    /// Screen rules for every agent, until runtime plugins supply them per runtime (M1).
    pub screen_rules: Option<xherd_term::ScreenRules>,
}

impl Options {
    pub fn new(driver: Arc<dyn Driver>, clock: Arc<dyn Clock>) -> Options {
        Options {
            driver,
            clock,
            xherd_bin: "xherd".into(),
            builtin_plugins: xherd_plugin::builtins(),
            scheduler: None,
            delivery_ack: std::time::Duration::from_secs(10),
            screen_rules: None,
        }
    }
}

/// Cheap to clone; every field is shared.
#[derive(Clone)]
pub struct Daemon {
    pub paths: Paths,
    pub config: Arc<Config>,
    pub secrets: Arc<Secrets>,
    pub store: Store,
    pub plugins: Arc<Registry>,
    pub packs: Arc<BTreeMap<String, Pack>>,
    pub hub: Hub,
    pub sup: SupervisorHandle,
    pub driver: Arc<dyn Driver>,
    pub clock: Arc<dyn Clock>,
    /// Absolute path of the `xherd` binary (hook commands, `XHERD_BIN`).
    pub xherd_bin: String,
}

impl Daemon {
    /// Prepare the data root, migrate config and database, load plugins and
    /// agents, replay queued reports, reconcile sessions, start the scheduler.
    pub async fn start(paths: Paths, opts: Options) -> Result<Daemon, StartError> {
        paths.ensure()?;
        let (mut config, migrated) = Config::load(&paths.config())?;
        let new_token = config.user_token.is_none();
        if new_token {
            config.user_token = Some(support::generate_token());
        }
        if new_token || migrated || !paths.config().exists() {
            config.save(&paths.config())?;
        }
        let secrets = Secrets::load(&paths.secrets())?;
        let store = Store::open(&paths.db(), &paths.backups())?;

        let user_token = config.user_token.clone().expect("set above");
        if store.find_token(&hash_token(&user_token)).await?.is_none() {
            store
                .insert_token(
                    &xherd_core::new_id(),
                    support::TOKEN_USER,
                    None,
                    &["*"],
                    &hash_token(&user_token),
                    opts.clock.now(),
                )
                .await?;
        }

        let plugins = Registry::load(opts.builtin_plugins, &paths.plugins());
        for b in plugins.broken() {
            tracing::warn!(plugin = %b.name, error = %b.error, "broken plugin");
        }

        let packs = Arc::new(BTreeMap::from([(
            xherd_flow::XHSPEC.to_string(),
            xherd_flow::xhspec(),
        )]));
        let hub = support::hub();
        let settings = ActorSettings {
            idle_alert_minutes: config.idle_alert_minutes,
            gate_park_minutes: config.gate_park_minutes,
            xherd_bin: opts.xherd_bin.clone(),
            xherd_url: config.url(),
            delivery_ack: opts.delivery_ack,
            screen_rules: opts.screen_rules,
        };
        let env = actor::Env {
            store: store.clone(),
            driver: opts.driver.clone(),
            hub: hub.clone(),
            clock: opts.clock.clone(),
            settings: Arc::new(settings),
            sup: tokio::sync::mpsc::unbounded_channel().0, // replaced by the supervisor
            packs: packs.clone(),
            git: xherd_git::Git::new("git"),
        };
        let agents = store.list_agents().await?;
        let sup = supervisor::spawn(agents, config.max_concurrent_agents, env);

        let d = Daemon {
            paths,
            config: Arc::new(config),
            secrets: Arc::new(secrets),
            store,
            plugins: Arc::new(plugins),
            packs,
            hub,
            sup,
            driver: opts.driver,
            clock: opts.clock,
            xherd_bin: opts.xherd_bin,
        };
        d.import_ledgers().await;
        let replayed = d.replay_queue().await?;
        if replayed > 0 {
            tracing::info!(replayed, "replayed queued reports");
        }
        d.reconcile().await;
        if let Some(iv) = opts.scheduler {
            scheduler::spawn(d.clone(), iv);
        }
        Ok(d)
    }

    pub async fn create_agent(&self, a: Agent) -> Result<Agent, ActionError> {
        self.sup.create(a).await
    }

    /// Route a report from an agent. `None` = unknown agent.
    pub async fn report(&self, r: Report) -> Option<ReportResult> {
        self.sup.report(r, Origin::Agent).await
    }

    /// Route an XAP call from an agent (design §6.5).
    pub async fn xap(&self, call: XapCall) -> Result<serde_json::Value, Refusal> {
        self.sup.xap(call).await
    }

    pub async fn action(&self, who: &str, a: UserAction) -> Result<Agent, ActionError> {
        self.sup.action(who, a).await
    }

    /// Apply `hook-queue.jsonl` in order (R-014). Returns how many were applied.
    pub async fn replay_queue(&self) -> std::io::Result<usize> {
        let path = self.paths.hook_queue();
        let items = tokio::task::spawn_blocking({
            let p = path.clone();
            move || queue::take(&p)
        })
        .await
        .map_err(std::io::Error::other)??;
        let mut applied = 0;
        for item in items {
            match item {
                Queued::Report(r) => {
                    if self.report(r).await == Some(ReportResult::Applied) {
                        applied += 1;
                    }
                }
                Queued::Xap(call) => match self.xap(call).await {
                    Ok(_) => applied += 1,
                    Err(e) => {
                        tracing::warn!(code = e.code, error = %e.message, "queued XAP call refused")
                    }
                },
            }
        }
        queue::done(&path)?;
        Ok(applied)
    }

    /// Re-read approval ledgers of agents with a workspace: rows missing from
    /// the database are added, so the file wins (design §3.4).
    async fn import_ledgers(&self) {
        let Ok(agents) = self.store.list_agents().await else {
            return;
        };
        for a in agents {
            let Some(wd) = &a.workdir else { continue };
            let Ok(text) = std::fs::read_to_string(
                std::path::Path::new(wd).join(".xherd/ledger/approvals.jsonl"),
            ) else {
                continue;
            };
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                    tracing::warn!(agent = %a.name, "skipping bad approval ledger line");
                    continue;
                };
                let (Some(file), Some(sha)) = (v["file"].as_str(), v["sha256"].as_str()) else {
                    continue;
                };
                let row = xherd_store::Approval {
                    agent_id: a.id.clone(),
                    file: file.into(),
                    sha256: sha.into(),
                    at: v["at"].as_i64().unwrap_or(0),
                    actor: v["actor"].as_str().unwrap_or("user").into(),
                    auto: v["auto"].as_bool().unwrap_or(false),
                    reason: v["reason"].as_str().map(str::to_string),
                };
                if let Err(e) = self.store.insert_approval(row).await {
                    tracing::error!(agent = %a.name, error = %e, "approval import failed");
                }
            }
        }
    }

    /// Compare live agents with the driver's sessions (R-011).
    pub async fn reconcile(&self) {
        match self.driver.sessions().await {
            Ok(alive) => self.sup.reconcile(alive),
            Err(e) => tracing::warn!(error = %e, "reconcile: cannot list sessions"),
        }
    }

    pub async fn prune_events(&self) {
        let days = self.config.event_retention_days as i64;
        let before = self.clock.now() - days * 24 * 3600 * 1000;
        match self.store.prune_events(before).await {
            Ok(n) if n > 0 => tracing::info!(pruned = n, "old events removed"),
            Ok(_) => {}
            Err(e) => tracing::error!(error = %e, "event pruning failed"),
        }
    }
}
