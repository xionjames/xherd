//! The web UI (design §13.1): `ui/dist` embedded at compile time in release
//! builds and read from disk in debug builds (`rust-embed`). Without a built
//! UI the page says how to build it.

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
#[allow_missing = true]
struct Assets;

const NOT_BUILT: &str = "<!doctype html><meta charset=utf-8><title>xherd</title>\
<p style=\"font:14px system-ui;margin:40px\">The xherd UI is not built. Run \
<code>pnpm install &amp;&amp; pnpm build</code> in <code>ui/</code>, then rebuild the daemon.</p>";

fn file(path: &str) -> Option<Response> {
    let f = Assets::get(path)?;
    let mime = f.metadata.mimetype().to_string();
    // Hashed asset names never change content; index.html must be re-read.
    let cache = if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Some(
        (
            [
                (header::CONTENT_TYPE, mime),
                (header::CACHE_CONTROL, cache.to_string()),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
            ],
            f.data.into_owned(),
        )
            .into_response(),
    )
}

pub async fn index() -> Response {
    file("index.html").unwrap_or_else(|| Html(NOT_BUILT).into_response())
}

pub async fn asset(Path(rest): Path<String>) -> Response {
    file(&format!("assets/{rest}")).unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}
