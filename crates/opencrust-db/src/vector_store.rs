use opencrust_common::{Error, Result};
use rusqlite::{Connection, ffi::sqlite3_auto_extension, params};
use std::path::Path;
use std::sync::{Mutex, Once};
use tracing::{info, warn};

static SQLITE_VEC_INIT: Once = Once::new();
static mut SQLITE_VEC_LOADED: bool = false;

/// Register sqlite-vec as an auto-extension. This is process-global and only
/// needs to happen once. Safe to call multiple times (no-op after first).
fn ensure_sqlite_vec_registered() -> bool {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        let func = std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ());
        sqlite3_auto_extension(Some(func));
        SQLITE_VEC_LOADED = true;
        info!("sqlite-vec auto-extension registered");
    });
    unsafe { SQLITE_VEC_LOADED }
}

/// Vector database for semantic search and memory embeddings.
/// Uses sqlite-vec for KNN vector similarity operations with a fallback
/// to in-Rust cosine similarity if the extension cannot be loaded.
pub struct VectorStore {
    conn: Mutex<Connection>,
    vec_enabled: bool,
}

impl VectorStore {
    pub fn open(db_path: &Path) -> Result<Self> {
        info!("opening vector store at {}", db_path.display());
        let vec_enabled = ensure_sqlite_vec_registered();

        let conn = Connection::open(db_path)
            .map_err(|e| Error::Database(format!("failed to open vector database: {e}")))?;

        // Verify sqlite-vec is actually working
        let vec_enabled = if vec_enabled {
            verify_vec_extension(&conn)
        } else {
            false
        };

        let store = Self {
            conn: Mutex::new(conn),
            vec_enabled,
        };
        store.run_migrations()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self> {
        let vec_enabled = ensure_sqlite_vec_registered();

        let conn = Connection::open_in_memory()
            .map_err(|e| Error::Database(format!("failed to open in-memory vector db: {e}")))?;

        let vec_enabled = if vec_enabled {
            verify_vec_extension(&conn)
        } else {
            false
        };

        let store = Self {
            conn: Mutex::new(conn),
            vec_enabled,
        };
        store.run_migrations()?;
        Ok(store)
    }

    /// Whether the sqlite-vec extension is available.
    pub fn vec_enabled(&self) -> bool {
        self.vec_enabled
    }

    fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| Error::Database("vector store lock poisoned".into()))
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.connection()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS embeddings (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                content TEXT NOT NULL,
                embedding BLOB,
                metadata TEXT DEFAULT '{}',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            -- Mapping table: vec0 requires integer rowids but memory IDs are UUIDs.
            CREATE TABLE IF NOT EXISTS vec_id_map (
                rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                entry_id TEXT NOT NULL UNIQUE
            );",
        )
        .map_err(|e| Error::Database(format!("vector store migration failed: {e}")))?;

        Ok(())
    }

    /// Create or verify that a `vec0` virtual table exists for the given dimensionality.
    /// This is a no-op if sqlite-vec is not loaded.
    pub fn ensure_vec_table(&self, dimensions: usize) -> Result<()> {
        if !self.vec_enabled {
            return Ok(());
        }

        let conn = self.connection()?;
        let table_name = format!("vec_embeddings_{dimensions}");

        // Check if the table already exists
        let exists: bool = conn
            .query_row(
                "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name=?",
                params![table_name],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(format!("failed to check vec table: {e}")))?;

        if !exists {
            let sql = format!(
                "CREATE VIRTUAL TABLE [{table_name}] USING vec0(embedding float[{dimensions}])"
            );
            conn.execute_batch(&sql)
                .map_err(|e| Error::Database(format!("failed to create vec table: {e}")))?;
            info!("created vec0 table: {table_name} ({dimensions} dims)");
        }

        Ok(())
    }

    /// Insert an embedding vector into the vec0 virtual table.
    /// Maps the string `id` to an integer rowid via `vec_id_map`.
    pub fn insert_embedding(&self, id: &str, embedding: &[f32], dimensions: usize) -> Result<()> {
        if !self.vec_enabled {
            return Ok(());
        }

        let conn = self.connection()?;
        let table_name = format!("vec_embeddings_{dimensions}");
        let blob = embedding_to_blob(embedding);

        // Upsert into the ID mapping table
        conn.execute(
            "INSERT OR IGNORE INTO vec_id_map (entry_id) VALUES (?)",
            params![id],
        )
        .map_err(|e| Error::Database(format!("failed to insert vec id mapping: {e}")))?;

        let rowid: i64 = conn
            .query_row(
                "SELECT rowid FROM vec_id_map WHERE entry_id = ?",
                params![id],
                |row| row.get(0),
            )
            .map_err(|e| Error::Database(format!("failed to get vec rowid: {e}")))?;

        conn.execute(
            &format!("INSERT OR REPLACE INTO [{table_name}] (rowid, embedding) VALUES (?, ?)"),
            params![rowid, blob],
        )
        .map_err(|e| Error::Database(format!("failed to insert vec embedding: {e}")))?;

        Ok(())
    }

    /// KNN search: find the nearest `limit` embeddings to `query`.
    /// Returns `(entry_id, distance)` pairs ordered by distance ascending.
    pub fn search_nearest(
        &self,
        query: &[f32],
        dimensions: usize,
        limit: usize,
    ) -> Result<Vec<(String, f64)>> {
        if !self.vec_enabled {
            return Ok(Vec::new());
        }

        let conn = self.connection()?;
        let table_name = format!("vec_embeddings_{dimensions}");
        let blob = embedding_to_blob(query);

        let mut stmt = conn
            .prepare(&format!(
                "SELECT m.entry_id, v.distance
                 FROM [{table_name}] v
                 JOIN vec_id_map m ON m.rowid = v.rowid
                 WHERE v.embedding MATCH ? AND k = ?"
            ))
            .map_err(|e| Error::Database(format!("failed to prepare KNN query: {e}")))?;

        let rows = stmt
            .query_map(params![blob, limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(|e| Error::Database(format!("KNN query failed: {e}")))?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| Error::Database(format!("failed to collect KNN results: {e}")))
    }
}

/// Verify that sqlite-vec functions are available on this connection.
fn verify_vec_extension(conn: &Connection) -> bool {
    match conn.query_row("SELECT vec_version()", [], |row| row.get::<_, String>(0)) {
        Ok(version) => {
            info!("sqlite-vec {version} available");
            true
        }
        Err(e) => {
            warn!("sqlite-vec not functional: {e} (falling back to in-Rust cosine similarity)");
            false
        }
    }
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for v in embedding {
        bytes.extend(v.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_creates_embeddings_table() {
        let store = VectorStore::in_memory().expect("should open in-memory vector store");
        let conn = store.connection().expect("lock not poisoned");
        let exists: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='embeddings'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1);
    }

    #[test]
    fn vec_table_lifecycle() {
        let store = VectorStore::in_memory().expect("should open in-memory vector store");
        if !store.vec_enabled() {
            eprintln!("sqlite-vec not available, skipping vec table test");
            return;
        }

        store.ensure_vec_table(3).unwrap();

        // Insert
        store.insert_embedding("id-1", &[1.0, 0.0, 0.0], 3).unwrap();
        store.insert_embedding("id-2", &[0.0, 1.0, 0.0], 3).unwrap();

        // Search
        let results = store.search_nearest(&[0.9, 0.1, 0.0], 3, 2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "id-1"); // closest
    }
}
