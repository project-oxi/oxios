//! Mount SQLite persistence (RFC-025).
//!
//! The `mounts` table lives in the same `memory.db` as memories and the
//! legacy `projects` table. Coexists with RFC-011's `projects` table during
//! the migration window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Result;

use super::{Mount, MountMeta, MountSource};

/// Schema DDL for the `mounts` table.
pub const MOUNT_SCHEMA: &str = r#"
-- ─────────────────────────────────────────────
-- Mounts (RFC-025) — path aliases
-- ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS mounts (
    id                     TEXT PRIMARY KEY,
    name                   TEXT NOT NULL UNIQUE,
    paths                  TEXT NOT NULL,            -- JSON array of path strings
    auto_description       TEXT NOT NULL DEFAULT '',
    auto_meta              TEXT NOT NULL DEFAULT '{}', -- JSON MountMeta
    source                 TEXT NOT NULL DEFAULT 'manual',
    last_marker_snapshot   TEXT NOT NULL DEFAULT '{}', -- JSON {path_str: rfc3339_or_secs}
    enrichment_pending     INTEGER NOT NULL DEFAULT 0,
    last_enriched_at       TEXT,
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL,
    last_active_at         TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mounts_name ON mounts(name);

-- ─────────────────────────────────────────────
-- Mount dismissals (RFC-025 Phase 5) — tombstones for deleted
-- AutoPromoted Mounts, so the scanner does not re-create them.
-- ─────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS mount_dismissals (
    root_path TEXT PRIMARY KEY
);
"#;

/// Ensure the `mounts` table exists.
pub fn ensure_mount_schema(conn: &rusqlite::Connection) -> Result<()> {
    conn.execute_batch(MOUNT_SCHEMA)?;
    Ok(())
}

/// Save (upsert) a Mount.
pub fn save_mount(conn: &rusqlite::Connection, mount: &Mount) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO mounts
         (id, name, paths, auto_description, auto_meta, source,
          last_marker_snapshot, enrichment_pending, last_enriched_at,
          created_at, updated_at, last_active_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            mount.id.to_string(),
            mount.name,
            serde_json::to_string(&mount.paths)?,
            mount.auto_description,
            serde_json::to_string(&mount.auto_meta)?,
            mount.source.to_string(),
            serde_json::to_string(&serialize_snapshot(&mount.last_marker_snapshot))?,
            mount.enrichment_pending as i32,
            mount.last_enriched_at.map(|t| t.to_rfc3339()),
            mount.created_at.to_rfc3339(),
            mount.updated_at.to_rfc3339(),
            mount.last_active_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// List all Mounts, ordered by name.
pub fn list_mounts(conn: &rusqlite::Connection) -> Result<Vec<Mount>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, paths, auto_description, auto_meta, source,
                last_marker_snapshot, enrichment_pending, last_enriched_at,
                created_at, updated_at, last_active_at
         FROM mounts ORDER BY name",
    )?;
    let rows = stmt.query_map([], row_to_mount)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// Delete a Mount by ID.
pub fn delete_mount(conn: &rusqlite::Connection, id: &str) -> Result<()> {
    conn.execute("DELETE FROM mounts WHERE id = ?1", rusqlite::params![id])?;
    Ok(())
}

/// RFC-025 Phase 5: load all dismissed root paths (tombstones).
///
/// These are roots the user explicitly removed after the scanner
/// auto-promoted them. The scanner skips them so it never re-creates a
/// Mount the user has rejected (Promo-3).
pub fn list_dismissed_roots(conn: &rusqlite::Connection) -> Result<Vec<PathBuf>> {
    let mut stmt = conn.prepare("SELECT root_path FROM mount_dismissals")?;
    let rows = stmt.query_map([], |row| {
        let s: String = row.get(0)?;
        Ok(PathBuf::from(s))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// RFC-025 Phase 5: record a dismissed root path (tombstone).
pub fn add_dismissed_root(conn: &rusqlite::Connection, root: &Path) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO mount_dismissals (root_path) VALUES (?1)",
        rusqlite::params![root.to_string_lossy()],
    )?;
    Ok(())
}

/// Convert a SQLite row into a [`Mount`].
fn row_to_mount(row: &rusqlite::Row<'_>) -> rusqlite::Result<Mount> {
    use chrono::{DateTime, Utc};

    let id_str: String = row.get(0)?;
    let name: String = row.get(1)?;
    let paths_str: String = row
        .get::<_, Option<String>>(2)?
        .unwrap_or_else(|| "[]".to_string());
    let auto_description: String = row.get::<_, Option<String>>(3)?.unwrap_or_default();
    let auto_meta_str: String = row
        .get::<_, Option<String>>(4)?
        .unwrap_or_else(|| "{}".to_string());
    let source_str: String = row
        .get::<_, Option<String>>(5)?
        .unwrap_or_else(|| "manual".to_string());
    let snapshot_str: String = row
        .get::<_, Option<String>>(6)?
        .unwrap_or_else(|| "{}".to_string());
    let enrichment_pending: bool = row.get::<_, Option<i32>>(7)?.unwrap_or(0) != 0;
    let last_enriched_str: Option<String> = row.get(8)?;
    let created_at: String = row.get(9)?;
    let updated_at: String = row.get(10)?;
    let last_active_at: String = row.get(11)?;

    let id = uuid::Uuid::parse_str(&id_str).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let paths: Vec<PathBuf> = serde_json::from_str(&paths_str).unwrap_or_default();
    let auto_meta: MountMeta = serde_json::from_str(&auto_meta_str).unwrap_or_default();
    let last_marker_snapshot = deserialize_snapshot(&snapshot_str);
    let source = match source_str.as_str() {
        "auto_detected" => MountSource::AutoDetected,
        "auto_promoted" => MountSource::AutoPromoted,
        _ => MountSource::Manual,
    };
    let last_enriched_at = last_enriched_str
        .as_deref()
        .and_then(|s| s.parse::<DateTime<Utc>>().ok());

    Ok(Mount {
        id,
        name,
        paths,
        auto_description,
        auto_meta,
        source,
        last_marker_snapshot,
        enrichment_pending,
        last_enriched_at,
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

// ── SystemTime snapshot (de)serialization helpers ──────────────────────
//
// SystemTime isn't directly JSON-serializable in a stable way, so we store
// the snapshot as {path_string: seconds_since_epoch}. This is enough for
// drift comparison (we only need to detect change, not exact timestamps).

fn serialize_snapshot(snap: &HashMap<PathBuf, SystemTime>) -> HashMap<String, u64> {
    snap.iter()
        .map(|(k, v)| {
            let secs = v
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (k.to_string_lossy().to_string(), secs)
        })
        .collect()
}

fn deserialize_snapshot(json: &str) -> HashMap<PathBuf, SystemTime> {
    let map: HashMap<String, u64> = serde_json::from_str(json).unwrap_or_default();
    map.into_iter()
        .map(|(k, secs)| {
            let time = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
            (PathBuf::from(k), time)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_db::KernelDatabase;

    fn open_db() -> KernelDatabase {
        let db = KernelDatabase::open_in_memory().expect("open db");
        ensure_mount_schema(&db.conn()).expect("schema");
        db
    }

    #[test]
    fn test_save_and_list_mount() {
        let db = open_db();
        let mut m =
            Mount::from_name_and_path("oxios", PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios"));
        m.auto_description = "Agent OS".to_string();
        m.auto_meta.summary = "Rust agent OS".to_string();
        m.auto_meta.languages = vec!["rust".to_string()];
        save_mount(&db.conn(), &m).expect("save");

        let listed = list_mounts(&db.conn()).expect("list");
        assert_eq!(listed.len(), 1);
        let got = &listed[0];
        assert_eq!(got.name, "oxios");
        assert_eq!(got.auto_description, "Agent OS");
        assert_eq!(got.auto_meta.languages, vec!["rust".to_string()]);
        assert!(!got.enrichment_pending);
    }

    #[test]
    fn test_delete_mount() {
        let db = open_db();
        let m = Mount::from_name_and_path("temp", PathBuf::from("/tmp"));
        save_mount(&db.conn(), &m).expect("save");
        assert_eq!(list_mounts(&db.conn()).unwrap().len(), 1);
        delete_mount(&db.conn(), &m.id.to_string()).expect("delete");
        assert_eq!(list_mounts(&db.conn()).unwrap().len(), 0);
    }

    #[test]
    fn test_upsert_replaces() {
        let db = open_db();
        let mut m = Mount::from_name_and_path("oxios", PathBuf::from("/a"));
        save_mount(&db.conn(), &m).expect("save");
        m.auto_description = "updated".to_string();
        save_mount(&db.conn(), &m).expect("upsert");
        let listed = list_mounts(&db.conn()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].auto_description, "updated");
    }

    #[test]
    fn test_marker_snapshot_roundtrip() {
        let db = open_db();
        let mut m = Mount::from_name_and_path("oxios", PathBuf::from("/a"));
        m.last_marker_snapshot.insert(
            PathBuf::from("/a/Cargo.toml"),
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
        );
        save_mount(&db.conn(), &m).expect("save");
        let got = list_mounts(&db.conn()).unwrap().pop().unwrap();
        let stored = got
            .last_marker_snapshot
            .get(&PathBuf::from("/a/Cargo.toml"))
            .expect("snapshot present");
        let secs = stored
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(secs, 1_700_000_000);
    }

    #[test]
    fn test_dismissed_roots_roundtrip() {
        let db = open_db();
        // Starts empty.
        assert!(list_dismissed_roots(&db.conn()).unwrap().is_empty());
        // Insert two tombstones.
        add_dismissed_root(&db.conn(), &PathBuf::from("/proj/a")).expect("add");
        add_dismissed_root(&db.conn(), &PathBuf::from("/proj/b")).expect("add");
        // Idempotent — re-adding the same root is a no-op.
        add_dismissed_root(&db.conn(), &PathBuf::from("/proj/a")).expect("re-add");
        let roots = list_dismissed_roots(&db.conn()).unwrap();
        assert_eq!(roots.len(), 2);
        assert!(roots.contains(&PathBuf::from("/proj/a")));
        assert!(roots.contains(&PathBuf::from("/proj/b")));
    }
}
