use super::{
    ChatMessage, ChatRole, ContentBlock, ContentBlockDelta, LlmProvider, LlmRequest, LlmResponse,
    LlmStreamResponse, MessagePart, Usage,
};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt};
use opencrust_common::{Error, Result};
use reqwest::Client;
use serde_json::json;
use std::env;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    api_key: String,
    client: Client,
    base_url: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: Client::new(),
            base_url: ANTHROPIC_API_URL.to_string(),
        }
    }

    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
    }

    pub fn from_env() -> Result<Self> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .map_err(|_| Error::Config("ANTHROPIC_API_KEY not set".to_string()))?;
        Ok(Self::new(api_key))
    }

    async fn process_messages(&self, messages: &[ChatMessage]) -> Result<Vec<serde_json::Value>> {
        let mut processed_messages = Vec::new();

        for msg in messages {
            let content = match &msg.content {
                MessagePart::Text(text) => json!(text),
                MessagePart::Parts(parts) => {
                    let mut processed_parts = Vec::new();
                    for part in parts {
                        match part {
                            ContentBlock::Text { text } => {
                                processed_parts.push(json!({
                                    "type": "text",
                                    "text": text
                                }));
                            }
                            ContentBlock::Image { url } => {
                                let (media_type, data) = if url.starts_with("data:") {
                                    let parts: Vec<&str> = url.split(',').collect();
                                    if parts.len() != 2 {
                                        return Err(Error::Agent("Invalid data URL".to_string()));
                                    }
                                    let meta = parts[0];
                                    let data = parts[1];
                                    let media_type = meta
                                        .split(';')
                                        .next()
                                        .ok_or_else(|| Error::Agent("Invalid data URL".to_string()))?
                                        .trim_start_matches("data:");
                                    (media_type.to_string(), data.to_string())
                                } else if url.starts_with("http") {
                                    let response = self.client.get(url).send().await.map_err(|e: reqwest::Error| Error::Agent(format!("Network error: {}", e)))?;
                                    let media_type = response
                                        .headers()
                                        .get(reqwest::header::CONTENT_TYPE)
                                        .and_then(|v| v.to_str().ok())
                                        .map(|s| s.to_string())
                                        .unwrap_or_else(|| "image/jpeg".to_string());
                                    let bytes = response.bytes().await.map_err(|e: reqwest::Error| Error::Agent(format!("Network error: {}", e)))?;
                                    (media_type, BASE64.encode(bytes))
                                } else {
                                    return Err(Error::Agent("Unsupported image URL scheme".to_string()));
                                };

                                processed_parts.push(json!({
                                    "type": "image",
                                    "source": {
                                        "type": "base64",
                                        "media_type": media_type,
                                        "data": data
                                    }
                                }));
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                processed_parts.push(json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input
                                }));
                            }
                            ContentBlock::ToolResult { tool_use_id, content } => {
                                processed_parts.push(json!({
                                    "type": "tool_result",
                                    "tool_use_id": tool_use_id,
                                    "content": content
                                }));
                            }
                        }
                    }
                    json!(processed_parts)
                }
            };

            processed_messages.push(json!({
                "role": match msg.role {
                    ChatRole::User => "user",
                    ChatRole::Assistant => "assistant",
                    ChatRole::Tool => "user", // Anthropic expects tool_result in user role
                    ChatRole::System => return Err(Error::Agent("System messages should be passed via the `system` field, not in `messages`".to_string())),
                },
                "content": content
            }));
        }
        Ok(processed_messages)
    }

    async fn create_request_body(&self, request: &LlmRequest, stream: bool) -> Result<serde_json::Value> {
         let messages = self.process_messages(&request.messages).await?;

        let mut body = json!({
            "model": request.model,
            "messages": messages,
            "max_tokens": request.max_tokens.unwrap_or(1024),
            "stream": stream,
        });

        if let Some(system) = &request.system {
            body["system"] = json!(system);
        }

        if let Some(temp) = request.temperature {
            body["temperature"] = json!(temp);
        }

        if !request.tools.is_empty() {
             body["tools"] = json!(request.tools.iter().map(|t| json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema
            })).collect::<Vec<_>>());
        }

        Ok(body)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn provider_id(&self) -> &str {
        "anthropic"
    }

    async fn complete(&self, request: &LlmRequest) -> Result<LlmResponse> {
        let body = self.create_request_body(request, false).await?;

        let response = self.client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e: reqwest::Error| Error::Agent(format!("Network error: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Agent(format!("Anthropic API error: {}", error_text)));
        }

        let raw_response: serde_json::Value = response.json().await.map_err(|e: reqwest::Error| Error::Serialization(serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::Other, e))))?;

        let content_blocks = raw_response["content"].as_array()
            .ok_or_else(|| Error::Agent("Missing content".to_string()))?
            .iter()
            .map(|block| {
                let type_ = block["type"].as_str().unwrap_or_default();
                match type_ {
                    "text" => Ok(ContentBlock::Text {
                        text: block["text"].as_str().unwrap_or_default().to_string(),
                    }),
                    "tool_use" => Ok(ContentBlock::ToolUse {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                        input: block["input"].clone(),
                    }),
                    _ => Err(Error::Agent(format!("Unknown content block type: {}", type_))),
                }
            })
            .collect::<Result<Vec<_>>>()?;

        let usage = raw_response["usage"].as_object().map(|u| Usage {
            input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
            output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(LlmResponse {
            content: content_blocks,
            model: raw_response["model"].as_str().unwrap_or_default().to_string(),
            usage,
            stop_reason: raw_response["stop_reason"].as_str().map(|s: &str| s.to_string()),
        })
    }

    async fn stream(
        &self,
        request: &LlmRequest,
    ) -> Result<BoxStream<'static, Result<LlmStreamResponse>>> {
        let body = self.create_request_body(request, true).await?;

        let response = self.client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e: reqwest::Error| Error::Agent(format!("Network error: {}", e)))?;

        if !response.status().is_success() {
             let error_text = response.text().await.unwrap_or_default();
            return Err(Error::Agent(format!("Anthropic API error: {}", error_text)));
        }

        let stream = response.bytes_stream().boxed();
        let buffer = Vec::new();

        let s = stream::try_unfold((stream, buffer), |(mut stream, mut buffer): (BoxStream<'static, reqwest::Result<Bytes>>, Vec<u8>)| async move {
            loop {
                 if let Some(i) = buffer.iter().position(|&b| b == b'\n') {
                     let line_bytes: Vec<u8> = buffer.drain(0..=i).collect();
                     let line = String::from_utf8_lossy(&line_bytes).trim().to_string();

                     if line.starts_with("data: ") {
                         let data = &line[6..];
                         if data == "[DONE]" {
                             // End of stream?
                         } else {
                             match serde_json::from_str::<serde_json::Value>(data) {
                                 Ok(json) => {
                                     if let Some(event) = parse_anthropic_event(&json) {
                                         return Ok(Some((event, (stream, buffer))));
                                     }
                                 }
                                 Err(_) => {} // skip invalid json
                             }
                         }
                     }
                     // If line doesn't start with data:, just skip and continue to next line
                     continue;
                 }

                 match stream.next().await {
                     Some(Ok(chunk)) => {
                         buffer.extend_from_slice(&chunk);
                     }
                     Some(Err(e)) => return Err(Error::Agent(format!("Network error: {}", e))),
                     None => {
                         if !buffer.is_empty() {
                             // Process remaining if needed?
                         }
                         return Ok(None);
                     }
                 }
            }
        });

        Ok(Box::pin(s))
    }

    async fn health_check(&self) -> Result<bool> {
        let body = json!({
            "model": "claude-3-haiku-20240307",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "ping"}]
        });

        let response = self.client
            .post(&self.base_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}

