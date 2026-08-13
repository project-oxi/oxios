//! MountManager: CRUD + detection for Mounts (RFC-025).
//!
//! Mirrors `ProjectManager`'s structure for consistency. Mounts are persisted
//! in the `mounts` SQLite table (same `memory.db`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;

use crate::kernel_db::KernelDatabase;

use super::mount_db;
use super::path_promotion;
use super::{DetectionResult, Mount, MountId, MountMeta, MountSource, detect_mounts};
use crate::event_bus::{EventBus, KernelEvent};

/// Errors from MountManager operations.
#[derive(thiserror::Error, Debug)]
pub enum MountManagerError {
    /// Mount not found.
    #[error("Mount not found: {0}")]
    NotFound(MountId),
    /// Mount name already taken.
    #[error("Mount name already exists: {0}")]
    DuplicateName(String),
    /// Invalid operation.
    #[error("Invalid operation: {0}")]
    Invalid(String),
}

/// Manages Mounts: CRUD, lookup, and detection.
///
/// Mounts are persisted in the `mounts` SQLite table
/// (same `memory.db` as memories and the legacy `projects` table).
pub struct MountManager {
    /// In-memory index of all Mounts (loaded at startup).
    mounts: RwLock<HashMap<MountId, Mount>>,
    /// Name → ID index for fast name lookup.
    name_index: RwLock<HashMap<String, MountId>>,
    /// RFC-025 Phase 5: roots the user has explicitly dismissed (Promo-3).
    ///
    /// When an `AutoPromoted` Mount is removed, its canonicalized root paths
    /// are recorded here (and in `mount_dismissals`) so the scanner never
    /// re-creates a Mount the user has rejected. Canonicalized form is used
    /// so that the comparison is path-stable across symlinks.
    dismissed_roots: RwLock<HashSet<PathBuf>>,
    /// SQLite database for persistence.
    db: Arc<KernelDatabase>,
    /// Event bus for publishing Mount events.
    event_bus: Option<EventBus>,
}

impl MountManager {
    /// Create a new MountManager, loading existing Mounts from SQLite.
    pub fn new(db: Arc<KernelDatabase>, event_bus: Option<EventBus>) -> Result<Self> {
        // Ensure the schema exists (idempotent).
        mount_db::ensure_mount_schema(&db.conn())?;

        let mut mounts = HashMap::new();
        let mut name_index = HashMap::new();
        for mount in mount_db::list_mounts(&db.conn())? {
            name_index.insert(mount.name.clone(), mount.id);
            mounts.insert(mount.id, mount);
        }

        // Promo-3: load dismissal tombstones so re-promoted mounts stay dead.
        let dismissed_roots = mount_db::list_dismissed_roots(&db.conn())?
            .into_iter()
            .collect::<HashSet<_>>();

        tracing::info!(
            count = mounts.len(),
            dismissed = dismissed_roots.len(),
            "MountManager initialized"
        );

        Ok(Self {
            mounts: RwLock::new(mounts),
            name_index: RwLock::new(name_index),
            dismissed_roots: RwLock::new(dismissed_roots),
            db,
            event_bus,
        })
    }

    /// List all Mounts.
    pub fn list_mounts(&self) -> Vec<Mount> {
        self.mounts.read().values().cloned().collect()
    }

    /// Get a Mount by ID.
    pub fn get_mount(&self, id: MountId) -> Option<Mount> {
        self.mounts.read().get(&id).cloned()
    }

    /// Get a Mount by name.
    pub fn get_mount_by_name(&self, name: &str) -> Option<Mount> {
        let name_index = self.name_index.read();
        let id = name_index.get(name)?;
        self.mounts.read().get(id).cloned()
    }

    /// Get several Mounts by ID, preserving the request order. Missing IDs
    /// are skipped (they may have been deleted).
    pub fn get_mounts_ordered(&self, ids: &[MountId]) -> Vec<Mount> {
        let mounts = self.mounts.read();
        ids.iter()
            .filter_map(|id| mounts.get(id).cloned())
            .collect()
    }

