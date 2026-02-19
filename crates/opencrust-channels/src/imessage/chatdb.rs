use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags};
use tracing::{debug, warn};

/// A single incoming iMessage read from chat.db.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    pub rowid: i64,
    pub text: String,
    pub sender: String,
    pub timestamp: i64,
}

/// Read-only handle to `~/Library/Messages/chat.db`.
pub struct ChatDb {
    conn: Connection,
    last_seen_rowid: i64,
}

/// macOS Core Data epoch offset: seconds between Unix epoch (1970) and Apple epoch (2001).
const CORE_DATA_EPOCH_OFFSET: i64 = 978_307_200;

/// Convert a macOS Core Data nanosecond timestamp to Unix epoch seconds.
pub fn core_data_ns_to_unix(ns: i64) -> i64 {
    ns / 1_000_000_000 + CORE_DATA_EPOCH_OFFSET
}

/// Default path to the iMessage database.
pub fn default_chat_db_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join("Library/Messages/chat.db")
}

impl ChatDb {
    /// Open the chat database read-only and initialise `last_seen_rowid` to the
    /// current maximum so we only pick up messages arriving after startup.
    pub fn open(path: &std::path::Path) -> std::result::Result<Self, String> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;

        let conn = Connection::open_with_flags(path, flags).map_err(|e| {
            format!(
                "failed to open chat.db at {}: {e}. \
                 Ensure Full Disk Access is granted to the terminal / OpenCrust binary \
                 in System Settings → Privacy & Security → Full Disk Access.",
                path.display()
            )
        })?;

        let max_rowid: i64 = conn
            .query_row("SELECT COALESCE(MAX(ROWID), 0) FROM message", [], |row| {
                row.get(0)
            })
            .map_err(|e| format!("failed to query max ROWID: {e}"))?;

        debug!(
            "opened chat.db at {}, last_seen_rowid = {max_rowid}",
            path.display()
        );

        Ok(Self {
            conn,
            last_seen_rowid: max_rowid,
        })
    }

    /// Poll for new incoming direct messages since the last poll.
    ///
    /// Returns messages ordered by date ascending. Group chat messages
    /// (where `cache_roomnames` is non-empty) are excluded.
    pub fn poll(&mut self) -> Vec<IncomingMessage> {
        let mut stmt = match self.conn.prepare(
            "SELECT m.ROWID, m.text, m.date, m.is_from_me, m.cache_roomnames, \
                    h.id AS sender_id \
             FROM message m \
             JOIN handle h ON m.handle_id = h.ROWID \
             WHERE m.ROWID > ?1 AND m.is_from_me = 0 \
               AND (m.cache_roomnames IS NULL OR m.cache_roomnames = '') \
             ORDER BY m.date ASC",
        ) {
            Ok(s) => s,
            Err(e) => {
                warn!("imessage: failed to prepare poll query: {e}");
                return Vec::new();
            }
        };

        let rows = match stmt.query_map([self.last_seen_rowid], |row| {
            let rowid: i64 = row.get(0)?;
            let text: Option<String> = row.get(1)?;
            let date: i64 = row.get(2)?;
            let sender: String = row.get(5)?;
            Ok((rowid, text, date, sender))
        }) {
            Ok(r) => r,
            Err(e) => {
                warn!("imessage: failed to execute poll query: {e}");
                return Vec::new();
            }
        };

        let mut messages = Vec::new();
        for row in rows {
            match row {
                Ok((rowid, Some(text), date, sender)) if !text.is_empty() => {
                    if rowid > self.last_seen_rowid {
                        self.last_seen_rowid = rowid;
                    }
                    messages.push(IncomingMessage {
                        rowid,
                        text,
                        sender,
                        timestamp: core_data_ns_to_unix(date),
                    });
                }
                Ok((rowid, _, _, _)) => {
                    // NULL or empty text — skip but advance cursor
                    if rowid > self.last_seen_rowid {
                        self.last_seen_rowid = rowid;
                    }
                }
                Err(e) => {
                    warn!("imessage: error reading message row: {e}");
                }
            }
        }

        messages
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_conversion() {
        // 2024-01-01 00:00:00 UTC in Core Data nanoseconds:
        // Unix timestamp for 2024-01-01 = 1704067200
        // Core Data seconds = 1704067200 - 978307200 = 725760000
        // Core Data nanoseconds = 725760000 * 1_000_000_000
        let core_data_ns: i64 = 725_760_000 * 1_000_000_000;
        let unix = core_data_ns_to_unix(core_data_ns);
        assert_eq!(unix, 1_704_067_200);
    }

    #[test]
    fn timestamp_zero_is_apple_epoch() {
        // Core Data timestamp 0 = 2001-01-01 00:00:00 UTC = Unix 978307200
        assert_eq!(core_data_ns_to_unix(0), CORE_DATA_EPOCH_OFFSET);
    }
}
