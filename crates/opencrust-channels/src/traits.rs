use async_trait::async_trait;
use opencrust_common::{Message, Result};
use serde::{Deserialize, Serialize};

/// Lifecycle management for a messaging channel (connect, disconnect, status).
#[async_trait]
pub trait ChannelLifecycle: Send {
    /// Human-readable display name.
    fn display_name(&self) -> &str;

    /// Start the channel, connecting to the external service.
    async fn connect(&mut self) -> Result<()>;

    /// Gracefully disconnect from the external service.
    async fn disconnect(&mut self) -> Result<()>;

    /// Current connection status.
    fn status(&self) -> ChannelStatus;

    /// Create a lightweight send-only handle for this channel.
    ///
    /// The returned sender is independent of the lifecycle and can be shared
    /// via `Arc` for scheduled message delivery while the channel runs its
    /// polling loop in a separate task.
    fn create_sender(&self) -> Box<dyn ChannelSender>;
}

/// Send-only interface for delivering outbound messages through a channel.
///
/// Designed to be wrapped in `Arc` and shared across tasks (e.g. the scheduler).
#[async_trait]
pub trait ChannelSender: Send + Sync {
    /// Unique identifier for this channel type.
    fn channel_type(&self) -> &str;

    /// Send a message through this channel.
    async fn send_message(&self, message: &Message) -> Result<()>;
}

/// Convenience trait combining lifecycle and send capabilities.
///
/// Kept for backward compatibility with `ChannelRegistry`.
pub trait Channel: ChannelLifecycle + ChannelSender {}
impl<T: ChannelLifecycle + ChannelSender> Channel for T {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChannelStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum ChannelEvent {
    MessageReceived(Message),
    StatusChanged(ChannelStatus),
    Error(String),
}