    /// Create a new Mount with the minimal RFC-025 input (name + paths).
    pub fn create_mount(
        &self,
        name: String,
        paths: Vec<PathBuf>,
        source: MountSource,
    ) -> Result<Mount> {
        let name = validate_mount_name(&name)?;
        if paths.is_empty() {
            return Err(MountManagerError::Invalid(
                "a Mount requires at least one path".to_string(),
            )
            .into());
        }
        // Reject overly broad system paths (lightweight safety, not a sandbox).
        for p in &paths {
            validate_mount_path(p)?;
        }

        let mut mount = Mount::new(&name, source);
        mount.paths = paths;

        // Hold the write locks across the uniqueness check, the DB write, and
        // the in-memory insert. The previous version used a *read* lock for the
        // name check and a *separate* write lock for the insert, leaving a
        // TOCTOU window in which two concurrent `create_mount` calls with the
        // same name both passed the check. Acquiring both locks in the
        // consistent order used throughout the manager (mounts → name_index)
        // closes the window entirely.
        let mut mounts = self.mounts.write();
        let mut name_index = self.name_index.write();
        if name_index.contains_key(&name) {
            return Err(MountManagerError::DuplicateName(name).into());
        }

        mount_db::save_mount(&self.db.conn(), &mount)?;

        name_index.insert(mount.name.clone(), mount.id);
        mounts.insert(mount.id, mount.clone());
        drop(name_index);
        drop(mounts);

        if let Some(ref event_bus) = self.event_bus {
            let _ = event_bus.publish(KernelEvent::ProjectCreated {
                // Reuse ProjectCreated for now; a MountCreated variant can be
                // added when the frontend needs to distinguish them.
                project_id: mount.id,
                name: mount.name.clone(),
                source: source.to_string(),
            });
        }

        tracing::info!(name = %mount.name, id = %mount.id, "Mount created");
        Ok(mount)
    }

    /// Update a Mount's auto-enriched fields (agent-driven, RFC-025 Phase 3).
    ///
    /// Only `auto_description` and `auto_meta` are writable here — `name` and
    /// `paths` are user-level and go through [`Self::update`].
    pub fn update_enrichment(
        &self,
        id: MountId,
        auto_description: Option<String>,
        auto_meta: Option<MountMeta>,
    ) -> Result<Mount> {
        let mut mounts = self.mounts.write();
        let mount = mounts.get_mut(&id).ok_or(MountManagerError::NotFound(id))?;

        if let Some(desc) = auto_description {
            // Bounded per RFC-025 cost guard (≤ 500 chars).
            mount.auto_description = desc.chars().take(500).collect();
        }
        if let Some(meta) = auto_meta {
            mount.auto_meta = meta;
        }
        mount.last_enriched_at = Some(Utc::now());
        mount.enrichment_pending = false;
        mount.updated_at = Utc::now();

        let mount_clone = mount.clone();
        drop(mounts);
        mount_db::save_mount(&self.db.conn(), &mount_clone)?;
        tracing::info!(name = %mount_clone.name, id = %id, "Mount enriched");
        Ok(mount_clone)
    }

    /// Update a Mount's user-level fields: name and/or paths.
    ///
    /// Replaces the former rename-only path. When `paths` changes, the cached
    /// enrichment (description, tech-stack, marker mtimes) is invalidated:
    /// `enrichment_pending` is set so rescan or agent enrichment re-seeds it.
    pub fn update(
        &self,
        id: MountId,
        name: Option<String>,
        paths: Option<Vec<PathBuf>>,
    ) -> Result<Mount> {
        // Validate before taking locks.
        let new_name = match &name {
            Some(n) => Some(validate_mount_name(n)?),
            None => None,
        };
        if let Some(new_paths) = &paths {
            if new_paths.is_empty() {
                return Err(MountManagerError::Invalid(
                    "a Mount requires at least one path".to_string(),
                )
                .into());
            }
            for p in new_paths {
                validate_mount_path(p)?;
            }
        }

        let mut mounts = self.mounts.write();
        let mut name_index = self.name_index.write();
        let mount = mounts.get_mut(&id).ok_or(MountManagerError::NotFound(id))?;
        let mut changed = false;

        if let Some(new_name) = new_name
            && new_name != mount.name
        {
            if name_index.contains_key(&new_name) {
                return Err(MountManagerError::DuplicateName(new_name).into());
            }
            name_index.remove(&mount.name);
            name_index.insert(new_name.clone(), id);
            mount.name = new_name;
            changed = true;
        }

        if let Some(new_paths) = paths {
            // A path change invalidates the cached enrichment: the description,
            // tech-stack tags, and marker mtimes all pointed at the old roots.
            if new_paths != mount.paths {
                mount.paths = new_paths;
                mount.last_marker_snapshot.clear();
                mount.enrichment_pending = true;
                changed = true;
            }
        }

        if changed {
            mount.updated_at = Utc::now();
            let mount_clone = mount.clone();
            drop(mounts);
            drop(name_index);
            mount_db::save_mount(&self.db.conn(), &mount_clone)?;
            tracing::info!(name = %mount_clone.name, id = %id, "Mount updated");
            return Ok(mount_clone);
        }
        // No-op: skip the DB write.
        let mount_clone = mount.clone();
        drop(mounts);
        drop(name_index);
        Ok(mount_clone)
    }

