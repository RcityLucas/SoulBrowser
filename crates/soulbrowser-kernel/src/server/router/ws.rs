use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use chrono::Utc;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::json;
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::server::ServeState;

pub(crate) fn router() -> Router<ServeState> {
    Router::new().route("/ws", get(websocket_handler))
}

#[derive(Deserialize)]
struct ClientMessage {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    _payload: serde_json::Value,
}

async fn websocket_handler(
    State(_state): State<ServeState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        handle_socket(socket).await;
    })
}

async fn handle_socket(mut socket: WebSocket) {
    let welcome = json!({
        "type": "connected",
        "payload": {
            "sessionId": Uuid::new_v4().to_string(),
            "serverVersion": env!("CARGO_PKG_VERSION"),
            "capabilities": ["tasks", "logs"],
            "clientIp": "unknown",
        },
        "timestamp": Utc::now().timestamp_millis(),
    });
    if socket
        .send(Message::Text(welcome.to_string()))
        .await
        .is_err()
    {
        return;
    }

    while let Some(msg) = socket.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(request) = serde_json::from_str::<ClientMessage>(&text) {
                    if request.kind.eq_ignore_ascii_case("ping") {
                        let _ = socket
                            .send(Message::Text(
                                json!({
                                    "type": "pong",
                                    "payload": {},
                                    "timestamp": Utc::now().timestamp_millis(),
                                })
                                .to_string(),
                            ))
                            .await;
                    } else {
                        debug!(target: "ws", "Unhandled client message: {}", request.kind);
                    }
                } else {
                    warn!(target: "ws", "Failed to parse client websocket message");
                }
            }
            Ok(Message::Ping(payload)) => {
                let _ = socket.send(Message::Pong(payload)).await;
            }
            Ok(Message::Close(frame)) => {
                debug!(target: "ws", ?frame, "WebSocket closed by client");
                break;
            }
            Ok(Message::Binary(_)) | Ok(Message::Pong(_)) => {}
            Err(err) => {
                error!(?err, "WebSocket error");
                break;
            }
        }
    }
}
