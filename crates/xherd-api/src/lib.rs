//! HTTP API (design §7): health, state, agents and their actions,
//! `POST /hooks` for status reports, `POST /xap` for the agent protocol,
//! `GET /ws` for the live state stream, and the web UI at `/`.

use std::net::SocketAddr;

use axum::extract::connect_info::ConnectInfo;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use xherd_core::report::Report;
use xherd_core::xap::XapCall;
use xherd_daemon::support::{SCOPE_HOOKS, SCOPE_XAP, TOKEN_AGENT, TOKEN_USER};
use xherd_daemon::{ActionError, Daemon, UserAction, hash_token};

mod term;
mod ui;
mod ws;

pub const BODY_LIMIT: usize = 5 * 1024 * 1024;

pub fn router(d: Daemon) -> Router {
    Router::new()
        .route("/", get(ui::index))
        .route("/assets/{*rest}", get(ui::asset))
        .route("/api/v1/healthz", get(healthz))
        .route("/api/v1/ws", get(ws::state_stream))
        .route("/api/v1/ws/term/{id}", get(term::terminal))
        .route("/api/v1/ws/setup/{id}", get(term::setup_terminal))
        .route("/api/v1/state", get(state))
        .route("/api/v1/backups", get(backups).post(new_backup))
        .route("/api/v1/agents", get(list_agents).post(new_agent))
        .route("/api/v1/agents/pause-all", post(pause_all))
        .route("/api/v1/agents/{id}", get(get_agent))
        .route("/api/v1/agents/{id}/diff", get(diff))
        .route("/api/v1/agents/{id}/docs", get(docs))
        .route("/api/v1/agents/{id}/findings", get(findings))
        .route("/api/v1/agents/{id}/pr-review", get(pr_review))
        .route(
            "/api/v1/agents/{id}/pr-decisions",
            axum::routing::put(put_pr_decisions),
        )
        .route("/api/v1/agents/{id}/file", get(file))
        .route(
            "/api/v1/agents/{id}/viewed",
            get(get_viewed).post(post_viewed),
        )
        .route("/api/v1/agents/{id}/open-targets", get(open_targets))
        .route("/api/v1/agents/{id}/open", post(open_in))
        .route("/api/v1/agents/{id}/{action}", post(agent_action))
        .route("/api/v1/companion/next", get(companion_next))
        .route("/api/v1/open-in", get(open_in_view))
        .route("/api/v1/repos", get(repos))
        .route("/api/v1/repos/resolve", post(resolve_repo))
        .route("/api/v1/install", post(install))
        .route("/api/v1/config", get(get_config).put(put_config))
        .route("/api/v1/checks", get(checks))
        .route("/api/v1/security", get(security))
        .route("/api/v1/profiles", get(profiles_list).post(profile_create))
        .route("/api/v1/profiles/export", get(profiles_export))
        .route("/api/v1/profiles/import", post(profiles_import))
        .route(
            "/api/v1/profiles/{id}",
            axum::routing::put(profile_update).delete(profile_delete),
        )
        .route("/api/v1/todos", get(todos_list).post(todo_create))
        .route(
            "/api/v1/todos/{id}",
            axum::routing::put(todo_update).delete(todo_delete),
        )
        .route("/api/v1/todos/{id}/complete", post(todo_complete))
        .route("/api/v1/todos/{id}/check", post(todo_check))
        .route("/api/v1/todos/{id}/reschedule", post(todo_reschedule))
        .route("/api/v1/worklog", get(worklog))
        .route("/api/v1/agents/{id}/user-time", post(user_time))
        .route("/api/v1/reviews", get(reviews))
        .route("/api/v1/reviews/seen", post(reviews_seen))
        .route("/api/v1/checks/{id}/fix", post(fix_check))
        .route("/api/v1/git/hosts", get(git_hosts).put(put_git_hosts))
        .route("/api/v1/git/doctor", post(git_doctor))
        .route("/api/v1/agents/{id}/git-access", get(agent_git_access))
        .route("/api/v1/git/known-hosts", post(git_known_host))
        .route("/api/v1/git/login", post(git_login))
        .route("/api/v1/plugins/{name}/login", post(plugin_login))
        .route("/api/v1/toolchains", get(toolchains))
        .route("/api/v1/toolchains/install", post(toolchain_install))
        .route(
            "/api/v1/toolchains/{tool}/versions",
            get(toolchain_versions),
        )
        .route("/api/v1/git/credential", post(git_credential))
        .route("/api/v1/hooks", post(hooks))
        .route("/api/v1/xap", post(xap))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(d)
}