    /// Remove a Mount.
    ///
    /// DB-first ordering (matches `create_mount`): if the DB delete fails the
    /// in-memory state is left untouched so the caller can retry and the Mount
    /// doesn't silently reappear on restart.
    ///
    /// If the Mount was `AutoPromoted`, its canonicalized root paths are
    /// recorded as dismissals (tombstones) so the background scanner does
    /// not immediately re-promote them (Promo-3). Manual mounts are removed
    /// without recording a tombstone (the user may still want auto-promotion
    /// for that root).
    pub fn remove_mount(&self, id: MountId) -> Result<()> {
        // Preserve NotFound semantics + capture the Mount for tombstoning.
        let removed = {
            let mounts = self.mounts.read();
            mounts
                .get(&id)
                .cloned()
                .ok_or(MountManagerError::NotFound(id))?
        };
        // Delete from the DB before touching memory.
        mount_db::delete_mount(&self.db.conn(), &id.to_string())?;
        {
            let mut mounts = self.mounts.write();
            let mut name_index = self.name_index.write();
            if let Some(mount) = mounts.remove(&id) {
                name_index.remove(&mount.name);
            }
        }

        // Promo-3: tombstone auto-promoted roots so they aren't re-created.
        if removed.source == MountSource::AutoPromoted {
            self.record_dismissal(&removed.paths);
        }

        tracing::info!(id = %id, "Mount removed");
        Ok(())
    }

    /// Canonicalize each path and record it as a dismissed root, both
    /// in-memory and in SQLite (Promo-3). Best-effort: paths that fail to
    /// canonicalize are stored in their raw form so the tombstone still
    /// matches the exact string the scanner would normalize to.
    fn record_dismissal(&self, paths: &[PathBuf]) {
        let to_record: Vec<PathBuf> = paths
            .iter()
            .map(|p| Self::canonicalize_for_index(p))
            .collect();

        {
            let mut dismissed = self.dismissed_roots.write();
            for p in &to_record {
                dismissed.insert(p.clone());
            }
        }
        for p in &to_record {
            if let Err(e) = mount_db::add_dismissed_root(&self.db.conn(), p) {
                tracing::warn!(
                    path = %p.display(),
                    error = %e,
                    "failed to persist mount dismissal"
                );
            }
        }
        tracing::debug!(count = to_record.len(), "recorded mount dismissals");
    }

