//! Git-based version control layer using gix.
//! Provides in-process commits, logs, tags, restore, and diffs.
//!
//! # RFC-013 Improvements
//!
//! - **B1**: `Signature` captures fresh timestamp per commit (not `OnceLock` cached).
//! - **B2**: `restore_file` traverses nested paths (e.g. `audit/2024-05.audit`).
//! - **D1**: `CommitContext` enables per-agent author tracking.
//! - **D2**: `diff_commits` / `file_at_commit` for Ouroboros evaluate.
//! - **D3**: Removed hex round-trips; `list_tags` uses `Category::Tag`.

use anyhow::{Result, bail};
use gix::bstr::BStr;
use gix::hash::ObjectId;
use gix::objs::tree::EntryKind;
use gix::refs::transaction::PreviousValue;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const GITIGNORE: &str = r#"# Oxios
*.tmp
*.lock
.env
api-keys.json
"#;

// ── Public types ────────────────────────────────────────────────────────────

/// Commit information returned after a successful commit.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommitInfo {
    /// Full commit hash (hex).
    pub hash: String,
    /// Short hash (7 chars).
    pub short_hash: String,
    /// Commit message.
    pub message: String,
    /// ISO-8601 timestamp.
    pub timestamp: String,
    /// Author name.
    pub author: String,
}

/// A single commit log entry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    /// Full commit hash (hex).
    pub hash: String,
    /// Short hash (7 chars).
    pub short_hash: String,
    /// Commit message.
    pub message: String,
    /// Timestamp string.
    pub timestamp: String,
    /// Author name.
    pub author: String,
}

/// Commit metadata supplied by the caller to identify who is committing.
///
/// Enables per-agent author tracking while keeping the existing
/// `commit_file(path, msg)` API fully backward-compatible.
#[derive(Default, Debug, Clone)]
pub struct CommitContext {
    /// Agent ID — if present the author becomes `agent-{short_id}`,
    /// otherwise `"oxios"`.
    pub agent_id: Option<uuid::Uuid>,
    /// Extra tag such as `"memory"`, `"audit"`, `"cron"`.
    pub tag: Option<&'static str>,
}

impl CommitContext {
    /// Default system commit (no agent context).
    pub fn system() -> Self {
        Self::default()
    }

    /// Agent commit.
    pub fn agent(agent_id: uuid::Uuid) -> Self {
        Self {
            agent_id: Some(agent_id),
            tag: None,
        }
    }

    /// Tagged commit (no agent).
    pub fn tagged(tag: &'static str) -> Self {
        Self {
            tag: Some(tag),
            ..Default::default()
        }
    }

    /// Derive the author name for this context.
    fn author_name(&self) -> String {
        match &self.agent_id {
            Some(id) => {
                let hex = id.to_string();
                format!("agent-{}", &hex[..8])
            }
            None => "oxios".to_string(),
        }
    }

    /// Build a prefix for the commit message (e.g. `[audit] `).
    fn message_prefix(&self) -> String {
        let mut parts = Vec::new();
        if let Some(tag) = self.tag {
            parts.push(format!("[{tag}]"));
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("{} ", parts.join(" "))
        }
    }
}

// ── Diff types (Phase 3) ────────────────────────────────────────────────────

/// Change kind for a single file.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub enum DiffKind {
    /// New file added.
    Added,
    /// File deleted.
    Deleted,
    /// File content changed.
    Modified,
}

/// Change record for a single file between two commits.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileDiff {
    /// File path (relative to repo root).
    pub path: String,
    /// Hex hash in the "from" commit (None for added files).
    pub old_hash: Option<String>,
    /// Hex hash in the "to" commit (None for deleted files).
    pub new_hash: Option<String>,
    /// Kind of change.
    pub kind: DiffKind,
    /// Unified diff text (None for binary files).
    pub patch: Option<String>,
}

/// Aggregate diff statistics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiffStats {
    /// Number of files changed.
    pub files_changed: usize,
    /// Total lines added.
    pub additions: usize,
    /// Total lines removed.
    pub deletions: usize,
}

/// Diff result between two commits.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommitDiff {
    /// Hex hash of the "from" commit.
    pub from_hash: String,
    /// Hex hash of the "to" commit.
    pub to_hash: String,
    /// Per-file changes.
    pub files: Vec<FileDiff>,
    /// Aggregate statistics.
    pub stats: DiffStats,
}

// ── Internal types ──────────────────────────────────────────────────────────

/// Default committer email used across all commits.
const DEFAULT_EMAIL: &str = "oxios@oxios";

/// Owned signature that captures the timestamp at creation time.
///
/// Fixes B1: the old `self_signature_ref()` used `OnceLock` and cached the
/// timestamp for the entire process lifetime, causing all commits to share
/// the same timestamp.
struct Signature {
    name: String,
    email: String,
    time: String,
}

impl Signature {
    /// Create a new signature with the current timestamp.
    fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
            time: gix::date::Time::now_local_or_utc().to_string(),
        }
    }

    /// Produce a `SignatureRef` valid for as long as `self` lives.
    fn as_ref(&self) -> gix::actor::SignatureRef<'_> {
        gix::actor::SignatureRef {
            name: self.name.as_str().into(),
            email: self.email.as_str().into(),
            time: &self.time,
        }
    }
}

// ── GitLayer ────────────────────────────────────────────────────────────────

/// Git-based version control layer.
///
/// Uses `gix` for in-process git operations — no subprocess spawning,
/// no performance overhead of forking `git` CLI commands.
pub struct GitLayer {
    repo: Arc<Mutex<gix::Repository>>,
    root: PathBuf,
    #[allow(dead_code)]
    committer_email: String,
    enabled: bool,
}

