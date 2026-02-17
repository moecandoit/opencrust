use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::watch;
use tracing::{info, warn};

use crate::model::AppConfig;

const DEBOUNCE_MS: u64 = 500;

/// Watches a config file and broadcasts new `AppConfig` values via a
/// `tokio::sync::watch` channel whenever the file changes on disk.
pub struct ConfigWatcher {
    // Hold the watcher to keep it alive; dropping it stops watching.
    _watcher: RecommendedWatcher,
}

impl ConfigWatcher {
    /// Start watching `config_path`. Returns a receiver that yields the latest
    /// config whenever the file changes. The initial value is `initial_config`.
    pub fn start(
        config_path: PathBuf,
        initial_config: AppConfig,
    ) -> Result<(Self, watch::Receiver<AppConfig>), notify::Error> {
        let (tx, rx) = watch::channel(initial_config);

        // We watch the parent directory because editors often write to a temp
        // file and rename, which doesn't trigger events on the file itself.
        let watch_dir = config_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let target_filename = config_path.file_name().unwrap_or_default().to_os_string();

        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<()>(8);

        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if let Ok(event) = event {
                    let dominated =
                        matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_));
                    if dominated {
                        // Only fire if this event touches our config file
                        let touches_config = event
                            .paths
                            .iter()
                            .any(|p| p.file_name().map(|f| f == target_filename).unwrap_or(false));
                        if touches_config {
                            let _ = notify_tx.try_send(());
                        }
                    }
                }
            })?;

        watcher.watch(&watch_dir, RecursiveMode::NonRecursive)?;

        // Debounced reload loop
        let cfg_path = config_path.clone();
        tokio::spawn(async move {
            loop {
                // Wait for any filesystem event
                if notify_rx.recv().await.is_none() {
                    break; // channel closed
                }
                // Debounce: drain further events within the window
                tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
                while notify_rx.try_recv().is_ok() {}

                // Re-read the config
                match reload_config(&cfg_path) {
                    Ok(new_config) => {
                        info!("config reloaded from {}", cfg_path.display());
                        let _ = tx.send(new_config);
                    }
                    Err(e) => {
                        warn!("config reload failed (keeping previous config): {e}");
                    }
                }
            }
        });

        info!("watching config file: {}", config_path.display());
        Ok((Self { _watcher: watcher }, rx))
    }
}

fn reload_config(path: &Path) -> Result<AppConfig, String> {
    let contents = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "yml" | "yaml" => {
            serde_yaml::from_str(&contents).map_err(|e| format!("YAML parse error: {e}"))
        }
        "toml" => toml::from_str(&contents).map_err(|e| format!("TOML parse error: {e}")),
        other => Err(format!("unsupported config extension: {other}")),
    }
}