    /// Canonicalize a path for index comparison, falling back to the raw
    /// path when canonicalization fails (e.g. the path was removed).
    fn canonicalize_for_index(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    /// RFC-025 Phase 5: is `root` in the dismissed set (Promo-3)?
    ///
    /// Compares against both the canonicalized and raw forms of stored
    /// tombstones so that a root matches regardless of symlink resolution.
    fn is_dismissed(&self, root: &Path) -> bool {
        let dismissed = self.dismissed_roots.read();
        if dismissed.contains(root) {
            return true;
        }
        let canonical = Self::canonicalize_for_index(root);
        dismissed.contains(&canonical)
    }

    /// Record that a Mount was used in a session.
    pub fn touch(&self, id: MountId) {
        let to_save = {
            let mut mounts = self.mounts.write();
            if let Some(mount) = mounts.get_mut(&id) {
                mount.touch();
                Some(mount.clone())
            } else {
                None
            }
        };
        if let Some(mount) = to_save
            && let Err(e) = mount_db::save_mount(&self.db.conn(), &mount)
        {
            tracing::warn!(id = %id, error = %e, "touch: failed to save Mount");
        }
    }

    /// Try to detect a Mount from a user message.
    pub fn detect(&self, message: &str) -> DetectionResult {
        let mounts = self.list_mounts();
        detect_mounts(message, &mounts)
    }

    /// Seed `auto_meta` from the filesystem (RFC-025 §Auto-Meta).
    ///
    /// Cheap heuristic detection on marker files. The agent refines this
    /// during enrichment. Idempotent — safe to call multiple times.
    pub fn seed_auto_meta(&self, id: MountId) -> Result<()> {
        let mount = {
            let mounts = self.mounts.read();
            mounts
                .get(&id)
                .ok_or(MountManagerError::NotFound(id))?
                .clone()
        };
        let Some(primary) = mount.primary_path().cloned() else {
            return Ok(()); // nothing to scan
        };
        if !primary.exists() {
            tracing::debug!(path = %primary.display(), "Mount path missing, skip meta seed");
            return Ok(());
        }
        // detect_meta is cheap heuristics only — it must NOT clear the
        // enrichment nudge. Route it directly (not through update_enrichment,
        // which would stamp `last_enriched_at` and clear `enrichment_pending`),
        // so the agent is still prompted to do real enrichment.
        let meta = super::meta_detection::detect_meta(&primary);
        let to_save = {
            let mut mounts = self.mounts.write();
            let Some(mount) = mounts.get_mut(&id) else {
                return Ok(()); // removed while detecting
            };
            mount.auto_meta = meta;
            mount.enrichment_pending = true;
            mount.last_enriched_at = None;
            mount.updated_at = Utc::now();
            mount.clone()
        };
        if let Err(e) = mount_db::save_mount(&self.db.conn(), &to_save) {
            tracing::warn!(id = %id, error = %e, "seed_auto_meta: failed to save Mount");
        }
        tracing::info!(name = %to_save.name, id = %id, "Mount auto_meta seeded");
        Ok(())
    }

    /// Check marker-file drift and set `enrichment_pending` (RFC-025 §Enrichment).
    ///
    /// Compares current marker mtimes against the stored snapshot. Returns
    /// `true` if any marker drifted (and the flag was set). Cheap: a handful
    /// of `stat()` calls.
    pub fn check_drift(&self, id: MountId) -> Result<bool> {
        // Acquire a read lock only to clone the primary path and the current
        // snapshot, then drop it so the filesystem I/O (snapshot_markers) runs
        // lock-free (M8: don't do I/O under the write lock).
        let (primary, old_snapshot) = {
            let mounts = self.mounts.read();
            let mount = mounts.get(&id).ok_or(MountManagerError::NotFound(id))?;
            let Some(primary) = mount.primary_path().cloned() else {
                return Ok(false);
            };
            (primary, mount.last_marker_snapshot.clone())
        };

        // Filesystem I/O — no lock held.
        let current = super::meta_detection::snapshot_markers(&primary);
        let drifted = markers_drifted(&old_snapshot, &current);
        let current_map: HashMap<PathBuf, SystemTime> = current.into_iter().collect();

        // Re-acquire a write lock to apply results (re-checking the Mount
        // still exists — it may have been removed while we read the fs).
        let to_save = {
            let mut mounts = self.mounts.write();
            let Some(mount) = mounts.get_mut(&id) else {
                return Ok(drifted);
            };
            // Skip the mutation + DB write when nothing drifted and the
            // snapshot is unchanged (m4: don't write on every drift check).
            if !drifted && mount.last_marker_snapshot == current_map {
                None
            } else {
                if drifted {
                    mount.enrichment_pending = true;
                    mount.updated_at = Utc::now();
                }
                // Refresh the snapshot so the next comparison is accurate.
                mount.last_marker_snapshot = current_map;
                Some(mount.clone())
            }
        };

        if let Some(mount) = to_save
            && let Err(e) = mount_db::save_mount(&self.db.conn(), &mount)
        {
            tracing::warn!(id = %id, error = %e, "check_drift: failed to save Mount");
        }
        Ok(drifted)
    }

    /// Check drift for all Mounts (Dream-time refresh, RFC-025).
    ///
    /// Returns the IDs of Mounts whose content drifted.
    pub fn check_all_drift(&self) -> Vec<MountId> {
        let ids: Vec<MountId> = self.mounts.read().keys().copied().collect();
        let mut drifted = Vec::new();
        for id in ids {
            match self.check_drift(id) {
                Ok(true) => drifted.push(id),
                Ok(false) => {}
                Err(e) => tracing::warn!(error = %e, %id, "check_drift failed for mount"),
            }
        }
        drifted
    }

    /// RFC-025 Phase 5: scan session history and auto-create Mounts for paths
    /// that cross the frequency threshold.
    ///
    /// Returns the IDs of newly-created Mounts (empty if none promoted). Safe
    /// to call repeatedly — paths already covered by an existing Mount are
    /// skipped, as are name collisions.
    pub fn promote_frequent_paths(
        &self,
        sessions: &[crate::state_store::Session],
        config: &path_promotion::PromotionConfig,
    ) -> Vec<MountId> {
        if !config.enabled {
            return Vec::new();
        }

        let freqs = path_promotion::tally_frequencies(sessions, config);
        // Sort deterministically: most frequent first, then alphabetically by
        // path. This guarantees that when two roots derive the same name the
        // most-used one wins consistently across runs (HashMap iteration order
        // is otherwise non-deterministic).
        let mut sorted_freqs: Vec<_> = freqs.into_iter().collect();
        sorted_freqs.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(&b.0)));
        let mut created = Vec::new();

