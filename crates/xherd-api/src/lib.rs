//! HTTP API (design §7): health, state, agents and their actions,
//! `POST /hooks` for status reports, `POST /xap` for the agent protocol,
//! `GET /ws` for the live state stream, and the web UI at `/`.

use std::net::SocketAddr;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::connect_info::ConnectInfo;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use xherd_core::report::Report;
use xherd_core::xap::XapCall;
use xherd_daemon::support::{SCOPE_HOOKS, SCOPE_XAP, TOKEN_AGENT, TOKEN_JOB, TOKEN_USER};
use xherd_daemon::{ActionError, Daemon, UserAction, hash_token};

mod term;
mod ui;
mod view;
mod ws;

pub const BODY_LIMIT: usize = 5 * 1024 * 1024;

/// Identifies this process start in `/healthz` (`startId`), so the UI can tell a restart
/// apart from a daemon that never went down. The start time, not the pid: a re-exec keeps
/// the pid. Forced in `router()`, so it is fixed from daemon start on.
static START_ID: LazyLock<String> = LazyLock::new(|| {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    nanos.to_string()
});

pub fn router(d: Daemon) -> Router {
    LazyLock::force(&START_ID);
    Router::new()
        .route("/", get(ui::index))
        .route("/assets/{*rest}", get(ui::asset))
        .route("/api/v1/healthz", get(healthz))
        .route("/api/v1/ws", get(ws::state_stream))
        .route("/api/v1/ws/term/{id}", get(term::terminal))
        .route("/api/v1/ws/setup/{id}", get(term::setup_terminal))
        .route("/api/v1/state", get(state))
        .route("/api/v1/backups", get(backups).post(new_backup))
        .route("/api/v1/jobs", get(list_jobs).post(new_job))
        .route("/api/v1/jobs/pause-all", post(pause_all))
        .route("/api/v1/jobs/{id}", get(get_job))
        .route("/api/v1/jobs/{id}/diff", get(diff))
        .route("/api/v1/jobs/{id}/diff/source", get(diff_source))
        .route("/api/v1/jobs/{id}/docs", get(docs))
        .route("/api/v1/jobs/{id}/findings", get(findings))
        .route("/api/v1/jobs/{id}/memory", get(memory))
        .route("/api/v1/jobs/{id}/agent-memory", get(agent_memory_proposal))
        .route(
            "/api/v1/jobs/{id}/agent-memory-accept",
            post(agent_memory_accept),
        )
        .route(
            "/api/v1/jobs/{id}/suggestion/accept",
            post(suggestion_accept),
        )
        .route(
            "/api/v1/jobs/{id}/suggestion/dismiss",
            post(suggestion_dismiss),
        )
        .route("/api/v1/jobs/{id}/save-as-agent", post(save_as_agent))
        .route("/api/v1/jobs/{id}/pr-review", get(pr_review))
        .route(
            "/api/v1/jobs/{id}/pr-decisions",
            axum::routing::put(put_pr_decisions),
        )
        .route("/api/v1/jobs/{id}/file", get(file))
        .route(
            "/api/v1/jobs/{id}/viewed",
            get(get_viewed).post(post_viewed),
        )
        .route("/api/v1/jobs/{id}/open-targets", get(open_targets))
        .route("/api/v1/jobs/{id}/{action}", post(agent_action))
        .route("/api/v1/open-in", get(open_in_view))
        .route(
            "/api/v1/open-in/terminal-handler.sh",
            get(terminal_handler_script),
        )
        .route(
            "/api/v1/open-in/terminal-handler",
            post(terminal_handler_report),
        )
        .route(
            "/api/v1/open-in/terminal-handler/install",
            post(terminal_handler_install),
        )
        .route("/api/v1/repos", get(repos))
        .route("/api/v1/repos/resolve", post(resolve_repo))
        .route("/api/v1/tickets/resolve", post(resolve_ticket))
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
        .route("/api/v1/jobs/{id}/user-time", post(user_time))
        .route("/api/v1/reviews", get(reviews))
        .route("/api/v1/reviews/seen", post(reviews_seen))
        .route("/api/v1/checks/{id}/fix", post(fix_check))
        .route("/api/v1/git/hosts", get(git_hosts).put(put_git_hosts))
        .route("/api/v1/git/doctor", post(git_doctor))
        .route("/api/v1/jobs/{id}/git-access", get(agent_git_access))
        .route("/api/v1/jobs/{id}/ticket", get(agent_ticket))
        .route("/api/v1/jobs/{id}/ticket-status", get(ticket_status))
        .route("/api/v1/jobs/{id}/ticket-summary", get(ticket_summary))
        .route("/api/v1/jobs/{id}/ticket-comment", post(ticket_comment))
        // The old job routes (R3): one release of `410 moved`.
        // Saved agents (R19). Static paths come first; `{id}` resolves an agent
        // before the old job path (R3).
        .route("/api/v1/agents", get(agents_list).post(agent_create))
        .route("/api/v1/agents/pause-all", any(gone_pause_all))
        .route("/api/v1/agents/export", get(agents_export))
        .route("/api/v1/agents/import", post(agents_import))
        .route("/api/v1/agents/{id}", any(agent_one))
        .route("/api/v1/agents/{id}/archive", post(agent_archive))
        .route("/api/v1/agents/{id}/unarchive", post(agent_unarchive))
        .route("/api/v1/agents/{id}/versions", get(agent_versions))
        .route(
            "/api/v1/agents/{id}/memory",
            get(agent_memory_get).put(agent_memory_put),
        )
        .route(
            "/api/v1/agents/{id}/jobs",
            get(agent_jobs).post(agent_new_job),
        )
        .route("/api/v1/agents/{id}/{*rest}", any(gone_job))
        // Which saved agent runs which stage of a workflow (R35).
        .route("/api/v1/flows", get(flows_list))
        .route("/api/v1/flows/{pack}/{flow}", get(flow_one))
        .route(
            "/api/v1/flows/{pack}/{flow}/assignments",
            get(flow_assignments).put(flow_assignments_put),
        )
        // The skills library (R25).
        .route("/api/v1/skills", get(skills_list).post(skill_create))
        .route("/api/v1/skills/{name}", get(skill_get).delete(skill_delete))
        .route("/api/v1/git/known-hosts", post(git_known_host))
        .route("/api/v1/git/login", post(git_login))
        .route(
            "/api/v1/plugins/{name}/view/{*path}",
            get(view::plugin_file),
        )
        .route("/api/v1/view-kit/{file}", get(view::kit_file))
        .route("/api/v1/plugins", get(plugins_list))
        .route("/api/v1/plugins/preview", post(plugin_preview))
        .route("/api/v1/plugins/install", post(plugin_install))
        .route(
            "/api/v1/plugins/{name}",
            axum::routing::delete(plugin_remove),
        )
        .route("/api/v1/plugins/{name}/update", post(plugin_update))
        .route("/api/v1/admin/restart", post(admin_restart))
        .route("/api/v1/plugins/{name}/login", post(plugin_login))
        .route(
            "/api/v1/plugins/{name}/secrets",
            get(plugin_secrets).put(put_plugin_secrets),
        )
        .route("/api/v1/toolchains", get(toolchains))
        .route("/api/v1/toolchains/installed", get(toolchains_installed))
        .route("/api/v1/toolchains/install", post(toolchain_install))
        .route("/api/v1/runtimes/{name}/models", get(runtime_models))
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
    location: Option<String>,
}

