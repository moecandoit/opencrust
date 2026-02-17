use opencrust_common::{Error, Result};
use serde::Deserialize;
use serenity::all::GatewayIntents;
use std::collections::HashMap;

/// Discord-specific configuration extracted from the generic `ChannelConfig`.
#[derive(Debug, Clone)]
pub struct DiscordConfig {
    /// Bot token for authenticating with Discord.
    pub bot_token: String,

    /// Discord application ID (snowflake).
    pub application_id: u64,

    /// Guild IDs for registering guild-specific slash commands.
    /// If empty, commands are registered globally.
    pub guild_ids: Vec<u64>,

    /// Gateway intents to request from Discord.
    pub intents: GatewayIntents,

    /// Optional command prefix for text-based commands.
    pub prefix: Option<String>,
}

/// Intermediate struct for deserializing from the settings map.
#[derive(Debug, Deserialize)]
struct RawDiscordConfig {
    bot_token: Option<String>,
    application_id: Option<u64>,
    #[serde(default)]
    guild_ids: Vec<u64>,
    prefix: Option<String>,
}

impl DiscordConfig {
    /// Build a `DiscordConfig` from the generic settings map in `ChannelConfig`.
    pub fn from_settings(settings: &HashMap<String, serde_json::Value>) -> Result<Self> {
        let value = serde_json::Value::Object(
            settings
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        );

        let raw: RawDiscordConfig = serde_json::from_value(value)
            .map_err(|e| Error::Config(format!("invalid discord config: {e}")))?;

        let bot_token = raw
            .bot_token
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| Error::Config("discord bot_token is required".into()))?;

        let application_id = raw
            .application_id
            .ok_or_else(|| Error::Config("discord application_id is required".into()))?;

        // Standard intents for a messaging bot:
        // - GUILDS: access to guild info
        // - GUILD_MESSAGES + MESSAGE_CONTENT: receive messages in server channels
        // - DIRECT_MESSAGES: receive DMs
        // - GUILD_MESSAGE_REACTIONS: reaction handling
        let intents = GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT
            | GatewayIntents::GUILD_MESSAGE_REACTIONS;

        Ok(Self {
            bot_token,
            application_id,
            guild_ids: raw.guild_ids,
            intents,
            prefix: raw.prefix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_settings(pairs: Vec<(&str, serde_json::Value)>) -> HashMap<String, serde_json::Value> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn valid_config_parses_successfully() {
        let settings = make_settings(vec![
            ("bot_token", serde_json::json!("my-secret-token")),
            ("application_id", serde_json::json!(123456789012345678_u64)),
            (
                "guild_ids",
                serde_json::json!([111111111111111111_u64, 222222222222222222_u64]),
            ),
            ("prefix", serde_json::json!("!")),
        ]);

        let config = DiscordConfig::from_settings(&settings).expect("should parse valid config");
        assert_eq!(config.bot_token, "my-secret-token");
        assert_eq!(config.application_id, 123456789012345678);
        assert_eq!(config.guild_ids.len(), 2);
        assert_eq!(config.prefix.as_deref(), Some("!"));
    }

    #[test]
    fn missing_bot_token_fails() {
        let settings = make_settings(vec![(
            "application_id",
            serde_json::json!(123456789012345678_u64),
        )]);

        let err = DiscordConfig::from_settings(&settings).expect_err("should fail without token");
        assert!(err.to_string().contains("bot_token"));
    }

    #[test]
    fn empty_bot_token_fails() {
        let settings = make_settings(vec![
            ("bot_token", serde_json::json!("   ")),
            ("application_id", serde_json::json!(123456789012345678_u64)),
        ]);

        let err =
            DiscordConfig::from_settings(&settings).expect_err("should fail with empty token");
        assert!(err.to_string().contains("bot_token"));
    }

    #[test]
    fn missing_application_id_fails() {
        let settings = make_settings(vec![("bot_token", serde_json::json!("my-secret-token"))]);

        let err = DiscordConfig::from_settings(&settings).expect_err("should fail without app id");
        assert!(err.to_string().contains("application_id"));
    }

    #[test]
    fn defaults_for_optional_fields() {
        let settings = make_settings(vec![
            ("bot_token", serde_json::json!("my-secret-token")),
            ("application_id", serde_json::json!(123456789012345678_u64)),
        ]);

        let config = DiscordConfig::from_settings(&settings).expect("should parse");
        assert!(config.guild_ids.is_empty());
        assert!(config.prefix.is_none());
    }
}
