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
pub mod checks;
mod create;
pub mod driver;
pub mod git_access;
pub mod install;
pub mod open_in;
pub mod pr_round;
pub mod repos;
pub mod scheduler;
pub mod security;
pub mod settings;
pub mod supervisor;
pub mod support;

pub use actor::{ActionError, ActorSettings, ReportResult, TickKind, UserAction};
pub use create::{Resumer, TrustWriter};
pub use driver::{AgentCtx, Driver, FakeDriver, TerminalDriver};
pub use supervisor::SupervisorHandle;
pub use support::{Change, Clock, Hub, ManualClock, SCOPE_CREDENTIAL, SystemClock, hash_token};

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
    /// Running in the xherd container (`XHERD_SANDBOX=1`): toolchains are
    /// installed, not only resolved (R-653).
    pub sandbox: bool,
    /// Peer addresses treated like loopback: the container's gateway, through
    /// which the port published on the host's 127.0.0.1 arrives.
    pub local_peers: Vec<std::net::IpAddr>,
    /// Limit for a task's `verify:` command (tests shorten it).
    pub verify_timeout: std::time::Duration,
    /// Time between PR polls; `None` = `prPollMinutes` of the config (tests).
    pub pr_poll: Option<std::time::Duration>,
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
            sandbox: false,
            local_peers: Vec::new(),
            verify_timeout: actor::VERIFY_TIMEOUT,
            pr_poll: None,
        }
    }
}

/// A plugin's context: settings and secrets resolved (R-506), its folders.
fn plugin_context(
    p: &xherd_plugin::Plugin,
    paths: &Paths,
    config: &Config,
    secrets: &Secrets,
    xherd_bin: &str,
) -> xherd_plugin::Context {
    let m = &p.manifest;
    let global = config
        .plugin_settings
        .get(&m.name)
        .and_then(serde_json::Value::as_object);
    xherd_plugin::Context {
        settings: xherd_plugin::resolve_settings(m, global, None),
        secrets: xherd_plugin::resolve_secrets(m, &secrets.for_plugin(&m.name)),
        xherd_bin: xherd_bin.to_string(),
        xherd_url: config.url(),
        config_dir: paths.plugin_config(&m.name),
        state_dir: paths.plugin_state(&m.name),
        ..xherd_plugin::Context::default()
    }
}

/// The host plugin for pull requests: `github`, built in or replaced by an
/// external plugin of that name.
fn host_plugin(
    plugins: &Registry,
    paths: &Paths,
    config: &Config,
    secrets: &Secrets,
    xherd_bin: &str,
) -> Option<Arc<xherd_plugin::host::Host>> {
    let p = plugins
        .get(xherd_plugin::host::GITHUB)
        .filter(|p| p.manifest.kind == xherd_plugin::Kind::Host)?;
    let mut ctx = plugin_context(p, paths, config, secrets, xherd_bin);
    // Host plugins reuse the token of Settings → Git access (R-685).
    if !ctx.secrets.contains_key("token")
        && let Some(t) = secrets.git_token("github.com")
    {
        ctx.secrets.insert("token".into(), t.into());
    }
    Some(Arc::new(xherd_plugin::host::Host {
        plugin: p.clone(),
        ctx,
    }))
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
    /// Running in the xherd container (`XHERD_SANDBOX=1`).
    pub sandbox: bool,
    /// See `Options::local_peers`.
    pub local_peers: Arc<Vec<std::net::IpAddr>>,
    /// Open-in requests for the host companion (R-704).
    pub companion: open_in::Companion,
}