impl ApiError {
    fn new(status: StatusCode, code: &str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            code: code.into(),
            message: message.into(),
            location: None,
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
            "unknown_job" => StatusCode::NOT_FOUND,
            "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
            "usage" => StatusCode::BAD_REQUEST,
            "bad_ticket" | "bad_name" | "name_required" | "unknown_flow" | "prompt_required"
            | "repo_url" | "not_a_repo" | "repo_required" | "base_unresolved"
            | "runtime_unusable" | "runtime_hooks" | "unknown_pack" | "unknown_runtime"
            | "bad_model" | "model_unavailable" | "bad_agent" | "unknown_stage" | "bad_skill"
            | "bad_memory" | "unknown_skill" | "bad_fallback" | "bad_assignment"
            | "mixed_runtime" => StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_agent" | "no_such_skill" => StatusCode::NOT_FOUND,
            "bad_path" | "bad_config" => StatusCode::BAD_REQUEST,
            "plugin_install_off" => StatusCode::FORBIDDEN,
            "unknown_plugin" => StatusCode::NOT_FOUND,
            "bad_plugin_source"
            | "bad_plugin_folder"
            | "bad_manifest"
            | "build_not_supported"
            | "kind_not_supported"
            | "no_manifest" => StatusCode::UNPROCESSABLE_ENTITY,
            "fetch_failed" => StatusCode::BAD_GATEWAY,
            "not_fixable" => StatusCode::NOT_FOUND,
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
        let mut body = json!({"error": self.message, "code": self.code});
        if let Some(l) = self.location {
            body["location"] = json!(l);
        }
        (self.status, Json(body)).into_response()
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
    pub job_id: String,
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
            Some(r) if (r.kind == TOKEN_JOB || r.kind == TOKEN_AGENT) && !r.revoked => {
                Ok(AgentAuth {
                    job_id: r.subject.unwrap_or_default(),
                    scopes: r.scopes,
                })
            }
            _ => Err(deny()),
        }
    }
}

