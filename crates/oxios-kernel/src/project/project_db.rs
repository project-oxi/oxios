//! Project SQLite persistence.
//!
//! The `projects` table stores the reduced project record (design §4.1):
//! `root_paths` as a JSON array of canonical paths, project instructions,
//! and RFC 3339 timestamps. A legacy table from the pre-`root_paths`
//! era is dropped on boot — a clean, user-approved break.

use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};
use std::path::PathBuf;

use super::Project;

/// Schema DDL for the projects table.
pub const PROJECT_SCHEMA: &str = r#"
-- ─────────────────────────────────────────────
-- Projects (reduced record: root_paths + instructions)
-- ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS projects (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    root_paths      TEXT NOT NULL DEFAULT '[]',
    instructions    TEXT NOT NULL DEFAULT '',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    last_active_at  TEXT NOT NULL,
    default_brain_space TEXT     -- brain space inherited by new chats (NULL = none)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_name ON projects(name);
"#;

/// Ensure the project table exists in the database.
///
/// Clean break: when an existing `projects` table lacks the `root_paths`
/// column (legacy schema), it is dropped and recreated from the current
/// DDL. Legacy project records are discarded by design, and the retired
/// Mount-era companion tables (`mounts`, `mount_dismissals`,
/// `project_memory`) are dropped in the same one-time guarded pass so
/// none of the old concept survives in the persistent schema.
pub fn ensure_project_schema(conn: &rusqlite::Connection) -> Result<()> {
    let has_table: bool = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='projects'",
        [],
        |r| r.get::<_, i64>(0),
    )? > 0;

    if has_table {
        let mut stmt = conn.prepare("PRAGMA table_info(projects)")?;
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        if !cols.iter().any(|c| c == "root_paths") {
            tracing::info!(
                "legacy projects table detected (no root_paths column) — dropping (clean break)"
            );
            // One-time guarded drop: the Mount-era companions shipped with
            // the legacy layout and must not outlive it.
            conn.execute_batch(
                "DROP TABLE projects;
                 DROP TABLE IF EXISTS mounts;
                 DROP TABLE IF EXISTS mount_dismissals;
                 DROP TABLE IF EXISTS project_memory;",
            )?;
        }
    }

    conn.execute_batch(PROJECT_SCHEMA)?;
    migrate_default_brain_space(conn)?;
    Ok(())
}

/// Migration: add the `default_brain_space` column (brain-chat binding) to
/// `projects`. Idempotent — checks `PRAGMA table_info` first. The current
/// DDL always creates the column, so this only fires for databases written
/// by an intermediate build whose table predates it.
pub fn migrate_default_brain_space(conn: &rusqlite::Connection) -> Result<()> {
    let mut stmt = conn.prepare("PRAGMA table_info(projects)")?;
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();

    if !cols.iter().any(|c| c == "default_brain_space") {
        conn.execute_batch("ALTER TABLE projects ADD COLUMN default_brain_space TEXT;")?;
    }
    Ok(())
}

/// Save a project (insert or update).
///
/// Returns `rusqlite::Result` so callers can distinguish the unique-name
/// constraint violation from other failures.
pub fn save_project(conn: &rusqlite::Connection, project: &Project) -> rusqlite::Result<()> {
    let root_paths = serde_json::to_string(&project.root_paths)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    conn.execute(
        "INSERT INTO projects
         (id, name, root_paths, instructions, default_brain_space,
          created_at, updated_at, last_active_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            name=excluded.name, root_paths=excluded.root_paths,
            instructions=excluded.instructions,
            default_brain_space=excluded.default_brain_space,
            updated_at=excluded.updated_at,
            last_active_at=excluded.last_active_at",
        rusqlite::params![
            project.id.to_string(),
            project.name,
            root_paths,
            project.instructions,
            project.default_brain_space,
            rfc3339(project.created_at),
            rfc3339(project.updated_at),
            rfc3339(project.last_active_at),
        ],
    )?;
    Ok(())
}

/// List all projects.
pub fn list_projects(conn: &rusqlite::Connection) -> Result<Vec<Project>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, root_paths, instructions, default_brain_space,
                created_at, updated_at, last_active_at
         FROM projects ORDER BY name",
    )?;
    let rows = stmt.query_map([], row_to_project)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Delete a project by ID.
///
/// Returns the raw `rusqlite::Error` (not an anyhow flattening) so
/// `ProjectManager::remove_project` can classify storage failures as
/// `ProjectManagerError::Database` at the API boundary.
pub fn delete_project(conn: &rusqlite::Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM projects WHERE id = ?1", rusqlite::params![id])?;
    Ok(())
}