/// Serve until `shutdown` resolves.
pub async fn serve(
    d: Daemon,
    listener: tokio::net::TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        router(d).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
}

// ---- errors ------------------------------------------------------------------

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    code: String,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            code: code.into(),
            message: message.into(),
        }
    }
    fn internal(e: impl std::fmt::Display) -> ApiError {
        tracing::error!(error = %e, "request failed");
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string())
    }
}

impl From<ActionError> for ApiError {
    fn from(e: ActionError) -> ApiError {
        let status = match e.code {
            "unknown_agent" => StatusCode::NOT_FOUND,
            "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
            "usage" => StatusCode::BAD_REQUEST,
            "bad_key" | "bad_slug" | "unknown_flow" | "prompt_required" | "repo_url"
            | "not_a_repo" | "base_unresolved" | "runtime_unusable" | "runtime_hooks"
            | "unknown_pack" | "unknown_runtime" => StatusCode::UNPROCESSABLE_ENTITY,
            "bad_path" | "bad_config" => StatusCode::BAD_REQUEST,
            "not_fixable" | "no_companion" => StatusCode::NOT_FOUND,
            "no_such_doc" => StatusCode::NOT_FOUND,
            "too_large" => StatusCode::PAYLOAD_TOO_LARGE,
            "io" | "git" | "install" => StatusCode::INTERNAL_SERVER_ERROR,
            "store" | "driver" | "ledger" | "flow" => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::CONFLICT,
        };
        ApiError::new(status, e.code, e.message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"error": self.message, "code": self.code})),
        )
            .into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

// ---- auth --------------------------------------------------------------------

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// Loopback peer, local Host header, no proxy headers (design §7.2).
///
/// In the container (`sandbox`) loopback is where the agents are, so it never
/// counts: only the gateway the published port arrives from (`local_peers`)
/// is the local user (R-1203, T2). The CLI there uses its token.
async fn is_local(parts: &mut Parts, d: &Daemon) -> bool {
    let peer_local = ConnectInfo::<SocketAddr>::from_request_parts(parts, d)
        .await
        .is_ok_and(|ConnectInfo(a)| {
            let ip = a.ip().to_canonical();
            (!d.sandbox && ip.is_loopback()) || d.local_peers.contains(&ip)
        });
    let host = parts
        .headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let host_name = host.rsplit_once(':').map_or(host, |(h, p)| {
        if p.chars().all(|c| c.is_ascii_digit()) {
            h
        } else {
            host
        }
    });
    let host_local = matches!(host_name, "localhost" | "127.0.0.1" | "[::1]");
    let proxied = [
        "forwarded",
        "x-forwarded-for",
        "x-forwarded-host",
        "x-real-ip",
    ]
    .iter()
    .any(|h| parts.headers.contains_key(*h));
    peer_local && host_local && !proxied && origin_local(&parts.headers)
}