// ---- handlers ------------------------------------------------------------------

async fn healthz() -> Json<Value> {
    Json(json!({
        "ok": true,
        "version": xherd_core::VERSION,
        "buildVersion": xherd_core::BUILD_VERSION,
        "startId": START_ID.as_str(),
    }))
}

async fn state(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    let agents = d.store.list_jobs().await.map_err(ApiError::internal)?;
    let records = d.store.plugin_records().await.map_err(ApiError::internal)?;
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
            // A runtime that takes a model (issue #112); the models it
            // suggests come from `GET /runtimes/{name}/models`.
            let models = xherd_plugin::runtime::Terminal::of(&p.manifest)
                .is_ok_and(|t| !t.model_args.is_empty());
            let mut info = json!({"name": p.name(), "kind": p.manifest.kind,
                   "version": p.manifest.version,
                   "label": p.manifest.label, "description": p.manifest.description,
                   "builtin": p.builtin(), "settings": settings,
                   "enabled": d.plugin_enabled(p.name()), "login": p.manifest.login.is_some(),
                   "chooseModel": models});
            // Where an installed plugin came from (R6).
            if let Some(r) = records.iter().find(|r| r.name == p.name()) {
                info["source"] = record_source(r);
            }
            // A Herd view plugin: the file the Herd page frames (R9).
            if p.manifest.kind == xherd_plugin::Kind::View
                && let Ok(entry) = xherd_plugin::manifest::view_entry(&p.manifest)
            {
                info["view"] = json!({"entry": entry});
            }
            info
        })
        .collect();
    Ok(Json(json!({
        "version": xherd_core::VERSION,
        "buildVersion": xherd_core::BUILD_VERSION,
        "platform": std::env::consts::OS,
        "config": config,
        "jobs": agents,
        "agents": d.agents().await?,
        "plugins": plugins,
        "trackerLinks": d.tracker_links(),
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
                    .map(|s| {
                        let mut v = json!({"id": s.id, "role": s.role, "class": s.class, "gate": s.gate.is_some()});
                        if let Some(label) = &s.label {
                            v["label"] = json!(label);
                        }
                        v
                    })
                    .collect();
                json!({"pack": p.name, "flow": name, "stages": stages})
            })
        })
        .collect()
}

async fn list_jobs(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    let agents = d.store.list_jobs().await.map_err(ApiError::internal)?;
    Ok(Json(json!({"jobs": agents})))
}

/// Create an agent (spec §5.1). Body: `NewJob`, unknown fields refused.
async fn new_job(
    _: User,
    State(d): State<Daemon>,
    Json(req): Json<xherd_core::new::NewJob>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    let agent = d.new_job(req).await?;
    Ok((StatusCode::CREATED, Json(json!({"job": agent}))))
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
struct ResolveTicketReq {
    ticket: String,
    #[serde(default)]
    repo: Option<String>,
}

async fn resolve_ticket(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<ResolveTicketReq>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        d.resolve_ticket_input(&r.ticket, r.repo.as_deref()).await?,
    ))
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
    group: Option<String>,
}

/// `GET /api/v1/worklog?days=N[&group=agent]` (spec §16.4): time per day and
/// ticket (or saved agent), and the delivery metrics per ticket.
async fn worklog(
    _: User,
    State(d): State<Daemon>,
    axum::extract::Query(q): axum::extract::Query<WorklogQuery>,
) -> ApiResult<Json<Value>> {
    let by_agent = q.group.as_deref() == Some("agent");
    Ok(Json(d.worklog(q.days.unwrap_or(14), by_agent).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserTimeBody {
    ms: u64,
}

/// `POST /jobs/:id/user-time`: the agent page was open for `ms` (heartbeat).
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
            let seen = seen.contains(&seen_id(&e.job_id, e.at, p["gate"].as_str().unwrap_or("")));
            json!({"jobId": e.job_id, "at": e.at, "gate": p["gate"], "files": p["files"], "reason": p["reason"], "seen": seen})
        })
        .collect();
    Ok(Json(json!({"reviews": items})))
}

