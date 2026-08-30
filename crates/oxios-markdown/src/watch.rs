//! Vault watcher — keeps the knowledge index in sync with external edits.
//!
//! External editors (oximemo, Obsidian, vim) write directly into the
//! vault directory, bypassing [`KnowledgeBase`] methods entirely.
//! [`KnowledgeBase::watch`] runs a `notify` watcher on the vault root
//! and, after a per-path debounce settle window, re-reads settled files
//! into the backlink index and fires [`crate::knowledge::FileChange`]
//! callbacks so channels (e.g. the semantic index) stay current.
//!
//! Invariants:
//! - **Read-only with respect to the vault** — the watcher never writes,
//!   renames, or deletes the files it watches; it only re-reads them.
//! - **Never crashes on a bad file** — read failures (permissions,
//!   malformed content, races with a concurrent delete) are logged at
//!   `warn` and skipped, not propagated.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::knowledge::{FileChange, KnowledgeBase};

/// Commands consumed by the debounce thread.
enum Cmd {
    /// A filesystem event for a path under the vault root.
    Event(PathBuf),
    /// Stop the debounce loop; sent by [`WatchGuard::drop`].
    Shutdown,
}

/// Handle for a running vault watcher.
///
/// Dropping the guard stops the fs watcher and joins the debounce
/// thread. Events already settled may still fire callbacks before the
/// drop returns; nothing fires after it.
pub struct WatchGuard {
    tx: mpsc::Sender<Cmd>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        // A send error means the thread is already gone — nothing to stop.
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl KnowledgeBase {
    /// Watch the vault root for external changes and reindex settled notes.
    ///
    /// `settle` is the debounce window, parameterizable per caller: a
    /// path is processed only after no further events have been
    /// observed for it within the window (editors emit bursts of
    /// create/write/rename events per save). A settled path that exists
    /// is re-read into the backlink index and reported as
    /// [`FileChange::Updated`]; a settled path that is gone is dropped
    /// from the index and reported as [`FileChange::Deleted`].
    ///
    /// Self-writes by oxios itself also surface here and re-fire
    /// callbacks; that double-fire is tolerated at debug level and
    /// absorbed by downstream dedup (I-3).
    ///
    /// The returned [`WatchGuard`] keeps the watcher alive. This takes
    /// `&Arc<Self>` because the debounce thread owns a clone for the
    /// lifetime of the watch.
    pub fn watch(self: &Arc<Self>, settle: Duration) -> Result<WatchGuard> {
        if settle.is_zero() {
            anyhow::bail!("watch settle window must be non-zero");
        }
        let root = self.root();
        let (tx, rx) = mpsc::channel::<Cmd>();
        let event_tx = tx.clone();
        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
                match res {
                    Ok(ev) => {
                        for path in ev.paths {
                            // Send failure = guard dropped; drop the event.
                            let _ = event_tx.send(Cmd::Event(path));
                        }
                    }
                    Err(e) => tracing::debug!(error = %e, "watch: fs event error"),
                }
            })
            .context("create fs watcher")?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .with_context(|| format!("watch vault root {}", root.display()))?;

        let kb = Arc::clone(self);
        let handle = std::thread::Builder::new()
            .name("kb-watch".into())
            .spawn(move || debounce_loop(kb, root, watcher, rx, settle))
            .context("spawn watcher thread")?;
        Ok(WatchGuard {
            tx,
            handle: Some(handle),
        })
    }
}