impl GitLayer {
    /// Create a new GitLayer, initializing a repo if needed.
    pub fn new(root: PathBuf, enabled: bool) -> Result<Self> {
        let repo = if root.join(".git").exists() {
            gix::open(&root)?
        } else {
            std::fs::create_dir_all(&root)?;
            gix::init(&root)?
        };

        // Write .gitignore
        let gitignore = root.join(".gitignore");
        if !gitignore.exists() {
            std::fs::write(&gitignore, GITIGNORE)?;
        }

        let repo_ref = Arc::new(Mutex::new(repo));

        // Create initial commit if repo is empty
        if Self::head_id_detached(&repo_ref).is_none() {
            Self::create_initial_commit(&repo_ref, &root)?;
        }

        Ok(Self {
            repo: repo_ref,
            root,
            committer_email: DEFAULT_EMAIL.into(),
            enabled,
        })
    }

    // ── Private helpers (repo-level) ──────────────────────────────────────

    fn head_id_detached(repo_arc: &Arc<Mutex<gix::Repository>>) -> Option<ObjectId> {
        let repo = repo_arc.lock();
        repo.head_id().ok().map(|id| id.detach())
    }

    fn head_id_detached_raw(repo: &gix::Repository) -> Option<ObjectId> {
        repo.head_id().ok().map(|id| id.detach())
    }
    /// Validate that `rel_path` is a relative path that stays within the git root.
    ///
    /// `Path::join` replaces the base when given an absolute path on Unix
    /// (`root.join("/etc/passwd") == "/etc/passwd"`) and `..` components escape
    /// the root. This guards every public commit/restore entry point so an
    /// attacker-controlled `rel_path` (e.g. from `infra_api::git_restore`,
    /// `KernelHandle::save_and_commit`, or `knowledge_curation::commit_file`)
    /// cannot read or write outside the repository.
    ///
    /// The check is lexical: it rejects `Component::ParentDir`, `RootDir`, and
    /// `Prefix`, and any absolute input. A purely-Normal-component relative
    /// path cannot escape `root.join(...)` on any platform.
    fn ensure_within_root(&self, rel_path: &str) -> Result<std::path::PathBuf> {
        use std::path::Component;
        let p = Path::new(rel_path);
        if p.is_absolute() {
            bail!("path must be relative to git root: {rel_path}");
        }
        for comp in p.components() {
            match comp {
                Component::ParentDir => {
                    bail!("parent-dir traversal not allowed: {rel_path}")
                }
                Component::RootDir => bail!("root-dir traversal not allowed: {rel_path}"),
                Component::Prefix(_) => bail!("path prefix not allowed: {rel_path}"),
                _ => {}
            }
        }
        Ok(self.root.join(rel_path))
    }

    fn create_initial_commit(repo: &Arc<Mutex<gix::Repository>>, root: &Path) -> Result<()> {
        let repo_lock = repo.lock();
        let gitignore = root.join(".gitignore");
        let content = std::fs::read(&gitignore)?;
        let blob_id = repo_lock.write_blob(&content)?;
        let empty_tree = ObjectId::empty_tree(repo_lock.object_hash());
        let mut editor = repo_lock.edit_tree(empty_tree)?;
        editor.upsert(".gitignore", EntryKind::Blob, blob_id)?;
        let tree_id = editor.write()?;
        let sig = Signature::new("oxios", DEFAULT_EMAIL);
        repo_lock.commit_as(
            sig.as_ref(),
            sig.as_ref(),
            "refs/heads/main",
            "Initial commit",
            tree_id.detach(),
            Vec::<ObjectId>::new(),
        )?;
        Ok(())
    }

    /// Get the current HEAD tree's ObjectId (no hex round-trip).
    fn head_tree_oid(repo: &gix::Repository) -> Result<ObjectId> {
        match Self::head_id_detached_raw(repo) {
            Some(id) => {
                let commit = repo.find_commit(id)?;
                let decoded = commit.decode()?;
                Ok(decoded.tree())
            }
            None => Ok(ObjectId::empty_tree(repo.object_hash())),
        }
    }

    /// Get tree ObjectId for a commit (no hex round-trip).
    fn commit_tree_id(repo: &gix::Repository, commit_id: ObjectId) -> Result<ObjectId> {
        let commit = repo.find_commit(commit_id)?;
        let decoded = commit.decode()?;
        Ok(decoded.tree())
    }

    /// Traverse path components through sub-trees to locate a blob.
    ///
    /// Supports nested paths like `audit/2024-05.audit`.
    fn find_blob_in_tree(
        repo: &gix::Repository,
        tree_id: ObjectId,
        rel_path: &str,
    ) -> Result<ObjectId> {
        let components: Vec<&str> = Path::new(rel_path)
            .iter()
            .filter_map(|c| c.to_str())
            .collect();
        anyhow::ensure!(!components.is_empty(), "empty path: {rel_path}");

        let mut current_tree_id = tree_id;

        for (i, component) in components.iter().enumerate() {
            let tree = repo.find_tree(current_tree_id)?;
            let decoded = tree.decode()?;
            let comp_bytes = BStr::new(component);
            let entry = decoded
                .entries
                .iter()
                .find(|e| e.filename == comp_bytes)
                .ok_or_else(|| {
                    anyhow::anyhow!("path component '{component}' not found in '{rel_path}'")
                })?;

            if i == components.len() - 1 {
                return Ok(entry.oid.to_owned());
            }
            current_tree_id = entry.oid.to_owned();
        }

        unreachable!()
    }

    // ── Public commit API ─────────────────────────────────────────────────

    /// Commit a single file with a message (backward-compatible).
    pub fn commit_file(&self, rel_path: &str, message: &str) -> Result<CommitInfo> {
        self.commit_file_with(rel_path, message, CommitContext::default())
    }