/// The event id that records an auto-approval as seen (one per item).
fn seen_id(job_id: &str, at: i64, gate: &str) -> String {
    format!("seen:{job_id}:{at}:{gate}")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SeenItem {
    job_id: String,
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
                id: seen_id(&i.job_id, i.at, &i.gate),
                job_id: i.job_id.clone(),
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
        json!({"path": c.path, "jobs": c.jobs, "warnings": c.warnings}),
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

/// Settings → Open in (R-706): the link entries and the handler's state.
async fn open_in_view(_: User, State(d): State<Daemon>) -> Json<Value> {
    Json(d.open_in_view())
}

/// The handler's installer (R-707), for `curl … | sh` on the user's
/// computer; it reports back to the address the request came to.
async fn terminal_handler_script(
    _: User,
    State(d): State<Daemon>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | ':' | '[' | ']' | '-'))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_host",
            "the Host header is not a plain host name",
        ));
    }
    let script =
        xherd_daemon::open_in::handler_script(&d.worktrees_root(), &format!("http://{host}"))
            .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "bad_config", m))?;
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        script,
    )
        .into_response())
}

/// The installer's report: what it installed, or `{"installed": false}`.
async fn terminal_handler_report(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<xherd_daemon::open_in::HandlerReport>,
) -> ApiResult<Json<Value>> {
    let rec = r
        .into_record(d.clock.now())
        .map_err(|m| ApiError::new(StatusCode::BAD_REQUEST, "bad_report", m))?;
    d.record_handler(rec.as_ref()).map_err(ApiError::internal)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandlerInstall {
    #[serde(default)]
    uninstall: bool,
}

/// Native only: the daemon installs (or removes) the handler itself.
async fn terminal_handler_install(
    _: User,
    State(d): State<Daemon>,
    Json(b): Json<HandlerInstall>,
) -> ApiResult<Json<Value>> {
    let (ok, output) = d.install_terminal_handler(b.uninstall).await?;
    Ok(Json(json!({"ok": ok, "output": output})))
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

/// Which tracker reads the agent's ticket, and whether a summary was posted.
async fn ticket_status(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.ticket_status(&id).await?))
}

/// The draft summary to post on the ticket; nothing is posted.
async fn ticket_summary(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.ticket_summary(&id).await?))
}

#[derive(serde::Deserialize)]
struct CommentReq {
    body: String,
}

/// Post the comment the user accepted on the agent's ticket (R-308).
async fn ticket_comment(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(r): Json<CommentReq>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.post_ticket_comment(&id, &r.body).await?))
}

/// The agent's ticket and its comments, read through a tracker plugin.
async fn agent_ticket(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.agent_ticket(&id).await?))
}

/// `GET /jobs/:id/git-access` (R-688): the doctor for the agent's host.
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

/// Which of a plugin's secret settings have a value (never the values).
async fn plugin_secrets(
    _: User,
    State(d): State<Daemon>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.plugin_secret_status(&name)?))
}

#[derive(serde::Deserialize)]
struct SecretsReq {
    /// key → new value; `null` or empty clears it.
    values: std::collections::BTreeMap<String, Option<String>>,
}

/// Set or clear a plugin's secret settings; the answer is the status again.
async fn put_plugin_secrets(
    _: User,
    State(d): State<Daemon>,
    Path(name): Path<String>,
    Json(r): Json<SecretsReq>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.set_plugin_secrets(&name, r.values)?))
}

/// The recorded source of an installed plugin.
fn record_source(r: &xherd_store::PluginRecord) -> Value {
    json!({"url": r.source, "ref": r.data["ref"], "path": r.data["path"], "sha": r.sha,
           "digest": r.data["digest"], "installedAt": r.data["installedAt"],
           "updatedAt": r.data["updatedAt"]})
}

