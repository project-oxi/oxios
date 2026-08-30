//! Journaled, resumable migration of the legacy oxios home into the
//! canonical unified home.
//!
//! The unified-home layout moves oxios state from the pre-unification
//! `$HOME/.oxios` to `oxi_home()/oxios`. This module implements the
//! COPY-ONLY migration: the source is never deleted or modified — the
//! removal is deferred to a later cutover release.
//!
//! * [`preflight`] computes the plan without touching anything:
//!   source, destination, file count, total bytes, and a
//!   [`MigrationState`].
//! * [`migrate`] runs (or resumes) the migration. A JSON journal at
//!   `oxi_home()/oxios.migration-journal.json` is written atomically
//!   (temp + fsync + rename) BEFORE the first mutation and flipped to
//!   `complete` after the verify walk passes, so an interrupted run
//!   resumes by rerunning.
//! * Per-file copies are idempotent: a destination file with matching
//!   size + SHA-256 is skipped; anything else is copied to
//!   `<dest>.part-<pid>`, fsynced, and renamed into place.
//! * [`MigrationState::Conflict`] — destination exists and at least
//!   one source file differs (size, SHA-256, or missing) — aborts
//!   without touching either side. The one exception is an in-progress
//!   journal naming the same source and destination: those differences
//!   belong to an interrupted run of THIS migration, so the run is
//!   classified `Ready` and resumes.
//! * An explicit `OXIOS_HOME` never merges with the legacy home: under
//!   the override the migration is a no-op.
//!
//! All logic lives in path-injected `*_in` / `*_for` functions so
//! tests exercise real tempdir trees without process-env mutation.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Journal file name, anchored at [`crate::oxi_home::oxi_home`].
pub const JOURNAL_FILE_NAME: &str = "oxios.migration-journal.json";

/// Journal schema version.
const JOURNAL_VERSION: u32 = 1;

/// Chunk size for streaming SHA-256.
const HASH_CHUNK: usize = 64 * 1024;

/// Lifecycle of a migration as computed by [`preflight`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationState {
    /// No legacy home (or an explicit `OXIOS_HOME`, which never merges).
    NothingToDo,
    /// Legacy home present and the destination is either absent (fresh
    /// copy) or a matching in-progress journal owns the differences
    /// (resume).
    Ready,
    /// Destination exists and every source file is already identical
    /// (size + SHA-256) — nothing to copy.
    AlreadyMigrated,
    /// Destination exists and at least one source file differs without
    /// an owning journal — abort; neither side is touched.
    Conflict,
}

impl MigrationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            MigrationState::NothingToDo => "nothing_to_do",
            MigrationState::Ready => "ready",
            MigrationState::AlreadyMigrated => "already_migrated",
            MigrationState::Conflict => "conflict",
        }
    }
}

impl std::fmt::Display for MigrationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The migration plan, from [`preflight`].
#[derive(Debug, Clone)]
pub struct PreflightPlan {
    pub source: PathBuf,
    pub destination: PathBuf,
    /// Migratable entries: regular files + symlinks (runtime sockets,
    /// FIFOs, and devices are skipped — they are recreated by the
    /// daemon, never migrated).
    pub file_count: usize,
    pub total_bytes: u64,
    pub state: MigrationState,
    /// Source-relative paths whose destination counterpart is missing
    /// or differs. Empty unless `state == Conflict`.
    pub conflicts: Vec<PathBuf>,
}

/// Journal status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalStatus {
    InProgress,
    Complete,
}

/// The on-disk migration journal (contract schema v1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationJournal {
    pub version: u32,
    pub source: String,
    pub destination: String,
    pub status: JournalStatus,
    /// Unix seconds when the migration started.
    pub started_at: u64,
}

/// Outcome of [`migrate`].
#[derive(Debug, Clone)]
pub struct RunReport {
    pub plan: PreflightPlan,
    pub files_copied: usize,
    pub files_skipped: usize,
    /// A journal was written by this run (in_progress or complete).
    pub journal_written: bool,
    /// A previous interrupted run was detected and resumed.
    pub resumed: bool,
}

// ─── Environment-resolved entry points ───────────────────────────────

