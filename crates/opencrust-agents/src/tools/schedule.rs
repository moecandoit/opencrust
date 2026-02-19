use async_trait::async_trait;
use opencrust_common::{Error, Result};
use opencrust_db::SessionStore;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::tools::{Tool, ToolContext, ToolOutput};

/// Tool for scheduling a future "heartbeat" wake-up call for the agent.
pub struct ScheduleHeartbeat {
    store: Arc<Mutex<SessionStore>>,
}

impl ScheduleHeartbeat {
    pub fn new(store: Arc<Mutex<SessionStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ScheduleHeartbeat {
    fn name(&self) -> &'static str {
        "schedule_heartbeat"
    }

    fn description(&self) -> &'static str {
        "Schedule a wake-up call for yourself in the future. Use this to set reminders or check back on tasks."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "delay_seconds": {
                    "type": "integer",
                    "description": "Number of seconds to wait before waking up (e.g. 60 for 1 minute, 3600 for 1 hour)"
                },
                "reason": {
                    "type": "string",
                    "description": "Context/reason for the wake-up call (e.g. 'Check if deployment finished')"
                }
            },
            "required": ["delay_seconds", "reason"]
        })
    }

    async fn execute(&self, context: &ToolContext, args: serde_json::Value) -> Result<ToolOutput> {
        let delay = args["delay_seconds"].as_i64().ok_or_else(|| {
            Error::Agent("missing or invalid 'delay_seconds' argument".to_string())
        })?;

        let reason = args["reason"]
            .as_str()
            .ok_or_else(|| Error::Agent("missing or invalid 'reason' argument".to_string()))?;

        if delay <= 0 {
            return Err(Error::Agent("delay_seconds must be positive".to_string()));
        }

        let user_id = context
            .user_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let execute_at = chrono::Utc::now() + chrono::Duration::seconds(delay);

        let store = self.store.lock().await;
        let task_id = store.schedule_task(&context.session_id, &user_id, execute_at, reason)?;

        Ok(ToolOutput::success(format!(
            "Heartbeat scheduled for {} (in {} seconds). Task ID: {}",
            execute_at.to_rfc3339(),
            delay,
            task_id
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn schedules_task_in_store() {
        let store = SessionStore::in_memory().expect("in-memory store should open");
        let store = Arc::new(Mutex::new(store));

        {
            let guard = store.lock().await;
            guard
                .upsert_session("sess-1", "web", "u-1", &serde_json::json!({}))
                .expect("session upsert should succeed");
        }

        let tool = ScheduleHeartbeat::new(Arc::clone(&store));
        let context = ToolContext {
            session_id: "sess-1".to_string(),
            user_id: Some("u-1".to_string()),
        };

        let out = tool
            .execute(
                &context,
                serde_json::json!({
                    "delay_seconds": 1,
                    "reason": "ping me later"
                }),
            )
            .await
            .expect("tool execution should succeed");

        assert!(!out.is_error);
        assert!(out.content.contains("Task ID:"));
    }
}
