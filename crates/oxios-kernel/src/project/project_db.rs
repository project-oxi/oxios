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
    last_active_at  TEXT NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_name ON projects(name);
"#;

/// Ensure the project table exists in the database.
///
/// Clean break: when an existing `projects` table lacks the `root_paths`
/// column (legacy schema), it is dropped and recreated from the current
/// DDL. Legacy project records are discarded by design.
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
            conn.execute_batch("DROP TABLE projects;")?;
        }
    }

    conn.execute_batch(PROJECT_SCHEMA)?;
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
         (id, name, root_paths, instructions, created_at, updated_at, last_active_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
            name=excluded.name, root_paths=excluded.root_paths,
            instructions=excluded.instructions, updated_at=excluded.updated_at,
            last_active_at=excluded.last_active_at",
        rusqlite::params![
            project.id.to_string(),
            project.name,
            root_paths,
            project.instructions,
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
        "SELECT id, name, root_paths, instructions, created_at, updated_at, last_active_at
         FROM projects ORDER BY name",
    )?;
    let rows = stmt.query_map([], row_to_project)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Delete a project by ID.
pub fn delete_project(conn: &rusqlite::Connection, id: &str) -> Result<()> {
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
    let created_at: String = row.get(4)?;
    let updated_at: String = row.get(5)?;
    let last_active_at: String = row.get(6)?;

    let id = uuid::Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let root_paths: Vec<PathBuf> = serde_json::from_str(&root_paths_str).unwrap_or_default();

    Ok(Project {
        id,
        name,
        root_paths,
        instructions,
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

        // Seed a legacy (pre-root_paths) table — must be dropped on ensure.
        conn.execute_batch(
            "CREATE TABLE projects (
                id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, description TEXT,
                paths TEXT, tags TEXT, emoji TEXT NOT NULL DEFAULT '📦',
                source TEXT NOT NULL DEFAULT 'manual', memory_visible INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL, updated_at TEXT NOT NULL, last_active_at TEXT NOT NULL
             );
             INSERT INTO projects VALUES ('p1','stale',NULL,NULL,NULL,'📦','manual',1,'a','a','a');",
        )
        .expect("seed legacy schema");

        ensure_project_schema(&conn).expect("ensure schema");

        let stale: i64 = conn
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))
            .expect("count after drop");
        assert_eq!(stale, 0, "legacy rows must not survive the clean break");

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
}