/// A browser request from another site carries its `Origin`: only local
/// origins may use the local user's rights (no cross-site requests or
/// websockets against the daemon).
fn origin_local(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(rest) = origin.to_str().ok().and_then(|o| o.strip_prefix("http://")) else {
        return false;
    };
    let host = match rest.rsplit_once(':') {
        Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => h,
        _ => rest,
    };
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// The local user: full rights.
pub struct User;

impl FromRequestParts<Daemon> for User {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, d: &Daemon) -> Result<User, ApiError> {
        if let Some(t) = bearer(&parts.headers) {
            let row = d
                .store
                .find_token(&hash_token(t))
                .await
                .map_err(ApiError::internal)?;
            match row {
                Some(row) if row.kind == TOKEN_USER && !row.revoked => {
                    let _ = d.store.touch_token(&row.id, d.clock.now()).await;
                    return Ok(User);
                }
                // A token this daemon never issued (e.g. the host CLI's token
                // of a native install talking to the container) adds nothing
                // to a local request, which needs none.
                None if is_local(parts, d).await => return Ok(User),
                _ => {}
            }
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "bad_token",
                "invalid or revoked token",
            ));
        }
        if is_local(parts, d).await {
            return Ok(User);
        }
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "auth_required",
            "authentication required",
        ))
    }
}

/// An agent token: the agent it belongs to and its scopes.
pub struct AgentAuth {
    pub agent_id: String,
    pub scopes: Vec<String>,
}

impl AgentAuth {
    fn require(&self, scope: &str) -> ApiResult<()> {
        if self.scopes.iter().any(|s| s == scope) {
            Ok(())
        } else {
            Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "scope",
                format!("token lacks the {scope} scope"),
            ))
        }
    }
}

impl FromRequestParts<Daemon> for AgentAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, d: &Daemon) -> Result<AgentAuth, ApiError> {
        let deny = || {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "bad_token",
                "invalid or revoked agent token",
            )
        };
        let t = bearer(&parts.headers).ok_or_else(deny)?;
        let row = d
            .store
            .find_token(&hash_token(t))
            .await
            .map_err(ApiError::internal)?;
        match row {
            Some(r) if r.kind == TOKEN_AGENT && !r.revoked => Ok(AgentAuth {
                agent_id: r.subject.unwrap_or_default(),
                scopes: r.scopes,
            }),
            _ => Err(deny()),
        }
    }
}

// ---- handlers ------------------------------------------------------------------

async fn healthz() -> Json<Value> {
    Json(
        json!({"ok": true, "version": xherd_core::VERSION, "buildVersion": xherd_core::BUILD_VERSION}),
    )
}

async fn state(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    let agents = d.store.list_agents().await.map_err(ApiError::internal)?;
    let mut config = serde_json::to_value(&*d.config).map_err(ApiError::internal)?;
    if let Some(c) = config.as_object_mut() {
        c.remove("userToken");
    }
    let plugins: Vec<Value> = d
        .plugins
        .all()
        .map(|p| {
            let settings: Vec<Value> = p
                .manifest
                .settings
                .iter()
                .map(|s| {
                    json!({"key": s.key, "label": s.label, "type": s.kind, "default": s.default,
                           "secret": s.secret, "required": s.required})
                })
                .collect();
            json!({"name": p.name(), "kind": p.manifest.kind, "version": p.manifest.version,
                   "label": p.manifest.label, "description": p.manifest.description,
                   "builtin": p.builtin(), "settings": settings,
                   "enabled": d.plugin_enabled(p.name()), "login": p.manifest.login.is_some()})
        })
        .collect();
    Ok(Json(json!({
        "version": xherd_core::VERSION,
        "buildVersion": xherd_core::BUILD_VERSION,
        "platform": std::env::consts::OS,
        "config": config,
        "agents": agents,
        "plugins": plugins,
        "brokenPlugins": d.plugins.broken(),
        "flows": flows(&d),
        "sandbox": d.sandbox,
    })))
}

/// Every pack flow with its stages, for progress lines.
fn flows(d: &Daemon) -> Vec<Value> {
    d.packs
        .values()
        .flat_map(|p| {
            p.flows.iter().map(|(name, f)| {
                let stages: Vec<Value> = f
                    .stages
                    .iter()
                    .map(|s| json!({"id": s.id, "role": s.role, "class": s.class, "gate": s.gate.is_some()}))
                    .collect();
                json!({"pack": p.name, "flow": name, "stages": stages})
            })
        })
        .collect()
}

