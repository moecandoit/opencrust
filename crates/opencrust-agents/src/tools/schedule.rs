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
const MAX_PENDING_PER_SESSION: i64 = 20;

/// Maximum heartbeat chaining depth (0 = user request, 1-3 = heartbeat chains).
const MAX_HEARTBEAT_DEPTH: u8 = 3;

// ---------------------------------------------------------------------------
// ScheduleHeartbeat
// ---------------------------------------------------------------------------

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
        "Schedule a wake-up call for yourself in the future. Use this to set reminders, \
         check back on tasks, or create recurring schedules. You can specify a delay in \
         seconds, or an exact datetime with timezone. For recurring tasks, provide a \
         recurrence type ('interval' or 'cron') with the appropriate value. You can also \
         target a specific channel for delivery."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "delay_seconds": {
                    "type": "integer",
                    "description": "Number of seconds to wait before waking up (min 1, max 2592000 = 30 days). Ignored if execute_at_iso is provided."
                },
                "execute_at_iso": {
                    "type": "string",
                    "description": "ISO 8601 datetime for when to wake up (e.g. '2026-02-25T09:00:00'). Takes precedence over delay_seconds. Must be in the future."
                },
                "timezone": {
                    "type": "string",
                    "description": "IANA timezone name (e.g. 'America/New_York', 'Europe/London', 'Asia/Tokyo'). Used to interpret execute_at_iso and cron expressions. Always provide this when the user mentions a timezone. Defaults to 'UTC'."
                },
                "reason": {
                    "type": "string",
                    "description": "Context/reason for the wake-up call (e.g. 'Check if deployment finished')"
                },
                "recurrence": {
                    "type": "string",
                    "enum": ["interval", "cron"],
                    "description": "Type of recurrence. 'interval' repeats every N seconds, 'cron' uses a cron expression."
                },
                "interval_seconds": {
                    "type": "integer",
                    "description": "For recurrence='interval': repeat every this many seconds."
                },
                "cron_expression": {
                    "type": "string",
                    "description": "For recurrence='cron': a standard cron expression (e.g. '0 30 9 * * Mon-Fri *' for weekdays at 9:30 AM UTC). Uses 7-field format: sec min hour day month weekday year."
                },
                "recurrence_end_after_seconds": {
                    "type": "integer",
                    "description": "Stop recurring after this many seconds from the first execution time."
                },
                "deliver_to_channel": {
                    "type": "string",
                    "description": "Channel to deliver the heartbeat response to (e.g. 'telegram', 'discord', 'slack'). Defaults to the current session's channel."
                }
            },
            "required": ["reason"]
        })
    }

    async fn execute(&self, context: &ToolContext, args: serde_json::Value) -> Result<ToolOutput> {
        // Depth-limited chaining instead of blanket block
        if context.heartbeat_depth >= MAX_HEARTBEAT_DEPTH {
            return Err(Error::Agent(format!(
                "heartbeat chain depth limit reached (max {}). Cannot schedule further.",
                MAX_HEARTBEAT_DEPTH
            )));
        }

        let reason = args["reason"]
            .as_str()
            .ok_or_else(|| Error::Agent("missing or invalid 'reason' argument".to_string()))?;

        // Resolve execution time: execute_at_iso + timezone takes precedence over delay_seconds
        let execute_at = if let Some(iso_str) = args["execute_at_iso"].as_str() {
            let tz_name = args["timezone"].as_str().unwrap_or("UTC");
            let tz: chrono_tz::Tz = tz_name
                .parse()
                .map_err(|_| Error::Agent(format!("unknown timezone: '{tz_name}'")))?;

            let naive = chrono::NaiveDateTime::parse_from_str(iso_str, "%Y-%m-%dT%H:%M:%S")
                .or_else(|_| {
                    chrono::NaiveDateTime::parse_from_str(iso_str, "%Y-%m-%dT%H:%M:%S%.f")
                })
                .map_err(|e| {
                    Error::Agent(format!(
                        "invalid datetime format '{iso_str}'. Use ISO 8601 like '2026-02-25T09:00:00': {e}"
                    ))
                })?;

            let local_dt = naive.and_local_timezone(tz).single().ok_or_else(|| {
                Error::Agent(format!(
                    "ambiguous or invalid datetime '{iso_str}' in timezone '{tz_name}'"
                ))
            })?;

            let utc_dt = local_dt.with_timezone(&chrono::Utc);
            if utc_dt <= chrono::Utc::now() {
                return Err(Error::Agent(
                    "execute_at_iso must be in the future".to_string(),
                ));
            }
            utc_dt
        } else {
            let delay = args["delay_seconds"].as_i64().ok_or_else(|| {
                Error::Agent("must provide either 'delay_seconds' or 'execute_at_iso'".to_string())
            })?;

            if delay <= 0 {
                return Err(Error::Agent("delay_seconds must be positive".to_string()));
            }
            if delay > MAX_DELAY_SECONDS {
                return Err(Error::Agent(format!(
                    "delay_seconds cannot exceed {} (30 days)",
                    MAX_DELAY_SECONDS
                )));
            }
            chrono::Utc::now() + chrono::Duration::seconds(delay)
        };

        // Parse recurrence
        let recurrence_type = args["recurrence"].as_str();
        let (rec_type, rec_value) = match recurrence_type {
            Some("interval") => {
                let secs = args["interval_seconds"].as_i64().ok_or_else(|| {
                    Error::Agent("recurrence='interval' requires 'interval_seconds'".to_string())
                })?;
                if secs <= 0 || secs > MAX_DELAY_SECONDS {
                    return Err(Error::Agent(
                        "interval_seconds must be between 1 and 2592000".to_string(),
                    ));
                }
                (Some("interval"), Some(secs.to_string()))
            }
            Some("cron") => {
                let expr = args["cron_expression"].as_str().ok_or_else(|| {
                    Error::Agent("recurrence='cron' requires 'cron_expression'".to_string())
                })?;
                // Validate cron expression
                use std::str::FromStr;
                cron::Schedule::from_str(expr)
                    .map_err(|e| Error::Agent(format!("invalid cron expression '{expr}': {e}")))?;
                (Some("cron"), Some(expr.to_string()))
            }
            Some(other) => {
                return Err(Error::Agent(format!(
                    "unknown recurrence type: '{other}'. Use 'interval' or 'cron'."
                )));
            }
            None => (None, None),
        };

        let recurrence_end_at = args["recurrence_end_after_seconds"]
            .as_i64()
            .map(|secs| execute_at + chrono::Duration::seconds(secs));

        let deliver_to_channel = args["deliver_to_channel"].as_str();
        let timezone = args["timezone"].as_str();

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

        let next_depth = context.heartbeat_depth.saturating_add(1);

        let task_id = store.schedule_task_full(
            &context.session_id,
            &user_id,
            execute_at,
            reason,
            next_depth,
            rec_type,
            rec_value.as_deref(),
            recurrence_end_at,
            deliver_to_channel,
            timezone,
        )?;

        let mut msg = format!(
            "Heartbeat scheduled for {} (task ID: {})",
            execute_at.to_rfc3339(),
            task_id
        );
        if let Some(rt) = rec_type {
            msg.push_str(&format!(
                ". Recurring: {} = {}",
                rt,
                rec_value.as_deref().unwrap_or("?")
            ));
        }
        if let Some(ch) = deliver_to_channel {
            msg.push_str(&format!(". Delivery channel: {}", ch));
        }

        Ok(ToolOutput::success(msg))
    }
}