/// Debounce loop: track the last event time per path
/// (`HashMap<PathBuf, Instant>`), and process a path once it has been
/// quiet for `settle`.
fn debounce_loop(
    kb: Arc<KnowledgeBase>,
    root: PathBuf,
    // Keep `watcher` alive for the life of the loop: dropping it stops fs events.
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<Cmd>,
    settle: Duration,
) {
    let poll = (settle / 4).max(Duration::from_millis(1));
    let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
    loop {
        match rx.recv_timeout(poll) {
            Ok(Cmd::Event(path)) => {
                pending.insert(path, Instant::now());
                // Drain any burst already queued behind the first event.
                while let Ok(cmd) = rx.try_recv() {
                    match cmd {
                        Cmd::Event(path) => {
                            pending.insert(path, Instant::now());
                        }
                        Cmd::Shutdown => return,
                    }
                }
            }
            Ok(Cmd::Shutdown) => return,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        let now = Instant::now();
        let due: Vec<PathBuf> = pending
            .iter()
            .filter(|(_, seen)| now.duration_since(**seen) >= settle)
            .map(|(path, _)| path.clone())
            .collect();
        for path in due {
            pending.remove(&path);
            handle_settled(&kb, &root, &path);
        }
    }
}

/// Re-read one settled path into the index and notify callbacks.
///
/// Failures (unreadable file, containment rejection) are warned and
/// skipped — the watcher never panics and never exits on a bad file.
fn handle_settled(kb: &KnowledgeBase, root: &Path, path: &Path) {
    let Some(rel) = rel_path(root, path) else {
        tracing::debug!(path = ?path, "watch: event outside vault root; ignored");
        return;
    };
    if path.extension().is_none_or(|ext| ext != "md") {
        tracing::debug!(path = %rel, "watch: non-markdown path; ignored");
        return;
    }
    if path.exists() {
        tracing::debug!(path = %rel, "watch: reindexing externally changed note");
        match kb.reindex_one(&rel) {
            Ok(()) => kb.notify_change(&rel, FileChange::Updated(rel.clone())),
            Err(e) => tracing::warn!(path = %rel, error = %e, "watch: reindex failed; skipped"),
        }
    } else {
        tracing::debug!(path = %rel, "watch: externally deleted note");
        kb.forget_file(&rel);
        kb.notify_change(&rel, FileChange::Deleted(rel.clone()));
    }
}

/// Convert an absolute event path to a vault-relative POSIX path.
///
/// FSEvents (macOS) canonicalizes paths (`/var` → `/private/var`), so a
/// plain `strip_prefix` against the watch root can miss; retry through
/// the canonicalized parent directory before giving up.
fn rel_path(root: &Path, path: &Path) -> Option<String> {
    if let Ok(rel) = path.strip_prefix(root) {
        return Some(to_posix(rel));
    }
    let parent = path.parent()?.canonicalize().ok()?;
    let name = path.file_name()?;
    let canon_root = root.canonicalize().ok()?;
    parent
        .join(name)
        .strip_prefix(canon_root)
        .ok()
        .map(to_posix)
}

/// Render a relative path with `/` separators (KB path convention).
fn to_posix(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use crate::knowledge::{FileChange, KnowledgeBase};

    /// Poll `cond` every 25 ms until it returns true or 5 s elapse
    /// (generous timeout: fs event delivery is inherently timing-based).
    fn wait_until<F: Fn() -> bool>(cond: F) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    }

    #[test]
    fn external_write_refreshes_index_and_fires_callbacks() {
        let dir = std::env::temp_dir().join(format!("test-watch-{}", uuid::Uuid::new_v4()));
        let kb = Arc::new(KnowledgeBase::new(dir.clone()).unwrap());
        kb.note_write("Target.md", "# Target").unwrap();
        kb.index_all().unwrap();

        let deleted = Arc::new(AtomicUsize::new(0));
        let d = deleted.clone();
        kb.on_file_change(move |_path, change| {
            if matches!(change, FileChange::Deleted(_)) {
                d.fetch_add(1, Ordering::SeqCst);
            }
        });

        let guard = kb.watch(Duration::from_millis(50)).unwrap();
        std::fs::write(
            dir.join("ext.md"),
            "---\nid: e\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\n---\n[[Target]]",
        )
        .unwrap();
        assert!(
            wait_until(|| !kb.backlinks_for("Target.md").is_empty()),
            "external write never reindexed"
        );
        std::fs::remove_file(dir.join("ext.md")).unwrap();
        assert!(
            wait_until(|| deleted.load(Ordering::SeqCst) > 0),
            "external delete never notified"
        );
        assert!(
            wait_until(|| kb.backlinks_for("Target.md").is_empty()),
            "deleted note never dropped from the index"
        );
        drop(guard); // watcher stops, debounce thread joins
    }
}
