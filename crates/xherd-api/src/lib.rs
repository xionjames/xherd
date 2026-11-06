//! HTTP API (design §7): health, state, agents and their actions,
//! `POST /hooks` for status reports and `POST /xap` for the agent protocol.
//! Websockets and the UI come in M2.

use std::net::SocketAddr;

use axum::extract::connect_info::ConnectInfo;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, State};
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

pub const BODY_LIMIT: usize = 5 * 1024 * 1024;

pub fn router(d: Daemon) -> Router {
    Router::new()
        .route("/api/v1/healthz", get(healthz))
        .route("/api/v1/state", get(state))
        .route("/api/v1/agents", get(list_agents).post(new_agent))
        .route("/api/v1/agents/{id}", get(get_agent))
        .route("/api/v1/agents/{id}/{action}", post(agent_action))
        .route("/api/v1/install", post(install))
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
        .is_ok_and(|ConnectInfo(a)| a.ip().is_loopback());
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
    peer_local && host_local && !proxied
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
            if let Some(row) = row.filter(|r| r.kind == TOKEN_USER && !r.revoked) {
                let _ = d.store.touch_token(&row.id, d.clock.now()).await;
                return Ok(User);
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
            json!({"name": p.name(), "kind": p.manifest.kind, "version": p.manifest.version,
                   "label": p.manifest.label, "builtin": p.builtin()})
        })
        .collect();
    Ok(Json(json!({
        "version": xherd_core::VERSION,
        "platform": std::env::consts::OS,
        "config": config,
        "agents": agents,
        "plugins": plugins,
        "brokenPlugins": d.plugins.broken(),
    })))
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

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ActionBody {
    text: Option<String>,
    reason: Option<String>,
    comment: Option<String>,
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
        "approve" => UserAction::Approve { reason: b.reason },
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