    /// Commit a single file with a message and explicit commit context.
    pub fn commit_file_with(
        &self,
        rel_path: &str,
        message: &str,
        ctx: CommitContext,
    ) -> Result<CommitInfo> {
        if !self.enabled {
            return self.noop_commit(&ctx, message);
        }
        let repo = self.repo.lock();
        let abs = self.ensure_within_root(rel_path)?;
        if !abs.exists() {
            bail!("File not found: {rel_path}");
        }

        let content = std::fs::read(&abs)?;
        let blob_id = repo.write_blob(&content)?;
        let head_tree = Self::head_tree_oid(&repo)?;
        // I-3: Skip commit if the file content is byte-identical to the
        // existing HEAD tree entry. Avoids empty-diff commits from no-op
        // writes (agent re-saves, debounce no-ops, dream curation, etc.).
        if let Ok(existing) = Self::find_blob_in_tree(&repo, head_tree, rel_path)
            && existing == blob_id
        {
            tracing::debug!(path = rel_path, "Skipping identical-content commit");
            return self.noop_commit(&ctx, message);
        }
        let mut editor = repo.edit_tree(head_tree)?;
        editor.upsert(rel_path, EntryKind::Blob, blob_id)?;
        let tree_id = editor.write()?;

        let parent = repo.head_id().ok().map(|id| id.detach());
        let author_name = ctx.author_name();
        let full_message = format!("{}{}", ctx.message_prefix(), message);
        let sig = Signature::new(&author_name, &self.committer_email);
        let commit_id = repo.commit_as(
            sig.as_ref(),
            sig.as_ref(),
            "refs/heads/main",
            &full_message,
            tree_id.detach(),
            parent.into_iter().collect::<Vec<_>>(),
        )?;

        Ok(self.make_info(&commit_id, &full_message, &author_name))
    }

    /// Commit multiple files in a single commit (backward-compatible).
    pub fn commit_files(&self, rel_paths: &[&str], message: &str) -> Result<CommitInfo> {
        self.commit_files_with(rel_paths, message, CommitContext::default())
    }

    /// Commit multiple files with a message and explicit commit context.
    pub fn commit_files_with(
        &self,
        rel_paths: &[&str],
        message: &str,
        ctx: CommitContext,
    ) -> Result<CommitInfo> {
        if !self.enabled {
            return self.noop_commit(&ctx, message);
        }
        let repo = self.repo.lock();
        let head_tree = Self::head_tree_oid(&repo)?;
        let mut editor = repo.edit_tree(head_tree)?;

        for path in rel_paths {
            let abs = self.ensure_within_root(path)?;
            if abs.exists() {
                let content = std::fs::read(&abs)?;
                let blob_id = repo.write_blob(&content)?;
                editor.upsert(*path, EntryKind::Blob, blob_id)?;
            }
        }
        let tree_id = editor.write()?;

        let parent = repo.head_id().ok().map(|id| id.detach());
        let author_name = ctx.author_name();
        let full_message = format!("{}{}", ctx.message_prefix(), message);
        let sig = Signature::new(&author_name, &self.committer_email);
        let commit_id = repo.commit_as(
            sig.as_ref(),
            sig.as_ref(),
            "refs/heads/main",
            &full_message,
            tree_id.detach(),
            parent.into_iter().collect::<Vec<_>>(),
        )?;

        Ok(self.make_info(&commit_id, &full_message, &author_name))
    }

    /// Remove a file from the repo and commit.
    pub fn remove_file(&self, rel_path: &str, message: &str) -> Result<CommitInfo> {
        if !self.enabled {
            return self.noop_commit(&CommitContext::default(), message);
        }
        // Validate the tree key (defense-in-depth: remove() does not touch disk
        // but the path is used to locate the blob in the commit tree).
        self.ensure_within_root(rel_path)?;
        let repo = self.repo.lock();
        let head_tree = Self::head_tree_oid(&repo)?;
        let mut editor = repo.edit_tree(head_tree)?;
        editor.remove(rel_path)?;
        let tree_id = editor.write()?;

        let parent = repo.head_id().ok().map(|id| id.detach());
        let sig = Signature::new("oxios", &self.committer_email);
        let commit_id = repo.commit_as(
            sig.as_ref(),
            sig.as_ref(),
            "refs/heads/main",
            message,
            tree_id.detach(),
            parent.into_iter().collect::<Vec<_>>(),
        )?;

        Ok(self.make_info(&commit_id, message, "oxios"))
    }

