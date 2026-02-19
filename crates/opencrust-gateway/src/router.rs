use axum::Router;
use axum::response::Html;
use axum::routing::get;
use tower_governor::GovernorLayer;
use tower_governor::governor::GovernorConfigBuilder;

use crate::state::SharedState;
use crate::ws;

/// Build the main application router with all routes.
pub fn build_router(
    state: SharedState,
    whatsapp_state: opencrust_channels::whatsapp::webhook::WhatsAppState,
) -> Router {
    // Per-IP rate limit: 60 requests per minute (1 per second average, burst of 60).
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(1)
        .burst_size(60)
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
        .with_state(state)
        .merge(whatsapp_routes)
        .layer(governor_layer)
}

async fn health() -> &'static str {
    "ok"
}

async fn web_chat() -> Html<&'static str> {
    Html(include_str!("webchat.html"))
}

async fn status(
    axum::extract::State(state): axum::extract::State<SharedState>,
) -> axum::Json<serde_json::Value> {
    let channels = state.channels.list();
    axum::Json(serde_json::json!({
        "status": "running",
        "channels": channels,
        "sessions": state.sessions.len(),
    }))
}
