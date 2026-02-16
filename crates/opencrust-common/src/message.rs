use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{ChannelId, SessionId, UserId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: SessionId,
    pub channel_id: ChannelId,
    pub user_id: UserId,
    pub direction: MessageDirection,
    pub content: MessageContent,
    pub timestamp: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageDirection {
    Incoming,
    Outgoing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageContent {
    Text(String),
    Image {
        url: String,
        caption: Option<String>,
    },
    Audio {
        url: String,
        duration_secs: Option<f64>,
    },
    Video {
        url: String,
        caption: Option<String>,
    },
    File {
        url: String,
        filename: String,
    },
    Location {
        latitude: f64,
        longitude: f64,
    },
    Reaction {
        emoji: String,
        target_message_id: String,
    },
    System(String),
}

impl Message {
    pub fn text(
        session_id: SessionId,
        channel_id: ChannelId,
        user_id: UserId,
        direction: MessageDirection,
        text: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id,
            channel_id,
            user_id,
            direction,
            content: MessageContent::Text(text.into()),
            timestamp: Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ChannelId, SessionId, UserId};

    #[test]
    fn test_message_text_factory() {
        let session_id = SessionId::new();
        let channel_id = ChannelId::new();
        let user_id = UserId::new();
        let direction = MessageDirection::Incoming;
        let text = "Hello, world!";

        let start_time = Utc::now();
        let message = Message::text(
            session_id.clone(),
            channel_id.clone(),
            user_id.clone(),
            direction.clone(),
            text,
        );
        let end_time = Utc::now();

        assert!(!message.id.is_empty());
        assert_eq!(message.session_id, session_id);
        assert_eq!(message.channel_id, channel_id);
        assert_eq!(message.user_id, user_id);
        assert!(matches!(message.direction, MessageDirection::Incoming));

        if let MessageContent::Text(content_text) = message.content {
            assert_eq!(content_text, text);
        } else {
            panic!("Expected MessageContent::Text");
        }

        assert!(message.timestamp >= start_time);
        assert!(message.timestamp <= end_time);
        assert!(message.metadata.is_null());
    }
}