impl Daemon {
    /// Prepare the data root, migrate config and database, load plugins and
    /// agents, replay queued reports, reconcile sessions, start the scheduler.
    pub async fn start(paths: Paths, opts: Options) -> Result<Daemon, StartError> {
        // With a runner (the container, R-1203) nothing works without it:
        // stop, so the container restarts, rather than fail every agent.
        if let Some(sock) = xherd_core::runner::runner_socket() {
            let mut up = false;
            for _ in 0..60 {
                if xherd_core::runner::ping(sock).await {
                    up = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            if !up {
                return Err(StartError::Io(std::io::Error::other(format!(
                    "the runner at {} does not answer",
                    sock.display()
                ))));
            }
        }
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
            xherd_flow::BUILTIN.to_string(),
            xherd_flow::builtin(),
        )]));
        let host = host_plugin(&plugins, &paths, &config, &secrets, &opts.xherd_bin);
        let hub = support::hub();
        let settings = ActorSettings {
            idle_alert_minutes: config.idle_alert_minutes,
            gate_park_minutes: config.gate_park_minutes,
            fresh_sessions: config.fresh_sessions,
            xherd_bin: opts.xherd_bin.clone(),
            xherd_url: config.url(),
            delivery_ack: opts.delivery_ack,
            screen_rules: opts.screen_rules,
            toolchain_install: opts.sandbox,
            verify_timeout: opts.verify_timeout,
            pr_poll: opts
                .pr_poll
                .unwrap_or(std::time::Duration::from_secs(config.pr_poll_minutes * 60)),
            max_session_minutes: config.max_session_minutes,
            max_review_loops: config.max_review_loops,
            max_auto_approvals_in_row: config.max_auto_approvals_in_row,
            agent_token_budget: config.agent_token_budget,
            daily_token_budget: config.daily_token_budget,
            max_diff_lines: config.max_diff_lines,
            sensitive_paths: config.sensitive_paths.clone(),
        };
        let trust = plugins
            .get(xherd_flow::approve::DEFAULT_RUNTIME)
            .and_then(|p| xherd_plugin::runtime::Terminal::of(&p.manifest).ok())
            .and_then(|t| t.trust)
            .map(|trust| {
                Arc::new(TrustWriter {
                    trust,
                    root: config.workspace_root().join("worktrees"),
                })
            });
        let terminal = plugins
            .get(xherd_flow::approve::DEFAULT_RUNTIME)
            .and_then(|p| xherd_plugin::runtime::Terminal::of(&p.manifest).ok());
        let resume = terminal
            .as_ref()
            .and_then(|t| t.resume.clone())
            .map(|r| Arc::new(Resumer(r)));
        let usage = terminal.and_then(|t| t.usage).map(Arc::new);
        let toolchains = plugins
            .of_kind(xherd_plugin::Kind::Toolchain)
            .filter_map(|p| match xherd_plugin::toolchain::Toolchain::of(&p.manifest) {
                Ok(t) => Some((p.manifest.name.clone(), t)),
                Err(error) => {
                    tracing::warn!(plugin = %p.manifest.name, %error, "toolchain plugin unusable");
                    None
                }
            })
            .collect::<Vec<_>>();
        let env = actor::Env {
            store: store.clone(),
            driver: opts.driver.clone(),
            hub: hub.clone(),
            clock: opts.clock.clone(),
            settings: Arc::new(settings),
            sup: tokio::sync::mpsc::unbounded_channel().0, // replaced by the supervisor
            packs: packs.clone(),
            git: xherd_git::Git::new("git"),
            host,
            toolchains: Arc::new(toolchains),
            trust,
            resume,
            usage,
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
            sandbox: opts.sandbox,
            local_peers: Arc::new(opts.local_peers),
            companion: open_in::Companion::default(),
        };
        d.import_ledgers().await;
        let replayed = d.replay_queue().await?;
        if replayed > 0 {
            tracing::info!(replayed, "replayed queued reports");
        }
        d.reconcile_with(true).await;
        if let Err(e) = d.write_gitconfig() {
            tracing::warn!(error = %e.message, "managed git config not written");
        }
        if d.sandbox {
            // The container is set up without a shell: commands go where the
            // agents find them at every start (user-edited files are kept).
            let env = xherd_core::runner::agent_env;
            if let Err(e) = d
                .install(Some(xherd_flow::approve::DEFAULT_RUNTIME), None, &env)
                .await
            {
                tracing::warn!(error = %e.message, "installing the xherd commands failed");
            }
        }
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

    pub(crate) fn plugin_context(&self, p: &xherd_plugin::Plugin) -> xherd_plugin::Context {
        plugin_context(p, &self.paths, &self.config, &self.secrets, &self.xherd_bin)
    }

    /// The agent's documents for the UI: every `*.md` in the spec folder
    /// (`spec.md` form) plus the open gate's files, gate files first.
    pub async fn docs(&self, who: &str) -> Result<Vec<String>, ActionError> {
        let a = self.action(who, UserAction::Get).await?;
        let wd = workdir_of(&a)?;
        let mut out: Vec<String> = match &a.gate {
            Some(xherd_core::domain::Gate::Docs { files, .. }) => {
                files.iter().map(|f| f.path.clone()).collect()
            }
            _ => Vec::new(),
        };
        let dir = wd.join(xherd_flow::xap::SPEC_DIR);
        let mut found: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| n.ends_with(".md"))
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        for f in found {
            if !out.contains(&f) {
                out.push(f);
            }
        }
        Ok(out)
    }

    /// The findings of the agent's `review.md` for the UI, each with whether
    /// its fixing tasks are done. No review yet: an empty list. A malformed
    /// document: `error` says where (the gate refuses it, so this is the
    /// agent's work in progress).
    pub async fn findings(&self, who: &str) -> Result<serde_json::Value, ActionError> {
        let a = self.action(who, UserAction::Get).await?;
        let text = match self.doc(who, "review.md").await {
            Ok(d) => d["text"].as_str().unwrap_or_default().to_string(),
            Err(e) if e.code == "no_such_doc" => String::new(),
            Err(e) => return Err(e),
        };
        Ok(match xherd_flow::review::parse(&text) {
            Ok(list) => {
                let rows: Vec<_> = list
                    .iter()
                    .map(|f| {
                        let mut v = serde_json::to_value(f).unwrap_or_default();
                        v["resolved"] = xherd_flow::review::resolved(f, &a.tasks).into();
                        v
                    })
                    .collect();
                serde_json::json!({"findings": rows})
            }
            Err(e) => serde_json::json!({"findings": [], "error": e}),
        })
    }

    /// One document as the UI shows it (R-107): its text, the hash an
    /// approval would record (`doc_sha256`), and the text last approved, if
    /// any, for "changed since you reviewed". `path` as in XAP calls.
    pub async fn doc(&self, who: &str, path: &str) -> Result<serde_json::Value, ActionError> {
        const MAX: u64 = 2 * 1024 * 1024;
        let fail = |code: &'static str, message: String| ActionError { code, message };
        let a = self.action(who, UserAction::Get).await?;
        let wd = workdir_of(&a)?;
        let canon = xherd_flow::xap::doc_path(path, wd).map_err(|r| fail("bad_path", r.message))?;
        let abs = wd
            .join(xherd_flow::xap::worktree_rel(&canon))
            .canonicalize()
            .ok()
            .filter(|p| wd.canonicalize().is_ok_and(|w| p.starts_with(w)) && p.is_file())
            .ok_or_else(|| fail("no_such_doc", format!("{path} does not exist")))?;
        if std::fs::metadata(&abs).map(|m| m.len()).unwrap_or(0) > MAX {
            return Err(fail("too_large", format!("{path} is larger than 2 MiB")));
        }
        let bytes = std::fs::read(&abs).map_err(|e| fail("io", e.to_string()))?;
        let flow = a
            .pack
            .as_deref()
            .and_then(|p| self.packs.get(p))
            .and_then(|p| p.flow(a.flow.as_deref().unwrap_or_default()));
        let sha = xherd_flow::approve::doc_sha256(flow, wd, &canon, &bytes);
        let reviewed = self
            .store
            .reviewed(&a.id, &canon)
            .await
            .map_err(|e| fail("store", e.to_string()))?;
        // Images (diff preview) travel as a data URL; their bytes are not text.
        let mime = match abs
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => Some("image/png"),
            Some("jpg" | "jpeg") => Some("image/jpeg"),
            Some("gif") => Some("image/gif"),
            Some("webp") => Some("image/webp"),
            Some("svg") => Some("image/svg+xml"),
            _ => None,
        };
        let data_url = mime.map(|m| {
            use base64::Engine;
            format!(
                "data:{m};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            )
        });
        Ok(serde_json::json!({
            "path": canon,
            "text": if mime.is_some() { String::new() } else { String::from_utf8_lossy(&bytes).into_owned() },
            "dataUrl": data_url, "sha256": sha,
            "reviewed": reviewed,
        }))
    }

    /// What the user reviews before approving (xap.md §5, R-104): with a
    /// commit gate open, the diff of exactly the proposed files against HEAD
    /// (new and deleted files included) and the other uncommitted changes left
    /// out of it; at a ship gate, the branch as it will be pushed
    /// (`<base>...HEAD`) and the uncommitted files that would stay behind;
    /// otherwise the diff of every uncommitted change. Read-only git work, run
    /// here rather than in the actor.
    pub async fn diff(
        &self,
        who: &str,
        view: Option<&str>,
        commit: Option<&str>,
    ) -> Result<serde_json::Value, ActionError> {
        let fail = |code: &'static str, message: String| ActionError { code, message };
        let a = self.action(who, UserAction::Get).await?;
        let wd = a
            .workdir
            .as_deref()
            .map(std::path::Path::new)
            .ok_or_else(|| fail("no_workspace", format!("{} has no worktree", a.name)))?;
        let git = xherd_git::Git::new("git");
        let git_err = |e: xherd_git::GitError| fail("git", e.to_string());
        let head = git.rev_parse(wd, "HEAD").await.map_err(git_err)?;
        let changed = git.changed_files(wd, "HEAD").await.map_err(git_err)?;
        // Explicit views (the Changes tabs): everything uncommitted; the whole
        // branch against its base, committed and not; one commit.
        match view {
            None => {}
            Some("uncommitted") => {
                let diff = git.diff_proposed(wd, &changed).await.map_err(git_err)?;
                return Ok(serde_json::json!({
                    "agent": a.name, "head": head, "view": "uncommitted", "files": changed,
                    "others": Vec::<String>::new(), "diff": diff,
                }));
            }
            Some("branch") => {
                let base = a
                    .base
                    .as_deref()
                    .ok_or_else(|| fail("no_base", format!("{} has no base branch", a.name)))?;
                let mb = git
                    .run(wd, &["merge-base", base, "HEAD"])
                    .await
                    .map_err(git_err)?;
                // Tracked changes since the merge base (committed or not),
                // then the files git does not know yet.
                let mut files: Vec<String> = git
                    .run(wd, &["diff", "--name-only", &mb, "--"])
                    .await
                    .map_err(git_err)?
                    .lines()
                    .map(str::to_string)
                    .collect();
                let mut diff = git.diff(wd, Some(&mb), &[]).await.map_err(git_err)?;
                let untracked: Vec<String> = git
                    .run(wd, &["ls-files", "--others", "--exclude-standard"])
                    .await
                    .map_err(git_err)?
                    .lines()
                    .map(str::to_string)
                    .collect();
                if !untracked.is_empty() {
                    diff.push_str(&git.diff_proposed(wd, &untracked).await.map_err(git_err)?);
                    files.extend(untracked);
                }
                files.sort();
                files.dedup();
                return Ok(serde_json::json!({
                    "agent": a.name, "head": head, "view": "branch", "branch": format!("{base}...HEAD"),
                    "files": files, "others": Vec::<String>::new(), "diff": diff,
                }));
            }
            Some("commit") => {
                let id =
                    commit.ok_or_else(|| fail("bad_request", "commit=<id> is needed".into()))?;
                let c = a
                    .commits
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or_else(|| fail("commit_unknown", format!("no commit {id}")))?;
                let (files, diff) = match &c.sha {
                    Some(sha) => (
                        git.commit_files(wd, sha).await.map_err(git_err)?,
                        git.run(
                            wd,
                            &["show", "--no-color", "--no-ext-diff", "--format=", sha],
                        )
                        .await
                        .map_err(git_err)?,
                    ),
                    // Not committed yet: what would be committed.
                    None => (
                        c.files.clone(),
                        git.diff_proposed(wd, &c.files).await.map_err(git_err)?,
                    ),
                };
                return Ok(serde_json::json!({
                    "agent": a.name, "head": head, "view": "commit", "commit": c,
                    "files": files, "others": Vec::<String>::new(), "diff": diff,
                }));
            }
            Some(v) => {
                return Err(fail("bad_request", format!("unknown view {v:?}")));
            }
        }
        if let (
            Some(xherd_core::domain::Gate::Docs {
                role: xherd_core::domain::Role::Ship,
                ..
            }),
            Some(base),
        ) = (&a.gate, &a.base)
        {
            let range = format!("{base}...HEAD");
            let files: Vec<String> = git
                .run(wd, &["diff", "--name-only", &range, "--"])
                .await
                .map_err(git_err)?
                .lines()
                .map(str::to_string)
                .collect();
            let diff = git.diff(wd, Some(&range), &[]).await.map_err(git_err)?;
            return Ok(serde_json::json!({
                "agent": a.name, "head": head, "branch": range, "files": files,
                "others": changed, "diff": diff,
            }));
        }
        let (commit, files, others) = match &a.gate {
            Some(xherd_core::domain::Gate::Commit(c)) => {
                let others = changed
                    .into_iter()
                    .filter(|f| !c.files.contains(f))
                    .collect();
                (Some(c), c.files.clone(), others)
            }
            _ => (None, changed, Vec::new()),
        };
        let diff = git.diff_proposed(wd, &files).await.map_err(git_err)?;
        Ok(serde_json::json!({
            "agent": a.name, "head": head, "commit": commit, "files": files,
            "others": others, "diff": diff,
        }))
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
        self.reconcile_with(false).await;
    }

    /// `recover` at daemon start: agents whose session is gone (the container
    /// or terminal server restarted) are parked at their gate or resumed.
    pub async fn reconcile_with(&self, recover: bool) {
        match self.driver.sessions().await {
            Ok(alive) => self.sup.reconcile(alive, recover),
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

pub(crate) fn workdir_of(a: &Agent) -> Result<&std::path::Path, ActionError> {
    a.workdir
        .as_deref()
        .map(std::path::Path::new)
        .ok_or_else(|| ActionError {
            code: "no_workspace",
            message: format!("{} has no worktree", a.name),
        })
}