async fn list_agents(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    let agents = d.store.list_agents().await.map_err(ApiError::internal)?;
    Ok(Json(json!({"agents": agents})))
}

/// Create an agent (spec §5.1). Body: `NewAgent`, unknown fields refused.
async fn new_agent(
    _: User,
    State(d): State<Daemon>,
    Json(req): Json<xherd_core::new::NewAgent>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let agent = d.new_agent(req).await?;
    Ok((StatusCode::CREATED, Json(json!({"agent": agent}))))
}

/// Repositories the New agent screen offers (earlier agents', xherd's clones).
async fn repos(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"repos": d.known_repos().await?})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveReq {
    repo: String,
}

/// A path or URL → the local repository (a URL is cloned or fetched first),
/// its branches and default base (R-001, R-605).
async fn resolve_repo(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<ResolveReq>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.resolve_repo(&r.repo).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InstallReq {
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    pack: Option<String>,
}

/// `xherd install`: the pack's commands for the runtimes, written where the
/// daemon (and so the agents) run.
async fn install(
    _: User,
    State(d): State<Daemon>,
    Json(req): Json<InstallReq>,
) -> ApiResult<Json<Value>> {
    let env = xherd_core::runner::agent_env;
    let installed = d
        .install(req.runtime.as_deref(), req.pack.as_deref(), &env)
        .await?;
    Ok(Json(json!({"installed": installed})))
}

async fn get_config(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(d.config_view()?))
}

/// Saved to `config.json`; applies at the next daemon start (`restartNeeded`).
async fn put_config(
    _: User,
    State(d): State<Daemon>,
    Json(patch): Json<xherd_daemon::settings::ConfigPatch>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.save_config(patch)?))
}

async fn profiles_list(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"profiles": d.profiles().await?})))
}

async fn profile_create(
    _: User,
    State(d): State<Daemon>,
    Json(p): Json<xherd_core::profile::Profile>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.create_profile(p).await?)))
}

async fn profile_update(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(p): Json<xherd_core::profile::Profile>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.update_profile(&id, p).await?)))
}

async fn profile_delete(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    d.delete_profile(&id).await?;
    Ok(Json(json!({"ok": true})))
}

async fn profiles_export(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(d.export_profiles().await?))
}

async fn profiles_import(
    _: User,
    State(d): State<Daemon>,
    Json(i): Json<xherd_daemon::profiles::Import>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.import_profiles(i).await?))
}

async fn todos_list(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"todos": d.todos().await?})))
}

async fn todo_create(
    _: User,
    State(d): State<Daemon>,
    Json(i): Json<xherd_daemon::todos::TodoInput>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.create_todo(i).await?)))
}

async fn todo_update(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(i): Json<xherd_daemon::todos::TodoInput>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.update_todo(&id, i).await?)))
}

async fn todo_delete(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    d.delete_todo(&id).await?;
    Ok(Json(json!({"ok": true})))
}

async fn todo_complete(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.complete_todo(&id).await?)))
}

async fn todo_check(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.check_todo(&id).await?)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RescheduleBody {
    due: i64,
}

async fn todo_reschedule(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(b): Json<RescheduleBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.reschedule_todo(&id, b.due).await?)))
}

#[derive(Deserialize)]
struct WorklogQuery {
    days: Option<u32>,
}