/// The plugins with their sources, and whether a restart is needed (R7).
async fn plugins_list(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    let records = d.store.plugin_records().await.map_err(ApiError::internal)?;
    let plugins: Vec<Value> = d
        .plugins
        .all()
        .map(|p| {
            let mut info = json!({"name": p.name(), "kind": p.manifest.kind,
                "version": p.manifest.version, "builtin": p.builtin()});
            if let Some(r) = records.iter().find(|r| r.name == p.name()) {
                info["source"] = record_source(r);
            }
            info
        })
        .collect();
    Ok(Json(json!({
        "plugins": plugins,
        "restartNeeded": d.plugin_changes(),
        "installEnabled": d.plugin_install,
    })))
}

#[derive(Deserialize)]
struct PluginSourceReq {
    url: String,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    path: Option<String>,
    /// The digest the user saw in the preview (install only).
    digest: Option<String>,
}

/// Fetch a source and show what installing it would do; nothing is written (R1).
async fn plugin_preview(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<PluginSourceReq>,
) -> ApiResult<Json<Value>> {
    let p = d
        .plugin_preview(&r.url, r.git_ref.as_deref(), r.path.as_deref())
        .await?;
    Ok(Json(serde_json::to_value(p).map_err(ApiError::internal)?))
}

/// Install what the user previewed: refused when the source changed since (R4).
async fn plugin_install(
    _: User,
    State(d): State<Daemon>,
    Json(r): Json<PluginSourceReq>,
) -> ApiResult<Json<Value>> {
    let digest = r
        .digest
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "usage",
                "digest is required: install what a preview showed",
            )
        })?;
    let i = d
        .plugin_install(&r.url, r.git_ref.as_deref(), r.path.as_deref(), digest)
        .await?;
    Ok(Json(serde_json::to_value(i).map_err(ApiError::internal)?))
}

#[derive(Deserialize)]
struct PluginUpdateReq {
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    /// Without it: the preview (manifest diff); with it: apply that preview.
    digest: Option<String>,
}

async fn plugin_update(
    _: User,
    State(d): State<Daemon>,
    Path(name): Path<String>,
    Json(r): Json<PluginUpdateReq>,
) -> ApiResult<Json<Value>> {
    let v = match r.digest.as_deref().filter(|s| !s.is_empty()) {
        None => {
            let p = d.plugin_update_preview(&name, r.git_ref.as_deref()).await?;
            serde_json::to_value(p)
        }
        Some(digest) => {
            let u = d.plugin_update(&name, r.git_ref.as_deref(), digest).await?;
            serde_json::to_value(u)
        }
    };
    Ok(Json(v.map_err(ApiError::internal)?))
}

#[derive(Deserialize, Default)]
struct PluginRemoveQuery {
    #[serde(default)]
    forget: bool,
}

async fn plugin_remove(
    _: User,
    State(d): State<Daemon>,
    Path(name): Path<String>,
    Query(q): Query<PluginRemoveQuery>,
) -> ApiResult<Json<Value>> {
    let r = d.plugin_remove(&name, q.forget).await?;
    Ok(Json(serde_json::to_value(r).map_err(ApiError::internal)?))
}

/// Ask the daemon to restart itself (R7a); the process re-executes.
async fn admin_restart(_: User, State(d): State<Daemon>) -> (StatusCode, Json<Value>) {
    d.request_restart();
    (StatusCode::ACCEPTED, Json(json!({"restarting": true})))
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

/// The versions already installed, per tool (native and container).
async fn toolchains_installed(_: User, State(d): State<Daemon>) -> Json<Value> {
    Json(json!({"installed": d.toolchain_installed().await}))
}

/// Settings → Toolchains: the tools and their default versions.
async fn toolchains(_: User, State(d): State<Daemon>) -> Json<Value> {
    Json(json!({"tools": d.toolchain_tools(), "sandbox": d.sandbox}))
}

/// The versions a tool can install, each marked installed or not.
async fn runtime_models(
    _: User,
    State(d): State<Daemon>,
    Path(name): Path<String>,
) -> ApiResult<Json<xherd_daemon::RuntimeModels>> {
    Ok(Json(d.runtime_models(&name).await?))
}

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

async fn get_job(
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
    Ok(Json(json!({"job": agent, "events": events})))
}

#[derive(Deserialize)]
struct DiffQuery {
    /// `uncommitted`, `branch` or `commit` (with `commit=<id>`); none: what the
    /// open gate shows.
    view: Option<String>,
    commit: Option<String>,
}

/// `GET /jobs/:id/diff`: the open commit proposal's diff, else every
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceQuery {
    path: String,
    view: Option<String>,
    commit: Option<String>,
}

/// `GET /jobs/:id/diff/source`: the whole new-side file of a diff, so the
/// Changes panel can expand context around hunks (`Daemon::diff_source`).
async fn diff_source(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Query(q): Query<SourceQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        d.diff_source(&id, &q.path, q.view.as_deref(), q.commit.as_deref())
            .await?,
    ))
}

