pub mod chatdb;
pub mod sender;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

use crate::traits::{Channel, ChannelStatus};
use opencrust_common::{Message, MessageContent, Result};

/// Callback invoked when the bot receives a text message from iMessage.
///
/// Arguments: `(sender_id, sender_id_as_name, text, delta_tx)`.
/// `delta_tx` is always `None` for iMessage (no streaming support).
/// Return `Err("__blocked__")` to silently drop the message (unauthorized user).
pub type IMessageOnMessageFn = Arc<
    dyn Fn(
            String,
            String,
            String,
            Option<mpsc::Sender<String>>,
        ) -> Pin<Box<dyn Future<Output = std::result::Result<String, String>> + Send>>
        + Send
        + Sync,
>;

pub struct IMessageChannel {
    poll_interval: Duration,
    status: ChannelStatus,
    on_message: IMessageOnMessageFn,
    shutdown_tx: Option<watch::Sender<bool>>,
}

impl IMessageChannel {
    pub fn new(poll_interval_secs: u64, on_message: IMessageOnMessageFn) -> Self {
        Self {
            poll_interval: Duration::from_secs(poll_interval_secs),
            status: ChannelStatus::Disconnected,
            on_message,
            shutdown_tx: None,
        }
    }
}

#[async_trait]
impl Channel for IMessageChannel {
    fn channel_type(&self) -> &str {
        "imessage"
    }

    fn display_name(&self) -> &str {
        "iMessage"
    }

    async fn connect(&mut self) -> Result<()> {
        let db_path = chatdb::default_chat_db_path();
        let mut db = chatdb::ChatDb::open(&db_path).map_err(|e| {
            opencrust_common::Error::Channel(format!("imessage connect failed: {e}"))
        })?;

        let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let on_message = Arc::clone(&self.on_message);
        let poll_interval = self.poll_interval;

        tokio::spawn(async move {
            info!(
                "imessage poll loop started (interval = {}s)",
                poll_interval.as_secs()
            );

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(poll_interval) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            info!("imessage poll loop shutting down");
                            break;
                        }
                    }
                }

                let messages = db.poll();
                for msg in messages {
                    info!(
                        "imessage from {} ({} chars, rowid={})",
                        msg.sender,
                        msg.text.len(),
                        msg.rowid
                    );

                    let on_message = Arc::clone(&on_message);
                    let sender = msg.sender.clone();
                    let text = msg.text;

                    tokio::spawn(async move {
                        // sender_id and sender_name are both the handle (phone/email)
                        let result = on_message(sender.clone(), sender.clone(), text, None).await;

                        match result {
                            Ok(response) => {
                                if let Err(e) = sender::send_imessage(&sender, &response).await {
                                    error!("imessage: failed to send reply to {sender}: {e}");
                                }
                            }
                            Err(e) if e == "__blocked__" => {
                                // Silently drop — unauthorized user
                            }
                            Err(e) => {
                                warn!("imessage: agent error for {sender}: {e}");
                                let _ = sender::send_imessage(
                                    &sender,
                                    &format!("Sorry, an error occurred: {e}"),
                                )
                                .await;
                            }
                        }
                    });
                }
            }

            info!("imessage poll loop stopped");
        });

        self.status = ChannelStatus::Connected;
        info!("imessage channel connected");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<()> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(true);
        }
        self.status = ChannelStatus::Disconnected;
        info!("imessage channel disconnected");
        Ok(())
    }

    async fn send_message(&self, message: &Message) -> Result<()> {
        let to = message
            .metadata
            .get("imessage_sender")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                opencrust_common::Error::Channel("missing imessage_sender in metadata".into())
            })?;

        let text = match &message.content {
            MessageContent::Text(t) => t.clone(),
            _ => {
                return Err(opencrust_common::Error::Channel(
                    "only text messages are supported for imessage send".into(),
                ));
            }
        };

        sender::send_imessage(to, &text)
            .await
            .map_err(|e| opencrust_common::Error::Channel(format!("imessage send failed: {e}")))?;

        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_type_is_imessage() {
        let on_msg: IMessageOnMessageFn =
            Arc::new(|_from, _user, _text, _delta_tx| Box::pin(async { Ok("test".to_string()) }));
        let channel = IMessageChannel::new(2, on_msg);
        assert_eq!(channel.channel_type(), "imessage");
        assert_eq!(channel.display_name(), "iMessage");
        assert_eq!(channel.status(), ChannelStatus::Disconnected);
    }
}