// ---------------------------------------------------------------------------
// CancelHeartbeat
// ---------------------------------------------------------------------------

/// Tool for cancelling a pending scheduled heartbeat.
pub struct CancelHeartbeat {
    store: Arc<Mutex<SessionStore>>,
}

impl CancelHeartbeat {
    pub fn new(store: Arc<Mutex<SessionStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for CancelHeartbeat {
    fn name(&self) -> &'static str {
        "cancel_heartbeat"
    }

    fn description(&self) -> &'static str {
        "Cancel a pending scheduled heartbeat by its task ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "The task ID returned when the heartbeat was scheduled"
                }
            },
            "required": ["task_id"]
        })
    }

    async fn execute(&self, context: &ToolContext, args: serde_json::Value) -> Result<ToolOutput> {
        let task_id = args["task_id"]
            .as_str()
            .ok_or_else(|| Error::Agent("missing or invalid 'task_id' argument".to_string()))?;

        let store = self.store.lock().await;
        let cancelled = store.cancel_task(task_id, &context.session_id)?;

        if cancelled {
            Ok(ToolOutput::success(format!(
                "Heartbeat {} cancelled.",
                task_id
            )))
        } else {
            Ok(ToolOutput::error(format!(
                "No pending heartbeat found with ID {} in this session.",
                task_id
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// ListHeartbeats
// ---------------------------------------------------------------------------

/// Tool for listing pending scheduled heartbeats for the current session.
pub struct ListHeartbeats {
    store: Arc<Mutex<SessionStore>>,
}

impl ListHeartbeats {
    pub fn new(store: Arc<Mutex<SessionStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ListHeartbeats {
    fn name(&self) -> &'static str {
        "list_heartbeats"
    }

    fn description(&self) -> &'static str {
        "List all pending scheduled heartbeats for the current session."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, context: &ToolContext, _args: serde_json::Value) -> Result<ToolOutput> {
        let store = self.store.lock().await;
        let tasks = store.list_pending_tasks(&context.session_id)?;

        if tasks.is_empty() {
            return Ok(ToolOutput::success("No pending heartbeats."));
        }

        let mut lines = vec![format!("{} pending heartbeat(s):", tasks.len())];
        for task in &tasks {
            let mut line = format!(
                "- [{}] at {} - \"{}\"",
                task.id,
                task.execute_at.to_rfc3339(),
                task.payload
            );
            if let Some(ref rec_type) = task.recurrence_type {
                let tz_suffix = if rec_type == "cron" {
                    task.timezone
                        .as_deref()
                        .filter(|tz| *tz != "UTC")
                        .map(|tz| format!(" [{tz}]"))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                line.push_str(&format!(
                    " (recurring: {} = {}{})",
                    rec_type,
                    task.recurrence_value.as_deref().unwrap_or("?"),
                    tz_suffix,
                ));
            }
            if let Some(ref channel) = task.deliver_to_channel {
                line.push_str(&format!(" -> {}", channel));
            }
            if task.retry_count > 0 {
                line.push_str(&format!(
                    " [retry {}/{}]",
                    task.retry_count, task.max_retries
                ));
            }
            lines.push(line);
        }

        Ok(ToolOutput::success(lines.join("\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context(session_id: &str) -> ToolContext {
        ToolContext {
            session_id: session_id.to_string(),
            user_id: Some("u-1".to_string()),
            heartbeat_depth: 0,
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
        assert!(out.content.contains("task ID:"));
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
    async fn rejects_scheduling_at_max_depth() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let context = ToolContext {
            session_id: "sess-1".to_string(),
            user_id: Some("u-1".to_string()),
            heartbeat_depth: MAX_HEARTBEAT_DEPTH,
        };

        let err = tool
            .execute(
                &context,
                serde_json::json!({ "delay_seconds": 60, "reason": "too deep" }),
            )
            .await;

        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("depth limit"));
    }

    #[tokio::test]
    async fn allows_scheduling_below_max_depth() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let context = ToolContext {
            session_id: "sess-1".to_string(),
            user_id: Some("u-1".to_string()),
            heartbeat_depth: MAX_HEARTBEAT_DEPTH - 1,
        };

        let out = tool
            .execute(
                &context,
                serde_json::json!({ "delay_seconds": 60, "reason": "chain ok" }),
            )
            .await
            .expect("should succeed below max depth");

        assert!(!out.is_error);
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
                    heartbeat_depth: 0,
                },
                serde_json::json!({ "delay_seconds": 60, "reason": "s2 ok" }),
            )
            .await
            .unwrap();

        assert!(!out.is_error);
    }

    #[tokio::test]
    async fn cancel_heartbeat_works() {
        let store = setup_store("sess-1").await;
        let schedule_tool = ScheduleHeartbeat::new(Arc::clone(&store));
        let cancel_tool = CancelHeartbeat::new(Arc::clone(&store));

        let out = schedule_tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({ "delay_seconds": 3600, "reason": "will cancel" }),
            )
            .await
            .unwrap();

        // Extract task ID from output
        let task_id = out
            .content
            .split("task ID: ")
            .nth(1)
            .unwrap()
            .trim_end_matches(')')
            .to_string();

        let cancel_out = cancel_tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({ "task_id": task_id }),
            )
            .await
            .unwrap();

        assert!(!cancel_out.is_error);
        assert!(cancel_out.content.contains("cancelled"));

        // Verify it's gone from pending
        let guard = store.lock().await;
        let pending = guard.count_pending_tasks_for_session("sess-1").unwrap();
        assert_eq!(pending, 0);
    }

    #[tokio::test]
    async fn cancel_wrong_session_fails() {
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

        let schedule_tool = ScheduleHeartbeat::new(Arc::clone(&store));
        let cancel_tool = CancelHeartbeat::new(Arc::clone(&store));

        let out = schedule_tool
            .execute(
                &test_context("s1"),
                serde_json::json!({ "delay_seconds": 3600, "reason": "s1 task" }),
            )
            .await
            .unwrap();

        let task_id = out
            .content
            .split("task ID: ")
            .nth(1)
            .unwrap()
            .trim_end_matches(')')
            .to_string();

        // Try to cancel from s2 - should fail
        let cancel_out = cancel_tool
            .execute(
                &ToolContext {
                    session_id: "s2".to_string(),
                    user_id: Some("u2".to_string()),
                    heartbeat_depth: 0,
                },
                serde_json::json!({ "task_id": task_id }),
            )
            .await
            .unwrap();

        assert!(cancel_out.is_error);
        assert!(cancel_out.content.contains("No pending heartbeat"));
    }

    #[tokio::test]
    async fn list_heartbeats_shows_pending() {
        let store = setup_store("sess-1").await;
        let schedule_tool = ScheduleHeartbeat::new(Arc::clone(&store));
        let list_tool = ListHeartbeats::new(Arc::clone(&store));

        // Empty list
        let out = list_tool
            .execute(&test_context("sess-1"), serde_json::json!({}))
            .await
            .unwrap();
        assert!(out.content.contains("No pending"));

        // Add two tasks
        schedule_tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({ "delay_seconds": 60, "reason": "first" }),
            )
            .await
            .unwrap();
        schedule_tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({ "delay_seconds": 120, "reason": "second" }),
            )
            .await
            .unwrap();

        let out = list_tool
            .execute(&test_context("sess-1"), serde_json::json!({}))
            .await
            .unwrap();

        assert!(out.content.contains("2 pending"));
        assert!(out.content.contains("first"));
        assert!(out.content.contains("second"));
    }

    #[tokio::test]
    async fn schedule_with_timezone() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        // Schedule 1 hour from now in UTC
        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        let iso = future.format("%Y-%m-%dT%H:%M:%S").to_string();

        let out = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({
                    "execute_at_iso": iso,
                    "timezone": "UTC",
                    "reason": "timezone test"
                }),
            )
            .await
            .expect("should succeed");

        assert!(!out.is_error);
    }

    #[tokio::test]
    async fn rejects_past_execute_at() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let err = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({
                    "execute_at_iso": "2020-01-01T00:00:00",
                    "timezone": "UTC",
                    "reason": "past"
                }),
            )
            .await;

        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("in the future"));
    }

    #[tokio::test]
    async fn rejects_invalid_timezone() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        let iso = future.format("%Y-%m-%dT%H:%M:%S").to_string();

        let err = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({
                    "execute_at_iso": iso,
                    "timezone": "Mars/Olympus_Mons",
                    "reason": "bad tz"
                }),
            )
            .await;

        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("unknown timezone"));
    }

    #[tokio::test]
    async fn schedule_recurring_interval() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(Arc::clone(&store));

        let out = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({
                    "delay_seconds": 60,
                    "reason": "repeating check",
                    "recurrence": "interval",
                    "interval_seconds": 300
                }),
            )
            .await
            .expect("should succeed");

        assert!(!out.is_error);
        assert!(out.content.contains("Recurring: interval = 300"));
    }

    #[tokio::test]
    async fn schedule_with_deliver_to_channel() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let out = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({
                    "delay_seconds": 60,
                    "reason": "cross-channel",
                    "deliver_to_channel": "telegram"
                }),
            )
            .await
            .expect("should succeed");

        assert!(!out.is_error);
        assert!(out.content.contains("Delivery channel: telegram"));
    }

    #[tokio::test]
    async fn rejects_missing_delay_and_execute_at() {
        let store = setup_store("sess-1").await;
        let tool = ScheduleHeartbeat::new(store);

        let err = tool
            .execute(
                &test_context("sess-1"),
                serde_json::json!({ "reason": "no time specified" }),
            )
            .await;

        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("delay_seconds"));
    }
}