        for (root, freq) in sorted_freqs {
            if freq.count < config.threshold {
                continue;
            }
            // Skip if any existing Mount already covers this root.
            if self.root_already_covered(&root) {
                continue;
            }
            // Promo-3: skip roots the user has explicitly dismissed, so a
            // deleted AutoPromoted Mount is not immediately re-created.
            if self.is_dismissed(&root) {
                tracing::debug!(
                    path = %root.display(),
                    "auto-promotion skipped: root was dismissed"
                );
                continue;
            }
            // Derive a name from the final path component.
            let Some(name) = root
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
            else {
                continue;
            };
            // Skip if the name is already taken (collision → leave for the
            // user to resolve, rather than inventing "name-2").
            if self.get_mount_by_name(&name).is_some() {
                continue;
            }

            match self.create_mount(
                name.clone(),
                vec![root.clone()],
                super::MountSource::AutoPromoted,
            ) {
                Ok(mount) => {
                    tracing::info!(
                        name = %mount.name,
                        path = %root.display(),
                        count = freq.count,
                        "RFC-025: auto-promoted frequent path to Mount"
                    );
                    // Seed auto_meta immediately so the new Mount is useful.
                    let _ = self.seed_auto_meta(mount.id);
                    created.push(mount.id);
                }
                Err(e) => {
                    tracing::debug!(
                        path = %root.display(),
                        error = %e,
                        "auto-promotion skipped"
                    );
                }
            }
        }

        created
    }

    /// Returns the id of an existing Mount whose `paths` includes (or is an
    /// ancestor/descendant of) `root`, if any. Used by the RFC-025 migration
    /// to avoid creating a duplicate Mount for a path the user already
    /// registered under a different name.
    pub fn covering_mount_id(&self, root: &PathBuf) -> Option<MountId> {
        let mounts = self.mounts.read();
        mounts.values().find_map(|m| {
            m.paths
                .iter()
                .any(|p| root.starts_with(p) || p.starts_with(root))
                .then_some(m.id)
        })
    }

    /// Returns `true` if some existing Mount's `paths` already includes (or is
    /// an ancestor of) `root`, meaning the root is already covered.
    fn root_already_covered(&self, root: &PathBuf) -> bool {
        self.covering_mount_id(root).is_some()
    }
}

/// Compare a stored marker snapshot against the current state.
/// Returns `true` if any marker was added, removed, or changed mtime.
fn markers_drifted(
    stored: &HashMap<PathBuf, SystemTime>,
    current: &[(std::path::PathBuf, SystemTime)],
) -> bool {
    if stored.len() != current.len() {
        return true; // marker added or removed
    }
    for (path, mtime) in current {
        match stored.get(path) {
            Some(stored_time) if stored_time == mtime => continue,
            _ => return true, // new, removed, or changed
        }
    }
    false
}

