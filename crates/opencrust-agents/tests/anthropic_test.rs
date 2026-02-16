use axum::{
    extract::Json,
    response::{IntoResponse, Sse},
    routing::post,
    Router,
};
use axum::response::sse::{Event, KeepAlive};
use futures::stream::{self, StreamExt};
use opencrust_agents::providers::{
    AnthropicProvider, ChatMessage, ChatRole, LlmProvider, LlmRequest, LlmStreamResponse,
    MessagePart, ContentBlock,
};
use opencrust_common::Result;
use serde_json::json;
use std::net::SocketAddr;
use tokio::sync::oneshot;
use std::io;

// Mock server setup
async fn start_mock_server() -> (SocketAddr, oneshot::Sender<()>) {
    let (tx, rx) = oneshot::channel::<()>();

    let app = Router::new().route("/v1/messages", post(mock_messages));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                rx.await.ok();
            })
            .await
            .unwrap();
    });

    (addr, tx)
}

async fn mock_messages(Json(payload): Json<serde_json::Value>) -> impl IntoResponse {
    let stream = payload["stream"].as_bool().unwrap_or(false);

    if stream {
        let stream = stream::iter(vec![
            Ok::<_, io::Error>(Event::default().data(
                json!({
                    "type": "message_start",
                    "message": {
                        "id": "msg_123",
                        "type": "message",
                        "role": "assistant",
                        "content": [],
                        "model": "claude-3-opus-20240229",
                        "stop_reason": null,
                        "stop_sequence": null,
                        "usage": {"input_tokens": 10, "output_tokens": 1}
                    }
                })
                .to_string(),
            )),
            Ok::<_, io::Error>(Event::default().data(
                json!({
                    "type": "content_block_start",
                    "index": 0,
                    "content_block": {"type": "text", "text": ""}
                })
                .to_string(),
            )),
            Ok::<_, io::Error>(Event::default().data(
                json!({
                    "type": "content_block_delta",
                    "index": 0,
                    "delta": {"type": "text_delta", "text": "Hello"}
                })
                .to_string(),
            )),
            Ok::<_, io::Error>(Event::default().data(
                json!({
                    "type": "content_block_stop",
                    "index": 0
                })
                .to_string(),
            )),
            Ok::<_, io::Error>(Event::default().data(
                json!({
                    "type": "message_delta",
                    "delta": {"stop_reason": "end_turn", "stop_sequence": null},
                    "usage": {"output_tokens": 5}
                })
                .to_string(),
            )),
            Ok::<_, io::Error>(Event::default().data(
                json!({
                    "type": "message_stop"
                })
                .to_string(),
            )),
        ]);

        Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response()
    } else {
        Json(json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                {
                    "type": "text",
                    "text": "Hello world"
                }
            ],
            "model": "claude-3-opus-20240229",
            "stop_reason": "end_turn",
            "stop_sequence": null,
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5
            }
        }))
        .into_response()
    }
}

#[tokio::test]
async fn test_anthropic_complete() -> Result<()> {
    let (addr, _shutdown_tx) = start_mock_server().await;
    let base_url = format!("http://{}/v1/messages", addr);

    let provider = AnthropicProvider::new("test-key".to_string()).with_base_url(base_url);

    let request = LlmRequest {
        model: "claude-3-opus-20240229".to_string(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: MessagePart::Text("Hello".to_string()),
        }],
        system: None,
        max_tokens: Some(100),
        temperature: None,
        tools: vec![],
    };

    let response = provider.complete(&request).await?;

    assert_eq!(response.content.len(), 1);
    match &response.content[0] {
        ContentBlock::Text { text } => assert_eq!(text, "Hello world"),
        _ => panic!("Expected text content"),
    }

    Ok(())
}

#[tokio::test]
async fn test_anthropic_system_message_error() -> Result<()> {
    let (addr, _shutdown_tx) = start_mock_server().await;
    let base_url = format!("http://{}/v1/messages", addr);

    let provider = AnthropicProvider::new("test-key".to_string()).with_base_url(base_url);

    let request = LlmRequest {
        model: "claude-3-opus-20240229".to_string(),
        messages: vec![ChatMessage {
            role: ChatRole::System,
            content: MessagePart::Text("You are a helpful assistant.".to_string()),
        }],
        system: None,
        max_tokens: Some(100),
        temperature: None,
        tools: vec![],
    };

    let result = provider.complete(&request).await;
    assert!(result.is_err());
    if let Err(opencrust_common::Error::Agent(msg)) = result {
        assert_eq!(msg, "System messages should be passed via the `system` field, not in `messages`");
    } else {
        panic!("Expected Agent error");
    }

    Ok(())
}

#[tokio::test]
async fn test_anthropic_stream() -> Result<()> {
    let (addr, _shutdown_tx) = start_mock_server().await;
    let base_url = format!("http://{}/v1/messages", addr);

    let provider = AnthropicProvider::new("test-key".to_string()).with_base_url(base_url);

    let request = LlmRequest {
        model: "claude-3-opus-20240229".to_string(),
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: MessagePart::Text("Hello".to_string()),
        }],
        system: None,
        max_tokens: Some(100),
        temperature: None,
        tools: vec![],
    };

    let mut stream = provider.stream(&request).await?;
    let mut chunks = Vec::new();

    while let Some(chunk) = stream.next().await {
        chunks.push(chunk?);
    }

    assert!(chunks.len() >= 2);

    let has_message_start = chunks.iter().any(|c| match c {
        LlmStreamResponse::MessageStart { usage } => {
            usage.as_ref().map(|u| u.input_tokens).unwrap_or(0) == 10
        },
        _ => false,
    });
    assert!(has_message_start, "Missing MessageStart with input_tokens=10");

    let has_hello = chunks.iter().any(|c| match c {
        LlmStreamResponse::ContentBlockDelta { delta, .. } => {
             match delta {
                 opencrust_agents::providers::ContentBlockDelta::Text { text } => text == "Hello",
                 _ => false
             }
        },
        _ => false,
    });

    assert!(has_hello);

    Ok(())
}
