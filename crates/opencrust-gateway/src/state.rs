use std::sync::Arc;

use dashmap::DashMap;
use opencrust_agents::{AgentRuntime, ChatMessage};
use opencrust_channels::ChannelRegistry;
use opencrust_config::AppConfig;
use opencrust_db::SessionStore;
use tracing::warn;
use uuid::Uuid;

/// Shared application state accessible from all request handlers.
pub struct AppState {
    pub config: AppConfig,
    pub channels: ChannelRegistry,
    pub agents: AgentRuntime,
    pub sessions: DashMap<String, SessionState>,
    pub session_store: Option<Arc<SessionStore>>,
}

/// Per-connection session tracking.
pub struct SessionState {
    pub id: String,
    pub user_id: Option<String>,
    pub channel_id: Option<String>,
    pub history: Vec<ChatMessage>,
}

impl AppState {
    pub fn new(config: AppConfig, session_store: Option<Arc<SessionStore>>) -> Self {
        Self {
            config,
            channels: ChannelRegistry::new(),
            agents: AgentRuntime::new(),
            sessions: DashMap::new(),
            session_store,
        }
    }

    pub fn create_session(&self) -> String {
        let id = Uuid::new_v4().to_string();
        self.sessions.insert(
            id.clone(),
            SessionState {
                id: id.clone(),
                user_id: None,
                channel_id: None,
                history: Vec::new(),
            },
        );

        // Persist to SQLite (best-effort)
        if let Some(store) = &self.session_store
            && let Err(e) = store.create_session(&id, None, None)
        {
            warn!("failed to persist session to db: {}", e);
        }

        id
    }
}

pub type SharedState = Arc<AppState>;