/// `GET /jobs/:id/pr-review`: this round's PR comments, the agent's
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

/// `PUT /jobs/:id/pr-decisions`: replace the user's decisions on the
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

/// `GET /jobs/:id/findings`: the findings of `review.md`.
async fn findings(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.findings(&id).await?))
}

/// `GET /jobs/:id/memory`: the pending shared-memory proposal (§16.6).
async fn memory(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.memory(&id).await?))
}

/// `GET /jobs/:id/agent-memory`: the pending agent-memory proposal (R29).
async fn agent_memory_proposal(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.agent_memory_proposal(&id).await?))
}

/// `POST /jobs/:id/agent-memory-accept {text?}`: save it as the agent's memory.
async fn agent_memory_accept(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    body: Option<Json<ActionBody>>,
) -> ApiResult<Json<Value>> {
    let text = body.and_then(|b| b.0.text);
    Ok(Json(d.agent_memory_accept(&id, text).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveAgentBody {
    agent: xherd_core::agent_def::AgentDef,
}

/// `POST /jobs/:id/suggestion/accept {agent}` (R32): create the agent from
/// the form and attach the job.
async fn suggestion_accept(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(b): Json<SaveAgentBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.save_as_agent(&id, b.agent, true).await?))
}

/// `POST /jobs/:id/save-as-agent {agent}` (R32): the same, with no suggestion.
async fn save_as_agent(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(b): Json<SaveAgentBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.save_as_agent(&id, b.agent, false).await?))
}

/// `POST /jobs/:id/suggestion/dismiss`: remembered for the job (R32).
async fn suggestion_dismiss(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"job": d.dismiss_suggestion(&id).await?})))
}

/// `GET /jobs/:id/docs`: the agent's documents, open gate's first.
async fn docs(_: User, State(d): State<Daemon>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"docs": d.docs(&id).await?})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileQuery {
    path: String,
}

/// `GET /jobs/:id/file?path=spec.md`: a document with its approval hash
/// and the text last approved (R-107).
async fn file(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Query(q): Query<FileQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.doc(&id, &q.path).await?))
}

/// `GET /jobs/:id/viewed`: the files of the Changes panel marked viewed.
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

/// `POST /jobs/:id/viewed`: mark a file viewed, or clear the mark.
async fn post_viewed(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(b): Json<ViewedBody>,
) -> ApiResult<Json<Value>> {
    d.set_viewed(&id, &b.path, b.fingerprint.as_deref()).await?;
    Ok(Json(json!({"ok": true})))
}

/// `POST /api/v1/jobs/pause-all` (R-032): the kill switch.
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
    /// Model: a name, or null/empty for the runtime's default (issue #112).
    model: Option<String>,
    /// Cancel: also remove the worktree (R12).
    #[serde(rename = "removeWorktree")]
    remove_worktree: Option<bool>,
}

/// `410 moved` for a route that now lives elsewhere (R3, issue #141).
fn moved(location: String) -> ApiError {
    let mut e = ApiError::new(
        StatusCode::GONE,
        "moved",
        format!("agents are jobs now: use {location}"),
    );
    e.location = Some(location);
    e
}

async fn gone_pause_all(_: User) -> ApiError {
    moved("/api/v1/jobs/pause-all".into())
}

