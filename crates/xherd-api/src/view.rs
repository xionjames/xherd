//! Herd view plugins (R8, R10, R11): their static files and the view kit.
//! Unauthenticated, like `/assets`, and locked so a view never runs with the
//! UI's rights: every answer carries a CSP starting with `sandbox`, which
//! gives the document an opaque origin even when opened in a tab of its own.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use xherd_daemon::Daemon;
use xherd_plugin::{Kind, view};

/// Kit files: xherd's stylesheet and the protocol script.
const KIT_CSS: &str = include_str!("../../../ui/src/app.css");
const KIT_JS: &str = include_str!("../../../ui/view-kit/xherd-view.js");

/// `GET /api/v1/plugins/{name}/view/{*path}`: a file of an enabled view
/// plugin; 404 for anything else, 413 over `view::MAX_FILE`.
pub async fn plugin_file(
    State(d): State<Daemon>,
    Path((name, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let Some(plugin) = d
        .plugins
        .get(&name)
        .filter(|p| p.manifest.kind == Kind::View && d.plugin_enabled(&name))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(path) = view::clean_path(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let bytes = match &plugin.root {
        None => match view::builtin_file(&name, path) {
            Some(b) => b.to_vec(),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
        Some(root) => {
            let (root, rel) = (root.clone(), path.to_string());
            match tokio::task::spawn_blocking(move || read_under(&root, &rel)).await {
                Ok(Ok(b)) => b,
                Ok(Err(status)) => return status.into_response(),
                Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
    };
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    locked(view::content_type(path), view::csp(host, &name), bytes)
}

/// `GET /api/v1/view-kit/{file}`: `xherd.css` or `xherd-view.js`.
pub async fn kit_file(Path(file): Path<String>) -> Response {
    let (kind, body) = match file.as_str() {
        "xherd.css" => (view::content_type(&file), KIT_CSS),
        "xherd-view.js" => (view::content_type(&file), KIT_JS),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    // Loaded as a subresource the policy does not apply; opened on its own,
    // the file is shown as an inert document.
    let csp = "sandbox; default-src 'none'".to_string();
    locked(kind, csp, body.as_bytes().to_vec())
}

/// A regular file at `rel` under the plugin folder `root`, after resolving
/// symlinks: nothing outside the folder, at most `view::MAX_FILE` bytes.
fn read_under(root: &std::path::Path, rel: &str) -> Result<Vec<u8>, StatusCode> {
    let root = root.canonicalize().map_err(|_| StatusCode::NOT_FOUND)?;
    let file = root
        .join(rel)
        .canonicalize()
        .map_err(|_| StatusCode::NOT_FOUND)?;
    if !file.starts_with(&root) {
        return Err(StatusCode::NOT_FOUND);
    }
    let meta = std::fs::metadata(&file).map_err(|_| StatusCode::NOT_FOUND)?;
    if !meta.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }
    if meta.len() > view::MAX_FILE {
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }
    std::fs::read(&file).map_err(|_| StatusCode::NOT_FOUND)
}

/// The headers every view and kit file carries (design §3).
fn locked(content_type: &str, csp: String, body: Vec<u8>) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (header::CONTENT_SECURITY_POLICY, csp),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
            (header::CACHE_CONTROL, "no-cache".into()),
            (header::REFERRER_POLICY, "no-referrer".into()),
            (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".into()),
        ],
        body,
    )
        .into_response()
}
