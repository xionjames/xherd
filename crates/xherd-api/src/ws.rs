//! `GET /ws`: the state stream (design §13.1). Every agent change from the
//! daemon's hub is forwarded as `{"type": "agent", "agent": …}`; a client that
//! falls behind is closed and reloads `/state` on reconnect.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use tokio::sync::broadcast::error::RecvError;
use xherd_daemon::Daemon;

use crate::User;

pub async fn state_stream(_: User, State(d): State<Daemon>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| forward(socket, d))
}

async fn forward(mut socket: WebSocket, d: Daemon) {
    let mut rx = d.hub.subscribe();
    loop {
        tokio::select! {
            change = rx.recv() => match change {
                Ok(c) => {
                    let Ok(text) = serde_json::to_string(&c) else { continue };
                    if socket.send(Message::text(text)).await.is_err() {
                        return;
                    }
                }
                // Lagged: the client missed changes; closing makes it reload.
                Err(RecvError::Lagged(_)) | Err(RecvError::Closed) => {
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}