/// `(source, destination)` when a migration may run: `None` under an
/// explicit `OXIOS_HOME` (portable deployments never merge with the
/// legacy home) or when no legacy home exists.
fn migration_paths_for(
    oxios_override: Option<&OsStr>,
    legacy: Option<&Path>,
    canonical: &Path,
) -> Option<(PathBuf, PathBuf)> {
    if oxios_override.is_some() {
        return None;
    }
    let legacy = legacy?;
    Some((legacy.to_path_buf(), canonical.to_path_buf()))
}

fn migration_paths() -> Option<(PathBuf, PathBuf)> {
    migration_paths_for(
        std::env::var_os("OXIOS_HOME").as_deref(),
        crate::oxi_home::legacy_home_dir().as_deref(),
        &crate::oxi_home::oxios_home(),
    )
}

/// Path of the migration journal (contract: anchored at `oxi_home()`).
pub fn journal_path() -> PathBuf {
    crate::oxi_home::oxi_home().join(JOURNAL_FILE_NAME)
}

/// Compute the migration plan without touching anything.
pub fn preflight() -> PreflightPlan {
    match migration_paths() {
        Some((source, destination)) => preflight_in(&source, &destination),
        None => PreflightPlan {
            source: std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".oxios"))
                .unwrap_or_else(|| PathBuf::from("~/.oxios")),
            destination: crate::oxi_home::oxios_home(),
            file_count: 0,
            total_bytes: 0,
            state: MigrationState::NothingToDo,
            conflicts: Vec::new(),
        },
    }
}

/// Run (or resume) the migration. `dry_run` computes and returns the
/// plan without mutating anything.
pub fn migrate(dry_run: bool) -> Result<RunReport> {
    match migration_paths() {
        Some((source, destination)) => migrate_in(&source, &destination, &journal_path(), dry_run),
        None => Ok(RunReport {
            plan: preflight(),
            files_copied: 0,
            files_skipped: 0,
            journal_written: false,
            resumed: false,
        }),
    }
}

// ─── Pure, path-injected core ────────────────────────────────────────

/// A migratable entry.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryKind {
    File {
        size: u64,
    },
    /// Unix symlink, recreated (not followed) at the destination —
    /// managed-launcher relative symlinks must survive byte-identical.
    Symlink {
        target: PathBuf,
    },
}

/// Deterministic-order walk of `root`, returning `(relative, kind)`
/// for regular files and symlinks. Sockets / FIFOs / devices are
/// skipped (runtime state, never migrated).
fn walk(root: &Path) -> std::io::Result<Vec<(PathBuf, EntryKind)>> {
    let mut out = Vec::new();
    walk_into(root, root, &mut out)?;
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn walk_into(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, EntryKind)>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
        let meta = fs::symlink_metadata(&path)?;
        if meta.is_symlink() {
            #[cfg(unix)]
            out.push((
                rel,
                EntryKind::Symlink {
                    target: fs::read_link(&path)?,
                },
            ));
            #[cfg(not(unix))]
            let _ = rel;
        } else if meta.is_dir() {
            walk_into(root, &path, out)?;
        } else if meta.is_file() {
            out.push((rel, EntryKind::File { size: meta.len() }));
        }
        // Sockets, FIFOs, devices: skipped by design.
    }
    Ok(())
}

/// Streaming SHA-256 of a file.
fn sha256_file(path: &Path) -> std::io::Result<[u8; 32]> {
    use sha2::Digest;
    let mut file = fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    let mut buf = vec![0u8; HASH_CHUNK];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().into())
}

/// True iff the destination counterpart of `rel` exists and is
/// identical to the source entry (regular file: size + SHA-256;
/// symlink: link target).
fn entries_match(source_root: &Path, dest_root: &Path, rel: &Path, kind: &EntryKind) -> bool {
    let src = source_root.join(rel);
    let dst = dest_root.join(rel);
    match kind {
        EntryKind::File { size } => {
            let (Ok(_sm), Ok(dm)) = (fs::symlink_metadata(&src), fs::symlink_metadata(&dst)) else {
                return false;
            };
            if dm.is_symlink() || !dm.is_file() || dm.len() != *size {
                return false;
            }
            match (sha256_file(&src), sha256_file(&dst)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            }
        }
        EntryKind::Symlink { target } => {
            #[cfg(unix)]
            {
                fs::read_link(&dst).is_ok_and(|t| t == *target)
            }
            #[cfg(not(unix))]
            {
                let _ = target;
                false
            }
        }
    }
}

