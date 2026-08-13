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

    /// One-time migration (RFC-047): copy the mount/project tables from the
    /// legacy `memory.db` (which `MemoryDatabase` shared with the kernel) into
    /// this `kernel.db`.
    ///
    /// The old boot passed the SAME `memory.db` connection to both the memory
    /// store and `MountManager`/`ProjectManager`, so existing mounts and
    /// projects live in `memory.db`'s `mounts`/`mount_dismissals`/`projects`/
    /// `project_memory` tables. On the first boot after the migration the new
    /// `kernel.db` is fresh and empty — without copying these rows, a user
    /// with existing mounts/projects would silently lose them.
    ///
    /// Idempotent: only copies a table when the legacy DB has it AND the
    /// kernel table is empty (so re-runs and concurrent writes are safe).
    /// Forward-only: `memory.db` is never modified.
    pub fn migrate_legacy_mount_project(&self, legacy_path: &Path) -> anyhow::Result<()> {
        if !legacy_path.exists() {
            return Ok(());
        }
        let conn = self.conn();
        // Ensure the kernel's own tables exist (the migration runs before the
        // MountManager/ProjectManager constructors create them).
        crate::mount::mount_db::ensure_mount_schema(&conn)?;
        crate::project::project_db::ensure_project_schema(&conn)?;
        // ATTACH requires a literal SQL string, not a bound param.
        let escaped = legacy_path.to_string_lossy().replace('\'', "''");
        conn.execute_batch(&format!("ATTACH DATABASE '{escaped}' AS legacy"))
            .with_context(|| format!("attaching legacy db {}", legacy_path.display()))?;

        let result = (|| -> anyhow::Result<()> {
            for table in ["projects", "project_memory", "mounts", "mount_dismissals"] {
                let legacy_has: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )?;
                if legacy_has == 0 {
                    continue;
                }
                let kernel_count: i64 =
                    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
                if kernel_count > 0 {
                    continue; // already populated — leave it.
                }
                // Copy only the columns shared by both schemas. The kernel
                // schema may be a superset (e.g. projects gained `mount_ids`
                // and `instructions`); new columns keep their defaults.
                let cols = |schema: &str| -> anyhow::Result<Vec<String>> {
                    let mut stmt = conn.prepare(&format!("PRAGMA {schema}.table_info({table})"))?;
                    let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
                    let mut out = Vec::new();
                    for row in rows {
                        out.push(row?);
                    }
                    Ok(out)
                };
                let legacy_cols = cols("legacy")?;
                let kernel_cols = cols("main")?;
                let common: Vec<&str> = kernel_cols
                    .iter()
                    .filter(|c| legacy_cols.contains(c))
                    .map(|s| s.as_str())
                    .collect();
                if common.is_empty() {
                    continue;
                }
                let col_list = common.join(", ");
                let copied = conn.execute(
                    &format!(
                        "INSERT INTO {table} ({col_list})
                         SELECT {col_list} FROM legacy.{table}"
                    ),
                    [],
                )?;
                tracing::info!(
                    table,
                    copied,
                    "migrated legacy mount/project rows from memory.db into kernel.db"
                );
            }
            Ok(())
        })();

        let _ = conn.execute_batch("DETACH DATABASE legacy");
        result
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

    #[test]
    fn migrate_legacy_mount_project_copies_rows_once() {
        // Legacy db with one mount + one project.
        let dir = tempfile::TempDir::new().expect("tempdir");
        let legacy_path = dir.path().join("memory.db");
        let legacy = Connection::open(&legacy_path).expect("legacy open");
        legacy
            .execute_batch(
                "CREATE TABLE mounts (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, paths TEXT NOT NULL,
                    auto_description TEXT NOT NULL DEFAULT '', auto_meta TEXT NOT NULL DEFAULT '{}',
                    source TEXT NOT NULL DEFAULT 'manual',
                    last_marker_snapshot TEXT NOT NULL DEFAULT '{}',
                    enrichment_pending INTEGER NOT NULL DEFAULT 0,
                    last_enriched_at TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                    last_active_at TEXT NOT NULL
                 );
                 CREATE TABLE projects (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, description TEXT,
                    paths TEXT, tags TEXT, emoji TEXT NOT NULL DEFAULT '📦',
                    source TEXT NOT NULL DEFAULT 'manual', memory_visible INTEGER NOT NULL DEFAULT 1,
                    created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_active_at TEXT NOT NULL
                 );
                 INSERT INTO mounts VALUES (
                    'm1','oxios','[\"/x\"]','desc','{}','manual','{}',0,NULL,'a','a','a'
                 );
                 INSERT INTO projects VALUES (
                    'p1','proj',NULL,NULL,NULL,'📦','manual',1,'a','a','a'
                 );",
            )
            .expect("seed legacy");
        drop(legacy);

        let kernel = KernelDatabase::open(dir.path().join("kernel.db")).expect("kernel open");
        kernel
            .migrate_legacy_mount_project(&legacy_path)
            .expect("migrate");

        let mounts: i64 = kernel
            .conn()
            .query_row("SELECT COUNT(*) FROM mounts", [], |r| r.get(0))
            .expect("mounts count");
        let projects: i64 = kernel
            .conn()
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .expect("projects count");
        assert_eq!(mounts, 1, "mount row copied");
        assert_eq!(projects, 1, "project row copied");

        // Idempotent: re-run does not duplicate.
        kernel
            .migrate_legacy_mount_project(&legacy_path)
            .expect("migrate again");
        let mounts2: i64 = kernel
            .conn()
            .query_row("SELECT COUNT(*) FROM mounts", [], |r| r.get(0))
            .expect("mounts count");
        assert_eq!(mounts2, 1, "no duplicate on re-run");
    }
}