/// `GET /api/v1/worklog?days=N` (spec §16.4): time per day and ticket, and
/// the delivery metrics per ticket.
async fn worklog(
    _: User,
    State(d): State<Daemon>,
    axum::extract::Query(q): axum::extract::Query<WorklogQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.worklog(q.days.unwrap_or(14)).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserTimeBody {
    ms: u64,
}

/// `POST /agents/:id/user-time`: the agent page was open for `ms` (heartbeat).
async fn user_time(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(b): Json<UserTimeBody>,
) -> ApiResult<Json<Value>> {
    d.add_user_time(&id, b.ms).await?;
    Ok(Json(json!({"ok": true})))
}

/// `GET /api/v1/reviews` (R-111): what xherd approved by itself, newest first.
async fn reviews(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    let events =
        d.store.auto_approvals(100).await.map_err(|e| {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "store", e.to_string())
        })?;
    let seen =
        d.store.reviews_seen().await.map_err(|e| {
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "store", e.to_string())
        })?;
    let items: Vec<Value> = events
        .into_iter()
        .map(|e| {
            let p = e.payload.unwrap_or_default();
            let seen = seen.contains(&seen_id(&e.agent_id, e.at, p["gate"].as_str().unwrap_or("")));
            json!({"agentId": e.agent_id, "at": e.at, "gate": p["gate"], "files": p["files"], "reason": p["reason"], "seen": seen})
        })
        .collect();
    Ok(Json(json!({"reviews": items})))
}

/// The event id that records an auto-approval as seen (one per item).
fn seen_id(agent_id: &str, at: i64, gate: &str) -> String {
    format!("seen:{agent_id}:{at}:{gate}")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SeenItem {
    agent_id: String,
    at: i64,
    gate: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SeenBody {
    items: Vec<SeenItem>,
}

/// `POST /api/v1/reviews/seen` (R-111, "Mark seen"): the listed
/// auto-approvals are marked as looked at.
async fn reviews_seen(
    _: User,
    State(d): State<Daemon>,
    Json(b): Json<SeenBody>,
) -> ApiResult<Json<Value>> {
    let now = d.clock.now();
    for i in b.items.iter().take(500) {
        d.store
            .insert_event(xherd_store::Event {
                id: seen_id(&i.agent_id, i.at, &i.gate),
                agent_id: i.agent_id.clone(),
                kind: "review-seen".into(),
                payload: None,
                at: now,
            })
            .await
            .map_err(|e| {
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "store", e.to_string())
            })?;
    }
    Ok(Json(json!({"ok": true})))
}

/// `GET /api/v1/backups` (R-1402): the archives in the backup folder, newest first.
async fn backups(_: User, State(d): State<Daemon>) -> Json<Value> {
    let dir = d.backup_dir();
    let items: Vec<Value> = xherd_daemon::backup::list(&dir)
        .into_iter()
        .map(|(name, size, at)| json!({"name": name, "size": size, "at": at}))
        .collect();
    Json(json!({"dir": dir, "backups": items}))
}

#[derive(serde::Deserialize, Default)]
struct NewBackup {
    /// `manual` (default) or `update` (before an update; the last 3 are kept, R-1405).
    label: Option<String>,
}

/// `POST /api/v1/backups` (R-1401): take a backup now.
async fn new_backup(
    _: User,
    State(d): State<Daemon>,
    body: Option<Json<NewBackup>>,
) -> ApiResult<Json<Value>> {
    let label = body
        .and_then(|b| b.0.label)
        .unwrap_or_else(|| "manual".into());
    if !matches!(label.as_str(), "manual" | "update") {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "bad_label",
            "label is manual or update",
        ));
    }
    let c = d
        .backup_create(&label)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "backup_failed", e))?;
    Ok(Json(
        json!({"path": c.path, "agents": c.agents, "warnings": c.warnings}),
    ))
}

async fn security(_: User, State(d): State<Daemon>) -> Json<Value> {
    Json(json!({
        "findings": d.security_check().await,
        "pending": xherd_daemon::security::PENDING,
    }))
}

async fn checks(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    let env = xherd_core::runner::agent_env;
    Ok(Json(json!({"checks": d.checks(&env).await})))
}

async fn fix_check(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let env = xherd_core::runner::agent_env;
    d.fix_check(&id, &env).await?;
    Ok(Json(json!({"checks": d.checks(&env).await})))
}

// ---- open in ---------------------------------------------------------------------

