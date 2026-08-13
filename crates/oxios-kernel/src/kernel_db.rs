//! Kernel SQLite connection — the shared backend for mount/project tables.
//!
//! Extracted from `oxios-memory::memory::sqlite::MemoryDatabase` (RFC-047):
//! that type mixed the generic connection with the memory schema, sqlite-vec
//! registration, and an embedding dimension. The kernel only needs the
//! connection: `MountManager` and `ProjectManager` create their own tables
//! through `conn()`.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use rusqlite::Connection;
use std::path::Path;

/// A plain SQLite connection in WAL mode with foreign keys enabled.
///
/// Thread-safe via `Mutex<Connection>` (SQLite serialised access). No schema
/// is owned here — consumers bootstrap their own tables.
#[derive(Debug)]
pub struct KernelDatabase {
    conn: Mutex<Connection>,
}

impl KernelDatabase {
    /// Open (or create) the database at `path`, ensuring the parent directory
    /// exists. Sets WAL mode, `synchronous=NORMAL`, and foreign keys.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating database directory: {}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening kernel DB: {}", path.display()))?;
        Self::init(conn)
    }

    /// Open an in-memory database (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Get a locked connection reference.
    ///
    /// Returns a `MutexGuard<Connection>` for executing queries.
    /// `parking_lot::Mutex` is `Send` and safe to use in async contexts.
    /// IMPORTANT: always drop the guard before any `.await` point.
    pub fn conn(&self) -> parking_lot::MutexGuard<'_, Connection> {
        self.conn.lock()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_executes_queries() {
        let db = KernelDatabase::open_in_memory().expect("open in-memory db");
        let n: i64 = db
            .conn()
            .query_row("SELECT 1", [], |r| r.get(0))
            .expect("query");
        assert_eq!(n, 1);
    }

    #[test]
    fn open_creates_wal_capable_file() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let db_path = dir.path().join("kernel.db");
        let db = KernelDatabase::open(&db_path).expect("open file db");
        let mode: String = db
            .conn()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .expect("journal mode");
        assert_eq!(mode, "wal");
        assert!(db_path.exists(), "db file created");
    }

    #[test]
    fn foreign_keys_enabled() {
        let db = KernelDatabase::open_in_memory().expect("db");
        let fk: i64 = db
            .conn()
            .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
            .expect("fk");
        assert_eq!(fk, 1);
    }
}
