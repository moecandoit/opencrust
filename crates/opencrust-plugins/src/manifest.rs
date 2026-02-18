use opencrust_common::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The main plugin manifest structure (plugin.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub plugin: PluginMetadata,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub limits: Limits,
}

/// Metadata about the plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMetadata {
    pub name: String,
    pub version: String,
    pub description: String,
}

/// Capability-based permissions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Permissions {
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub filesystem: bool, // For now, just a boolean based on example.
    #[serde(default)]
    pub env_vars: Vec<String>,
}

/// Resource limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_memory")]
    pub max_memory_mb: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout(),
            max_memory_mb: default_memory(),
        }
    }
}

fn default_timeout() -> u64 {
    30
}

fn default_memory() -> u64 {
    64
}

impl PluginManifest {
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let manifest: Self = toml::from_str(&content)
            .map_err(|e| Error::Plugin(format!("invalid manifest: {}", e)))?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_parsing() {
        let toml = r#"
[plugin]
name = "test-plugin"
version = "0.1.0"
description = "A test plugin"

[permissions]
filesystem = true
network = ["example.com"]
env_vars = ["TEST_VAR"]

[limits]
timeout_secs = 10
max_memory_mb = 128
"#;
        let manifest: PluginManifest = toml::from_str(toml).unwrap();
        assert_eq!(manifest.plugin.name, "test-plugin");
        assert!(manifest.permissions.filesystem);
        assert_eq!(manifest.permissions.network, vec!["example.com"]);
        assert_eq!(manifest.limits.timeout_secs, 10);
    }

    #[test]
    fn test_manifest_defaults() {
        let toml = r#"
[plugin]
name = "defaults"
version = "0.0.1"
description = "minimal"
"#;
        let manifest: PluginManifest = toml::from_str(toml).unwrap();
        assert!(!manifest.permissions.filesystem);
        assert!(manifest.permissions.network.is_empty());
        assert_eq!(manifest.limits.timeout_secs, 30); // Default
    }
}