async fn open_targets(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"targets": d.open_targets(&id).await?})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenReq {
    app: String,
}

async fn open_in(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(r): Json<OpenReq>,
) -> ApiResult<Json<Value>> {
    d.open_with_companion(&id, &r.app).await?;
    Ok(Json(json!({"ok": true})))
}

/// Settings → Open in (R-706): link entries and the companion's offer.
async fn open_in_view(_: User, State(d): State<Daemon>) -> Json<Value> {
    Json(d.open_in_view())
}

#[derive(Deserialize)]
struct CompanionQuery {
    #[serde(default)]
    apps: String,
    /// Request kinds it understands: `terminal`, `commands` (R-706, R-707).
    #[serde(default)]
    features: String,
    #[serde(default)]
    wait: Option<u64>,
}

/// The host companion's long poll (`xherd companion`): `{request}` or
/// `{request: null}` after `wait` seconds (at most 30).
async fn companion_next(
    _: User,
    State(d): State<Daemon>,
    axum::extract::Query(q): axum::extract::Query<CompanionQuery>,
) -> ApiResult<Json<Value>> {
    let list = |s: &str| {
        s.split(',')
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string)
            .collect()
    };
    let offer = xherd_daemon::open_in::Offer {
        apps: list(&q.apps),
        features: list(&q.features),
    };
    let wait = std::time::Duration::from_secs(q.wait.unwrap_or(25).min(30));
    let r = d.companion.next(offer, wait).await;
    Ok(Json(json!({"request": r})))
}

// ---- git access ----------------------------------------------------------------

async fn git_hosts(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(d.git_hosts()))
}

async fn put_git_hosts(
    _: User,
    State(d): State<Daemon>,
    Json(u): Json<xherd_daemon::git_access::HostsUpdate>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.set_git_hosts(u)?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DoctorReq {
    host: String,
    #[serde(default)]
    repo: Option<String>,
}

/// `GET /agents/:id/git-access` (R-688): the doctor for the agent's host.
async fn agent_git_access(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.agent_git_access(&id).await?))
}

async fn git_doctor(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<DoctorReq>,
) -> ApiResult<Json<Value>> {
    let findings = d.git_doctor(r.host.trim(), r.repo.as_deref()).await;
    Ok(Json(json!({"host": r.host, "findings": findings})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KnownHostReq {
    host: String,
    fingerprint: String,
}

async fn git_known_host(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<KnownHostReq>,
) -> ApiResult<Json<Value>> {
    d.accept_host_key(r.host.trim(), r.fingerprint.trim())
        .await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginReq {
    host: String,
}

/// Start the browser login (`/ws/setup/git-login` shows it).
async fn git_login(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<LoginReq>,
) -> ApiResult<Json<Value>> {
    d.start_git_login(r.host.trim()).await?;
    Ok(Json(json!({"session": xherd_daemon::git_access::LOGIN_ID})))
}

/// Start a plugin's login (`/ws/setup/plugin-login` shows it).
async fn plugin_login(
    _: User,
    State(d): State<Daemon>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    d.start_plugin_login(&name).await?;
    Ok(Json(
        json!({"session": xherd_daemon::git_access::PLUGIN_LOGIN_ID}),
    ))
}

/// Settings → Toolchains: the tools and their default versions.
async fn toolchains(_: User, State(d): State<Daemon>) -> Json<Value> {
    Json(json!({"tools": d.toolchain_tools(), "sandbox": d.sandbox}))
}

/// The versions a tool can install, each marked installed or not.
async fn toolchain_versions(
    _: User,
    State(d): State<Daemon>,
    Path(tool): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({"versions": d.toolchain_versions(&tool).await?}),
    ))
}

#[derive(Deserialize)]
struct ToolchainInstallReq {
    tool: String,
    version: String,
}

/// Install a toolchain version in a terminal (`/ws/setup/plugin-login` shows it).
async fn toolchain_install(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<ToolchainInstallReq>,
) -> ApiResult<Json<Value>> {
    d.start_toolchain_install(&r.tool, &r.version).await?;
    Ok(Json(
        json!({"session": xherd_daemon::toolchains::INSTALL_ID}),
    ))
}

#[derive(Deserialize)]
struct CredentialReq {
    protocol: String,
    host: String,
}

/// `xherd credential get` (R-684): the user, or an agent token with the
/// `credential` scope. 404 = no answer (git asks its next helper).
async fn git_credential(
    user: Result<User, ApiError>,
    agent: Result<AgentAuth, ApiError>,
    State(d): State<Daemon>,
    Json(r): Json<CredentialReq>,
) -> ApiResult<Json<Value>> {
    if user.is_err() {
        agent?.require(xherd_daemon::SCOPE_CREDENTIAL)?;
    }
    match d.credential(&r.protocol, &r.host) {
        Some((username, password)) => Ok(Json(json!({"username": username, "password": password}))),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "no_credential",
            format!("xherd holds no credential for {}", r.host),
        )),
    }
}