/// Compute the migration plan. Journal-aware: an in-progress journal
/// naming this source and destination reclassifies destination
/// differences from `Conflict` to `Ready` (resume).
pub fn preflight_in(source: &Path, destination: &Path) -> PreflightPlan {
    preflight_with_journal(source, destination, read_journal(&journal_path()).as_ref())
}

fn preflight_with_journal(
    source: &Path,
    destination: &Path,
    journal: Option<&MigrationJournal>,
) -> PreflightPlan {
    let plan = PreflightPlan {
        source: source.to_path_buf(),
        destination: destination.to_path_buf(),
        file_count: 0,
        total_bytes: 0,
        state: MigrationState::NothingToDo,
        conflicts: Vec::new(),
    };
    if !source.is_dir() {
        return plan;
    }
    let entries = match walk(source) {
        Ok(entries) => entries,
        // An unreadable source is not a safe migration source.
        Err(_) => return plan,
    };
    let file_count = entries.len();
    let total_bytes = entries
        .iter()
        .map(|(_, kind)| match kind {
            EntryKind::File { size } => *size,
            EntryKind::Symlink { .. } => 0,
        })
        .sum();
    let plan = PreflightPlan {
        file_count,
        total_bytes,
        ..plan
    };
    if !destination.exists() {
        return PreflightPlan {
            state: MigrationState::Ready,
            ..plan
        };
    }
    let conflicts: Vec<PathBuf> = entries
        .iter()
        .filter(|(rel, kind)| !entries_match(source, destination, rel, kind))
        .map(|(rel, _)| rel.clone())
        .collect();
    let state = if conflicts.is_empty() {
        MigrationState::AlreadyMigrated
    } else if journal_owns(journal, source, destination) {
        // The differences belong to an interrupted run of THIS
        // migration — resumable, not a conflict.
        MigrationState::Ready
    } else {
        MigrationState::Conflict
    };
    PreflightPlan {
        state,
        conflicts,
        ..plan
    }
}

/// True iff `journal` is in-progress and names exactly this migration.
fn journal_owns(journal: Option<&MigrationJournal>, source: &Path, destination: &Path) -> bool {
    journal.is_some_and(|j| {
        j.status == JournalStatus::InProgress
            && j.source == source.to_string_lossy()
            && j.destination == destination.to_string_lossy()
    })
}

pub fn read_journal(path: &Path) -> Option<MigrationJournal> {
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `<dest>.part-<pid>` staging path for atomic copies.
fn part_path(dest: &Path, pid: u32) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(format!(".part-{pid}"));
    PathBuf::from(s)
}

/// Atomic journal write: temp file + fsync + rename.
fn write_journal(path: &Path, journal: &MigrationJournal) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let data = serde_json::to_string_pretty(journal).context("serializing migration journal")?;
    let tmp = part_path(path, std::process::id());
    {
        let mut f =
            fs::File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        f.write_all(data.as_bytes())
            .and_then(|()| f.sync_all())
            .with_context(|| format!("writing {}", tmp.display()))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("publishing {}", path.display()))?;
    Ok(())
}

/// Copy one entry into place via `<dest>.part-<pid>` + fsync + rename.
fn copy_entry(
    source: &Path,
    destination: &Path,
    rel: &Path,
    kind: &EntryKind,
    pid: u32,
) -> Result<()> {
    let src = source.join(rel);
    let dst = destination.join(rel);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let part = part_path(&dst, pid);
    match kind {
        EntryKind::File { .. } => {
            let mut from =
                fs::File::open(&src).with_context(|| format!("reading {}", src.display()))?;
            let mut to =
                fs::File::create(&part).with_context(|| format!("staging {}", part.display()))?;
            std::io::copy(&mut from, &mut to)
                .with_context(|| format!("copying {} → {}", src.display(), dst.display()))?;
            to.sync_all()
                .with_context(|| format!("fsync {}", part.display()))?;
            drop(to);
            fs::rename(&part, &dst).with_context(|| format!("publishing {}", dst.display()))?;
        }
        EntryKind::Symlink { target } => {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(target, &part)
                    .with_context(|| format!("staging symlink {}", part.display()))?;
                fs::rename(&part, &dst)
                    .with_context(|| format!("publishing symlink {}", dst.display()))?;
            }
            #[cfg(not(unix))]
            {
                let _ = (&part, target);
                anyhow::bail!("symlink migration is unsupported on this platform");
            }
        }
    }
    Ok(())
}