    /// Append an audit entry to a monthly audit log file and commit it.
    pub fn log_action(
        &self,
        agent: &str,
        action: &str,
        target: &str,
        allowed: bool,
        detail: Option<&str>,
    ) -> Result<()> {
        let now = chrono::Utc::now();
        let filename = format!("audit/{}.audit", now.format("%Y-%m"));
        let entry = format!(
            "{} | {} | {} | {} | {} | {}\n",
            now.to_rfc3339(),
            agent,
            action,
            target,
            if allowed { "ALLOW" } else { "DENY" },
            detail.unwrap_or("-")
        );
        let dir = self.root.join("audit");
        std::fs::create_dir_all(&dir)?;
        use std::io::Write;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join(&filename))?
            .write_all(entry.as_bytes())?;
        self.commit_file(&filename, &format!("audit: {agent} {action} {target}"))?;
        Ok(())
    }

    // ── Tags ──────────────────────────────────────────────────────────────

    /// Create an annotated tag at the current HEAD.
    pub fn tag(&self, name: &str, message: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let repo = self.repo.lock();
        let head_id = repo
            .head_id()
            .ok()
            .map(|id| id.detach())
            .ok_or_else(|| anyhow::anyhow!("No HEAD commit to tag"))?;
        let sig = Signature::new("oxios", &self.committer_email);
        repo.tag(
            name,
            head_id,
            gix::objs::Kind::Commit,
            Some(sig.as_ref()),
            message,
            PreviousValue::MustNotExist,
        )?;
        Ok(())
    }

    /// List all tags in the repository.
    ///
    /// Uses `Category::Tag` to correctly filter only tag refs.
    pub fn list_tags(&self) -> Result<Vec<String>> {
        let repo = self.repo.lock();
        let mut tags = Vec::new();
        for reference in repo.references()?.all()? {
            let reference = reference.map_err(|e| anyhow::anyhow!("ref iter: {e:#}"))?;
            if reference
                .name()
                .category()
                .is_some_and(|c| matches!(c, gix::refs::Category::Tag))
            {
                tags.push(reference.name().shorten().to_string());
            }
        }
        Ok(tags)
    }

    /// Delete a tag by name. Fails if the tag does not exist.
    pub fn delete_tag(&self, name: &str) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let repo = self.repo.lock();
        // Locate the tag reference first, then delete it after the search
        // iterator is dropped so we don't hold a borrow across the edit.
        let target = repo
            .references()?
            .all()?
            .filter_map(|r| r.ok())
            .find(|r| {
                r.name()
                    .category()
                    .is_some_and(|c| matches!(c, gix::refs::Category::Tag))
                    && r.name().shorten() == name
            })
            .ok_or_else(|| anyhow::anyhow!("tag not found: {name}"))?;
        target
            .delete()
            .map_err(|e| anyhow::anyhow!("delete tag: {e:#}"))?;
        Ok(())
    }

    // ── Log / resolve ─────────────────────────────────────────────────────

    /// Return commit log entries, most recent first.
    pub fn log(&self, max_count: usize) -> Result<Vec<LogEntry>> {
        let repo = self.repo.lock();
        let head_id = repo.head_id()?.detach();
        let mut entries = Vec::new();
        let mut current_id: Option<ObjectId> = Some(head_id);

        while let Some(id) = current_id {
            if entries.len() >= max_count {
                break;
            }
            let commit = repo.find_commit(id)?;
            let decoded = commit.decode()?;
            let msg_ref = decoded.message();
            let msg = if let Some(body) = msg_ref.body {
                format!("{}\n\n{}", msg_ref.title, body)
            } else {
                msg_ref.title.to_string()
            };
            let timestamp = decoded.time().map(|t| t.to_string()).unwrap_or_default();
            let author = decoded
                .author()
                .map(|a| a.name.to_string())
                .unwrap_or_default();
            let hex = id.to_hex().to_string();
            entries.push(LogEntry {
                hash: hex.clone(),
                short_hash: hex[..7].into(),
                message: msg,
                timestamp,
                author,
            });
            current_id = decoded.parents().next();
        }

        Ok(entries)
    }

    /// Resolve a partial commit hash to full ObjectId.
    pub fn resolve_partial_hash(&self, partial: &str) -> Result<ObjectId> {
        if partial.len() < 4 {
            bail!("Partial hash too short (minimum 4 characters)");
        }
        if partial.len() >= 40 {
            return Ok(ObjectId::from_hex(partial.as_bytes())?);
        }
        let repo = self.repo.lock();
        let id = repo.rev_parse_single(BStr::new(partial))?;
        Ok(id.detach())
    }

    /// Resolve a hash string using a pre-locked repo.
    fn resolve_hash_inner(&self, repo: &gix::Repository, partial: &str) -> Result<ObjectId> {
        if partial.len() >= 40 {
            return Ok(ObjectId::from_hex(partial.as_bytes())?);
        }
        if partial.len() < 4 {
            bail!("Hash too short (minimum 4 characters)");
        }
        let id = repo.rev_parse_single(BStr::new(partial))?;
        Ok(id.detach())
    }

    // ── Restore ───────────────────────────────────────────────────────────

    /// Restore a file to its state in a specific commit.
    ///
    /// Supports nested paths like `audit/2024-05.audit` by traversing
    /// each path component through sub-trees.
    pub fn restore_file(&self, rel_path: &str, hash: &str) -> Result<()> {
        // Validate the destination before resolving the blob — defense-in-depth
        // against writing attacker-controlled content to an arbitrary path.
        let dest = self.ensure_within_root(rel_path)?;
        let commit_id = self.resolve_partial_hash(hash)?;
        let repo = self.repo.lock();
        let commit_tree_id = Self::commit_tree_id(&repo, commit_id)?;
        let blob_id = Self::find_blob_in_tree(&repo, commit_tree_id, rel_path)?;
        let blob = repo.find_blob(blob_id)?;
        std::fs::write(dest, &blob.data)?;
        Ok(())
    }

    /// Read a file's content at a specific commit without writing to disk.
    ///
    /// Unlike [`restore_file`], this does NOT touch the filesystem — it
    /// returns the blob data directly. Callers that need to write the
    /// content should pass it through `KnowledgeBase::note_restore`,
    /// which writes atomically (temp file + rename) via
    /// `frontformat::write_note`. This avoids a race where
    /// `restore_file`'s direct `std::fs::write` clobbers a concurrent
    /// `note_write` (I-4).
    pub fn file_at_commit(&self, rel_path: &str, hash: &str) -> Result<Vec<u8>> {
        // Validate path (defense-in-depth, same as restore_file).
        self.ensure_within_root(rel_path)?;
        let commit_id = self.resolve_partial_hash(hash)?;
        let repo = self.repo.lock();
        let commit_tree_id = Self::commit_tree_id(&repo, commit_id)?;
        let blob_id = Self::find_blob_in_tree(&repo, commit_tree_id, rel_path)?;
        let blob = repo.find_blob(blob_id)?;
        Ok(blob.data.to_vec())
    }

    /// Return commit log entries for a specific file, most recent first.
    ///
    /// Walks the commit graph and includes only commits where the file's
    /// blob OID actually changed (tree diff, not message string matching).
    /// Replaces the old approach of filtering `log(N)` by commit message
    /// substring, which was inaccurate and limited to N entries.
    pub fn log_for_file(&self, rel_path: &str, max_count: usize) -> Result<Vec<LogEntry>> {
        if !self.enabled {
            return Ok(Vec::new());
        }
        let repo = self.repo.lock();
        let head_id = repo.head_id()?.detach();
        let mut entries = Vec::new();
        let mut current_id: Option<ObjectId> = Some(head_id);
        let mut prev_blob: Option<ObjectId> = None;

        while let Some(id) = current_id {
            if entries.len() >= max_count {
                break;
            }
            let commit = repo.find_commit(id)?;
            let decoded = commit.decode()?;
            let tree_id = decoded.tree();
            let current_blob = Self::find_blob_in_tree(&repo, tree_id, rel_path).ok();
            if current_blob != prev_blob {
                let msg_ref = decoded.message();
                let msg = if let Some(body) = msg_ref.body {
                    format!("{}\n\n{}", msg_ref.title, body)
                } else {
                    msg_ref.title.to_string()
                };
                let timestamp = decoded.time().map(|t| t.to_string()).unwrap_or_default();
                let author = decoded
                    .author()
                    .map(|a| a.name.to_string())
                    .unwrap_or_default();
                let hex = id.to_hex().to_string();
                entries.push(LogEntry {
                    hash: hex.clone(),
                    short_hash: hex[..7].into(),
                    message: msg,
                    timestamp,
                    author,
                });
            }
            prev_blob = current_blob;
            current_id = decoded.parents().next();
        }
        Ok(entries)
    }

    // ── Diff API (Phase 3) ────────────────────────────────────────────────

    /// Compute the diff between two commits.
    pub fn diff_commits(&self, from_hash: &str, to_hash: &str) -> Result<CommitDiff> {
        let repo = self.repo.lock();
        let from_id = self.resolve_hash_inner(&repo, from_hash)?;
        let to_id = self.resolve_hash_inner(&repo, to_hash)?;

        let from_tree_id = Self::commit_tree_id(&repo, from_id)?;
        let to_tree_id = Self::commit_tree_id(&repo, to_id)?;

        let mut files = Vec::new();
        Self::diff_trees(&repo, from_tree_id, to_tree_id, "", &mut files)?;

        // Compute patches for modified/added files.
        for fd in &mut files {
            let old_data = fd
                .old_hash
                .as_ref()
                .and_then(|h| ObjectId::from_hex(h.as_bytes()).ok())
                .and_then(|id| repo.find_blob(id).ok())
                .map(|b| b.data.to_vec());
            let new_data = fd
                .new_hash
                .as_ref()
                .and_then(|h| ObjectId::from_hex(h.as_bytes()).ok())
                .and_then(|id| repo.find_blob(id).ok())
                .map(|b| b.data.to_vec());

            match (&old_data, &new_data) {
                (Some(old), Some(new)) => {
                    fd.patch = compute_unified_diff(old, new, &fd.path);
                }
                (None, Some(new)) => {
                    fd.patch = compute_unified_diff(&[], new, &fd.path);
                }
                _ => {}
            }
        }

        let stats = DiffStats {
            files_changed: files.len(),
            additions: files
                .iter()
                .filter_map(|f| f.patch.as_ref())
                .map(|p| {
                    p.lines()
                        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
                        .count()
                })
                .sum(),
            deletions: files
                .iter()
                .filter_map(|f| f.patch.as_ref())
                .map(|p| {
                    p.lines()
                        .filter(|l| l.starts_with('-') && !l.starts_with("---"))
                        .count()
                })
                .sum(),
        };

        Ok(CommitDiff {
            from_hash: from_id.to_hex().to_string(),
            to_hash: to_id.to_hex().to_string(),
            files,
            stats,
        })
    }

    // file_at_commit is defined above (line 692) — this duplicate removed.

    // ── Diff helpers ──────────────────────────────────────────────────────

    /// Recursively compare two trees and collect changed files.
    fn diff_trees(
        repo: &gix::Repository,
        old_tree: ObjectId,
        new_tree: ObjectId,
        prefix: &str,
        changes: &mut Vec<FileDiff>,
    ) -> Result<()> {
        let old_tree_obj = repo.find_tree(old_tree)?;
        let old_decoded = old_tree_obj.decode()?;
        let new_tree_obj = repo.find_tree(new_tree)?;
        let new_decoded = new_tree_obj.decode()?;

        let old_entries: std::collections::HashMap<&BStr, &gix::objs::tree::EntryRef<'_>> =
            old_decoded
                .entries
                .iter()
                .map(|e| (e.filename, e))
                .collect();
        let new_entries: std::collections::HashMap<&BStr, &gix::objs::tree::EntryRef<'_>> =
            new_decoded
                .entries
                .iter()
                .map(|e| (e.filename, e))
                .collect();

        // Detect additions and modifications.
        for (name, new_entry) in &new_entries {
            let path = format!("{prefix}{name}");
            match old_entries.get(name) {
                None => {
                    if new_entry.mode.is_tree() {
                        let empty = ObjectId::empty_tree(repo.object_hash());
                        Self::diff_trees(
                            repo,
                            empty,
                            new_entry.oid.to_owned(),
                            &format!("{path}/"),
                            changes,
                        )?;
                    } else {
                        changes.push(FileDiff {
                            path,
                            old_hash: None,
                            new_hash: Some(new_entry.oid.to_hex().to_string()),
                            kind: DiffKind::Added,
                            patch: None,
                        });
                    }
                }
                Some(old_entry) => {
                    if old_entry.oid == new_entry.oid {
                        continue;
                    }
                    if new_entry.mode.is_tree() && old_entry.mode.is_tree() {
                        Self::diff_trees(
                            repo,
                            old_entry.oid.to_owned(),
                            new_entry.oid.to_owned(),
                            &format!("{path}/"),
                            changes,
                        )?;
                    } else {
                        changes.push(FileDiff {
                            path,
                            old_hash: Some(old_entry.oid.to_hex().to_string()),
                            new_hash: Some(new_entry.oid.to_hex().to_string()),
                            kind: DiffKind::Modified,
                            patch: None,
                        });
                    }
                }
            }
        }

        // Detect deletions.
        for (name, old_entry) in &old_entries {
            if new_entries.contains_key(name) {
                continue;
            }
            let path = format!("{prefix}{name}");
            changes.push(FileDiff {
                path,
                old_hash: Some(old_entry.oid.to_hex().to_string()),
                new_hash: None,
                kind: DiffKind::Deleted,
                patch: None,
            });
        }

        Ok(())
    }

    // ── Verify / accessors ────────────────────────────────────────────────

    /// Verify repository integrity.
    pub fn verify(&self) -> Result<bool> {
        let repo = self.repo.lock();
        let refs = repo.references()?;
        for reference in refs.all()? {
            let _ = reference.map_err(|e| anyhow::anyhow!("ref verify: {e:#}"))?;
        }
        if repo.head_id().is_err() {
            tracing::debug!("verify: no HEAD yet (empty repository)");
        }
        Ok(true)
    }

    /// Whether auto-commit is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the root path of this git repository.
    pub fn root(&self) -> &Path {
        &self.root
    }

    // ── Private info builders ─────────────────────────────────────────────

    fn noop_commit(&self, ctx: &CommitContext, message: &str) -> Result<CommitInfo> {
        Ok(CommitInfo {
            hash: "(disabled)".into(),
            short_hash: "(dis)".into(),
            message: message.into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            author: ctx.author_name(),
        })
    }

    fn make_info(&self, id: &gix::Id, message: &str, author: &str) -> CommitInfo {
        let hex = id.to_hex().to_string();
        CommitInfo {
            short_hash: hex[..7].into(),
            hash: hex,
            message: message.into(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            author: author.into(),
        }
    }
}

