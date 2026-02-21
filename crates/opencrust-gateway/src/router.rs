use std::sync::Arc;

use axum::Router;
use axum::response::Html;
use axum::routing::{get, post};
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;
use tower_http::services::ServeDir;

use crate::a2a;
use crate::api;
use crate::state::SharedState;
use crate::ws;

/// Build the main application router with all routes.
pub fn build_router(
    state: SharedState,
    whatsapp_state: opencrust_channels::whatsapp::webhook::WhatsAppState,
) -> Router {
    // Per-IP rate limit from config (default: 1 req/sec, burst 60).
    let rl = &state.config.gateway.rate_limit;
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(rl.per_second)
        .burst_size(rl.burst_size)
        .finish()
        .expect("governor config should be valid");
    let governor_limiter = governor_conf.limiter().clone();
    let governor_layer = GovernorLayer::new(governor_conf);

    // Spawn a background task to clean up rate-limiter state for inactive IPs.
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(60);
        loop {
            tokio::time::sleep(interval).await;
            governor_limiter.retain_recent();
        }
    });

    let whatsapp_routes = Router::new()
        .route(
            "/webhooks/whatsapp",
            get(opencrust_channels::whatsapp::webhook::whatsapp_verify)
                .post(opencrust_channels::whatsapp::webhook::whatsapp_webhook),
        )
        .with_state(whatsapp_state);

    Router::new()
        .route("/", get(web_chat))
        .route("/health", get(health))
        .route("/ws", get(ws::ws_handler))
        .route("/api/status", get(status))
        .route("/api/auth-check", get(auth_check))
        .route(
            "/api/sessions",
            get(api::list_sessions).post(api::create_session),
        )
        .route("/api/sessions/{id}/messages", post(api::send_message))
        .route("/api/sessions/{id}/history", get(api::session_history))
        .route("/api/providers", get(list_providers).post(add_provider))
        .route("/api/mcp", get(list_mcp_servers))
        // A2A protocol endpoints
        .route("/.well-known/agent.json", get(a2a::agent_card))
        .route("/a2a/tasks", post(a2a::create_task))
        .route("/a2a/tasks/{id}", get(a2a::get_task))
        .route("/a2a/tasks/{id}/cancel", post(a2a::cancel_task))
        .nest_service("/assets", ServeDir::new("assets"))
        .with_state(state)
        .merge(whatsapp_routes)
        .layer(governor_layer)
}

async fn health() -> &'static str {
    "ok"
}

async fn web_chat() -> Html<String> {
    // Hot-reload during local development if the source file is present
    if let Ok(content) = std::fs::read_to_string("crates/opencrust-gateway/src/webchat.html") {
        return Html(content);
    }

    // Fall back to the statically compiled version for release binaries
    Html(include_str!("webchat.html").to_string())
}

