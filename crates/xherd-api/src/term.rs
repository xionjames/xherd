//! `GET /ws/term/:id?cols=&rows=`: the agent's tmux session in the browser
//! (spec §17.2). `GET /ws/setup/:id`: the session a setup fix started (the
//! runtime's trust dialog, a git host login), so setup needs no shell on the host. Binary frames carry terminal bytes both ways; a text frame
//! `{"type":"resize","cols":…,"rows":…}` resizes. Closing the socket detaches
//! the tmux client; the session keeps running.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use xherd_daemon::{Daemon, UserAction};
use xherd_term::Tmux;
use xherd_term::pty::PtyBridge;

use crate::{ApiError, User};

#[derive(Deserialize)]
pub struct Size {
    #[serde(default = "cols")]
    cols: u16,
    #[serde(default = "rows")]
    rows: u16,
}
fn cols() -> u16 {
    120
}
fn rows() -> u16 {
    32
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
enum ClientMsg {
    Resize { cols: u16, rows: u16 },
}

pub async fn terminal(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Query(size): Query<Size>,
    ws: WebSocketUpgrade,
) -> Response {
    let agent = match d.action(&id, UserAction::Get).await {
        Ok(a) => a,
        Err(e) => return ApiError::from(e).into_response(),
    };
    attach(&d, &agent.name, size, ws).await
}

pub async fn setup_terminal(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Query(size): Query<Size>,
    ws: WebSocketUpgrade,
) -> Response {
    let session = match id.as_str() {
        xherd_daemon::checks::TRUST_CHECK => xherd_daemon::checks::TRUST_SESSION,
        xherd_daemon::git_access::LOGIN_ID => xherd_daemon::git_access::LOGIN_SESSION,
        _ => {
            return ApiError::new(
                StatusCode::NOT_FOUND,
                "no_session",
                format!("no setup session {id}"),
            )
            .into_response();
        }
    };
    attach(&d, session, size, ws).await
}

async fn attach(d: &Daemon, session: &str, size: Size, ws: WebSocketUpgrade) -> Response {
    let tmux = Tmux::new(d.config.tmux_socket.clone());
    if !tmux.has_session(session).await {
        return ApiError::new(
            StatusCode::CONFLICT,
            "no_session",
            format!("{session} has no running session"),
        )
        .into_response();
    }
    let argv = tmux.attach_argv(session);
    ws.on_upgrade(move |socket| bridge(socket, argv, size))
}

async fn bridge(mut socket: WebSocket, argv: Vec<String>, size: Size) {
    let (mut pty, mut out) = match PtyBridge::spawn(&argv, size.cols, size.rows) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "terminal: cannot start the tmux client");
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    loop {
        tokio::select! {
            chunk = out.recv() => match chunk {
                Some(bytes) => {
                    if socket.send(Message::Binary(bytes.into())).await.is_err() {
                        return;
                    }
                }
                // The client exited (session ended or detached).
                None => {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Binary(b))) => {
                    if pty.input(&b).is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Text(t))) => {
                    if let Ok(ClientMsg::Resize { cols, rows }) = serde_json::from_str(t.as_str()) {
                        let _ = pty.resize(cols, rows);
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}
