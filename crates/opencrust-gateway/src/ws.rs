use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::SinkExt;
use futures::stream::StreamExt;
use tracing::{info, warn};

use crate::state::SharedState;

const MAX_WS_FRAME_BYTES: usize = 64 * 1024;
const MAX_WS_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_WS_TEXT_BYTES: usize = 32 * 1024;

/// WebSocket upgrade handler.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.max_frame_size(MAX_WS_FRAME_BYTES)
        .max_message_size(MAX_WS_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: SharedState) {
    let session_id = state.create_session();
    info!("new WebSocket connection: session={}", session_id);

    let (mut sender, mut receiver) = socket.split();

    // Send welcome message
    let welcome = serde_json::json!({
        "type": "connected",
        "session_id": session_id,
    });
    if sender
        .send(Message::Text(welcome.to_string().into()))
        .await
        .is_err()
    {
        return;
    }

    // Message loop
    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let text_len = text.len();
                info!("received message: session={}, len={}", session_id, text_len);
                if text_message_too_large(text_len) {
                    warn!(
                        "dropping oversized ws text message: session={}, len={}, limit={}",
                        session_id, text_len, MAX_WS_TEXT_BYTES
                    );
                    let err = serde_json::json!({
                        "type": "error",
                        "code": "message_too_large",
                        "max_bytes": MAX_WS_TEXT_BYTES,
                    });
                    let _ = sender.send(Message::Text(err.to_string().into())).await;
                    break;
                }
                // TODO: Route to agent runtime
                let echo = serde_json::json!({
                    "type": "message",
                    "session_id": session_id,
                    "content": format!("echo: {}", text),
                });
                if sender
                    .send(Message::Text(echo.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket closed: session={}", session_id);
                break;
            }
            Err(e) => {
                warn!("WebSocket error: session={}, error={}", session_id, e);
                break;
            }
            _ => {}
        }
    }

    state.sessions.remove(&session_id);
    info!("session cleaned up: {}", session_id);
}

fn text_message_too_large(len: usize) -> bool {
    len > MAX_WS_TEXT_BYTES
}

#[cfg(test)]
mod tests {
    use super::{MAX_WS_TEXT_BYTES, text_message_too_large};

    #[test]
    fn text_message_size_guard_uses_strict_upper_bound() {
        assert!(!text_message_too_large(MAX_WS_TEXT_BYTES));
        assert!(text_message_too_large(MAX_WS_TEXT_BYTES + 1));
    }
}
