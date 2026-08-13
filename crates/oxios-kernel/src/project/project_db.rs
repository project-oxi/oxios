//! Project-related SQLite operations.
//!
//! Extracted from `memory/database.rs` — project tables (`projects`,
//! `project_memory`) are a kernel concern, not a memory concern.
//! Uses `KernelDatabase::conn()` for SQL execution.

use anyhow::Result;

use super::{Project, ProjectSource};

/// Schema DDL for project tables.
pub const PROJECT_SCHEMA: &str = r#"
-- ─────────────────────────────────────────────
-- Projects (RFC-011 / RFC-025)
-- ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS projects (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    description     TEXT,
    paths           TEXT,            -- JSON array of PathBuf strings
    tags            TEXT,            -- JSON array of strings
    emoji           TEXT NOT NULL DEFAULT '📦',
    source          TEXT NOT NULL DEFAULT 'manual',
    memory_visible  INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    last_active_at  TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_projects_name ON projects(name);

-- ─────────────────────────────────────────────
-- Project-Memory junction (RFC-011)
-- ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS project_memory (
    project_id  TEXT NOT NULL,
    memory_id   TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (project_id, memory_id)
);

CREATE INDEX IF NOT EXISTS idx_pm_project ON project_memory(project_id);
CREATE INDEX IF NOT EXISTS idx_pm_memory ON project_memory(memory_id);
"#;

/// Migration: add RFC-025 columns (`mount_ids`, `instructions`) to `projects`.
/// Idempotent — checks `PRAGMA table_info` first.
pub fn migrate_rfc025(conn: &rusqlite::Connection) -> Result<()> {
    // Check existing columns.
    let mut stmt = conn.prepare("PRAGMA table_info(projects)")?;
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();

    if !cols.iter().any(|c| c == "mount_ids") {
        conn.execute_batch(
            "ALTER TABLE projects ADD COLUMN mount_ids TEXT NOT NULL DEFAULT '[]';",
        )?;
    }
    if !cols.iter().any(|c| c == "instructions") {
        conn.execute_batch(
            "ALTER TABLE projects ADD COLUMN instructions TEXT NOT NULL DEFAULT '';",
        )?;
    }
    Ok(())
}

/// Ensure project tables exist in the database.
pub fn ensure_project_schema(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(PROJECT_SCHEMA)?;
    migrate_rfc025(conn)?;
    Ok(())
}

/// Save a project (insert or replace).
pub fn save_project(conn: &rusqlite::Connection, project: &Project) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO projects
         (id, name, description, paths, tags, emoji, source, memory_visible,
          mount_ids, instructions, created_at, updated_at, last_active_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
         ON CONFLICT(id) DO UPDATE SET
            name=excluded.name, description=excluded.description, paths=excluded.paths,
            tags=excluded.tags, emoji=excluded.emoji, source=excluded.source,
            memory_visible=excluded.memory_visible, mount_ids=excluded.mount_ids,
            instructions=excluded.instructions, updated_at=excluded.updated_at,
            last_active_at=excluded.last_active_at",
        rusqlite::params![
            project.id.to_string(),
            project.name,
            project.description,
            serde_json::to_string(&project.paths)?,
            serde_json::to_string(&project.tags)?,
            project.emoji,
            project.source.to_string(),
            project.memory_visible as i32,
            serde_json::to_string(&project.mount_ids)?,
            project.instructions,
            project.created_at.to_rfc3339(),
            project.updated_at.to_rfc3339(),
            project.last_active_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// List all projects.
pub fn list_projects(conn: &rusqlite::Connection) -> Result<Vec<Project>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, description, paths, tags, emoji, source, memory_visible,
                mount_ids, instructions, created_at, updated_at, last_active_at
         FROM projects ORDER BY name",
    )?;
    let rows = stmt.query_map([], row_to_project)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Delete a project by ID.
pub fn delete_project(conn: &rusqlite::Connection, id: &str) -> Result<()> {
    // Delete junction entries first
    conn.execute(
        "DELETE FROM project_memory WHERE project_id = ?1",
        rusqlite::params![id],
    )?;
    conn.execute("DELETE FROM projects WHERE id = ?1", rusqlite::params![id])?;
    Ok(())
}

/// Link a memory to a project.
pub fn link_project_memory(
    conn: &rusqlite::Connection,
    project_id: &str,
    memory_id: &str,
) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO project_memory (project_id, memory_id, created_at) VALUES (?1, ?2, datetime('now'))",
        rusqlite::params![project_id, memory_id],
    )?;
    Ok(())
}

/// Unlink a memory from a project.
pub fn unlink_project_memory(
    conn: &rusqlite::Connection,
    project_id: &str,
    memory_id: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM project_memory WHERE project_id = ?1 AND memory_id = ?2",
        rusqlite::params![project_id, memory_id],
    )?;
    Ok(())
}

/// Get all memory IDs associated with a project.
pub fn get_project_memory_ids(
    conn: &rusqlite::Connection,
    project_id: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT memory_id FROM project_memory WHERE project_id = ?1 ORDER BY created_at DESC",
    )?;
    let rows = stmt.query_map(rusqlite::params![project_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Convert a SQLite row into a Project struct.
fn row_to_project(row: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    use chrono::{DateTime, Utc};
    use std::path::PathBuf;

    let id_str: String = row.get(0)?;
    let name: String = row.get(1)?;
    let description: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
    let paths_str: String = row
        .get::<_, Option<String>>(3)?
        .unwrap_or_else(|| "[]".to_string());
    let tags_str: String = row
        .get::<_, Option<String>>(4)?
        .unwrap_or_else(|| "[]".to_string());
    let emoji: String = row
        .get::<_, Option<String>>(5)?
        .unwrap_or_else(|| "📦".to_string());
    let source_str: String = row
        .get::<_, Option<String>>(6)?
        .unwrap_or_else(|| "manual".to_string());
    let memory_visible: bool = row.get::<_, Option<i32>>(7)?.unwrap_or(1) != 0;
    let mount_ids_str: String = row
        .get::<_, Option<String>>(8)?
        .unwrap_or_else(|| "[]".to_string());
    let instructions: String = row.get::<_, Option<String>>(9)?.unwrap_or_default();
    let created_at: String = row.get(10)?;
    let updated_at: String = row.get(11)?;
    let last_active_at: String = row.get(12)?;

    let id = uuid::Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let paths: Vec<PathBuf> = serde_json::from_str(&paths_str).unwrap_or_default();
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
    let mount_ids: Vec<crate::mount::MountId> =
        serde_json::from_str(&mount_ids_str).unwrap_or_default();
    let source = match source_str.as_str() {
        "auto_detected" => ProjectSource::AutoDetected,
        _ => ProjectSource::Manual,
    };

    Ok(Project {
        id,
        name,
        description,
        paths,
        tags,
        emoji,
        source,
        memory_visible,
        mount_ids,
        instructions,
        created_at: created_at
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now()),
        updated_at: updated_at
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now()),
        last_active_at: last_active_at
            .parse::<DateTime<Utc>>()
            .unwrap_or_else(|_| Utc::now()),
    })
}