async fn status(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> axum::Json<serde_json::Value> {
    let channels = state.channels.list();

    // Gather LLM provider info from config
    let llm: serde_json::Value = state
        .config
        .llm
        .iter()
        .map(|(name, cfg)| {
            let mut info = serde_json::json!({ "provider": cfg.provider });
            if let Some(m) = &cfg.model {
                info["model"] = serde_json::Value::String(m.clone());
            }
            (name.clone(), info)
        })
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();

    // Check for available update (from cached check file)
    let latest_version = read_cached_latest_version();

    let mut resp = serde_json::json!({
        "status": "running",
        "version": env!("CARGO_PKG_VERSION"),
        "channels": channels,
        "sessions": state.sessions.len(),
        "llm": llm,
    });
    if let Some(latest) = latest_version {
        let current = env!("CARGO_PKG_VERSION");
        if latest.trim_start_matches('v') != current {
            resp["latest_version"] = serde_json::Value::String(latest);
        }
    }

    axum::Json(resp)
}

async fn auth_check(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> axum::Json<serde_json::Value> {
    axum::Json(serde_json::json!({
        "auth_required": state.config.gateway.api_key.is_some(),
    }))
}

/// Known provider types that can be added at runtime.
const KNOWN_PROVIDERS: &[(&str, &str, bool)] = &[
    ("anthropic", "Anthropic", true),
    ("openai", "OpenAI", true),
    ("deepseek", "DeepSeek", true),
    ("mistral", "Mistral", true),
    ("sansa", "Sansa", true),
    ("gemini", "Google Gemini", true),
    ("falcon", "Falcon", true),
    ("jais", "Jais", true),
    ("qwen", "Qwen", true),
    ("yi", "Yi", true),
    ("cohere", "Cohere", true),
    ("minimax", "MiniMax", true),
    ("moonshot", "Moonshot K2", true),
    ("ollama", "Ollama", false),
];

/// GET /api/providers — list known provider types with activation status.
async fn list_providers(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> axum::Json<serde_json::Value> {
    let active_ids = state.agents.provider_ids();
    let default_id = state.agents.default_provider_id();

    let mut providers: Vec<serde_json::Value> = Vec::with_capacity(KNOWN_PROVIDERS.len());
    for (id, display, needs_key) in KNOWN_PROVIDERS {
        let active = active_ids.contains(&id.to_string());
        let mut model = None;
        let mut models = Vec::new();

        if active && let Some(provider) = state.agents.get_provider(id) {
            model = provider.configured_model().map(|m| m.to_string());
            match provider.available_models().await {
                Ok(mut available) => {
                    available.retain(|m| !m.trim().is_empty());
                    available.sort();
                    available.dedup();
                    models = available;
                }
                Err(err) => {
                    tracing::warn!("failed to list models for provider {}: {}", id, err);
                }
            }
        }

        if let Some(selected) = model.as_ref()
            && !models.iter().any(|m| m == selected)
        {
            models.insert(0, selected.clone());
        }

        providers.push(serde_json::json!({
            "id": id,
            "display_name": display,
            "active": active,
            "is_default": default_id.as_deref() == Some(*id),
            "needs_api_key": *needs_key,
            "model": model,
            "models": models,
        }));
    }

    axum::Json(serde_json::json!({ "providers": providers }))
}

#[derive(serde::Deserialize)]
struct AddProviderRequest {
    provider_type: String,
    api_key: Option<String>,
    model: Option<String>,
    base_url: Option<String>,
    set_default: Option<bool>,
}

/// POST /api/providers — add a new LLM provider at runtime.
async fn add_provider(
    axum::extract::State(state): axum::extract::State<SharedState>,
    axum::Json(body): axum::Json<AddProviderRequest>,
) -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
    let provider_type = body.provider_type.as_str();

    // Check if this provider type already exists
    let existing = state.agents.provider_ids();
    if existing.contains(&provider_type.to_string()) {
        // If requesting set_default, just switch
        if body.set_default == Some(true) {
            state.agents.set_default_provider_id(provider_type);
            return (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({
                    "status": "ok",
                    "message": format!("switched default provider to {provider_type}"),
                })),
            );
        }
        return (
            axum::http::StatusCode::CONFLICT,
            axum::Json(serde_json::json!({
                "status": "error",
                "message": format!("provider '{provider_type}' is already active"),
            })),
        );
    }

    // Build and register the provider
    match provider_type {
        "anthropic" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for anthropic",
                    })),
                );
            };
            let provider = opencrust_agents::AnthropicProvider::new(
                key.clone(),
                body.model.clone(),
                body.base_url.clone(),
            );
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("ANTHROPIC_API_KEY", key);
        }
        "openai" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for openai",
                    })),
                );
            };
            let provider = opencrust_agents::OpenAiProvider::new(
                key.clone(),
                body.model.clone(),
                body.base_url.clone(),
            );
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("OPENAI_API_KEY", key);
        }
        "sansa" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for sansa",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.sansaml.com".to_string()));
            let model = body
                .model
                .clone()
                .or_else(|| Some("sansa-auto".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("sansa");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("SANSA_API_KEY", key);
        }
        "deepseek" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for deepseek",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.deepseek.com".to_string()));
            let model = body
                .model
                .clone()
                .or_else(|| Some("deepseek-chat".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("deepseek");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("DEEPSEEK_API_KEY", key);
        }
        "mistral" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for mistral",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.mistral.ai".to_string()));
            let model = body
                .model
                .clone()
                .or_else(|| Some("mistral-large-latest".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("mistral");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("MISTRAL_API_KEY", key);
        }
        "gemini" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for gemini",
                    })),
                );
            };
            let base_url = body.base_url.clone().or_else(|| {
                Some("https://generativelanguage.googleapis.com/v1beta/openai/".to_string())
            });
            let model = body
                .model
                .clone()
                .or_else(|| Some("gemini-2.5-flash".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("gemini");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("GEMINI_API_KEY", key);
        }
        "falcon" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for falcon",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.ai71.ai/v1".to_string()));
            let model = body
                .model
                .clone()
                .or_else(|| Some("tiiuae/falcon-180b-chat".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("falcon");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("FALCON_API_KEY", key);
        }
        "jais" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for jais",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.core42.ai/v1".to_string()));
            let model = body
                .model
                .clone()
                .or_else(|| Some("jais-adapted-70b-chat".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("jais");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("JAIS_API_KEY", key);
        }
        "qwen" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for qwen",
                    })),
                );
            };
            let base_url = body.base_url.clone().or_else(|| {
                Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1".to_string())
            });
            let model = body.model.clone().or_else(|| Some("qwen-plus".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("qwen");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("QWEN_API_KEY", key);
        }
        "yi" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for yi",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.lingyiwanwu.com/v1".to_string()));
            let model = body.model.clone().or_else(|| Some("yi-large".to_string()));
            let provider =
                opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url).with_name("yi");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("YI_API_KEY", key);
        }
        "cohere" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for cohere",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.cohere.com/compatibility/v1".to_string()));
            let model = body
                .model
                .clone()
                .or_else(|| Some("command-r-plus".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("cohere");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("COHERE_API_KEY", key);
        }
        "minimax" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for minimax",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.minimaxi.chat/v1".to_string()));
            let model = body
                .model
                .clone()
                .or_else(|| Some("MiniMax-Text-01".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("minimax");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("MINIMAX_API_KEY", key);
        }
        "moonshot" => {
            let Some(key) = &body.api_key else {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "status": "error",
                        "message": "api_key is required for moonshot",
                    })),
                );
            };
            let base_url = body
                .base_url
                .clone()
                .or_else(|| Some("https://api.moonshot.cn/v1".to_string()));
            let model = body
                .model
                .clone()
                .or_else(|| Some("kimi-k2-0711-preview".to_string()));
            let provider = opencrust_agents::OpenAiProvider::new(key.clone(), model, base_url)
                .with_name("moonshot");
            state.agents.register_provider(Arc::new(provider));
            persist_api_key("MOONSHOT_API_KEY", key);
        }
        "ollama" => {
            let provider =
                opencrust_agents::OllamaProvider::new(body.model.clone(), body.base_url.clone());
            state.agents.register_provider(Arc::new(provider));
        }
        other => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "status": "error",
                    "message": format!("unknown provider type: {other}"),
                })),
            );
        }
    }

    if body.set_default == Some(true) {
        state.agents.set_default_provider_id(provider_type);
    }

    (
        axum::http::StatusCode::CREATED,
        axum::Json(serde_json::json!({
            "status": "ok",
            "message": format!("provider '{provider_type}' activated"),
        })),
    )
}

/// Best-effort: persist an API key in the vault.
fn persist_api_key(vault_key: &str, value: &str) {
    if let Some(vault_path) = crate::bootstrap::default_vault_path() {
        opencrust_security::try_vault_set(&vault_path, vault_key, value);
    }
}

/// GET /api/mcp — list connected MCP servers with tool counts and status.
async fn list_mcp_servers(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> axum::Json<serde_json::Value> {
    let servers = if let Some(mgr) = &state.mcp_manager_arc {
        let list = mgr.list_servers().await;
        list.into_iter()
            .map(|(name, tool_count, connected)| {
                serde_json::json!({
                    "name": name,
                    "tools": tool_count,
                    "connected": connected,
                })
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    axum::Json(serde_json::json!({ "servers": servers }))
}

/// Read the cached latest version from ~/.opencrust/update-check.json.
fn read_cached_latest_version() -> Option<String> {
    let path = opencrust_config::ConfigLoader::default_config_dir().join("update-check.json");
    let contents = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&contents).ok()?;
    v.get("latest_version")?.as_str().map(|s| s.to_string())
}