fn parse_anthropic_event(json: &serde_json::Value) -> Option<LlmStreamResponse> {
    let type_ = json["type"].as_str().unwrap_or_default();
    match type_ {
        "message_start" => {
            let usage = json["message"]["usage"].as_object().map(|u| Usage {
                input_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
                output_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
            });
            Some(LlmStreamResponse::MessageStart { usage })
        },
        "content_block_start" => {
            let index = json["index"].as_u64().unwrap_or(0) as u32;
            let block = &json["content_block"];
            let type_ = block["type"].as_str().unwrap_or_default();
            if type_ == "text" {
                 Some(LlmStreamResponse::ContentBlockStart {
                     index,
                     content_block: ContentBlock::Text { text: block["text"].as_str().unwrap_or_default().to_string() }
                 })
            } else if type_ == "tool_use" {
                Some(LlmStreamResponse::ContentBlockStart {
                     index,
                     content_block: ContentBlock::ToolUse {
                        id: block["id"].as_str().unwrap_or_default().to_string(),
                        name: block["name"].as_str().unwrap_or_default().to_string(),
                        input: json!({}),
                    }
                 })
            } else {
                None
            }
        },
        "content_block_delta" => {
            let index = json["index"].as_u64().unwrap_or(0) as u32;
            let delta = &json["delta"];
            let type_ = delta["type"].as_str().unwrap_or_default();
            if type_ == "text_delta" {
                 Some(LlmStreamResponse::ContentBlockDelta {
                     index,
                     delta: ContentBlockDelta::Text { text: delta["text"].as_str().unwrap_or_default().to_string() }
                 })
            } else if type_ == "input_json_delta" {
                 Some(LlmStreamResponse::ContentBlockDelta {
                     index,
                     delta: ContentBlockDelta::ToolUse { partial_json: delta["partial_json"].as_str().unwrap_or_default().to_string() }
                 })
            } else {
                None
            }
        },
        "content_block_stop" => {
            let index = json["index"].as_u64().unwrap_or(0) as u32;
            Some(LlmStreamResponse::ContentBlockStop { index })
        },
        "message_delta" => {
            let stop_reason = json["delta"]["stop_reason"].as_str().map(|s| s.to_string());
            let usage = json["usage"]["output_tokens"].as_u64().map(|tokens| Usage { input_tokens: 0, output_tokens: tokens as u32 });
            Some(LlmStreamResponse::MessageStop { stop_reason, usage })
        },
        "ping" => Some(LlmStreamResponse::Ping),
        _ => None,
    }
}
