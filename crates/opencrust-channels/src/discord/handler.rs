use serenity::all::{
    self as serenity_model, Context, EventHandler, Message as SerenityMessage, Ready,
};
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::traits::{ChannelEvent, ChannelStatus};

use super::{commands, convert};

/// Serenity event handler that bridges Discord events into OpenCrust `ChannelEvent`s.
pub struct DiscordHandler {
    /// Broadcast sender for emitting channel events to subscribers.
    event_tx: broadcast::Sender<ChannelEvent>,

    /// The channel identifier string used in OpenCrust messages.
    channel_id: String,

    /// Guild IDs for slash command registration. Empty means global commands.
    guild_ids: Vec<u64>,
}

impl DiscordHandler {
    pub fn new(
        event_tx: broadcast::Sender<ChannelEvent>,
        channel_id: String,
        guild_ids: Vec<u64>,
    ) -> Self {
        Self {
            event_tx,
            channel_id,
            guild_ids,
        }
    }

    fn emit(&self, event: ChannelEvent) {
        if let Err(e) = self.event_tx.send(event) {
            warn!("no subscribers for channel event: {e}");
        }
    }
}

#[serenity::async_trait]
impl EventHandler for DiscordHandler {
    /// Fired when the bot successfully connects and is ready.
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!(
            "Discord bot connected as {}#{} (guilds: {})",
            ready.user.name,
            ready
                .user
                .discriminator
                .map(|d| d.to_string())
                .unwrap_or_default(),
            ready.guilds.len()
        );

        let command_defs = commands::all_commands();
        if let Err(e) = commands::register_commands(&_ctx, &self.guild_ids, &command_defs).await {
            warn!("failed to register discord slash commands: {e}");
        } else {
            info!("registered {} discord slash command(s)", command_defs.len());
        }

        self.emit(ChannelEvent::StatusChanged(ChannelStatus::Connected));
    }

    /// Fired when the bot resumes a previously interrupted gateway connection.
    async fn resume(&self, _ctx: Context, _: serenity_model::ResumedEvent) {
        info!("Discord gateway connection resumed");
        self.emit(ChannelEvent::StatusChanged(ChannelStatus::Connected));
    }

    /// Fired when a message is received in any channel the bot can see.
    async fn message(&self, _ctx: Context, msg: SerenityMessage) {
        // Ignore messages from bots (including ourselves) to prevent loops
        if msg.author.bot {
            return;
        }

        let opencrust_msg = convert::discord_message_to_opencrust(&msg, &self.channel_id);

        tracing::debug!(
            message_id = %msg.id,
            author = %msg.author.name,
            channel = %msg.channel_id,
            "received discord message"
        );

        self.emit(ChannelEvent::MessageReceived(opencrust_msg));
    }

    /// Fired when a reaction is added to a message.
    async fn reaction_add(&self, _ctx: Context, reaction: serenity_model::Reaction) {
        let opencrust_msg = convert::reaction_to_opencrust(&reaction, &self.channel_id);

        tracing::debug!(
            emoji = ?reaction.emoji,
            message_id = %reaction.message_id,
            "received discord reaction"
        );

        self.emit(ChannelEvent::MessageReceived(opencrust_msg));
    }

    /// Fired when a thread is created.
    async fn thread_create(&self, _ctx: Context, thread: serenity_model::GuildChannel) {
        info!(
            thread_id = %thread.id,
            thread_name = %thread.name,
            "new discord thread created"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::broadcast;

    #[test]
    fn handler_construction() {
        let (tx, _rx) = broadcast::channel::<ChannelEvent>(16);
        let handler = DiscordHandler::new(tx, "discord".to_string(), vec![]);
        assert_eq!(handler.channel_id, "discord");
        assert!(handler.guild_ids.is_empty());
    }

    #[test]
    fn emit_with_no_subscribers_does_not_panic() {
        let (tx, _) = broadcast::channel::<ChannelEvent>(16);
        let handler = DiscordHandler::new(tx, "discord".to_string(), vec![]);
        // Drop the only receiver — emit should not panic
        handler.emit(ChannelEvent::StatusChanged(ChannelStatus::Connected));
    }
}
