use async_trait::async_trait;
use opencrust_common::{Error, Result};
use opencrust_db::SessionStore;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::tools::{Tool, ToolContext, ToolOutput};

/// Maximum delay: 30 days in seconds.
const MAX_DELAY_SECONDS: i64 = 30 * 24 * 60 * 60;

/// Maximum pending heartbeats per session.
const MAX_PENDING_PER_SESSION: i64 = 5;

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
                    "description": "Number of seconds to wait before waking up (min 1, max 2592000 = 30 days)"
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
        // Prevent recursive scheduling from within heartbeat execution
        if context.is_heartbeat {
            return Err(Error::Agent(
                "cannot schedule a heartbeat from within a heartbeat execution".to_string(),
            ));
        }

        let delay = args["delay_seconds"].as_i64().ok_or_else(|| {
            Error::Agent("missing or invalid 'delay_seconds' argument".to_string())
        })?;

        let reason = args["reason"]
            .as_str()
            .ok_or_else(|| Error::Agent("missing or invalid 'reason' argument".to_string()))?;

        if delay <= 0 {
            return Err(Error::Agent("delay_seconds must be positive".to_string()));
        }

        if delay > MAX_DELAY_SECONDS {
            return Err(Error::Agent(format!(
                "delay_seconds cannot exceed {} (30 days)",
                MAX_DELAY_SECONDS
            )));
        }

        let user_id = context
            .user_id
            .clone()
            .unwrap_or_else(|| "unknown".to_string());

        let store = self.store.lock().await;

        // Enforce per-session pending task limit
        let pending = store.count_pending_tasks_for_session(&context.session_id)?;
        if pending >= MAX_PENDING_PER_SESSION {
            return Err(Error::Agent(format!(
                "session already has {} pending heartbeats (max {})",
                pending, MAX_PENDING_PER_SESSION
            )));
        }

        let execute_at = chrono::Utc::now() + chrono::Duration::seconds(delay);
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

    fn test_context(session_id: &str) -> ToolContext {
        ToolContext {
            session_id: session_id.to_string(),
            user_id: Some("u-1".to_string()),
            is_heartbeat: false,
        }
    }

    async fn setup_store(session_id: &str) -> Arc<Mutex<SessionStore>> {
        let store = SessionStore::in_memory().expect("in-memory store should open");
        let store = Arc::new(Mutex::new(store));
        {
            let guard = store.lock().await;
            guard
                .upsert_session(session_id, "web", "u-1", &serde_json::json!({}))
                .expect("session upsert should succeed");
        }
        store
    }

    #[tokio::test]
    async fn schedules_task_in_store() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(Arc::clone(&store));

        let out = tool
            .execute(
                &test_context("sess-1"),
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

    #[tokio::test]
    async fn rejects_negative_delay() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let err = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({ "delay_seconds": -5, "reason": "bad" }),
            )
            .await;

        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("must be positive"));
    }

    #[tokio::test]
    async fn rejects_excessive_delay() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let err = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({ "delay_seconds": MAX_DELAY_SECONDS + 1, "reason": "way too long" }),
            )
            .await;

        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("30 days"));
    }

    #[tokio::test]
    async fn rejects_scheduling_from_heartbeat_context() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let context = ToolContext {
            session_id: "sess-1".to_string(),
            user_id: Some("u-1".to_string()),
            is_heartbeat: true,
        };

        let err = tool
            .execute(
                &context,
                serde_json::json!({ "delay_seconds": 60, "reason": "recursive" }),
            )
            .await;

        assert!(err.is_err());
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("cannot schedule a heartbeat from within")
        );
    }

    #[tokio::test]
    async fn rejects_when_too_many_pending() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(Arc::clone(&store));

        // Fill up to the limit
        for i in 0..MAX_PENDING_PER_SESSION {
            tool.execute(
                &test_context("sess-1"),
                serde_json::json!({ "delay_seconds": 3600, "reason": format!("task {}", i) }),
            )
            .await
            .expect("should succeed under limit");
        }

        // One more should fail
        let err = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({ "delay_seconds": 3600, "reason": "one too many" }),
            )
            .await;

        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("pending heartbeats"));
    }

    #[tokio::test]
    async fn pending_limit_is_per_session() {
        let store = SessionStore::in_memory().expect("in-memory store should open");
        let store = Arc::new(Mutex::new(store));
        {
            let guard = store.lock().await;
            guard
                .upsert_session("s1", "web", "u1", &serde_json::json!({}))
                .unwrap();
            guard
                .upsert_session("s2", "web", "u2", &serde_json::json!({}))
                .unwrap();
        }

        let tool = ScheduleHeartbeat::new(Arc::clone(&store));

        // Fill s1 to the limit
        for i in 0..MAX_PENDING_PER_SESSION {
            tool.execute(
                &test_context("s1"),
                serde_json::json!({ "delay_seconds": 3600, "reason": format!("s1-{}", i) }),
            )
            .await
            .unwrap();
        }

        // s2 should still work
        let out = tool
            .execute(
                &ToolContext {
                    session_id: "s2".to_string(),
                    user_id: Some("u2".to_string()),
                    is_heartbeat: false,
                },
                serde_json::json!({ "delay_seconds": 60, "reason": "s2 ok" }),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
    }
}
