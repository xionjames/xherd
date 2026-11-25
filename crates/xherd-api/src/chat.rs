//! `GET /ws/chat/:id?after=<seq>`: a job's chat (R6). Server to client only:
//! `{"type":"hello","unknown":n}`, then the logged events after `after` as
//! `{"type":"event","event":…}`, `{"type":"replayed"}`, the job's current
//! status as `{"type":"status","status":…}`, then live events and
//! `{"type":"unknown","count":n}`. A job whose runtime has no chat is
//! refused before the upgrade with `409 chat_unsupported`.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast::error::RecvError;
use xherd_daemon::chat::ChatMsg;
use xherd_daemon::{ChatSubscription, Daemon};

use crate::{ApiError, User};

#[derive(Deserialize)]
pub struct After {
    #[serde(default)]
    after: u64,
}

pub async fn chat(
    _: User,
    State(d): State<Daemon>,
    Path(id): Path<String>,
    Query(q): Query<After>,
    ws: WebSocketUpgrade,
) -> Response {
    match d.chat_subscribe(&id, q.after).await {
        Ok(sub) => ws.on_upgrade(move |socket| stream(socket, d, sub, q.after)),
        Err(e) => ApiError::from(e).into_response(),
    }
}

async fn send(socket: &mut WebSocket, v: Value) -> bool {
    socket.send(Message::text(v.to_string())).await.is_ok()
}

fn event(e: &xherd_core::chat::ChatEvent) -> Value {
    json!({"type": "event", "event": e})
}

async fn stream(mut socket: WebSocket, d: Daemon, sub: ChatSubscription, after: u64) {
    let ChatSubscription {
        job,
        unknown,
        replay,
        mut rx,
    } = sub;
    // Live events at or below this `seq` were in the replay already.
    let mut last = replay.last().map_or(after, |e| e.seq);
    if !send(&mut socket, json!({"type": "hello", "unknown": unknown})).await {
        return;
    }
    for e in &replay {
        if !send(&mut socket, event(e)).await {
            return;
        }
    }
    if !send(&mut socket, json!({"type": "replayed"})).await {
        return;
    }
    let mut status = job.status;
    if !send(&mut socket, json!({"type": "status", "status": status})).await {
        return;
    }
    let mut changes = d.hub.subscribe();
    loop {
        tokio::select! {
            m = rx.recv() => match m {
                Ok(ChatMsg::Event(e)) => {
                    if e.seq <= last {
                        continue;
                    }
                    last = e.seq;
                    if !send(&mut socket, event(&e)).await {
                        return;
                    }
                }
                Ok(ChatMsg::Unknown(count)) => {
                    if !send(&mut socket, json!({"type": "unknown", "count": count})).await {
                        return;
                    }
                }
                // Lagged: closing makes the client reconnect with `after`.
                Err(RecvError::Lagged(_)) | Err(RecvError::Closed) => {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            },
            c = changes.recv() => match c {
                Ok(change) => {
                    if let Some(s) = status_of(&change, &job.id) && s != status {
                        status = s;
                        if !send(&mut socket, json!({"type": "status", "status": status})).await {
                            return;
                        }
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => return,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}

fn status_of(c: &xherd_daemon::support::Change, id: &str) -> Option<xherd_core::status::Status> {
    match c {
        xherd_daemon::support::Change::Job { job } if job.id == id => Some(job.status),
        _ => None,
    }
}