/// The old `/api/v1/agents/{id}/…` routes: `410` when `{id}` is a job (its id,
/// name or ticket key), else `404`.
async fn gone_job(
    _: User,
    State(d): State<Daemon>,
    Path(p): Path<std::collections::HashMap<String, String>>,
) -> ApiError {
    let id = p.get("id").cloned().unwrap_or_default();
    if d.is_agent(&id).await {
        return ApiError::new(
            StatusCode::NOT_FOUND,
            "no_such_route",
            format!("agent {id:?} has no such route"),
        );
    }
    if d.action(&id, UserAction::Get).await.is_err() {
        return ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_job",
            format!("no job {id:?}"),
        );
    }
    let rest = p.get("rest").map(|r| format!("/{r}")).unwrap_or_default();
    moved(format!("/api/v1/jobs/{id}{rest}"))
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
        return Ok(Json(json!({"job": agent})));
    }
    if action == "model" {
        let agent = d.set_model(&id, b.model.as_deref()).await?;
        return Ok(Json(json!({"job": agent})));
    }
    let a = match action.as_str() {
        "pause" => UserAction::Pause,
        "resume" => UserAction::Resume,
        "cancel" => UserAction::Cancel {
            remove_worktree: b.remove_worktree.unwrap_or(false),
        },
        "remove-worktree" => UserAction::RemoveWorktree,
        "send" => UserAction::Send {
            text: required(b.text, "text")?,
        },
        "ask" => UserAction::Ask {
            text: required(b.text, "text")?,
        },
        "finish" => UserAction::Finish,
        "approve" => UserAction::Approve {
            reason: b.reason,
            expect: b.expect.unwrap_or_default(),
            message: b.message,
        },
        "memory-accept" => UserAction::MemoryAccept { text: b.text },
        "memory-dismiss" => UserAction::MemoryDismiss,
        "agent-memory-dismiss" => UserAction::AgentMemoryClear,
        "mark-done" => UserAction::MarkDone,
        "pr-check" => UserAction::CheckPr,
        "resolve-conflicts" => UserAction::ResolveConflicts,
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
    Ok(Json(json!({"job": agent})))
}

async fn hooks(
    auth: AgentAuth,
    State(d): State<Daemon>,
    Json(r): Json<Report>,
) -> ApiResult<Json<Value>> {
    auth.require(SCOPE_HOOKS)?;
    if r.job_id != auth.job_id {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "wrong_job",
            "token belongs to another job",
        ));
    }
    match d.report(r).await {
        Some(res) => Ok(Json(json!({"ok": true, "result": res}))),
        None => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "unknown_job",
            "no such job",
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
    if call.job_id != auth.job_id {
        return fail(
            StatusCode::FORBIDDEN,
            "wrong_job",
            "token belongs to another job".into(),
        );
    }
    match d.xap(call).await {
        Ok(result) => Json(json!({"ok": true, "result": result})).into_response(),
        Err(r) => {
            let status = match r.code {
                "unknown_job" => StatusCode::NOT_FOUND,
                "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
                "internal" => StatusCode::INTERNAL_SERVER_ERROR,
                _ => StatusCode::UNPROCESSABLE_ENTITY,
            };
            fail(status, r.code, r.message)
        }
    }
}

// ---- skills library (R25) -----------------------------------------------------

async fn skills_list(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"skills": d.skills().await?})))
}

async fn skill_get(
    _: User,
    State(d): State<Daemon>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.skill_detail(&name).await?)))
}

async fn skill_create(
    _: User,
    State(d): State<Daemon>,
    Json(req): Json<xherd_daemon::skills::NewSkill>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.create_skill(req).await?)))
}

async fn skill_delete(
    _: User,
    State(d): State<Daemon>,
    Path(name): Path<String>,
) -> ApiResult<Json<Value>> {
    d.delete_skill(&name).await?;
    Ok(Json(json!({"ok": true})))
}

// ---- saved agents (R19) -------------------------------------------------------

async fn agents_list(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"agents": d.agents().await?})))
}

async fn agent_create(
    _: User,
    State(d): State<Daemon>,
    Json(def): Json<xherd_core::agent_def::AgentDef>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.create_agent(def).await?)))
}

/// `GET`, `PUT` and `DELETE /agents/{id}`; any other path or method of a job
/// name is the old route (`410`).
async fn agent_one(
    user: User,
    State(d): State<Daemon>,
    method: axum::http::Method,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Response {
    if !d.is_agent(&id).await {
        let p = std::collections::HashMap::from([("id".to_string(), id)]);
        return gone_job(user, State(d), Path(p)).await.into_response();
    }
    let r: ApiResult<Json<Value>> = match method {
        axum::http::Method::GET => d
            .agent(&id)
            .await
            .map(|a| Json(json!(a)))
            .map_err(Into::into),
        axum::http::Method::PUT => match serde_json::from_slice(&body) {
            Ok(def) => d
                .update_agent(&id, def)
                .await
                .map(|a| Json(json!(a)))
                .map_err(Into::into),
            Err(e) => Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                e.to_string(),
            )),
        },
        axum::http::Method::DELETE => d
            .delete_agent(&id)
            .await
            .map(|()| Json(json!({"ok": true})))
            .map_err(Into::into),
        _ => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            format!("{method} {id}"),
        )),
    };
    r.into_response()
}