/// Validate a Mount name (RFC-025): non-empty after trim, ≤ 64 chars (by char
/// count), no control characters. Returns the trimmed name on success.
fn validate_mount_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(MountManagerError::Invalid("Mount name must not be empty".to_string()).into());
    }
    if trimmed.chars().count() > 64 {
        return Err(MountManagerError::Invalid(
            "Mount name must be at most 64 characters".to_string(),
        )
        .into());
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(MountManagerError::Invalid(
            "Mount name must not contain control characters".to_string(),
        )
        .into());
    }
    Ok(trimmed.to_string())
}

/// Reject paths that are too broad to be a meaningful project root.
///
/// This is a lightweight safety check, not a full sandbox — AccessManager
/// RBAC enforcement is the real boundary. We only reject the filesystem root
/// and a few system directories that would make detection hijack every path.
fn validate_mount_path(path: &Path) -> Result<()> {
    let forbidden = [
        "", "/etc", "/dev", "/proc", "/sys", "/var", "/usr", "/bin", "/sbin", "/boot",
    ];
    let normalized = path.to_str().map(|s| s.trim_end_matches('/')).unwrap_or("");
    if forbidden.contains(&normalized) {
        return Err(MountManagerError::Invalid(format!(
            "Mount path '{}' is a system directory; refusing to create an overly broad Mount",
            path.display()
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_manager() -> MountManager {
        let db = Arc::new(KernelDatabase::open_in_memory().expect("db"));
        MountManager::new(db, None).expect("manager")
    }

    #[test]
    fn test_create_and_get() {
        let mgr = open_manager();
        let m = mgr
            .create_mount(
                "oxios".to_string(),
                vec![PathBuf::from("/Volumes/MERCURY/PROJECTS/oxios")],
                MountSource::Manual,
            )
            .expect("create");
        assert_eq!(mgr.get_mount(m.id).unwrap().name, "oxios");
        assert_eq!(mgr.get_mount_by_name("oxios").unwrap().id, m.id);
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let mgr = open_manager();
        mgr.create_mount(
            "oxios".to_string(),
            vec![PathBuf::from("/a")],
            MountSource::Manual,
        )
        .expect("first");
        let err = mgr
            .create_mount(
                "oxios".to_string(),
                vec![PathBuf::from("/b")],
                MountSource::Manual,
            )
            .unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_empty_paths_rejected() {
        let mgr = open_manager();
        let err = mgr
            .create_mount("x".to_string(), vec![], MountSource::Manual)
            .unwrap_err();
        assert!(err.to_string().contains("at least one path"));
    }

    #[test]
    fn test_system_directory_path_rejected() {
        let mgr = open_manager();
        for bad in ["/", "/etc", "/dev", "/proc", "/usr"] {
            let err = mgr
                .create_mount(
                    "bad".to_string(),
                    vec![PathBuf::from(bad)],
                    MountSource::Manual,
                )
                .unwrap_err();
            assert!(
                err.to_string().contains("system directory"),
                "expected system directory rejection for {bad}"
            );
        }
    }

    #[test]
    fn test_update_enrichment_bounds_description() {
        let mgr = open_manager();
        let m = mgr
            .create_mount(
                "oxios".to_string(),
                vec![PathBuf::from("/a")],
                MountSource::Manual,
            )
            .expect("create");
        let long = "x".repeat(800);
        let updated = mgr
            .update_enrichment(m.id, Some(long.clone()), None)
            .expect("update");
        assert_eq!(updated.auto_description.chars().count(), 500);
        assert!(updated.last_enriched_at.is_some());
        assert!(!updated.enrichment_pending);
    }

    #[test]
    fn test_update_renames_mount() {
        let mgr = open_manager();
        let m = mgr
            .create_mount(
                "oxios".to_string(),
                vec![PathBuf::from("/a")],
                MountSource::Manual,
            )
            .expect("create");
        let updated = mgr
            .update(m.id, Some("new-name".to_string()), None)
            .expect("rename");
        assert_eq!(updated.name, "new-name");
        // name_index follows the rename.
        assert!(mgr.get_mount_by_name("oxios").is_none());
        assert_eq!(mgr.get_mount_by_name("new-name").unwrap().id, m.id);
    }

    #[test]
    fn test_update_rejects_duplicate_name() {
        let mgr = open_manager();
        mgr.create_mount(
            "alpha".to_string(),
            vec![PathBuf::from("/a")],
            MountSource::Manual,
        )
        .expect("first");
        let b = mgr
            .create_mount(
                "beta".to_string(),
                vec![PathBuf::from("/b")],
                MountSource::Manual,
            )
            .expect("second");
        let err = mgr
            .update(b.id, Some("alpha".to_string()), None)
            .unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn test_update_paths_invalidates_enrichment() {
        let mgr = open_manager();
        let m = mgr
            .create_mount(
                "oxios".to_string(),
                vec![PathBuf::from("/old")],
                MountSource::Manual,
            )
            .expect("create");
        // Seed enrichment so we can observe the invalidation.
        let enriched = mgr
            .update_enrichment(m.id, Some("a rust project".to_string()), None)
            .expect("enrich");
        assert!(!enriched.enrichment_pending);
        // Seed a stale marker snapshot pointing at the old root.
        {
            let mut mounts = mgr.mounts.write();
            mounts
                .get_mut(&m.id)
                .unwrap()
                .last_marker_snapshot
                .insert(PathBuf::from("/old/Cargo.toml"), SystemTime::now());
        }
        // Repoint the mount at a new path.
        let updated = mgr
            .update(m.id, None, Some(vec![PathBuf::from("/new")]))
            .expect("update paths");
        assert_eq!(updated.paths, vec![PathBuf::from("/new")]);
        // The cached description/tech-stack/markers are now stale.
        assert!(updated.enrichment_pending);
        assert!(updated.last_marker_snapshot.is_empty());
    }

    #[test]
    fn test_update_no_op_preserves_enrichment() {
        let mgr = open_manager();
        let m = mgr
            .create_mount(
                "oxios".to_string(),
                vec![PathBuf::from("/a")],
                MountSource::Manual,
            )
            .expect("create");
        mgr.update_enrichment(m.id, Some("desc".to_string()), None)
            .expect("enrich");
        // Passing the unchanged name and paths must NOT mark enrichment stale.
        let updated = mgr
            .update(
                m.id,
                Some("oxios".to_string()),
                Some(vec![PathBuf::from("/a")]),
            )
            .expect("noop");
        assert!(!updated.enrichment_pending);
    }

    #[test]
    fn test_remove_mount() {
        let mgr = open_manager();
        let m = mgr
            .create_mount(
                "temp".to_string(),
                vec![PathBuf::from("/t")],
                MountSource::Manual,
            )
            .expect("create");
        mgr.remove_mount(m.id).expect("remove");
        assert!(mgr.get_mount(m.id).is_none());
        assert!(mgr.get_mount_by_name("temp").is_none());
    }

    #[test]
    fn test_get_mounts_ordered_skips_missing() {
        let mgr = open_manager();
        let m1 = mgr
            .create_mount(
                "a".to_string(),
                vec![PathBuf::from("/a")],
                MountSource::Manual,
            )
            .unwrap();
        let m2 = mgr
            .create_mount(
                "b".to_string(),
                vec![PathBuf::from("/b")],
                MountSource::Manual,
            )
            .unwrap();
        let missing = MountId::new_v4();
        let got = mgr.get_mounts_ordered(&[m1.id, missing, m2.id]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "a");
        assert_eq!(got[1].name, "b");
    }

    #[test]
    fn test_promote_frequent_paths_creates_mount() {
        use crate::state_store::{Session, UserMessage};
        use chrono::Utc;

        let mgr = open_manager();
        // Use this crate's own source dir — it has Cargo.toml at its root,
        // so normalize_to_root will collapse to the oxios-kernel root.
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let file = root.join("src/lib.rs");

        // Frequency is counted per distinct root per session (Promo-7): one
        // session's repeated mentions count once. So we need three separate
        // sessions to cross the default threshold of 3.
        let sessions: Vec<Session> = (0..3)
            .map(|_| {
                let mut session = Session::new("test");
                session.user_messages.push(UserMessage {
                    content: format!("fix {} please", file.display()),
                    timestamp: Utc::now(),
                });
                session
            })
            .collect();

        let config = path_promotion::PromotionConfig::default();
        let created = mgr.promote_frequent_paths(&sessions, &config);
        assert_eq!(created.len(), 1, "expected exactly one promoted Mount");

        let mount = mgr.get_mount(created[0]).expect("promoted mount exists");
        assert_eq!(mount.source, MountSource::AutoPromoted);
        assert_eq!(mount.name, "oxios-kernel");
        // auto_meta should be seeded (Cargo.toml → rust).
        assert!(mount.auto_meta.languages.contains(&"rust".to_string()));
    }

    /// Build `n` sessions each mentioning `root` once (Promo-7: frequency is
    /// per distinct root per session, so we vary the *session* count, not
    /// the message count within one session).
    fn sessions_mentioning(root: &Path, n: u32) -> Vec<crate::state_store::Session> {
        use crate::state_store::{Session, UserMessage};
        use chrono::Utc;
        (0..n)
            .map(|_| {
                let mut s = Session::new("test");
                s.user_messages.push(UserMessage {
                    content: format!("work on {}/src/lib.rs", root.display()),
                    timestamp: Utc::now(),
                });
                s
            })
            .collect()
    }

    #[test]
    fn test_promote_skips_already_covered_root() {
        let mgr = open_manager();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        // Pre-create a Mount covering this root.
        mgr.create_mount(
            "manual-kernel".to_string(),
            vec![root.clone()],
            MountSource::Manual,
        )
        .unwrap();

        // Promo-7: 3 separate sessions (count=3) cross the default threshold,
        // so this exercises the coverage-skip path rather than trivially
        // passing because the count is below threshold.
        let sessions = sessions_mentioning(&root, 3);
        let config = path_promotion::PromotionConfig::default();
        let created = mgr.promote_frequent_paths(&sessions, &config);
        assert!(
            created.is_empty(),
            "should not promote an already-covered root"
        );
    }

    #[test]
    fn test_promote_respects_threshold() {
        let mgr = open_manager();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        // Promo-7: 2 sessions → count=2, below the default threshold of 3.
        let sessions = sessions_mentioning(&root, 2);
        let config = path_promotion::PromotionConfig::default();
        let created = mgr.promote_frequent_paths(&sessions, &config);
        assert!(created.is_empty(), "should not promote below threshold");
    }

    #[test]
    fn test_promote_skips_dismissed_root() {
        // Promo-3: removing an AutoPromoted Mount must tombstone its root so
        // the scanner never re-creates it.
        let mgr = open_manager();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let sessions = sessions_mentioning(&root, 3);
        let config = path_promotion::PromotionConfig::default();

        // First scan: promotes the root to an AutoPromoted Mount.
        let created = mgr.promote_frequent_paths(&sessions, &config);
        assert_eq!(created.len(), 1, "expected exactly one promoted Mount");
        let promoted_id = created[0];
        assert_eq!(
            mgr.get_mount(promoted_id).unwrap().source,
            MountSource::AutoPromoted
        );

        // User dismisses it.
        mgr.remove_mount(promoted_id).expect("remove");
        assert!(mgr.get_mount(promoted_id).is_none());

        // Second scan with the same evidence must NOT re-create it.
        let recreated = mgr.promote_frequent_paths(&sessions, &config);
        assert!(
            recreated.is_empty(),
            "dismissed root must not be re-promoted (got {:?})",
            recreated
        );
    }

    #[test]
    fn test_dismissal_only_for_auto_promoted() {
        // Promo-3: dismissing a *Manual* Mount must not tombstone the root,
        // since the user may still want auto-promotion for it later.
        let mgr = open_manager();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

        // Manual mount.
        let m = mgr
            .create_mount(
                "manual-kernel".to_string(),
                vec![root.clone()],
                MountSource::Manual,
            )
            .unwrap();
        mgr.remove_mount(m.id).expect("remove manual");

        // Dismissed set should be empty — no tombstone for manual mounts.
        assert!(
            mgr.dismissed_roots.read().is_empty(),
            "manual removal must not tombstone"
        );

        // Subsequent promotion is still possible. Promo-7: 3 sessions.
        let sessions = sessions_mentioning(&root, 3);
        let config = path_promotion::PromotionConfig::default();
        let created = mgr.promote_frequent_paths(&sessions, &config);
        assert_eq!(
            created.len(),
            1,
            "promotion must still work after manual removal"
        );
    }
}
