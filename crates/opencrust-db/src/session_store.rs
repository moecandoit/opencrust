use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use chrono::{DateTime, Utc};
use opencrust_common::{Error, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use tracing::info;
use uuid::Uuid;

/// Persistent storage for conversation sessions and message history.
pub struct SessionStore {
    conn: Mutex<Connection>,
}

/// A persisted session record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub channel_id: Option<String>,
    pub user_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A persisted message within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageRecord {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

impl SessionStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        info!("opening session store at {}", db_path.display());
        let conn = Connection::open(db_path)
            .map_err(|e| Error::Database(format!("failed to open database: {e}")))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| Error::Database(format!("failed to set pragmas: {e}")))?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.run_migrations()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .map_err(|e| Error::Database(format!("failed to open in-memory database: {e}")))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| Error::Database(format!("failed to set pragmas: {e}")))?;

        let store = Self {
            conn: Mutex::new(conn),
        };
        store.run_migrations()?;
        Ok(store)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| Error::Database("session store lock poisoned".into()))
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.connection()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                channel_id TEXT,
                user_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session
                ON messages(session_id, created_at);",
        )
        .map_err(|e| Error::Database(format!("migration failed: {e}")))?;

        Ok(())
    }

    pub fn create_session(
        &self,
        id: &str,
        user_id: Option<&str>,
        channel_id: Option<&str>,
    ) -> Result<()> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO sessions (id, channel_id, user_id) VALUES (?1, ?2, ?3)",
            params![id, channel_id, user_id],
        )
        .map_err(|e| Error::Database(format!("failed to create session: {e}")))?;
        Ok(())
    }

    pub fn get_session(&self, id: &str) -> Result<Option<SessionRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare("SELECT id, channel_id, user_id, created_at, updated_at FROM sessions WHERE id = ?1")
            .map_err(|e| Error::Database(format!("failed to prepare query: {e}")))?;

        let result = stmt
            .query_row(params![id], |row| {
                Ok(SessionRecord {
                    id: row.get(0)?,
                    channel_id: row.get(1)?,
                    user_id: row.get(2)?,
                    created_at: parse_datetime(row.get::<_, String>(3)?),
                    updated_at: parse_datetime(row.get::<_, String>(4)?),
                })
            })
            .ok();

        Ok(result)
    }

    pub fn delete_session(&self, id: &str) -> Result<()> {
        let conn = self.connection()?;
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])
            .map_err(|e| Error::Database(format!("failed to delete session: {e}")))?;
        Ok(())
    }

    pub fn touch_session(&self, id: &str) -> Result<()> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
            params![id],
        )
        .map_err(|e| Error::Database(format!("failed to update session timestamp: {e}")))?;
        Ok(())
    }

    pub fn append_message(&self, session_id: &str, role: &str, content: &str) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO messages (id, session_id, role, content) VALUES (?1, ?2, ?3, ?4)",
            params![id, session_id, role, content],
        )
        .map_err(|e| Error::Database(format!("failed to append message: {e}")))?;

        // Also bump the session's updated_at
        conn.execute(
            "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
            params![session_id],
        )
        .map_err(|e| Error::Database(format!("failed to touch session: {e}")))?;

        Ok(id)
    }

    pub fn get_messages(&self, session_id: &str, limit: usize) -> Result<Vec<MessageRecord>> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, session_id, role, content, created_at
                 FROM messages
                 WHERE session_id = ?1
                 ORDER BY created_at ASC
                 LIMIT ?2",
            )
            .map_err(|e| Error::Database(format!("failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![session_id, limit as i64], |row| {
                Ok(MessageRecord {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    created_at: parse_datetime(row.get::<_, String>(4)?),
                })
            })
            .map_err(|e| Error::Database(format!("failed to query messages: {e}")))?;

        let mut messages = Vec::new();
        for row in rows {
            messages.push(
                row.map_err(|e| Error::Database(format!("failed to read message row: {e}")))?,
            );
        }
        Ok(messages)
    }

    pub fn session_count(&self) -> Result<usize> {
        let conn = self.connection()?;
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .map_err(|e| Error::Database(format!("failed to count sessions: {e}")))?;
        Ok(count as usize)
    }
}

fn parse_datetime(s: String) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| {
            // SQLite datetime('now') produces "YYYY-MM-DD HH:MM:SS"
            chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                .map(|naive| naive.and_utc())
                .unwrap_or_else(|_| Utc::now())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_get_session_round_trip() {
        let store = SessionStore::in_memory().unwrap();
        store
            .create_session("sess-1", Some("user-1"), Some("web"))
            .unwrap();

        let session = store.get_session("sess-1").unwrap().unwrap();
        assert_eq!(session.id, "sess-1");
        assert_eq!(session.user_id.as_deref(), Some("user-1"));
        assert_eq!(session.channel_id.as_deref(), Some("web"));
    }

    #[test]
    fn create_session_with_no_user_or_channel() {
        let store = SessionStore::in_memory().unwrap();
        store.create_session("sess-2", None, None).unwrap();

        let session = store.get_session("sess-2").unwrap().unwrap();
        assert_eq!(session.id, "sess-2");
        assert!(session.user_id.is_none());
        assert!(session.channel_id.is_none());
    }

    #[test]
    fn get_missing_session_returns_none() {
        let store = SessionStore::in_memory().unwrap();
        assert!(store.get_session("nonexistent").unwrap().is_none());
    }

    #[test]
    fn append_and_retrieve_messages() {
        let store = SessionStore::in_memory().unwrap();
        store.create_session("sess-3", None, None).unwrap();

        store.append_message("sess-3", "user", "Hello").unwrap();
        store
            .append_message("sess-3", "assistant", "Hi there!")
            .unwrap();
        store
            .append_message("sess-3", "user", "How are you?")
            .unwrap();

        let messages = store.get_messages("sess-3", 100).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Hi there!");
        assert_eq!(messages[2].role, "user");
        assert_eq!(messages[2].content, "How are you?");
    }

    #[test]
    fn get_messages_respects_limit() {
        let store = SessionStore::in_memory().unwrap();
        store.create_session("sess-4", None, None).unwrap();

        for i in 0..10 {
            store
                .append_message("sess-4", "user", &format!("msg {i}"))
                .unwrap();
        }

        let messages = store.get_messages("sess-4", 3).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "msg 0");
    }

    #[test]
    fn delete_session_cascades_to_messages() {
        let store = SessionStore::in_memory().unwrap();
        store.create_session("sess-5", None, None).unwrap();
        store.append_message("sess-5", "user", "Hello").unwrap();

        store.delete_session("sess-5").unwrap();

        assert!(store.get_session("sess-5").unwrap().is_none());
        let messages = store.get_messages("sess-5", 100).unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn session_count_tracks_correctly() {
        let store = SessionStore::in_memory().unwrap();
        assert_eq!(store.session_count().unwrap(), 0);

        store.create_session("a", None, None).unwrap();
        store.create_session("b", None, None).unwrap();
        assert_eq!(store.session_count().unwrap(), 2);

        store.delete_session("a").unwrap();
        assert_eq!(store.session_count().unwrap(), 1);
    }
}
