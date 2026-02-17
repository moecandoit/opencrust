use std::sync::Arc;

use opencrust_common::Result;
use opencrust_config::AppConfig;
use opencrust_db::SessionStore;
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::router::build_router;
use crate::state::AppState;

/// The main gateway server that binds to a port and serves the API + WebSocket.
pub struct GatewayServer {
    config: AppConfig,
}

impl GatewayServer {
    pub fn new(config: AppConfig) -> Self {
        Self { config }
    }

    pub async fn run(self) -> Result<()> {
        let addr = format!("{}:{}", self.config.gateway.host, self.config.gateway.port);

        // Initialize session persistence
        let session_store = self.init_session_store();

        let state = Arc::new(AppState::new(self.config, session_store));
        let app = build_router(state);

        let listener = TcpListener::bind(&addr).await?;
        info!("OpenCrust gateway listening on {}", addr);

        axum::serve(listener, app)
            .await
            .map_err(|e| opencrust_common::Error::Gateway(format!("server error: {e}")))?;

        Ok(())
    }

    fn init_session_store(&self) -> Option<Arc<SessionStore>> {
        let data_dir = self.config.data_dir.clone().unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home)
                .join(".opencrust")
                .join("data")
        });

        if let Err(e) = std::fs::create_dir_all(&data_dir) {
            warn!(
                "failed to create data directory {}: {}",
                data_dir.display(),
                e
            );
            return None;
        }

        let db_path = data_dir.join("sessions.db");
        match SessionStore::open(&db_path) {
            Ok(store) => {
                info!("session store opened at {}", db_path.display());
                Some(Arc::new(store))
            }
            Err(e) => {
                warn!(
                    "failed to open session store, running without persistence: {}",
                    e
                );
                None
            }
        }
    }
}