async fn get_agent(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let agent = d.action(&id, UserAction::Get).await?;
    let events = d
        .store
        .events(&agent.id, 50)
        .await
        .map_err(ApiError::internal)?;
    let events: Vec<Value> = events
        .into_iter()
        .map(|e| json!({"id": e.id, "kind": e.kind, "payload": e.payload, "at": e.at}))
        .collect();
    Ok(Json(json!({"agent": agent, "events": events})))
}

#[derive(Deserialize)]
struct DiffQuery {
    /// `uncommitted`, `branch` or `commit` (with `commit=<id>`); none: what the
    /// open gate shows.
    view: Option<String>,
    commit: Option<String>,
}

/// `GET /agents/:id/diff`: the open commit proposal's diff, else every
/// uncommitted change (`Daemon::diff`).
async fn diff(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Query(q): Query<DiffQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        d.diff(&id, q.view.as_deref(), q.commit.as_deref()).await?,
    ))
}

/// `GET /agents/:id/pr-review`: this round's PR comments, the agent's
/// proposals and the user's decisions (R-306).
async fn pr_review(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.pr_review(&id).await?))
}

#[derive(serde::Deserialize)]
struct DecisionsReq {
    decisions: std::collections::BTreeMap<String, xherd_core::pr::Verdict>,
}

/// `PUT /agents/:id/pr-decisions`: replace the user's decisions on the
/// comments of this round (R-306).
async fn put_pr_decisions(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(req): Json<DecisionsReq>,
) -> ApiResult<Json<Value>> {
    d.set_pr_decisions(&id, req.decisions).await?;
    Ok(Json(json!({"ok": true})))
}

/// `GET /agents/:id/findings`: the findings of `review.md`.
async fn findings(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.findings(&id).await?))
}

/// `GET /agents/:id/docs`: the agent's documents, open gate's first.
async fn docs(_: User, State(d): State<Daemon>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"docs": d.docs(&id).await?})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileQuery {
    path: String,
}

/// `GET /agents/:id/file?path=spec.md`: a document with its approval hash
/// and the text last approved (R-107).
async fn file(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Query(q): Query<FileQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.doc(&id, &q.path).await?))
}

/// `GET /agents/:id/viewed`: the files of the Changes panel marked viewed.
async fn get_viewed(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"viewed": d.viewed(&id).await?})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ViewedBody {
    path: String,
    /// The UI's fingerprint of the file's diff; absent clears the mark.
    fingerprint: Option<String>,
}

/// `POST /agents/:id/viewed`: mark a file viewed, or clear the mark.
async fn post_viewed(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(b): Json<ViewedBody>,
) -> ApiResult<Json<Value>> {
    d.set_viewed(&id, &b.path, b.fingerprint.as_deref()).await?;
    Ok(Json(json!({"ok": true})))
}