/// Run (or resume) the migration between explicit paths. `dry_run`
/// mutates nothing: no journal, no directories, no copies.
pub fn migrate_in(
    source: &Path,
    destination: &Path,
    journal_path: &Path,
    dry_run: bool,
) -> Result<RunReport> {
    let journal = read_journal(journal_path);
    let plan = preflight_with_journal(source, destination, journal.as_ref());
    let noop = RunReport {
        plan: plan.clone(),
        files_copied: 0,
        files_skipped: 0,
        journal_written: false,
        resumed: false,
    };
    match plan.state {
        // Nothing to do — and a conflict must abort reporting both
        // paths without touching anything.
        MigrationState::NothingToDo | MigrationState::Conflict => Ok(noop),
        MigrationState::AlreadyMigrated => {
            // A previous run may have died between the last copy and
            // the journal flip — mark it complete. A tree that never
            // had a journal is left untouched.
            if !dry_run
                && journal
                    .as_ref()
                    .is_some_and(|j| j.status == JournalStatus::InProgress)
            {
                write_journal(
                    journal_path,
                    &MigrationJournal {
                        version: JOURNAL_VERSION,
                        source: source.to_string_lossy().into_owned(),
                        destination: destination.to_string_lossy().into_owned(),
                        status: JournalStatus::Complete,
                        started_at: journal
                            .as_ref()
                            .map(|j| j.started_at)
                            .unwrap_or_else(unix_now),
                    },
                )?;
                return Ok(RunReport {
                    journal_written: true,
                    ..noop
                });
            }
            Ok(noop)
        }
        MigrationState::Ready => {
            if dry_run {
                return Ok(noop);
            }
            let resumed = journal_owns(journal.as_ref(), source, destination);
            if !resumed {
                // The journal lands BEFORE the first mutation so any
                // crash leaves a resumable record on disk.
                write_journal(
                    journal_path,
                    &MigrationJournal {
                        version: JOURNAL_VERSION,
                        source: source.to_string_lossy().into_owned(),
                        destination: destination.to_string_lossy().into_owned(),
                        status: JournalStatus::InProgress,
                        started_at: journal
                            .as_ref()
                            .map(|j| j.started_at)
                            .unwrap_or_else(unix_now),
                    },
                )?;
            }
            let pid = std::process::id();
            let mut copied = 0usize;
            let mut skipped = 0usize;
            for (rel, kind) in walk(source).context("walking legacy home")? {
                if entries_match(source, destination, &rel, &kind) {
                    skipped += 1;
                    continue;
                }
                copy_entry(source, destination, &rel, &kind, pid)
                    .with_context(|| format!("migrating {}", destination.join(&rel).display()))?;
                copied += 1;
            }

            // Verify walk: every source entry must now match.
            let mismatched: Vec<PathBuf> = walk(source)
                .context("verifying migrated home")?
                .into_iter()
                .filter(|(rel, kind)| !entries_match(source, destination, rel, kind))
                .map(|(rel, _)| rel)
                .collect();
            anyhow::ensure!(
                mismatched.is_empty(),
                "migration verify failed for {} ent{} (journal left in_progress; rerun `oxios migrate` to resume): {}",
                mismatched.len(),
                if mismatched.len() == 1 { "y" } else { "ies" },
                mismatched
                    .iter()
                    .take(5)
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            write_journal(
                journal_path,
                &MigrationJournal {
                    version: JOURNAL_VERSION,
                    source: source.to_string_lossy().into_owned(),
                    destination: destination.to_string_lossy().into_owned(),
                    status: JournalStatus::Complete,
                    started_at: journal
                        .as_ref()
                        .map(|j| j.started_at)
                        .unwrap_or_else(unix_now),
                },
            )?;
            // The verify walk passed — reflect the post-run state in
            // the reported plan (a fresh preflight would now return
            // AlreadyMigrated).
            let plan = PreflightPlan {
                state: MigrationState::AlreadyMigrated,
                conflicts: Vec::new(),
                ..plan
            };
            Ok(RunReport {
                plan,
                files_copied: copied,
                files_skipped: skipped,
                journal_written: true,
                resumed,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(&format!("oxios-home-migrate-{name}-"))
            .tempdir()
            .expect("tempdir")
    }

    fn write_file(path: &Path, contents: &[u8]) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }

    /// Full content snapshot of a tree (symlinks read as empty), used
    /// for before/after purity assertions.
    fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        walk(root)
            .expect("walk")
            .into_iter()
            .map(|(rel, _)| {
                let data = fs::read(root.join(&rel)).unwrap_or_default();
                (rel, data)
            })
            .collect()
    }

    fn sample_source(source: &Path) {
        write_file(&source.join("config.toml"), b"[kernel]\nmax_agents = 10\n");
        write_file(&source.join("workspace/sessions/a.json"), b"{\"a\":1}");
        write_file(&source.join("logs/oxios.log"), b"log line\n");
        #[cfg(unix)]
        {
            std::fs::create_dir_all(source.join("bin")).expect("mkdir bin");
            // Managed-launcher style RELATIVE symlink — must be
            // recreated, not followed.
            std::os::unix::fs::symlink(
                Path::new("../versions/1.0/oxios"),
                source.join("bin/oxios"),
            )
            .expect("symlink");
        }
    }

    fn fixture(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf, PathBuf) {
        let tmp = tmp(name);
        let source = tmp.path().join("home/.oxios");
        let destination = tmp.path().join("home/.oxi/oxios");
        let journal = tmp.path().join("home/.oxi").join(JOURNAL_FILE_NAME);
        sample_source(&source);
        (tmp, source, destination, journal)
    }

    fn file_count() -> usize {
        #[cfg(unix)]
        {
            4
        }
        #[cfg(not(unix))]
        {
            3
        }
    }

    #[test]
    fn migrate_copies_tree_completes_journal_and_preserves_source() {
        let (_tmp, source, destination, journal) = fixture("success");

        let report = migrate_in(&source, &destination, &journal, false).expect("migrate");
        assert_eq!(report.plan.state, MigrationState::AlreadyMigrated);
        assert_eq!(report.plan.file_count, file_count());
        assert_eq!(report.files_copied, file_count());
        assert_eq!(report.files_skipped, 0);
        assert!(!report.resumed);

        // Destination content matches byte-for-byte.
        assert_eq!(
            fs::read(destination.join("config.toml")).expect("read"),
            b"[kernel]\nmax_agents = 10\n"
        );
        assert_eq!(
            fs::read(destination.join("workspace/sessions/a.json")).expect("read"),
            b"{\"a\":1}"
        );
        #[cfg(unix)]
        assert_eq!(
            fs::read_link(destination.join("bin/oxios")).expect("readlink"),
            Path::new("../versions/1.0/oxios"),
            "relative symlinks must be recreated, not dereferenced"
        );

        // Journal complete with the contract schema.
        let j = read_journal(&journal).expect("journal written");
        assert_eq!(j.version, 1);
        assert_eq!(j.status, JournalStatus::Complete);
        assert_eq!(j.source, source.to_string_lossy());
        assert_eq!(j.destination, destination.to_string_lossy());

        // Source preserved: identical snapshot before/after.
        let source_entries = walk(&source).expect("walk source");
        assert_eq!(source_entries.len(), file_count());
        for (rel, kind) in &source_entries {
            assert!(entries_match(&source, &source, rel, kind));
        }

        // No leftover staging files at the destination.
        for (rel, _) in walk(&destination).expect("walk dest") {
            assert!(
                !rel.to_string_lossy().contains(".part-"),
                "leftover staging file: {rel:?}"
            );
        }
    }

    #[test]
    fn rerun_resumes_after_partial_run() {
        let (_tmp, source, destination, journal) = fixture("resume");

        // First run completes…
        migrate_in(&source, &destination, &journal, false).expect("first run");
        // …then simulate a crash mid-migration: one destination file
        // lost, another truncated, journal flipped back to in_progress.
        fs::remove_file(destination.join("workspace/sessions/a.json")).expect("rm");
        write_file(&destination.join("logs/oxios.log"), b"trunc");
        let mut j = read_journal(&journal).expect("journal");
        j.status = JournalStatus::InProgress;
        let started_at = j.started_at;
        write_journal(&journal, &j).expect("rewrite journal");

        let report = migrate_in(&source, &destination, &journal, false).expect("resume run");
        assert!(report.resumed, "matching in-progress journal must resume");
        assert_eq!(report.files_copied, 2);
        assert_eq!(report.plan.state, MigrationState::AlreadyMigrated);
        let j = read_journal(&journal).expect("journal");
        assert_eq!(j.status, JournalStatus::Complete);
        assert_eq!(j.started_at, started_at, "resume preserves started_at");

        // Repaired byte-for-byte.
        assert_eq!(
            fs::read(destination.join("workspace/sessions/a.json")).expect("read"),
            b"{\"a\":1}"
        );
        assert_eq!(
            fs::read(destination.join("logs/oxios.log")).expect("read"),
            b"log line\n"
        );
    }

    #[test]
    fn conflict_aborts_and_touches_nothing() {
        let (_tmp, source, destination, journal) = fixture("conflict");

        // Destination exists with a DIFFERING file and no journal —
        // two diverged homes; the migration must not touch anything.
        write_file(&destination.join("config.toml"), b"different");
        let source_before = snapshot(&source);
        let destination_before = snapshot(&destination);

        let report = migrate_in(&source, &destination, &journal, false).expect("plan");
        assert_eq!(report.plan.state, MigrationState::Conflict);
        // The differing config.toml AND every source file missing from
        // the diverged destination are conflicts.
        assert!(
            report
                .plan
                .conflicts
                .contains(&PathBuf::from("config.toml"))
        );
        assert_eq!(report.plan.conflicts.len(), file_count());
        assert_eq!(report.files_copied, 0);
        assert!(!report.journal_written);

        assert_eq!(snapshot(&source), source_before, "source untouched");
        assert_eq!(
            snapshot(&destination),
            destination_before,
            "destination untouched"
        );
        assert!(!journal.exists(), "conflict must not write a journal");
    }

    #[test]
    fn destination_missing_a_source_file_is_conflict() {
        let (_tmp, source, destination, journal) = fixture("missing");

        // Destination exists but only carries a subset of the source
        // (e.g. the new app ran first) — a missing counterpart counts
        // as "differs" per the contract.
        write_file(
            &destination.join("config.toml"),
            b"[kernel]\nmax_agents = 10\n",
        );

        let report = migrate_in(&source, &destination, &journal, false).expect("plan");
        assert_eq!(report.plan.state, MigrationState::Conflict);
        assert!(
            report
                .plan
                .conflicts
                .contains(&PathBuf::from("workspace/sessions/a.json")),
            "missing destination file must be reported: {:?}",
            report.plan.conflicts
        );
        assert_eq!(report.files_copied, 0);
    }

    #[test]
    fn dry_run_mutates_nothing() {
        let (_tmp, source, destination, journal) = fixture("dry-run");

        let before = snapshot(&source);
        let report = migrate_in(&source, &destination, &journal, true).expect("dry run");

        assert_eq!(report.plan.state, MigrationState::Ready);
        assert_eq!(report.plan.file_count, file_count());
        assert!(report.plan.total_bytes > 0);
        assert_eq!(report.files_copied, 0);
        assert!(!report.journal_written);

        assert_eq!(snapshot(&source), before, "source untouched");
        assert!(!destination.exists(), "destination must not be created");
        assert!(!journal.exists(), "dry run must not write a journal");
    }

    #[test]
    fn rerun_after_complete_is_a_noop() {
        let (_tmp, source, destination, journal) = fixture("idempotent");

        migrate_in(&source, &destination, &journal, false).expect("first run");
        let j = read_journal(&journal).expect("journal");

        let report = migrate_in(&source, &destination, &journal, false).expect("rerun");
        assert_eq!(report.plan.state, MigrationState::AlreadyMigrated);
        assert_eq!(report.files_copied, 0);
        assert!(!report.journal_written, "complete journal left untouched");
        assert_eq!(read_journal(&journal).expect("journal"), j);
    }

    #[test]
    fn in_progress_journal_flips_to_complete_without_recopying() {
        let (_tmp, source, destination, journal) = fixture("flip");

        // First run completes the copy (journal = complete)…
        migrate_in(&source, &destination, &journal, false).expect("first run");
        // …then rewind the journal to in_progress — the
        // crashed-between-last-copy-and-journal-flip window. All files
        // are already identical.
        let mut seeded = read_journal(&journal).expect("journal");
        seeded.status = JournalStatus::InProgress;
        seeded.started_at = 42;
        write_journal(&journal, &seeded).expect("seed journal");

        let report = migrate_in(&source, &destination, &journal, false).expect("run");
        assert_eq!(report.plan.state, MigrationState::AlreadyMigrated);
        assert!(report.journal_written, "journal must flip to complete");
        assert_eq!(report.files_copied, 0, "everything already identical");
        assert_eq!(report.files_skipped, 0, "flip path copies nothing");
        let flipped = read_journal(&journal).expect("journal");
        assert_eq!(flipped.status, JournalStatus::Complete);
        assert_eq!(flipped.started_at, 42, "flip preserves started_at");
    }

    #[test]
    fn journal_is_written_before_first_mutation() {
        // Prove the ordering contract: when the FIRST copy fails, the
        // journal is already on disk as in_progress, so a rerun can
        // resume. Failure is forced via a read-protected destination
        // parent; skipped where permission bits are not enforced
        // (e.g. running as root).
        let tmp = tmp("journal-first");
        let source = tmp.path().join("home/.oxios");
        let destination = tmp.path().join("home/.oxi/oxios");
        // The journal sits at oxi_home() in production (a SIBLING of
        // the destination), so the test mirrors that: it must stay
        // writable while the destination parent is read-protected.
        let journal = tmp.path().join(JOURNAL_FILE_NAME);
        sample_source(&source);

        let dest_parent = destination.parent().expect("parent").to_path_buf();
        std::fs::create_dir_all(&dest_parent).expect("mkdir dest parent");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let probe = tmp.path().join("perm-probe");
            std::fs::write(&probe, b"x").expect("probe");
            std::fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).expect("chmod");
            let enforced = fs::File::open(&probe).is_err();
            let _ = std::fs::remove_file(&probe);
            if !enforced {
                eprintln!("skipping: permission bits not enforced (root?)");
                return;
            }

            std::fs::set_permissions(&dest_parent, fs::Permissions::from_mode(0o555))
                .expect("chmod dest parent");
            let result = migrate_in(&source, &destination, &journal, false);
            std::fs::set_permissions(&dest_parent, fs::Permissions::from_mode(0o755))
                .expect("restore dest parent");

            let err = result.expect_err("copy into read-only parent must fail after journal write");
            assert!(
                err.to_string().contains("migrating"),
                "failure must surface the migrating-entry context: {err:#}"
            );
            let j =
                read_journal(&journal).expect("journal must be written BEFORE the first mutation");
            assert_eq!(j.status, JournalStatus::InProgress);
            // And the interrupted run is resumable by rerunning.
            let resume = migrate_in(&source, &destination, &journal, false).expect("resume");
            assert!(resume.resumed);
            assert_eq!(resume.plan.state, MigrationState::AlreadyMigrated);
        }
    }

    #[test]
    fn explicit_oxios_home_never_merges_with_legacy() {
        let tmp = tmp("override");
        let legacy = tmp.path().join("home/.oxios");
        let canonical = tmp.path().join("portable-oxios");
        std::fs::create_dir_all(&legacy).expect("mkdir legacy");

        assert_eq!(
            migration_paths_for(Some(OsStr::new("/portable")), Some(&legacy), &canonical),
            None,
            "explicit OXIOS_HOME disables legacy merging entirely"
        );
        assert_eq!(
            migration_paths_for(None, Some(&legacy), &canonical),
            Some((legacy.clone(), canonical.clone()))
        );
        assert_eq!(migration_paths_for(None, None, &canonical), None);
    }

    #[test]
    fn nothing_to_do_when_source_absent() {
        let tmp = tmp("absent");
        let source = tmp.path().join("home/.oxios");
        let destination = tmp.path().join("home/.oxi/oxios");
        let journal = tmp.path().join("home/.oxi").join(JOURNAL_FILE_NAME);

        let report = migrate_in(&source, &destination, &journal, false).expect("plan");
        assert_eq!(report.plan.state, MigrationState::NothingToDo);
        assert!(!journal.exists());
        assert!(!destination.exists());
    }

    #[test]
    fn destination_only_files_do_not_block_completion() {
        let (_tmp, source, destination, journal) = fixture("dest-only");

        // Fresh copy, then the new app writes its own state into the
        // destination (logs, pid, …). Source files still all match, so
        // the migration stays complete — destination-only files are
        // new-app state, not conflicts.
        migrate_in(&source, &destination, &journal, false).expect("first run");
        write_file(&destination.join("state/kernel.db"), b"new app state");

        let report = migrate_in(&source, &destination, &journal, false).expect("rerun");
        assert_eq!(report.plan.state, MigrationState::AlreadyMigrated);
        assert_eq!(report.plan.conflicts, Vec::<PathBuf>::new());
    }
}