/// Render a timestamp as RFC 3339 with second precision (UTC).
fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Parse an RFC 3339 timestamp, falling back to now on malformed input.
fn parse_rfc3339(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

/// Convert a SQLite row into a Project struct.
fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    let id_str: String = row.get(0)?;
    let name: String = row.get(1)?;
    let root_paths_str: String = row
        .get::<_, Option<String>>(2)?
        .unwrap_or_else(|| "[]".to_string());
    let instructions: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
    let default_brain_space: Option<String> = row.get(4)?;
    let created_at: String = row.get(5)?;
    let updated_at: String = row.get(6)?;
    let last_active_at: String = row.get(7)?;

    let id = uuid::Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let root_paths: Vec<PathBuf> = serde_json::from_str(&root_paths_str).unwrap_or_default();

    Ok(Project {
        id,
        name,
        root_paths,
        instructions,
        default_brain_space,
        created_at: parse_rfc3339(&created_at),
        updated_at: parse_rfc3339(&updated_at),
        last_active_at: parse_rfc3339(&last_active_at),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_roundtrips_a_project_and_replaces_legacy_table() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");

        // Seed a legacy (pre-root_paths) layout — the projects table and
        // its Mount-era companions must all be dropped on ensure.
        conn.execute_batch(
            "CREATE TABLE projects (
                id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, description TEXT,
                paths TEXT, tags TEXT, emoji TEXT NOT NULL DEFAULT '📦',
                source TEXT NOT NULL DEFAULT 'manual', memory_visible INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_active_at TEXT NOT NULL
             );
             INSERT INTO projects VALUES ('p1','stale',NULL,NULL,NULL,'📦','manual',1,'a','a','a');
             CREATE TABLE mounts (
                id TEXT PRIMARY KEY, project_id TEXT NOT NULL, path TEXT NOT NULL
             );
             CREATE TABLE mount_dismissals (
                mount_id TEXT PRIMARY KEY, dismissed_at TEXT NOT NULL
             );
             CREATE TABLE project_memory (
                project_id TEXT NOT NULL, memory_id TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
             );",
        )
        .expect("seed legacy schema");

        ensure_project_schema(&conn).expect("ensure schema");

        let stale: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .expect("count after drop");
        assert_eq!(stale, 0, "legacy rows must not survive the clean break");

        // No Mount concept survives the clean break: all three companion
        // tables are gone, not just the legacy projects table.
        for orphan in ["mounts", "mount_dismissals", "project_memory"] {
            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    rusqlite::params![orphan],
                    |r| r.get(0),
                )
                .expect("query sqlite_master");
            assert_eq!(n, 0, "{orphan} must not survive the clean break");
        }

        let project = Project::new("oxios", vec![PathBuf::from("/tmp")], "instructions");
        save_project(&conn, &project).expect("save");
        let loaded = list_projects(&conn).expect("list");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, project.id);
        assert_eq!(loaded[0].root_paths, vec![PathBuf::from("/tmp")]);
        assert_eq!(loaded[0].instructions, "instructions");

        // Upsert by id: name change persists, row count stays 1.
        let mut renamed = project.clone();
        renamed.name = "oxios-2".to_string();
        save_project(&conn, &renamed).expect("upsert");
        let names: Vec<String> = list_projects(&conn)
            .expect("list")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, vec!["oxios-2".to_string()]);

        // Unique name index is enforced.
        let impostor = Project::new("oxios-2", vec![], "");
        let err = save_project(&conn, &impostor).expect_err("duplicate name must violate index");
        assert!(
            matches!(
                err.sqlite_error_code(),
                Some(rusqlite::ErrorCode::ConstraintViolation)
            ),
            "expected constraint violation, got {err:?}"
        );
    }

    /// `default_brain_space` survives a save/load round trip in all three
    /// states on the reduced schema: set, cleared (stored as NULL, not
    /// skipped), and re-pointed at a different brain.
    #[test]
    fn default_brain_space_round_trip() {
        let conn = rusqlite::Connection::open_in_memory().expect("in-memory sqlite");
        ensure_project_schema(&conn).expect("ensure schema");

        let mut project = Project::new("brainy", vec![], "");
        project.default_brain_space = Some("work".to_string());
        save_project(&conn, &project).expect("save");

        let loaded = list_projects(&conn).expect("list");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].default_brain_space.as_deref(), Some("work"));

        // Clearing must persist as NULL.
        project.default_brain_space = None;
        save_project(&conn, &project).expect("save cleared");
        let loaded = list_projects(&conn).expect("list cleared");
        assert_eq!(loaded[0].default_brain_space, None);

        // Re-point at a different brain.
        project.default_brain_space = Some("personal".to_string());
        save_project(&conn, &project).expect("save re-set");
        let loaded = list_projects(&conn).expect("list re-set");
        assert_eq!(loaded[0].default_brain_space.as_deref(), Some("personal"));
    }

    /// A reduced-schema table written before the brain-default column
    /// existed gets it via the guarded ALTER TABLE; existing rows read
    /// back with `default_brain_space: None`.
    #[test]
    fn default_brain_space_migration_adds_nullable_column() {
        let conn = rusqlite::Connection::open_in_memory().expect("conn");
        conn.execute_batch(
            "CREATE TABLE projects (
                id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE,
                root_paths TEXT NOT NULL DEFAULT '[]',
                instructions TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                last_active_at TEXT NOT NULL
            );
            INSERT INTO projects VALUES (
                '3f9d3b7a-8c4e-4f2a-9b1d-2e5c6a7d8e9f',
                'legacy','[]','','a','a','a'
            );",
        )
        .expect("seed pre-upgrade schema");

        migrate_default_brain_space(&conn).expect("migrate");
        migrate_default_brain_space(&conn).expect("migrate again (idempotent)");

        let loaded = list_projects(&conn).expect("list");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].default_brain_space, None);
    }
}
