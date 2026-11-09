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
        .route("/api/v1/agents", get(list_agents).post(new_agent))
        .route("/api/v1/agents/{id}", get(get_agent))
        .route("/api/v1/agents/{id}/diff", get(diff))
        .route("/api/v1/agents/{id}/docs", get(docs))
        .route("/api/v1/agents/{id}/file", get(file))
        .route("/api/v1/agents/{id}/open-targets", get(open_targets))
        .route("/api/v1/agents/{id}/open", post(open_in))
        .route("/api/v1/agents/{id}/{action}", post(agent_action))
        .route("/api/v1/companion/next", get(companion_next))
        .route("/api/v1/repos", get(repos))
        .route("/api/v1/repos/resolve", post(resolve_repo))
        .route("/api/v1/install", post(install))
        .route("/api/v1/config", get(get_config).put(put_config))
        .route("/api/v1/checks", get(checks))
        .route("/api/v1/security", get(security))
        .route("/api/v1/checks/{id}/fix", post(fix_check))
        .route("/api/v1/git/hosts", get(git_hosts).put(put_git_hosts))
        .route("/api/v1/git/doctor", post(git_doctor))
        .route("/api/v1/git/known-hosts", post(git_known_host))
        .route("/api/v1/git/login", post(git_login))
        .route("/api/v1/plugins/{name}/login", post(plugin_login))
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
async fn is_local(parts: &mut Parts, d: &Daemon) -> bool {
    let peer_local = ConnectInfo::<SocketAddr>::from_request_parts(parts, d)
        .await
        .is_ok_and(|ConnectInfo(a)| {
            let ip = a.ip().to_canonical();
            ip.is_loopback() || d.local_peers.contains(&ip)
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
    Json(json!({"ok": true, "version": xherd_core::VERSION}))
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
    let env = |k: &str| std::env::var(k).ok();
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

async fn security(_: User, State(d): State<Daemon>) -> Json<Value> {
    Json(json!({
        "findings": d.security_check(),
        "pending": xherd_daemon::security::PENDING,
    }))
}

async fn checks(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    let env = |k: &str| std::env::var(k).ok();
    Ok(Json(json!({"checks": d.checks(&env).await})))
}

async fn fix_check(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let env = |k: &str| std::env::var(k).ok();
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

#[derive(Deserialize)]
struct CompanionQuery {
    #[serde(default)]
    apps: String,
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
    let apps = q
        .apps
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(str::to_string)
        .collect();
    let wait = std::time::Duration::from_secs(q.wait.unwrap_or(25).min(30));
    let r = d.companion.next(apps, wait).await;
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