// ── Free functions ──────────────────────────────────────────────────────────

/// Produce a simple unified-style diff between two byte sequences.
fn compute_unified_diff(old: &[u8], new: &[u8], path: &str) -> Option<String> {
    let old_str = std::str::from_utf8(old).ok()?;
    let new_str = std::str::from_utf8(new).ok()?;

    use similar::{ChangeTag, TextDiff};
    let diff = TextDiff::from_lines(old_str, new_str);

    let mut output = format!("--- a/{path}\n+++ b/{path}\n");
    for change in diff.iter_all_changes() {
        let prefix = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        output.push_str(&format!("{prefix}{change}"));
    }

    Some(output)
}

/// T16 (vault-rooted git): compute the git-relative path for a knowledge
/// file. `kb_root` is the knowledge/vault root; `git_root` is the layer's
/// repository root. When `kb_root` sits inside `git_root`, the stripped
/// prefix is prepended; when the two coincide (the vault-rooted default) —
/// or `kb_root` is outside the repo — `path` is returned as-is, so an
/// auto-commit of `<vault>/a/b.md` targets the existing `a/b.md` instead
/// of silently falling back to a `knowledge/` prefix.
pub fn rel_path(kb_root: &Path, git_root: &Path, path: &str) -> String {
    match kb_root.strip_prefix(git_root) {
        Ok(prefix) if !prefix.as_os_str().is_empty() => {
            format!("{}/{}", prefix.display(), path)
        }
        _ => path.to_string(),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, GitLayer) {
        let dir = tempfile::tempdir().unwrap();
        let layer = GitLayer::new(dir.path().to_path_buf(), true).unwrap();
        (dir, layer)
    }

    #[test]
    fn test_init_creates_repo() {
        let (dir, _) = setup();
        assert!(dir.path().join(".git").exists());
    }

    #[test]
    fn test_commit_file() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("test.json"), b"{\"hello\":1}").unwrap();
        let info = layer.commit_file("test.json", "test commit").unwrap();
        assert!(!info.hash.is_empty());
        assert_eq!(info.short_hash.len(), 7);
        assert_eq!(info.message, "test commit");
        assert!(info.hash.starts_with(&info.short_hash));
    }

    #[test]
    fn test_log_query() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("a.json"), b"1").unwrap();
        layer.commit_file("a.json", "first").unwrap();
        std::fs::write(dir.path().join("a.json"), b"2").unwrap();
        layer.commit_file("a.json", "second").unwrap();
        let log = layer.log(10).unwrap();
        assert!(log.len() >= 2);
        assert!(log[0].message.contains("second"));
    }

    #[test]
    fn test_tag_create_list() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("x.json"), b"1").unwrap();
        layer.commit_file("x.json", "tag test").unwrap();
        layer.tag("v1", "first tag").unwrap();
        let tags = layer.list_tags().unwrap();
        assert!(tags.iter().any(|t| t == "v1"));
    }

    #[test]
    fn test_disabled_noop() {
        let dir = tempfile::tempdir().unwrap();
        let layer = GitLayer::new(dir.path().to_path_buf(), false).unwrap();
        std::fs::write(dir.path().join("test.json"), b"1").unwrap();
        let info = layer.commit_file("test.json", "noop").unwrap();
        assert_eq!(info.hash, "(disabled)");
        assert_eq!(info.short_hash, "(dis)");
    }

    #[test]
    fn test_log_action() {
        let (dir, layer) = setup();
        layer
            .log_action("agent-A", "read", "file.txt", true, None)
            .unwrap();
        let audit_file = dir
            .path()
            .join("audit")
            .join(format!("{}.audit", chrono::Utc::now().format("%Y-%m")));
        assert!(audit_file.exists());
        let content = std::fs::read_to_string(&audit_file).unwrap();
        assert!(content.contains("agent-A"));
        assert!(content.contains("ALLOW"));
    }

    #[test]
    fn test_verify() {
        let (_, layer) = setup();
        assert!(layer.verify().unwrap());
    }

    #[test]
    fn test_remove_file() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("todelete.json"), b"1").unwrap();
        layer.commit_file("todelete.json", "add file").unwrap();
        std::fs::remove_file(dir.path().join("todelete.json")).unwrap();
        let info = layer.remove_file("todelete.json", "remove file").unwrap();
        assert!(!info.hash.is_empty());
        assert!(info.hash != "(disabled)");
    }

    #[test]
    fn test_commit_files_batch() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("a.json"), b"1").unwrap();
        std::fs::write(dir.path().join("b.json"), b"2").unwrap();
        let info = layer
            .commit_files(&["a.json", "b.json"], "batch commit")
            .unwrap();
        assert!(!info.hash.is_empty());
        assert_eq!(info.message, "batch commit");
    }

    #[test]
    fn test_restore_file() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("state.json"), b"v1").unwrap();
        let first = layer.commit_file("state.json", "v1").unwrap();
        std::fs::write(dir.path().join("state.json"), b"v2").unwrap();
        layer.commit_file("state.json", "v2").unwrap();
        layer.restore_file("state.json", &first.short_hash).unwrap();
        let content = std::fs::read_to_string(dir.path().join("state.json")).unwrap();
        assert_eq!(content, "v1");
    }

    #[test]
    fn test_gitignore_created() {
        let (dir, _) = setup();
        assert!(dir.path().join(".gitignore").exists());
        let content = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(content.contains("Oxios"));
    }

    // ── B1: Signature timestamps ──────────────────────────────────────────

    #[test]
    fn test_signature_timestamps_are_fresh() {
        // B1 fix: each Signature captures its own timestamp at creation time,
        // not a process-wide cached value. Verify that signatures created 1s
        // apart produce different timestamps.
        let sig1 = Signature::new("a", "a@a");
        assert!(!sig1.time.is_empty());

        std::thread::sleep(std::time::Duration::from_millis(1100));
        let sig3 = Signature::new("c", "c@c");
        assert_ne!(
            sig1.time, sig3.time,
            "Signature created 1s later must have a different timestamp"
        );
    }

    // ── D1: Agent identification ──────────────────────────────────────────

    #[test]
    fn test_commit_file_with_agent_context() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("agent_work.json"), b"{\"result\":42}").unwrap();

        let agent_id = uuid::Uuid::new_v4();
        let ctx = CommitContext::agent(agent_id);
        layer
            .commit_file_with("agent_work.json", "agent did work", ctx)
            .unwrap();

        let log = layer.log(10).unwrap();
        let agent_commit = log
            .iter()
            .find(|e| e.message.contains("agent did work"))
            .expect("should find agent commit");

        let expected_author = format!("agent-{}", &agent_id.to_string()[..8]);
        assert_eq!(agent_commit.author, expected_author);
    }

    #[test]
    fn test_commit_file_with_tag() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("audit.json"), b"{\"event\":\"test\"}").unwrap();

        let ctx = CommitContext::tagged("audit");
        let info = layer
            .commit_file_with("audit.json", "flush audit trail", ctx)
            .unwrap();

        assert!(info.message.contains("[audit]"));
        assert!(info.message.contains("flush audit trail"));
    }

    #[test]
    fn test_default_context_is_oxios() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("sys.json"), b"1").unwrap();

        let info = layer
            .commit_file_with("sys.json", "system commit", CommitContext::default())
            .unwrap();

        assert_eq!(info.author, "oxios");
    }

    #[test]
    fn test_commit_context_author_name() {
        assert_eq!(CommitContext::default().author_name(), "oxios");
        assert_eq!(CommitContext::system().author_name(), "oxios");

        let id = uuid::Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap();
        assert_eq!(CommitContext::agent(id).author_name(), "agent-aaaaaaaa");

        assert_eq!(CommitContext::tagged("memory").author_name(), "oxios");
    }

    #[test]
    fn test_commit_context_message_prefix() {
        assert!(CommitContext::default().message_prefix().is_empty());
        assert_eq!(CommitContext::tagged("audit").message_prefix(), "[audit] ");

        assert_eq!(
            CommitContext::tagged("memory").message_prefix(),
            "[memory] "
        );
    }

    #[test]
    fn test_commit_files_with_context() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("a.json"), b"1").unwrap();
        std::fs::write(dir.path().join("b.json"), b"2").unwrap();

        let agent_id = uuid::Uuid::new_v4();
        let ctx = CommitContext::agent(agent_id);
        let info = layer
            .commit_files_with(&["a.json", "b.json"], "batch agent work", ctx)
            .unwrap();

        let expected_author = format!("agent-{}", &agent_id.to_string()[..8]);
        assert_eq!(info.author, expected_author);
    }

    #[test]
    fn test_backward_compat_commit_file_is_oxios() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("compat.json"), b"1").unwrap();
        let info = layer.commit_file("compat.json", "compat check").unwrap();
        assert_eq!(info.author, "oxios");
    }

    // ── B2: Nested path restore ───────────────────────────────────────────

    #[test]
    fn test_restore_nested_file() {
        let (dir, layer) = setup();

        // Create a nested file via log_action.
        layer
            .log_action("agent-X", "write", "secret.txt", true, None)
            .unwrap();

        let audit_rel = format!("audit/{}.audit", chrono::Utc::now().format("%Y-%m"));
        let audit_path = dir.path().join(&audit_rel);
        assert!(audit_path.exists(), "audit file should exist");

        // Overwrite it.
        let _original = std::fs::read_to_string(&audit_path).unwrap();
        std::fs::write(&audit_path, "CORRUPTED").unwrap();
        layer.commit_file(&audit_rel, "corrupt").unwrap();

        // Find the audit commit and restore.
        let log = layer.log(10).unwrap();
        let audit_commit = log
            .iter()
            .find(|e| e.message.contains("audit: agent-X"))
            .expect("should find audit commit");

        layer
            .restore_file(&audit_rel, &audit_commit.short_hash)
            .unwrap();

        let restored = std::fs::read_to_string(&audit_path).unwrap();
        assert!(restored.contains("agent-X"));
        assert!(!restored.contains("CORRUPTED"));
    }

    // ── D3b: list_tags filter ─────────────────────────────────────────────

    #[test]
    fn test_list_tags_excludes_non_tags() {
        let (dir, layer) = setup();
        std::fs::write(dir.path().join("t.json"), b"1").unwrap();
        layer.commit_file("t.json", "for tag").unwrap();
        layer.tag("release-v1", "first release").unwrap();
        let tags = layer.list_tags().unwrap();
        assert!(tags.iter().any(|t| t == "release-v1"));
        assert!(tags.iter().all(|t| t != "main" && t != "HEAD"));
    }

    // ── Phase 3: Diff ─────────────────────────────────────────────────────

    #[test]
    fn test_diff_added_file() {
        let (dir, layer) = setup();
        let first = layer.log(1).unwrap()[0].hash.clone();

        std::fs::write(dir.path().join("new.txt"), b"hello\n").unwrap();
        let info = layer.commit_file("new.txt", "add file").unwrap();

        let diff = layer.diff_commits(&first, &info.hash).unwrap();
        assert!(
            diff.files
                .iter()
                .any(|f| f.path == "new.txt" && f.kind == DiffKind::Added)
        );
    }

    #[test]
    fn test_diff_modified_file() {
        let (dir, layer) = setup();

        std::fs::write(dir.path().join("data.txt"), b"v1\n").unwrap();
        let first = layer.commit_file("data.txt", "v1").unwrap();

        std::fs::write(dir.path().join("data.txt"), b"v2\n").unwrap();
        let second = layer.commit_file("data.txt", "v2").unwrap();

        let diff = layer.diff_commits(&first.hash, &second.hash).unwrap();
        assert!(
            diff.files
                .iter()
                .any(|f| f.path == "data.txt" && f.kind == DiffKind::Modified)
        );

        let patch = diff
            .files
            .iter()
            .find(|f| f.path == "data.txt")
            .unwrap()
            .patch
            .as_ref()
            .expect("should have patch");
        assert!(patch.contains("-v1"));
        assert!(patch.contains("+v2"));
    }

    #[test]
    fn test_diff_deleted_file() {
        let (dir, layer) = setup();

        std::fs::write(dir.path().join("temp.txt"), b"bye\n").unwrap();
        let first = layer.commit_file("temp.txt", "add temp").unwrap();

        std::fs::remove_file(dir.path().join("temp.txt")).unwrap();
        let second = layer.remove_file("temp.txt", "remove temp").unwrap();

        let diff = layer.diff_commits(&first.hash, &second.hash).unwrap();
        assert!(
            diff.files
                .iter()
                .any(|f| f.path == "temp.txt" && f.kind == DiffKind::Deleted)
        );
    }

    #[test]
    fn test_file_at_commit() {
        let (dir, layer) = setup();

        std::fs::write(dir.path().join("state.json"), b"{\"v\":1}").unwrap();
        let first = layer.commit_file("state.json", "v1").unwrap();

        std::fs::write(dir.path().join("state.json"), b"{\"v\":2}").unwrap();
        layer.commit_file("state.json", "v2").unwrap();

        let content = layer
            .file_at_commit("state.json", &first.short_hash)
            .unwrap();
        assert_eq!(content, b"{\"v\":1}");
    }

    // ── T16 vault-rooted git: rel_path helper + P1 closure ──────────────

    /// `rel_path` must compute a path relative to `git_root` without an
    /// empty-string `kb_root.strip_prefix(git_root)` fallback. When
    /// `kb_root == git_root` (the new default — vault-rooted repo) the
    /// `path` is returned as-is, so an auto-commit of `<vault>/a/b.md`
    /// targets the existing `a/b.md` file instead of silently falling
    /// back to `knowledge/a/b.md`.
    #[test]
    fn rel_path_empty_prefix_is_path_as_is() {
        let root = Path::new("/v");
        assert_eq!(rel_path(root, root, "a/b.md"), "a/b.md");
        assert_eq!(
            rel_path(Path::new("/w/v"), Path::new("/w"), "a/b.md"),
            "v/a/b.md"
        );
    }

    /// P1 closure: a vault-rooted `GitLayer` (i.e. `GitLayer::new(kb_root, …)`
    /// with `kb_root` OUTSIDE any workspace) must accept `commit_file`,
    /// expose history via `log_for_file`, and restore via `restore_file`
    /// using paths computed by `rel_path`. This is the round-trip T15
    /// review flagged as silently broken under the default config
    /// (`auto_commit = true`, `kb_root = ~/.oxi/vault`).
    #[test]
    fn vault_rooted_commit_history_restore_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        // Vault is its own git root (NOT nested inside another repo).
        let vault = dir.path().to_path_buf();
        let layer = GitLayer::new(vault.clone(), true).unwrap();

        // Seed a knowledge file at the vault root.
        let rel = "notes/hello.md";
        std::fs::create_dir_all(vault.join("notes")).unwrap();
        std::fs::write(vault.join(rel), b"# v1\n").unwrap();

        // Compute the path the way the kernel does — no `knowledge/` prefix.
        let git_rel = rel_path(&vault, layer.root(), rel);
        assert_eq!(git_rel, rel);

        let v1 = layer
            .commit_file(&git_rel, "knowledge: update notes/hello.md")
            .unwrap();
        assert_ne!(v1.hash, "(disabled)");

        // History — kernel route equivalent of /history.
        let log = layer.log_for_file(&git_rel, 50).unwrap();
        assert!(
            log.iter().any(|e| e.hash == v1.hash),
            "log_for_file must surface the just-committed entry"
        );

        // Mutate, commit v2, then restore v1.
        std::fs::write(vault.join(rel), b"# v2\n").unwrap();
        let v2 = layer
            .commit_file(&git_rel, "knowledge: update notes/hello.md")
            .unwrap();
        assert_ne!(v2.hash, v1.hash);

        layer.restore_file(&git_rel, &v1.short_hash).unwrap();
        let restored = std::fs::read_to_string(vault.join(rel)).unwrap();
        assert_eq!(restored, "# v1\n");
    }
}
