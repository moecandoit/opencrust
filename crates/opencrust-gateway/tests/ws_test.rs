use axum::{
    routing::any,
    Router,
};
use futures::{SinkExt, StreamExt};
use opencrust_gateway::ws::ws_handler;
use opencrust_gateway::state::AppState;
use opencrust_config::AppConfig;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message as TungsteniteMessage;

#[tokio::test]
async fn test_websocket_message_limit() {
    // 1. Setup the server
    let config = AppConfig::default();
    let state = Arc::new(AppState::new(config));

    let app = Router::new()
        .route("/ws", any(ws_handler))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // 2. Connect client
    let url = format!("ws://{}/ws", addr);
    let (mut ws_stream, _) = connect_async(url).await.expect("Failed to connect");

    // 3. Receive welcome message
    let _ = ws_stream.next().await.unwrap().unwrap();

    // 4. Send a message slightly larger than 1MB
    // The limit is 1MB (1048576 bytes). We send 1MB + 100 bytes.
    let large_message = "a".repeat(1024 * 1024 + 100);
    ws_stream.send(TungsteniteMessage::Text(large_message.into())).await.unwrap();

    // 5. Expect the connection to close or error
    // With max_message_size set on the server, the server should detect the large frame/message and close the connection.
    // The client should see a Close frame or an error.
    let result = ws_stream.next().await;

    match result {
        Some(Ok(msg)) => {
             // If we receive a Close frame, that's good.
             // If we receive text, it means it wasn't blocked (bad).
             if let TungsteniteMessage::Close(frame) = msg {
                 println!("Connection closed as expected: {:?}", frame);
             } else {
                 panic!("Expected connection close, got: {:?}", msg);
             }
        }
        Some(Err(e)) => {
            println!("Connection error as expected: {:?}", e);
        }
        None => {
             println!("Connection closed without message");
        }
    }
}