/// `POST /api/v1/agents/pause-all` (R-032): the kill switch.
async fn pause_all(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"paused": d.pause_all().await?})))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ActionBody {
    text: Option<String>,
    reason: Option<String>,
    comment: Option<String>,
    /// Approve: document → hash the user read.
    expect: Option<std::collections::BTreeMap<String, String>>,
    /// Approve (commit gate): the commit message as edited.
    message: Option<String>,
    /// Autonomy: `ask` or `important`.
    level: Option<xherd_core::autonomy::Autonomy>,
    /// Mode: `ask`, `edits`, `auto` or `all` (R-002a).
    mode: Option<xherd_core::permission::PermissionMode>,
}

async fn agent_action(
    _: User,
    State(d): State<Daemon>,
    Path((id, action)): Path<(String, String)>,
    body: Option<Json<ActionBody>>,
) -> ApiResult<Json<Value>> {
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let required = |v: Option<String>, field: &str| {
        v.ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                format!("body {{\"{field}\": …}} required"),
            )
        })
    };
    if action == "mode" {
        let mode = b.mode.ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                "body {\"mode\": \"ask\"|\"edits\"|\"auto\"|\"all\"} required",
            )
        })?;
        let agent = d.set_permission_mode(&id, mode).await?;
        return Ok(Json(json!({"agent": agent})));
    }
    let a = match action.as_str() {
        "pause" => UserAction::Pause,
        "resume" => UserAction::Resume,
        "cancel" => UserAction::Cancel,
        "send" => UserAction::Send {
            text: required(b.text, "text")?,
        },
        "approve" => UserAction::Approve {
            reason: b.reason,
            expect: b.expect.unwrap_or_default(),
            message: b.message,
        },
        "request-changes" => UserAction::RequestChanges {
            comment: required(b.comment, "comment")?,
        },
        "autonomy" => UserAction::SetAutonomy {
            level: b.level.ok_or_else(|| {
                ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "bad_request",
                    "body {\"level\": \"ask\"|\"important\"} required",
                )
            })?,
        },
        _ => {
            return Err(ApiError::new(
                StatusCode::NOT_FOUND,
                "unknown_action",
                format!("no action {action:?}"),
            ));
        }
    };
    let agent = d.action(&id, a).await?;
    Ok(Json(json!({"agent": agent})))
}

async fn hooks(
    auth: AgentAuth,
    State(d): State<Daemon>,
    Json(r): Json<Report>,
) -> ApiResult<Json<Value>> {
    auth.require(SCOPE_HOOKS)?;
    if r.agent_id != auth.agent_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "wrong_agent",
            "token belongs to another agent",
        ));
    }
    match d.report(r).await {
        Some(res) => Ok(Json(json!({"ok": true, "result": res}))),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_agent",
            "no such agent",
        )),
    }
}

/// `POST /api/v1/xap` (xap.md §8): `200 {ok, result}` or `{ok: false, code, message}`.
async fn xap(auth: AgentAuth, State(d): State<Daemon>, body: axum::body::Bytes) -> Response {
    let fail = |status: StatusCode, code: &str, message: String| {
        (
            status,
            Json(json!({"ok": false, "code": code, "message": message})),
        )
            .into_response()
    };
    if let Err(e) = auth.require(SCOPE_XAP) {
        return e.into_response();
    }
    let call: XapCall = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            return fail(
                StatusCode::BAD_REQUEST,
                "usage",
                format!("bad XAP call: {e}"),
            );
        }
    };
    if call.agent_id != auth.agent_id {
        return fail(
            StatusCode::FORBIDDEN,
            "wrong_agent",
            "token belongs to another agent".into(),
        );
    }
    match d.xap(call).await {
        Ok(result) => Json(json!({"ok": true, "result": result})).into_response(),
        Err(r) => {
            let status = match r.code {
                "unknown_agent" => StatusCode::NOT_FOUND,
                "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
                "internal" => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::UNPROCESSABLE_ENTITY,
            };
            fail(status, r.code, r.message)
        }
    }
}