/// A static agent path (`archive`, `versions`…) of something that is not a
/// saved agent is an old job path.
async fn not_an_agent(d: &Daemon, id: &str, rest: &str) -> ApiError {
    let p = std::collections::HashMap::from([
        ("id".to_string(), id.to_string()),
        ("rest".to_string(), rest.to_string()),
    ]);
    gone_job(User, State(d.clone()), Path(p)).await
}

async fn agent_archive(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !d.is_agent(&id).await {
        return Err(not_an_agent(&d, &id, "archive").await);
    }
    Ok(Json(json!(d.archive_agent(&id, true).await?)))
}

async fn agent_unarchive(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !d.is_agent(&id).await {
        return Err(not_an_agent(&d, &id, "unarchive").await);
    }
    Ok(Json(json!(d.archive_agent(&id, false).await?)))
}

async fn agent_versions(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !d.is_agent(&id).await {
        return Err(not_an_agent(&d, &id, "versions").await);
    }
    Ok(Json(json!({"versions": d.agent_versions(&id).await?})))
}

/// `GET` and `PUT /agents/{id}/memory` (R28).
async fn agent_memory_get(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !d.is_agent(&id).await {
        return Err(not_an_agent(&d, &id, "memory").await);
    }
    Ok(Json(d.agent_memory(&id).await?))
}

async fn agent_memory_put(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> ApiResult<Json<Value>> {
    if !d.is_agent(&id).await {
        return Err(not_an_agent(&d, &id, "memory").await);
    }
    let Some(text) = body.get("text").and_then(Value::as_str) else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "bad_request",
            "text is required".to_string(),
        ));
    };
    Ok(Json(d.set_agent_memory(&id, text).await?))
}

async fn agent_jobs(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    if !d.is_agent(&id).await {
        return Err(not_an_agent(&d, &id, "jobs").await);
    }
    Ok(Json(json!({"jobs": d.agent_jobs(&id).await?})))
}

/// A job from a saved agent (R21): the body is `NewJob`; the name may be left out.
async fn agent_new_job(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Json(req): Json<xherd_core::new::NewJob>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    if !d.is_agent(&id).await {
        return Err(not_an_agent(&d, &id, "jobs").await);
    }
    let job = d.new_agent_job(&id, req).await?;
    Ok((StatusCode::CREATED, Json(json!({"job": job}))))
}

#[derive(Deserialize)]
struct AgentExportQuery {
    name: Option<String>,
}

// ---- stage assignments (R35) ------------------------------------------------

async fn flows_list(_: User, State(d): State<Daemon>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({"flows": d.flows().await?})))
}

async fn flow_one(
    _: User,
    State(d): State<Daemon>,
    Path((pack, flow)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(d.flow(&pack, &flow).await?)))
}

async fn flow_assignments(
    _: User,
    State(d): State<Daemon>,
    Path((pack, flow)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    Ok(Json(
        json!({"assignments": d.assignments(&pack, &flow).await?}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssignmentsBody {
    assignments: Vec<xherd_core::resolve::Assignment>,
}

/// Replaces the flow's assignments; answers the flow as `GET` does.
async fn flow_assignments_put(
    _: User,
    State(d): State<Daemon>,
    Path((pack, flow)): Path<(String, String)>,
    Json(body): Json<AssignmentsBody>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!(
        d.set_assignments(&pack, &flow, body.assignments).await?
    )))
}

async fn agents_export(
    _: User,
    State(d): State<Daemon>,
    Query(q): Query<AgentExportQuery>,
) -> ApiResult<Json<Value>> {
    Ok(Json(d.export_agents(q.name.as_deref()).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentImportQuery {
    dry_run: Option<String>,
}

async fn agents_import(
    _: User,
    State(d): State<Daemon>,
    Query(q): Query<AgentImportQuery>,
    Json(i): Json<xherd_daemon::agents::Import>,
) -> ApiResult<Json<Value>> {
    let dry = q
        .dry_run
        .is_some_and(|v| matches!(v.as_str(), "1" | "true"));
    Ok(Json(d.import_agents(i, dry).await?))
}
