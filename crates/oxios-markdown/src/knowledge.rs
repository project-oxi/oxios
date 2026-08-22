//! KnowledgeBase — markdown knowledge base application layer.
//!
//! Integrates `VirtualFs`, `BacklinkIndex`, and all app-layer features
//! (chat, journal, habits, checklist, etc.) into a single struct.
//!
//! **No kernel dependencies. No AI dependencies.**
//! This crate can be used standalone by any channel (web, CLI, etc.)
//! without going through the kernel.

use std::collections::HashSet;
use std::path::PathBuf;

use anyhow::Result;
use parking_lot::{Mutex as ParkingMutex, RwLock};

/// Callback type for file change notifications.
/// Used by [`KnowledgeLens`] to keep the semantic index in sync.
pub type FileChangeCallback = Box<dyn Fn(&str, FileChange) + Send + Sync>;

use crate::backlinks::{Backlink, BacklinkIndex, LinkGraph};
use crate::chat::{delete_chat_msg, move_from_chat, read_chat_msgs, rename_chat_msg};
use crate::checklist::{
    add_checklist_item, checklist_items, complete_checklist_item, incomplete_checklist_items,
    remove_checklist_item, remove_completed_checklist_items,
};
use crate::fs::VirtualFs;
use crate::habits::{habits, last_week_habits, write_habits};
use crate::html::markdown_to_html;
use crate::i18n::emoji_for;
use crate::journal::{add_emoji as journal_add_emoji, add_record as journal_add_record};
use crate::parser::{
    StemIndex, extract_headings, rewrite_link_targets, rewrite_wikilink_targets, similar,
};
use crate::plugins::world_clock_for_names;
use crate::stats::{done_today, today_report};
use crate::types::NoteMeta;
use crate::types::{CHAT_FILENAME, DIR_USER_ROOT, FileEntry, Habits, KnowledgeConfig};
#[cfg(test)]
use crate::types::{NoteQuality, NoteSource};
use crate::worker::{move_due_tasks, remove_completed_items};
use crate::{today_chat_header, today_journal_filename};

/// File change event emitted via `on_file_change` callbacks.
#[derive(Debug, Clone)]
pub enum FileChange {
    /// A new file was created.
    Created(String),
    /// An existing file was updated.
    Updated(String),
    /// A file was deleted.
    Deleted(String),
    /// A file was moved or renamed.
    Moved {
        /// Original path before the move.
        old: String,
        /// New path after the move.
        new: String,
    },
}

/// Knowledge search hit (file-name based).
#[derive(Debug, Clone)]
pub struct NoteHit {
    /// File path relative to knowledge root.
    pub path: String,
    /// Display name of the file.
    pub name: String,
    /// Content snippet.
    pub snippet: String,
    /// Number of backlinks pointing to this note.
    pub backlink_count: usize,
    /// Name similarity score (0–100).
    pub name_similarity: i32,
}

/// Markdown knowledge base application layer.
///
/// Wraps [`VirtualFs`] for sandboxed file I/O, [`BacklinkIndex`] for
/// link tracking, and provides all app-layer features (chat, journal,
/// habits, checklist, etc.).
///
/// **No kernel dependencies.** Can be used standalone by any channel.
pub struct KnowledgeBase {
    /// Sandboxed filesystem.
    fs: RwLock<VirtualFs>,
    /// Bidirectional link index.
    backlinks: RwLock<BacklinkIndex>,
    /// Files written by agents (not by the user).
    agent_writes: ParkingMutex<HashSet<String>>,
    /// Callbacks invoked on file changes.
    /// Used by [`KnowledgeLens`] to keep semantic index in sync.
    on_change: RwLock<Vec<FileChangeCallback>>,
}

impl std::fmt::Debug for KnowledgeBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnowledgeBase")
            .field("root", &self.fs.read().root())
            .finish()
    }
}

impl KnowledgeBase {
    /// Create a new KnowledgeBase for the given root directory.
    pub fn new(root: PathBuf) -> Result<Self> {
        let fs = VirtualFs::new(root)?;
        Ok(Self {
            fs: RwLock::new(fs),
            backlinks: RwLock::new(BacklinkIndex::new()),
            agent_writes: ParkingMutex::new(HashSet::new()),
            on_change: RwLock::new(Vec::new()),
        })
    }

    /// Create a new KnowledgeBase scoped to a Space's subdirectory.
    pub fn for_space(space_dir: &std::path::Path) -> Result<Self> {
        Self::new(space_dir.join("knowledge"))
    }

    /// Get the root path of the knowledge base.
    pub fn root(&self) -> PathBuf {
        self.fs.read().root().to_path_buf()
    }

    /// Register a callback to be invoked on every file change.
    ///
    /// The callback receives `(path, FileChange)`.
    /// Multiple callbacks can be registered.
    pub fn on_file_change<F>(&self, f: F)
    where
        F: Fn(&str, FileChange) + Send + Sync + 'static,
    {
        self.on_change.write().push(Box::new(f));
    }

    /// Emit file change notifications to all registered callbacks.
    fn notify_change(&self, path: &str, change: FileChange) {
        for cb in self.on_change.read().iter() {
            cb(path, change.clone());
        }
    }

    // ── File I/O ───────────────────────────────────────────────────

    /// Read a note's content.
    pub fn note_read(&self, path: &str) -> Result<Option<String>> {
        let fs = self.fs.read();
        match fs.read_path(path) {
            Ok(content) => Ok(Some(content)),
            Err(_) => Ok(None),
        }
    }

    /// Read a note's raw bytes — for binary assets (images, etc.) that aren't
    /// valid UTF-8. Text notes should use [`note_read`].
    pub fn note_read_bytes(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let fs = self.fs.read();
        match fs.read_path_bytes(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(_) => Ok(None),
        }
    }

