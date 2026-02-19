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
    fn text_builder_creates_correct_message() {
        let msg = Message::text(
            SessionId::from_string("sess-1"),
            ChannelId::from_string("ch-1"),
            UserId::from_string("user-1"),
            MessageDirection::Incoming,
            "hello world",
        );

        assert_eq!(msg.session_id.as_str(), "sess-1");
        assert_eq!(msg.channel_id.as_str(), "ch-1");
        assert_eq!(msg.user_id.as_str(), "user-1");
        assert!(!msg.id.is_empty());
        match &msg.content {
            MessageContent::Text(t) => assert_eq!(t, "hello world"),
            _ => panic!("expected Text content"),
        }
    }

    #[test]
    fn text_builder_accepts_string() {
        let msg = Message::text(
            SessionId::new(),
            ChannelId::new(),
            UserId::new(),
            MessageDirection::Outgoing,
            String::from("owned string"),
        );
        match &msg.content {
            MessageContent::Text(t) => assert_eq!(t, "owned string"),
            _ => panic!("expected Text content"),
        }
    }

    #[test]
    fn message_direction_serializes() {
        let json = serde_json::to_string(&MessageDirection::Incoming).unwrap();
        assert_eq!(json, r#""Incoming""#);
        let json = serde_json::to_string(&MessageDirection::Outgoing).unwrap();
        assert_eq!(json, r#""Outgoing""#);
    }
}