    /// Build a lowercase-stem → paths[] index over every `.md` file in the KB.
    ///
    /// Used by `resolve_wikilink` to canonicalize `[[bare-stem]]` targets
    /// during indexing and (transitively) to decide which wikilinks are
    /// safe to rewrite on rename. Walks the whole tree; cheap for the
    /// documented personal-KB scale (hundreds of files).
    ///
    /// MUST be called BEFORE acquiring the backlinks write lock — it takes
    /// the fs read lock, and we never nest the two.
    fn build_stem_index(&self) -> StemIndex {
        let mut index: StemIndex = StemIndex::new();
        let files = match self.list_all_md_files() {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(error = %e, "stem_index walk failed; wikilinks stay unresolved");
                return index;
            }
        };
        for (path, _size) in files {
            let stem = match path.rsplit('/').next() {
                Some(b) => b.trim_end_matches(".md"),
                None => path.as_str().trim_end_matches(".md"),
            }
            .to_lowercase();
            index.entry(stem).or_default().push(path);
        }
        index
    }
    /// Write a note — creates or overwrites.
    ///
    /// Writes the `.md` file via VirtualFs, updates the backlink index,
    /// and notifies registered `on_file_change` callbacks.
    pub fn note_write(&self, path: &str, content: &str) -> Result<()> {
        // Hold the write lock across the read-check + write so concurrent
        // writers cannot interleave their write_all calls (F1). Drop the
        // lock before notifying callbacks to avoid reentrancy deadlocks.
        let is_new = {
            let fs = self.fs.write();
            let is_new = fs.read_path(path).is_err();
            fs.write_path(path, content)?;
            is_new
        };
        // Build the stem index BEFORE taking the backlinks write lock
        // (fs read lock nests under nothing here).
        let stem_index = self.build_stem_index();
        {
            let mut backlinks = self.backlinks.write();
            backlinks.remove_file(path);
            backlinks.index_file_with(path, content, &stem_index);
        }

        self.notify_change(
            path,
            if is_new {
                FileChange::Created(path.to_string())
            } else {
                FileChange::Updated(path.to_string())
            },
        );
        Ok(())
    }

    /// Write a note with provenance metadata (RFC-022).
    ///
    /// Prepends a YAML frontmatter block with `oxios:` metadata,
    /// then delegates to `note_write`. If the file already has an
    /// `oxios:` frontmatter block, it is merged (preserving `saved_at`,
    /// updating `quality`/`source`). If the file has non-Oxios
    /// frontmatter (e.g., Obsidian tags), it is left intact and
    /// the note is treated as user-authored — no metadata is added.
    pub fn note_write_with_meta(&self, path: &str, content: &str, meta: &NoteMeta) -> Result<bool> {
        // Check existing content for frontmatter
        let existing = self.note_read(path).ok().flatten();
        let final_content = match existing {
            Some(ref existing_content) => {
                let (existing_meta, body) = parse_note_meta(existing_content);
                match existing_meta {
                    // Has Oxios frontmatter — merge
                    Some(old_meta) => {
                        let merged = NoteMeta {
                            saved_at: old_meta.saved_at.or(meta.saved_at.clone()),
                            ..meta.clone()
                        };
                        format_frontmatter(&merged, if body.is_empty() { content } else { &body })
                    }
                    // No Oxios frontmatter — user-authored or foreign frontmatter.
                    // Don't touch it. Return Ok without writing.
                    None => {
                        tracing::debug!(
                            path,
                            "Skipping note_write_with_meta on user-authored note"
                        );
                        return Ok(false);
                    }
                }
            }
            None => format_frontmatter(meta, content),
        };
        self.note_write(path, &final_content).map(|_| true)
    }

    /// List notes that need Dream review (RFC-022).
    ///
    /// Scans the vault for `.md` files with `needs_review: true` in their
    /// Oxios frontmatter. Reads only the frontmatter block (stops at the
    /// closing `---`) for efficiency.
    pub fn notes_needing_review(&self) -> Result<Vec<(String, NoteMeta)>> {
        let fs = self.fs.read();
        let mut result = Vec::new();

        let files = fs.all_md_files()?;
        for (path, _size) in &files {
            if let Ok(content) = fs.read_path(path) {
                let (meta, _body) = parse_note_meta(&content);
                if let Some(m) = meta
                    && m.needs_review
                {
                    result.push((path.clone(), m));
                }
            }
        }

        // Oldest first — they've been raw the longest
        result.sort_by(|a, b| {
            a.1.saved_at
                .as_deref()
                .unwrap_or("")
                .cmp(b.1.saved_at.as_deref().unwrap_or(""))
        });

        Ok(result)
    }
    /// Delete the note at `path`, removing it from the filesystem and
    /// dropping any recorded backlinks for that file.
    pub fn note_delete(&self, path: &str) -> Result<()> {
        {
            let fs = self.fs.write();
            fs.delete_path(path)?;
        }
        self.backlinks.write().remove_file(path);
        self.notify_change(path, FileChange::Deleted(path.to_string()));
        Ok(())
    }

    /// Restore a note's content without triggering file-change callbacks.
    ///
    /// Used when reverting to a previous git version — writes the file
    /// and updates the backlink index, but does **not** fire `on_file_change`
    /// callbacks. This prevents an infinite loop where restore → write →
    /// callback → git commit → ... repeats.
    pub fn note_restore(&self, path: &str, content: &str) -> Result<()> {
        {
            let fs = self.fs.write();
            fs.write_path(path, content)?;
        }
        let stem_index = self.build_stem_index();
        let mut backlinks = self.backlinks.write();
        backlinks.remove_file(path);
        backlinks.index_file_with(path, content, &stem_index);
        // Intentionally skip notify_change()
        Ok(())
    }
    /// Move/rename a note.
    ///
    /// In addition to the filesystem rename and backlink reindex, this
    /// rewrites every `[text](old_path)]` reference in **other** notes
    /// (and any self-reference in the moved note) to point at `new_path`,
    /// AND every `[[target]]` wikilink that resolves to old_path (with
    /// ambiguity guard for bare stems). Without this, renaming a note
    /// that other notes link to would silently orphan those links — a
    /// latent bug that affected both the F2 sidebar rename and the
    /// H1-driven rename.
    pub fn note_move(&self, old_path: &str, new_path: &str) -> Result<()> {
        // 0. Build the stem index BEFORE renaming. The bare-stem ambiguity
        //    check in `rewrite_wikilink_targets` needs old_path still
        //    present in the tree; after the rename, old_path is gone and
        //    the stem count would undercount. This is the rewrite-time
        //    index; step 5 builds a second (post-rename) one for reindex.
        let pre_stem_index = self.build_stem_index();

        // 1. Rename on disk + read the moved file's content under the fs lock.
        let new_content = {
            let fs = self.fs.write();
            fs.rename_path(old_path, new_path)?;
            fs.read_path(new_path).ok()
        };

        // 2. Snapshot the set of files that link to old_path BEFORE we
        //    tear down the index entry. Done under a read lock; the
        //    actual rewrites happen outside the lock to keep the critical
        //    section short.
        let sources: HashSet<String> = {
            let backlinks = self.backlinks.read();
            backlinks.sources_for(old_path)
        };

        // 3. Rewrite self-references in the moved note (a note can link
        //    to itself by its old name). This is what gets indexed and
        //    persisted.
        let indexed_content = match &new_content {
            Some(c) => {
                let (md_done, _) = rewrite_link_targets(c, old_path, new_path);
                let (wiki_done, _) =
                    rewrite_wikilink_targets(&md_done, old_path, new_path, Some(&pre_stem_index));
                if &wiki_done != c {
                    // Persist the self-reference fix.
                    let _ = self.fs.write().write_path(new_path, &wiki_done);
                }
                wiki_done
            }
            None => String::new(),
        };

        // 4. Rewrite references in every other note that linked to the
        //    old path. Collect (path, new_content) pairs to write + reindex.
        let mut touched: Vec<(String, String)> = Vec::with_capacity(sources.len());
        for src in &sources {
            if src == old_path || src == new_path {
                // Self-links already handled above; skip the moved file.
                continue;
            }
            if let Ok(content) = self.fs.read().read_path(src) {
                let (md_done, n_md) = rewrite_link_targets(&content, old_path, new_path);
                let (final_done, n_wiki) =
                    rewrite_wikilink_targets(&md_done, old_path, new_path, Some(&pre_stem_index));
                if (n_md > 0 || n_wiki > 0) && final_done != content {
                    touched.push((src.clone(), final_done));
                }
            }
        }

        // 5. Apply reindex: drop old, index new (with rewritten content),
        //    and reindex every touched source. Build the post-rename stem
        //    index so wikilinks in the reindexed notes re-resolve against
        //    the now-current tree.
        let post_stem_index = self.build_stem_index();
        {
            let mut backlinks = self.backlinks.write();
            backlinks.remove_file(old_path);
            if !indexed_content.is_empty() {
                backlinks.index_file_with(new_path, &indexed_content, &post_stem_index);
            }
            for (src, content) in &touched {
                backlinks.index_file_with(src, content, &post_stem_index);
            }
        }

        // 6. Persist the rewritten sources. Done AFTER reindexing so a
        //    crash between write and reindex leaves the index pointing at
        //    the on-disk content (idempotent on next scan).
        if !touched.is_empty() {
            let fs = self.fs.write();
            for (src, content) in &touched {
                let _ = fs.write_path(src, content);
            }
        }

        self.notify_change(
            old_path,
            FileChange::Moved {
                old: old_path.to_string(),
                new: new_path.to_string(),
            },
        );
        Ok(())
    }

    /// List notes in a directory.
    pub fn note_tree(&self, dir: &str) -> Result<Vec<FileEntry>> {
        let fs = self.fs.read();
        let dir = if dir.is_empty() || dir == "/" {
            DIR_USER_ROOT
        } else {
            dir
        };
        Ok(fs.files_and_dirs(dir)?)
    }

    /// List all markdown files in the knowledge base (path, size).
    /// Used by startup git reconciliation to detect post-crash drift.
    pub fn list_all_md_files(&self) -> Result<Vec<(String, i64)>> {
        let fs = self.fs.read();
        Ok(fs.all_md_files()?)
    }

    // ── Search (file-name based only) ────────────────────────────

    /// Search notes by file name fuzzy matching.
    ///
    /// **Note:** Semantic search is handled by `KnowledgeLens`,
    /// not by this method.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<NoteHit>> {
        let fs = self.fs.read();
        let files = fs.search_files_by_name(query)?;

        let hits: Vec<NoteHit> = files
            .into_iter()
            .take(limit)
            .map(|f| {
                let path = if f.parent_dir == DIR_USER_ROOT || f.parent_dir == "/" {
                    f.name.clone()
                } else {
                    format!("{}/{}", f.parent_dir, f.name)
                };
                let name_sim = similar(&f.display_name, query) as i32;
                let bl_count = self.backlinks.read().backlink_count(&path);
                NoteHit {
                    path,
                    name: f.display_name,
                    snippet: String::new(),
                    backlink_count: bl_count,
                    name_similarity: name_sim,
                }
            })
            .collect();

        Ok(hits)
    }

    // ── Backlinks & Graph ─────────────────────────────────────────

    /// Get backlinks for a note.
    pub fn backlinks_for(&self, path: &str) -> Vec<Backlink> {
        self.backlinks.read().backlinks_for(path)
    }

    /// Get the full link graph for visualization.
    pub fn link_graph(&self) -> LinkGraph {
        self.backlinks.read().link_graph()
    }

    /// Index all markdown files in the knowledge base.
    ///
    /// Walks the entire directory tree (at any depth) and builds the
    /// backlink index, including wikilink targets resolved against a
    /// stem index built from the same walk. Returns the number of files
    /// indexed.
    pub fn index_all(&self) -> Result<usize> {
        // Read every file's content under the fs read lock first; we need
        // the contents anyway and this avoids re-acquiring per file.
        let (paths_contents, stem_index) = {
            let fs = self.fs.read();
            let all = fs.all_md_files()?;
            let stem_index = {
                let mut idx: StemIndex = StemIndex::new();
                for (path, _size) in &all {
                    let stem = path
                        .rsplit('/')
                        .next()
                        .unwrap_or(path.as_str())
                        .trim_end_matches(".md")
                        .to_lowercase();
                    idx.entry(stem).or_default().push(path.clone());
                }
                idx
            };
            let mut paths_contents: Vec<(String, String)> = Vec::with_capacity(all.len());
            for (path, _size) in &all {
                if let Ok(content) = fs.read_path(path) {
                    paths_contents.push((path.clone(), content));
                }
            }
            (paths_contents, stem_index)
        };

        let mut count = 0;
        {
            let mut backlinks = self.backlinks.write();
            backlinks.clear();
            for (path, content) in &paths_contents {
                backlinks.index_file_with(path, content, &stem_index);
                count += 1;
            }
        }

        tracing::info!(files = count, "Knowledge base indexed");
        Ok(count)
    }

    // ── Chat / Inbox ───────────────────────────────────────────────

    /// Append a timestamped message to Chat.md.
    pub fn chat_append(&self, message: &str) -> Result<()> {
        let header = today_chat_header();
        let timestamp = chrono::Local::now().format("`15:04`").to_string();
        let entry = format!("- [ ] {timestamp} {message}");

        let mut content = self.note_read(CHAT_FILENAME)?.unwrap_or_default();
        if !content.contains(&header) {
            if !content.trim_end().ends_with('\n') {
                content.push('\n');
            }
            content.push_str(&header);
            content.push('\n');
        }
        content.push_str(&entry);
        content.push('\n');
        self.note_write(CHAT_FILENAME, &content)?;
        Ok(())
    }

    /// Parse Chat.md into structured message blocks.
    pub fn chat_messages(&self) -> Result<Vec<String>> {
        let content = self.note_read(CHAT_FILENAME)?.unwrap_or_default();
        Ok(read_chat_msgs(&content))
    }

    /// Delete a specific chat message by its content hash.
    pub fn chat_delete(&self, msg_hash: &str) -> Result<bool> {
        let content = self.note_read(CHAT_FILENAME)?.unwrap_or_default();
        match delete_chat_msg(&content, msg_hash) {
            Ok(new_content) => {
                self.note_write(CHAT_FILENAME, &new_content)?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// Rename a specific chat message by its content hash.
    pub fn chat_rename(&self, msg_hash: &str, new_body: &str) -> Result<bool> {
        let content = self.note_read(CHAT_FILENAME)?.unwrap_or_default();
        match rename_chat_msg(&content, msg_hash, new_body) {
            Ok(new_content) => {
                self.note_write(CHAT_FILENAME, &new_content)?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// Move a chat message to a target file as a checklist item.
    pub fn chat_move_to(&self, msg_hash: &str, target_path: &str) -> Result<bool> {
        let chat_content = self.note_read(CHAT_FILENAME)?.unwrap_or_default();
        let target_content = self.note_read(target_path)?.unwrap_or_default();
        let (new_chat, new_target) = move_from_chat(&chat_content, msg_hash, &target_content);
        if new_chat != chat_content {
            self.note_write(CHAT_FILENAME, &new_chat)?;
            self.note_write(target_path, &new_target)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    // ── Journal ───────────────────────────────────────────────────

    /// Add a timestamped record to today's journal entry.
    pub fn journal_add_record(&self, record: &str) -> Result<()> {
        let fs = self.fs.write();
        let tz = chrono::Local::now().offset().to_owned();
        journal_add_record(&fs, record, tz)?;
        Ok(())
    }

    /// Add an emoji to today's journal header.
    pub fn journal_add_emoji(&self, emoji: &str) -> Result<()> {
        let fs = self.fs.write();
        let tz = chrono::Local::now().offset().to_owned();
        journal_add_emoji(&fs, emoji, tz)?;
        Ok(())
    }

    /// Get today's journal file path (e.g., "journal/2026.05 May.md").
    pub fn journal_today_path(&self) -> String {
        let tz = chrono::Local::now().offset().to_owned();
        today_journal_filename(tz)
    }

    // ── Habits ───────────────────────────────────────────────────

    /// Read habit tracking data for a given year.
    pub fn habits(&self, year: i32) -> Result<Habits> {
        let fs = self.fs.read();
        Ok(habits(&fs, year)?)
    }

    /// Get last week's habit data.
    pub fn habits_last_week(&self) -> Result<Habits> {
        let fs = self.fs.read();
        let tz = chrono::Local::now().offset().to_owned();
        Ok(last_week_habits(&fs, tz)?)
    }

    /// Write habit data for a year.
    pub fn habits_write(&self, year: i32, habits: &Habits) -> Result<()> {
        let fs = self.fs.write();
        write_habits(&fs, year, habits)?;
        Ok(())
    }

    // ── Config ────────────────────────────────────────────────────

    /// Read the knowledge base config (config.json).
    pub fn config(&self) -> Result<KnowledgeConfig> {
        let fs = self.fs.read();
        match fs.read_path("config.json") {
            Ok(content) => Ok(serde_json::from_str(&content).unwrap_or_default()),
            Err(_) => Ok(KnowledgeConfig::default()),
        }
    }

    /// Write the knowledge base config.
    pub fn set_config(&self, config: &KnowledgeConfig) -> Result<()> {
        let json = serde_json::to_string_pretty(config)?;
        self.note_write("config.json", &json)?;
        Ok(())
    }

    // ── Checklist ────────────────────────────────────────────────

    /// Parse checklist items from a file.
    pub fn checklist_items(
        &self,
        path: &str,
    ) -> Result<(Vec<String>, std::collections::HashMap<String, bool>)> {
        let content = self.note_read(path)?.unwrap_or_default();
        Ok(checklist_items(&content))
    }

    /// Get incomplete checklist items from a file.
    pub fn checklist_incomplete(&self, path: &str) -> Result<Vec<String>> {
        let content = self.note_read(path)?.unwrap_or_default();
        Ok(incomplete_checklist_items(&content))
    }

    /// Add a checklist item to a file.
    pub fn checklist_add(&self, path: &str, item: &str, checked: bool) -> Result<()> {
        let content = self.note_read(path)?.unwrap_or_default();
        let updated = add_checklist_item(&content, item, checked);
        self.note_write(path, &updated)
    }

    /// Complete a checklist item by hash.
    pub fn checklist_complete(&self, path: &str, item_hash: &str) -> Result<bool> {
        let content = self.note_read(path)?.unwrap_or_default();
        let (new_content, found) = complete_checklist_item(&content, item_hash);
        if !found.is_empty() {
            self.note_write(path, &new_content)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Remove a checklist item by text or hash.
    pub fn checklist_remove(&self, path: &str, item_or_hash: &str) -> Result<bool> {
        let content = self.note_read(path)?.unwrap_or_default();
        let (new_content, removed) = remove_checklist_item(&content, item_or_hash);
        if !removed.is_empty() {
            self.note_write(path, &new_content)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Remove all completed checklist items.
    pub fn checklist_remove_completed(&self, path: &str) -> Result<(String, String)> {
        let content = self.note_read(path)?.unwrap_or_default();
        let (kept, removed) = remove_completed_checklist_items(&content);
        if !removed.is_empty() {
            self.note_write(path, &kept)?;
        }
        Ok((kept, removed))
    }

    // ── Worker ────────────────────────────────────────────────────

    /// Run nightly cleanup.
    pub fn run_nightly_cleanup(&self) -> Result<crate::worker::NightlyReport> {
        // Read config before acquiring the write lock — config() takes
        // a read lock and would otherwise deadlock against our write guard.
        let config = self.config()?;
        let fs = self.fs.write();
        Ok(remove_completed_items(&fs, &config)?)
    }

    /// Move due scheduled tasks to Chat.
    pub fn run_scheduled_tasks(&self) -> Result<Vec<String>> {
        // Read config first, take the write lock only for the worker pass,
        // then release it before set_config() (which calls note_write and
        // would re-acquire the lock).
        let mut config = self.config()?;
        let moved = {
            let fs = self.fs.write();
            move_due_tasks(&fs, &mut config)?
        };
        if !moved.is_empty() {
            self.set_config(&config)?;
        }
        Ok(moved)
    }

    // ── Stats ────────────────────────────────────────────────────

    /// Get today's completion report.
    pub fn today_report(&self) -> Result<crate::stats::TodayReport> {
        let fs = self.fs.read();
        Ok(today_report(&fs)?)
    }

    /// Get list of files completed today.
    pub fn done_today(&self) -> Result<Vec<FileEntry>> {
        let fs = self.fs.read();
        Ok(done_today(&fs)?)
    }

    // ── Utilities ───────────────────────────────────────────────

    /// Convert markdown to HTML.
    pub fn markdown_to_html(&self, md: &str) -> String {
        markdown_to_html(md)
    }

    /// Find an emoji for a keyword.
    pub fn auto_emoji(&self, text: &str) -> String {
        emoji_for(text)
    }

    /// Generate world clock report for given timezone names.
    pub fn world_clock(&self, timezone_names: &[&str]) -> Vec<crate::plugins::TimezoneEntry> {
        world_clock_for_names(timezone_names)
    }

    // ── Agent Write Tracking ──────────────────────────────────────

    /// Mark a file as having been written by an agent.
    pub fn mark_agent_write(&self, path: &str) {
        self.agent_writes.lock().insert(path.to_string());
    }

    /// Check if a file was written by an agent.
    pub fn is_agent_write(&self, path: &str) -> bool {
        self.agent_writes.lock().contains(path)
    }

    /// Clear the agent-write marker for a file.
    pub fn clear_agent_write(&self, path: &str) {
        self.agent_writes.lock().remove(path);
    }

    // ── Text extraction ──────────────────────────────────────────

    /// Extract text, images, and links from markdown content.
    pub fn extract_text_imgs_links(&self, text: &str) -> crate::tgtxt::ExtractResult {
        crate::tgtxt::extract_text_imgs_links(text)
    }

    // ── Headings (for tag extraction) ─────────────────────────────

    /// Extract headings from content for tag generation.
    pub fn extract_headings(&self, content: &str) -> Vec<String> {
        extract_headings(content).into_iter().take(5).collect()
    }
}

// ---------------------------------------------------------------------------
// Frontmatter helpers (RFC-022)
// ---------------------------------------------------------------------------

/// Parse Oxios frontmatter from a note's content.
///
/// Returns `(Some(NoteMeta), body)` if the `oxios:` key is present in the
/// frontmatter. Returns `(None, original_content)` if there is no frontmatter
/// or the frontmatter does not contain the `oxios:` key (e.g., user-written
/// Obsidian frontmatter). In the latter case, the full original content
/// (including any user frontmatter) is returned as the body.
pub fn parse_note_meta(content: &str) -> (Option<NoteMeta>, String) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (None, content.to_string());
    }

    // Find the closing ---
    let after_first = &trimmed[3..];
    let rest = after_first.trim_start_matches(['-', '\n', '\r']);
    if let Some(end_offset) = rest.find("\n---") {
        let yaml_block = &rest[..end_offset];
        let body_start = end_offset + 4; // skip \n---
        let body = rest[body_start..].trim_start().to_string();

        // Parse YAML looking for the `oxios:` key
        if !yaml_block.contains("oxios:") {
            // User frontmatter, not ours
            return (None, content.to_string());
        }

        #[derive(serde::Deserialize)]
        struct FrontmatterWrapper {
            oxios: NoteMeta,
        }

        match serde_yaml::from_str::<FrontmatterWrapper>(yaml_block) {
            Ok(wrapper) => (Some(wrapper.oxios), body),
            Err(_) => (None, content.to_string()),
        }
    } else {
        (None, content.to_string())
    }
}

/// Format a NoteMeta as YAML frontmatter prepended to content.
///
/// `serde_yaml::to_string` produces flat YAML like `author: agent\nsource: Hook\n`.
/// We must indent each line with 2 spaces so they become children of the
/// `oxios:` mapping key.
fn format_frontmatter(meta: &NoteMeta, body: &str) -> String {
    let yaml = serde_yaml::to_string(meta).unwrap_or_default();
    let indented: String = yaml
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n");
    format!("---\noxios:\n{}\n---\n\n{}", indented, body)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_kb() -> KnowledgeBase {
        let dir = std::env::temp_dir().join(format!("test-kb-{}", uuid::Uuid::new_v4()));
        KnowledgeBase::new(dir.join("kb")).expect("test knowledge base")
    }

    #[test]
    fn test_note_write_and_read() {
        let kb = make_test_kb();
        kb.note_write("brain/Rust.md", "# Rust\n\nHello world")
            .unwrap();
        let content = kb.note_read("brain/Rust.md").unwrap();
        assert_eq!(content, Some("# Rust\n\nHello world".to_string()));
    }

    #[test]
    fn test_note_read_missing() {
        let kb = make_test_kb();
        assert_eq!(kb.note_read("nonexistent.md").unwrap(), None);
    }

    #[test]
    fn test_note_delete() {
        let kb = make_test_kb();
        kb.note_write("del.md", "to delete").unwrap();
        kb.note_delete("del.md").unwrap();
        assert_eq!(kb.note_read("del.md").unwrap(), None);
    }

    #[test]
    fn test_note_move() {
        let kb = make_test_kb();
        kb.note_write("old.md", "content").unwrap();
        kb.note_move("old.md", "new.md").unwrap();
        assert_eq!(kb.note_read("old.md").unwrap(), None);
        assert_eq!(kb.note_read("new.md").unwrap(), Some("content".to_string()));
    }

    #[test]
    fn test_note_move_rewrites_inbound_links() {
        let kb = make_test_kb();
        // Two notes link to the target by its old name.
        kb.note_write("a.md", "See [target](target.md) and [again](target.md).")
            .unwrap();
        kb.note_write("b.md", "Ref [target](target.md).").unwrap();
        kb.note_write("target.md", "# Target\n\nbody").unwrap();
        // Re-resolve: a.md/b.md were indexed before target.md existed, so
        // the markdown links are exact-path matches (work regardless), but
        // a fresh index keeps the test self-consistent.
        kb.index_all().unwrap();

        kb.note_move("target.md", "renamed.md").unwrap();

        // Moved file content preserved.
        assert_eq!(kb.note_read("target.md").unwrap(), None);
        assert_eq!(
            kb.note_read("renamed.md").unwrap(),
            Some("# Target\n\nbody".to_string())
        );

        // Inbound links rewritten on disk.
        assert_eq!(
            kb.note_read("a.md").unwrap().as_deref(),
            Some("See [target](renamed.md) and [again](renamed.md).")
        );
        assert_eq!(
            kb.note_read("b.md").unwrap().as_deref(),
            Some("Ref [target](renamed.md).")
        );

        // Backlink index resolves links under the new name.
        let bl: HashSet<String> = kb
            .backlinks_for("renamed.md")
            .into_iter()
            .map(|b| b.source_path)
            .collect();
        assert_eq!(bl, HashSet::from(["a.md".to_string(), "b.md".to_string()]));
        assert_eq!(kb.backlinks_for("target.md").len(), 0);
    }

    #[test]
    fn test_note_move_rewrites_wikilinks() {
        let kb = make_test_kb();
        // Source references the target via every supported wikilink form.
        kb.note_write(
            "src.md",
            "Bare [[Target]] path [[dir/Target]] full [[dir/Target.md]] alias [[Target|T]].",
        )
        .unwrap();
        kb.note_write("dir/Target.md", "# Target\n\nbody").unwrap();
        // src.md was indexed before dir/Target.md existed; rebuild so its
        // wikilinks resolve against the now-complete tree.
        kb.index_all().unwrap();

        kb.note_move("dir/Target.md", "dir/Renamed.md").unwrap();

        // Every form rewrites to the new path; alias is preserved.
        assert_eq!(
            kb.note_read("src.md").unwrap().as_deref(),
            Some(
                "Bare [[Renamed]] path [[dir/Renamed]] full [[dir/Renamed.md]] alias [[Renamed|T]]."
            ),
        );
        // Backlinks now resolve under the new canonical path.
        assert_eq!(kb.backlinks_for("dir/Renamed.md").len(), 1);
        assert_eq!(kb.backlinks_for("dir/Target.md").len(), 0);
    }

    #[test]
    fn test_note_move_skips_ambiguous_bare_wikilink() {
        // Two files share the stem "Dup": the bare [[Dup]] in src is
        // ambiguous and must NOT be indexed → not rewritten when EITHER
        // Dup renames. The path-style [[a/Dup]] IS unambiguous and rewrites.
        let kb = make_test_kb();
        kb.note_write("src.md", "ambig [[Dup]] explicit [[a/Dup]]")
            .unwrap();
        kb.note_write("a/Dup.md", "# A").unwrap();
        kb.note_write("b/Dup.md", "# B").unwrap();
        // src.md was indexed before both Dups existed — rebuild so the
        // bare stem is now (correctly) ambiguous and dropped from the index.
        kb.index_all().unwrap();

        kb.note_move("a/Dup.md", "a/Moved.md").unwrap();

        let src = kb.note_read("src.md").unwrap().unwrap_or_default();
        // Bare link untouched (ambiguous); path-style link rewritten.
        assert!(
            src.contains("[[Dup]]"),
            "ambiguous bare link must be left alone: {src}"
        );
        assert!(
            src.contains("[[a/Moved]]"),
            "explicit path link must be rewritten: {src}"
        );
    }

    #[test]
    fn test_backlinks_track_wikilinks() {
        let kb = make_test_kb();
        kb.note_write("brain/Rust.md", "See [[Ownership]] and [[brain/Go]]")
            .unwrap();
        kb.note_write("brain/Ownership.md", "# Ownership").unwrap();
        kb.note_write("brain/Go.md", "# Go").unwrap();
        // Rust.md was indexed before Ownership/Go existed; rebuild so its
        // wikilinks resolve against the now-complete tree.
        kb.index_all().unwrap();

        // Both wikilinks resolve and appear as backlinks on their targets.
        let owners_of_ownership: HashSet<String> = kb
            .backlinks_for("brain/Ownership.md")
            .into_iter()
            .map(|b| b.source_path)
            .collect();
        assert!(owners_of_ownership.contains("brain/Rust.md"));
        let owners_of_go: HashSet<String> = kb
            .backlinks_for("brain/Go.md")
            .into_iter()
            .map(|b| b.source_path)
            .collect();
        assert!(owners_of_go.contains("brain/Rust.md"));
    }

    #[test]
    fn test_backlinks() {
        let kb = make_test_kb();
        kb.note_write("brain/Rust.md", "See [Ownership](brain/Ownership.md)")
            .unwrap();
        let bl = kb.backlinks_for("brain/Ownership.md");
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].source_path, "brain/Rust.md");
    }

    #[test]
    fn test_note_tree() {
        let kb = make_test_kb();
        kb.note_write("brain/Rust.md", "Rust").unwrap();
        let entries = kb.note_tree("brain").unwrap();
        assert!(!entries.is_empty());
    }

    #[test]
    fn test_search_by_name() {
        let kb = make_test_kb();
        kb.note_write("brain/Rust.md", "Rust content").unwrap();
        let hits = kb.search("Rust", 10).unwrap();
        assert!(!hits.is_empty());
    }

    #[test]
    fn test_link_graph() {
        let kb = make_test_kb();
        kb.note_write("a.md", "[b](b.md)").unwrap();
        let graph = kb.link_graph();
        assert!(!graph.edges.is_empty());
    }

    #[test]
    fn test_agent_write_tracking() {
        let kb = make_test_kb();
        assert!(!kb.is_agent_write("test.md"));
        kb.mark_agent_write("test.md");
        assert!(kb.is_agent_write("test.md"));
        kb.clear_agent_write("test.md");
        assert!(!kb.is_agent_write("test.md"));
    }

    #[test]
    fn test_index_all() {
        let kb = make_test_kb();
        kb.note_write("brain/Rust.md", "Rust [Go](brain/Go.md)")
            .unwrap();
        kb.note_write("brain/Go.md", "Go language").unwrap();
        kb.note_write("index.md", "Welcome").unwrap();
        let count = kb.index_all().unwrap();
        assert_eq!(count, 3);
        let bl = kb.backlinks_for("brain/Go.md");
        assert_eq!(bl.len(), 1);
    }

    #[test]
    fn test_on_file_change_callback() {
        let kb = make_test_kb();
        let _called = std::sync::atomic::AtomicBool::new(false);
        let path_clone: std::sync::Arc<std::sync::atomic::AtomicBool> =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = path_clone.clone();

        kb.on_file_change(move |path, change| {
            let _ = path;
            let _ = change;
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        });

        kb.note_write("test.md", "hello").unwrap();
        assert!(path_clone.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn test_chat_append() {
        let kb = make_test_kb();
        kb.chat_append("Test message").unwrap();
        let messages = kb.chat_messages().unwrap();
        // The captured message must be a parseable marker block (- [ ] `HH:MM` text),
        // not merged into the date header. chat_append must emit the `- [ ]` prefix
        // that read_chat_msgs splits on.
        assert!(
            messages
                .iter()
                .any(|m| m.starts_with("- [") && m.contains("Test message")),
            "captured message should be a parseable marker block: {messages:?}"
        );
    }

    #[test]
    fn test_config() {
        let kb = make_test_kb();
        let cfg = kb.config().unwrap();
        // Should return default for non-existent config
        let cfg2 = kb.config().unwrap();
        assert_eq!(cfg.language, cfg2.language);
    }

    #[test]
    fn test_markdown_to_html() {
        let kb = make_test_kb();
        let html = kb.markdown_to_html("# Hello\n\n**world**");
        // markdown_to_html wraps content in a <p> tag by default, check for content
        assert!(html.contains("Hello"), "HTML should contain Hello: {html}");
        assert!(html.contains("world"), "HTML should contain world: {html}");
    }

    #[test]
    fn test_auto_emoji() {
        let kb = make_test_kb();
        let emoji = kb.auto_emoji("cooking pasta");
        assert!(!emoji.is_empty());
    }

    #[test]
    fn test_extract_headings() {
        let kb = make_test_kb();
        let headings = kb.extract_headings("# Title\n\n## Section\n\n### Subsection");
        assert!(headings.len() >= 2);
    }

    #[test]
    fn test_frontmatter_roundtrip() {
        let meta = NoteMeta {
            author: "agent".to_string(),
            source: NoteSource::Hook,
            quality: NoteQuality::Raw,
            needs_review: true,
            session_id: Some("abc123".to_string()),
            message_index: Some(3),
            saved_at: Some("2026-06-13T00:00:00Z".to_string()),
        };
        let body = "## Test\n\nContent here.";
        let formatted = format_frontmatter(&meta, body);
        assert!(formatted.starts_with("---\noxios:\n"));
        let (parsed_meta, parsed_body) = parse_note_meta(&formatted);
        assert!(
            parsed_meta.is_some(),
            "Failed to parse round-tripped frontmatter"
        );
        let pm = parsed_meta.unwrap();
        assert_eq!(pm.author, "agent");
        assert_eq!(pm.session_id.as_deref(), Some("abc123"));
        assert_eq!(pm.message_index, Some(3));
        assert_eq!(parsed_body.trim(), body.trim());
    }

    #[test]
    fn test_parse_user_frontmatter_ignored() {
        let content = "---\ntags: [rust, design]\n---\n\n## My Note\nContent.";
        let (meta, body) = parse_note_meta(content);
        assert!(
            meta.is_none(),
            "User frontmatter should not be parsed as NoteMeta"
        );
        assert!(
            body.contains("tags: [rust, design]"),
            "User frontmatter preserved"
        );
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let content = "# Just a note\nSome content.";
        let (meta, body) = parse_note_meta(content);
        assert!(meta.is_none());
        assert_eq!(body, content);
    }

    // ----------------------------------------------------------------
    // T12 — format-aware note writes via frontformat::write_note
    // ----------------------------------------------------------------

    #[ignore = "vault-unification T12 spec: note_write path not yet converted to frontformat::write_note"]
    fn note_write_is_format_aware_and_noop_guarded() {
        let kb = make_test_kb();

        // 1. memo path → synthesized frontmatter (id/created/updated)
        kb.note_write("docs/a.md", "hello").unwrap();
        let first = kb.note_read("docs/a.md").unwrap().unwrap();
        assert!(
            first.starts_with("---\n"),
            "memo write must synthesize frontmatter; got: {first:?}"
        );

        // 2. identical second write → NoOp guard: file bytes unchanged
        kb.note_write("docs/a.md", "hello").unwrap();
        let second = kb.note_read("docs/a.md").unwrap().unwrap();
        assert_eq!(
            first, second,
            "NoOp guard must not rewrite unchanged memo content"
        );

        // 3. system path → raw, never synthesized frontmatter
        kb.note_write("Chat.md", "- [ ] x\n").unwrap();
        let chat = kb.note_read("Chat.md").unwrap().unwrap();
        assert_eq!(
            chat, "- [ ] x\n",
            "system path must be raw, no frontmatter; got: {chat:?}"
        );
        assert!(
            !chat.starts_with("---"),
            "system path must never carry frontmatter"
        );
    }

    #[ignore = "vault-unification T12 spec: note_write path not yet converted to frontformat::write_note"]
    fn note_write_noop_skips_backlink_reindex_and_callback() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let kb = make_test_kb();
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_cb = Arc::clone(&counter);
        kb.on_file_change(move |_path, _change| {
            counter_cb.fetch_add(1, AtomicOrdering::SeqCst);
        });

        kb.note_write("brain/Rust.md", "hello world").unwrap();
        let after_first = counter.load(AtomicOrdering::SeqCst);
        assert_eq!(
            after_first, 1,
            "first write must fire callback exactly once"
        );

        // identical re-write must be a NoOp → no callback
        kb.note_write("brain/Rust.md", "hello world").unwrap();
        let after_second = counter.load(AtomicOrdering::SeqCst);
        assert_eq!(
            after_second, 1,
            "NoOp write must NOT call notify_change; got {after_second} callbacks"
        );
    }

    #[test]
    fn note_write_with_meta_refuses_user_authored_frontmatter() {
        let kb = make_test_kb();

        // Pre-existing file with user-authored (foreign) frontmatter
        // — has frontmatter block but NO oxios: table.
        let user_note = "---\ntags: [rust, design]\nauthor: jane\n---\n\n# My note\n";
        kb.note_write("brain/User.md", user_note).unwrap();

        let meta = NoteMeta {
            author: "agent".to_string(),
            source: NoteSource::Hook,
            quality: NoteQuality::Raw,
            needs_review: false,
            session_id: None,
            message_index: None,
            saved_at: None,
        };

        // Must return Ok(false) — refuse to touch user-authored frontmatter
        let accepted = kb
            .note_write_with_meta("brain/User.md", "# My note\nnew body", &meta)
            .unwrap();
        assert!(
            !accepted,
            "user-authored frontmatter must refuse agent metadata write"
        );

        // File must still contain the user-authored frontmatter untouched
        let after = kb.note_read("brain/User.md").unwrap().unwrap();
        assert!(
            after.contains("tags: [rust, design]"),
            "user tags must survive unchanged; got: {after:?}"
        );
        assert!(
            !after.contains("oxios:"),
            "no oxios: must be synthesized on user-authored file; got: {after:?}"
        );
    }

    #[ignore = "vault-unification T12 spec: note_write path not yet converted to frontformat::write_note"]
    fn note_write_with_meta_synthesizes_and_merges() {
        let kb = make_test_kb();
        let meta = NoteMeta {
            author: "agent".to_string(),
            source: NoteSource::Hook,
            quality: NoteQuality::Raw,
            needs_review: true,
            session_id: Some("sess-1".to_string()),
            message_index: Some(2),
            saved_at: Some("2026-08-21T00:00:00Z".to_string()),
        };

        // Fresh file → synthesize frontmatter with oxios:
        let accepted = kb
            .note_write_with_meta("brain/New.md", "fresh content", &meta)
            .unwrap();
        assert!(accepted, "fresh memo must accept metadata write");
        let after = kb.note_read("brain/New.md").unwrap().unwrap();
        assert!(
            after.starts_with("---\n"),
            "must carry frontmatter; got: {after:?}"
        );
        assert!(
            after.contains("oxios:"),
            "must contain oxios: table; got: {after:?}"
        );

        // Second write merges: existing oxios: is preserved (id/created survive)
        let meta2 = NoteMeta {
            author: "agent2".to_string(),
            ..meta.clone()
        };
        kb.note_write_with_meta("brain/New.md", "edited body", &meta2)
            .unwrap();
        let after2 = kb.note_read("brain/New.md").unwrap().unwrap();
        assert!(
            after2.contains("id:"),
            "id must survive merge; got: {after2:?}"
        );
        assert!(
            after2.contains("agent2"),
            "author must be overwritten by new meta; got: {after2:?}"
        );
        assert!(
            after2.contains("edited body"),
            "body must reflect second write; got: {after2:?}"
        );
    }

    #[ignore = "vault-unification T12 spec: note_write path not yet converted to frontformat::write_note"]
    fn restore_merges_legacy_content() {
        let kb = make_test_kb();

        // Pre-migration blob: oxios: table without id/created/updated.
        // Restoring it must gain id/created/updated through write_document
        // synthesis while keeping the oxios: table.
        let legacy = "---\noxios:\n  author: agent\n  quality: raw\n---\nlegacy body\n";
        kb.note_restore("brain/Legacy.md", legacy).unwrap();

        let after = kb.note_read("brain/Legacy.md").unwrap().unwrap();
        assert!(
            after.contains("id:"),
            "id must be synthesized on legacy restore; got: {after:?}"
        );
        assert!(
            after.contains("created:"),
            "created must be synthesized on legacy restore; got: {after:?}"
        );
        assert!(
            after.contains("oxios:"),
            "oxios: table must survive; got: {after:?}"
        );
        assert!(
            after.contains("legacy body"),
            "body must survive; got: {after:?}"
        );

        // Restore must NOT fire on_file_change callbacks
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_cb = Arc::clone(&counter);
        let kb2 = make_test_kb();
        kb2.on_file_change(move |_p, _c| {
            counter_cb.fetch_add(1, AtomicOrdering::SeqCst);
        });
        kb2.note_restore("brain/Legacy2.md", legacy).unwrap();
        assert_eq!(
            counter.load(AtomicOrdering::SeqCst),
            0,
            "note_restore must suppress callbacks"
        );
    }

    #[test]
    fn notes_needing_review_reads_oxios_table() {
        let kb = make_test_kb();

        // Two memos, one flagged for review, one not.
        let flag_meta = NoteMeta {
            author: "agent".to_string(),
            source: NoteSource::Hook,
            quality: NoteQuality::Raw,
            needs_review: true,
            session_id: None,
            message_index: None,
            saved_at: Some("2026-08-21T00:00:00Z".to_string()),
        };
        let ok_meta = NoteMeta {
            needs_review: false,
            ..flag_meta.clone()
        };

        kb.note_write_with_meta("brain/Yes.md", "needs review", &flag_meta)
            .unwrap();
        kb.note_write_with_meta("brain/No.md", "no review", &ok_meta)
            .unwrap();

        let flagged = kb.notes_needing_review().unwrap();
        assert_eq!(flagged.len(), 1, "exactly one note flagged");
        let (path, _meta) = &flagged[0];
        assert!(
            path.starts_with("brain/Yes.md"),
            "only the flagged note must surface; got: {path}"
        );
    }
}
