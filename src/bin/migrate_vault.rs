//! One-time ecosystem vault migration binary (Task 19, design §7 of
//! `docs/designs/2026-08-20-vault-unification-design.md`).
//!
//! Joins two pre-unification sources into the shared ecosystem vault:
//!
//! 1. The oximemo pre-unification default vault
//!    (`~/Library/Application Support/com.oximemo.app/vault`), moved with
//!    `+++`TOML -> `---`YAML frontmatter conversion.
//! 2. The oxios knowledge tree (`~/.oxios/workspace/knowledge/`), moved
//!    with system-path/`Document` classification per §6: documents gain
//!    `id`/`created`/`updated` (RFC-022 notes keep their block and their
//!    `oxios:` table), system files move verbatim and stay
//!    frontmatter-less.
//!
//! Default invocation is a **dry-run**: the full plan (every move,
//! conversion, and synthesized id) is printed and nothing on disk is
//! touched. `--apply` is required to write: backup first, then the
//! moves, then the git history import (`git filter-repo
//! --subdirectory-filter knowledge`, fresh-init fallback), then the
//! workspace removal commit.
//!
//! Both-exist divergence (source and target populated with different
//! bytes) is a hard `MergeRequired`-style error listing every offender —
//! nothing is silently overwritten. Already-migrated state (source
//! missing AND target exists) is tolerated and reported.

#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::path::{Path, PathBuf};

use oxi_frontmatter::{NoteFormat, Parsed, Table, Value, atomic_write, emit, parse};

// ---------------------------------------------------------------------------
// Public shape (used by `main` and the in-file tests)
// ---------------------------------------------------------------------------

/// Classification of a vault-relative path per the §6 policy matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Moves verbatim, never carries frontmatter (system files, non-md).
    System,
    /// First-class document; gains/carries `id`/`created`/`updated`.
    Document,
}

/// Where a migrated document's `created` timestamp came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CreatedSource {
    /// The source frontmatter already carried `created`.
    Existing,
    /// The file's first git commit in the workspace repo.
    Git,
    /// File mtime (no git history for the path).
    Mtime,
    /// Neither git nor mtime available; migration time.
    Now,
}

/// Result of converting a knowledge document.
#[derive(Debug, Clone)]
struct ConvertedDoc {
    /// Full converted file content (frontmatter + body).
    content: String,
    /// Whether the source had a frontmatter block at all.
    had_fm: bool,
    /// The document's id (existing or synthesized).
    id: String,
    /// Whether `id` was synthesized by the migration.
    id_synthesized: bool,
}

/// One planned file operation.
#[derive(Debug, Clone)]
enum FileAction {
    /// Copy byte-identical (system files, non-md, symlinks, v3 `.html`).
    MoveVerbatim { src: PathBuf, rel: String },
    /// oximemo v3 `+++`TOML -> v4 `---`YAML conversion. `content` is the
    /// exact converted output the apply phase writes (and what the
    /// conflict check compared against the target).
    ConvertToml {
        src: PathBuf,
        rel: String,
        content: String,
    },
    /// Knowledge document gaining/extending its v4 frontmatter block.
    ConvertDoc {
        src: PathBuf,
        rel: String,
        content: String,
        created: String,
        created_src: CreatedSource,
        id: String,
        id_synthesized: bool,
        had_fm: bool,
    },
    /// Target already holds byte-identical (converted) content; only the
    /// source removal remains (idempotent re-run).
    AlreadySame { src: PathBuf, rel: String },
}

impl FileAction {
    fn rel(&self) -> &str {
        match self {
            FileAction::MoveVerbatim { rel, .. }
            | FileAction::ConvertToml { rel, .. }
            | FileAction::ConvertDoc { rel, .. }
            | FileAction::AlreadySame { rel, .. } => rel,
        }
    }
}

/// Tree-level status for one source root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeStatus {
    /// Source root absent and target absent too — nothing to do.
    NotPresent,
    /// Source root absent but target exists — earlier run won; continue.
    AlreadyMigrated,
    /// Source present; `usize` files planned.
    Planned(usize),
}

/// The complete migration plan (dry-run artifact; apply executes it).
#[derive(Debug, Clone)]
struct Plan {
    oximemo: TreeStatus,
    knowledge: TreeStatus,
    actions: Vec<FileAction>,
    /// Both-exist divergences (source vs *converted* target bytes).
    conflicts: Vec<String>,
    /// Malformed source frontmatter / unreadable entries — hard errors.
    malformed: Vec<(PathBuf, String)>,
    /// `~/.oxi/config.toml` lacks a `[vault]` table (would be written).
    vault_config_needed: bool,
    /// How many of `actions` (in order) come from the oximemo tree; the
    /// rest are knowledge-tree actions.
    n_oximemo: usize,
}

/// All paths the migration resolves between (injected for tests).
#[derive(Debug, Clone)]
struct Paths {
    workspace: PathBuf,
    knowledge_src: PathBuf,
    oximemo_old: PathBuf,
    oxi_config: PathBuf,
    vault_dest: PathBuf,
    backups_root: PathBuf,
}

/// What `--apply` actually did.
#[derive(Debug, Clone, Default)]
struct ApplyStats {
    backup_dirs: Vec<PathBuf>,
    moved_verbatim: usize,
    converted_toml: usize,
    converted_doc: usize,
    skipped_identical: usize,
    git_history: String,
    workspace_removal: String,
    notes: Vec<String>,
}

/// Result of one `run` — the plan always, apply stats when applied.
#[derive(Debug, Clone)]
struct Outcome {
    plan: Plan,
    apply: Option<ApplyStats>,
}

// ---------------------------------------------------------------------------
// Pure functions under test (Task 19 brief, step 1)
// ---------------------------------------------------------------------------

fn classify(rel: &str) -> Kind {
    if oxios_markdown::frontformat::is_system_path(rel) {
        Kind::System
    } else {
        Kind::Document
    }
}

/// Split a v3 markdown note into (TOML frontmatter text, body).
/// `Ok(None)` = no `+++` fence (system or foreign file).
fn split_v3_markdown(content: &str) -> Result<Option<(String, String)>, String> {
    let first_line_end = content.find('\n').unwrap_or(content.len());
    if content[..first_line_end].trim_end_matches('\r') != "+++" {
        return Ok(None);
    }
    let after_open = match content.find('\n') {
        Some(i) => i + 1,
        None => return Err("empty v3 frontmatter (unclosed fence)".into()),
    };
    let mut pos = after_open;
    while pos < content.len() {
        let line_end = content[pos..]
            .find('\n')
            .map(|i| pos + i)
            .unwrap_or(content.len());
        if content[pos..line_end].trim_end_matches('\r') == "+++" {
            let toml_text = content[after_open..pos].to_string();
            let body_start = if line_end < content.len() {
                line_end + 1
            } else {
                content.len()
            };
            let mut body = &content[body_start..];
            if body.starts_with('\n') {
                body = &body[1..];
            }
            return Ok(Some((toml_text, body.to_string())));
        }
        pos = if line_end < content.len() {
            line_end + 1
        } else {
            break;
        };
    }
    Err("missing closing `+++` delimiter".into())
}

/// v4 canonical core keys. A stray v3 key colliding with one of these is
/// rejected so it cannot silently overwrite the typed mapping.
const CORE_KEYS: &[&str] = &["id", "created", "updated", "favorite", "deleted"];

/// Map a parsed v3 TOML table onto the v4 table shape:
/// `id`/`created_at`/`updated_at`/`favorite`/`deleted_at` ->
/// `id`/`created`/`updated`/`favorite`/`deleted`; `hash` dropped
/// (recomputed from the body on read) and `tags` dropped (body-derived
/// in v4); every other key and app table kept.
fn map_v3_to_v4(raw: &toml::Table) -> Result<Table, String> {
    let mut out = Table::new();

    let id = match raw.get("id") {
        Some(toml::Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(format!(
                "field `id` must be a string, got {}",
                toml_kind(other)
            ));
        }
        None => return Err("missing required field `id`".into()),
    };
    out.insert("id".to_string(), Value::Str(id));
    out.insert(
        "created".to_string(),
        Value::Str(rfc3339_field(raw, "created_at")?),
    );
    out.insert(
        "updated".to_string(),
        Value::Str(rfc3339_field(raw, "updated_at")?),
    );
    let favorite = match raw.get("favorite") {
        Some(toml::Value::Boolean(b)) => *b,
        None => false,
        Some(other) => {
            return Err(format!(
                "field `favorite` must be a boolean, got {}",
                toml_kind(other)
            ));
        }
    };
    out.insert("favorite".to_string(), Value::Bool(favorite));
    if raw.contains_key("deleted_at") {
        out.insert(
            "deleted".to_string(),
            Value::Str(rfc3339_field(raw, "deleted_at")?),
        );
    }
    // Carry over unknown keys + app tables, but refuse to silently
    // overwrite a core v4 field the typed mapping already produced.
    for (key, value) in raw {
        if matches!(
            key.as_str(),
            "id" | "created_at" | "updated_at" | "favorite" | "deleted_at" | "hash" | "tags"
        ) {
            continue;
        }
        if CORE_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "stray field `{key}` collides with the v4 mapping; remove or rename it (the typed value is authoritative)"
            ));
        }
        if let Some(converted) =
            convert_value(value).map_err(|e| format!("field `{key}` cannot be converted: {e}"))?
        {
            out.insert(key.clone(), converted);
        }
    }
    Ok(out)
}

/// Convert a full v3 oximemo note (frontmatter + body) to v4 content.
/// The brief's tested entry point; the planner calls [`convert_v3_parts`]
/// on its already-split parts so the file is read exactly once.
#[cfg_attr(not(test), allow(dead_code))]
fn convert_toml_note(content: &str) -> Result<String, String> {
    let (toml_text, body) = split_v3_markdown(content)?
        .ok_or_else(|| "no v3 `+++` frontmatter fence found".to_string())?;
    convert_v3_parts(&toml_text, &body)
}

/// Convert already-split v3 parts (TOML text + body) to v4 content.
fn convert_v3_parts(toml_text: &str, body: &str) -> Result<String, String> {
    let raw: toml::Table =
        toml::from_str(toml_text).map_err(|e| format!("invalid TOML frontmatter: {e}"))?;
    let table = map_v3_to_v4(&raw)?;
    emit_guarded(&table, body)
}

/// Convert a knowledge document: carry the existing table (RFC-022
/// `oxios:` tables and unknown keys survive), synthesize `id`/`created`/
/// `updated` as needed. `created_hint` (git first-commit / mtime) wins
/// over `now` when the source lacks `created`.
fn convert_document(
    content: &str,
    created_hint: Option<&str>,
    now_rfc3339: &str,
) -> Result<ConvertedDoc, String> {
    let (mut table, body, had_fm) =
        match oxi_frontmatter::parse(content, NoteFormat::Markdown).map_err(|e| e.to_string())? {
            Parsed::Memo { table, body } => (table, body, true),
            Parsed::BodyOnly { body } => (Table::new(), body, false),
        };
    let id_synthesized = !table.contains_key("id");
    let id = match table.get("id") {
        Some(Value::Str(s)) => s.clone(),
        Some(other) => {
            return Err(format!("field `id` must be a string, got {other:?}"));
        }
        None => {
            let id = uuid::Uuid::now_v7().to_string();
            table.insert("id".to_string(), Value::Str(id.clone()));
            id
        }
    };
    if !table.contains_key("created") {
        let ts = created_hint.unwrap_or(now_rfc3339).to_string();
        table.insert("created".to_string(), Value::Str(ts));
    }
    if !table.contains_key("updated") {
        table.insert("updated".to_string(), Value::Str(now_rfc3339.to_string()));
    }
    let content = emit_guarded(&table, &body)?;
    Ok(ConvertedDoc {
        content,
        had_fm,
        id,
        id_synthesized,
    })
}

/// Emit + round-trip guard: never hand back a document the v4 parser
/// would read back differently (unknown keys, app tables, body — all of
/// it). Mirrors the oximemo-side v3 -> v4 bridge.
fn emit_guarded(table: &Table, body: &str) -> Result<String, String> {
    let out = emit(table, body, NoteFormat::Markdown);
    match parse(&out, NoteFormat::Markdown) {
        Ok(Parsed::Memo {
            table: read_back,
            body: read_body,
        }) if read_back == *table && read_body == body => Ok(out),
        Ok(_) => Err("converted document does not round-trip through the v4 grammar".into()),
        Err(e) => Err(format!("emitted v4 frontmatter does not re-parse: {e}")),
    }
}

/// Extract + validate an RFC3339 timestamp field (TOML datetime or
/// quoted string; v3's serde wrote the offset form).
fn rfc3339_field(raw: &toml::Table, name: &str) -> Result<String, String> {
    let Some(value) = raw.get(name) else {
        return Err(format!("missing required field `{name}`"));
    };
    let s = match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Datetime(d) => d.to_string(),
        other => {
            return Err(format!(
                "field `{name}` must be an RFC3339 timestamp, got {}",
                toml_kind(other)
            ));
        }
    };
    chrono::DateTime::parse_from_rfc3339(&s)
        .map_err(|e| format!("field `{name}` is not RFC3339 ({s}): {e}"))?;
    Ok(s)
}

/// Convert one unknown v3 value to the v4 grammar. `Ok(None)` means the
/// key is dropped: empty arrays (`key = []`) have no representable v4
/// form.
fn convert_value(value: &toml::Value) -> Result<Option<Value>, String> {
    Ok(match value {
        toml::Value::String(s) => Some(Value::Str(s.clone())),
        toml::Value::Integer(i) => Some(Value::Str(i.to_string())),
        toml::Value::Float(f) => Some(Value::Str(f.to_string())),
        toml::Value::Boolean(b) => Some(Value::Bool(*b)),
        toml::Value::Datetime(d) => Some(Value::Str(d.to_string())),
        toml::Value::Array(items) => {
            if items.is_empty() {
                None
            } else {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let s = match item {
                        toml::Value::String(s) => s.clone(),
                        toml::Value::Integer(i) => i.to_string(),
                        toml::Value::Float(f) => f.to_string(),
                        toml::Value::Boolean(b) => b.to_string(),
                        toml::Value::Datetime(d) => d.to_string(),
                        other => {
                            return Err(format!("nested {} inside an array", toml_kind(other)));
                        }
                    };
                    out.push(s);
                }
                Some(Value::Array(out))
            }
        }
        toml::Value::Table(sub) => {
            let mut map = Table::new();
            for (k, v) in sub {
                if let Some(converted) = convert_value(v)? {
                    map.insert(k.clone(), converted);
                }
            }
            Some(Value::Map(map))
        }
    })
}

/// Human-readable TOML kind for error messages.
fn toml_kind(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "a string",
        toml::Value::Integer(_) => "an integer",
        toml::Value::Float(_) => "a float",
        toml::Value::Boolean(_) => "a boolean",
        toml::Value::Datetime(_) => "a datetime",
        toml::Value::Array(_) => "an array",
        toml::Value::Table(_) => "a table",
    }
}

/// Oldest (first) author-date line of `git log --diff-filter=A
/// --format=%aI` output; git lists newest first, so the first commit
/// that added the file is the **last** non-empty line.
fn first_commit_date(output: &str) -> Option<&str> {
    output
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::trim)
}

/// created := git first-commit date | mtime | now.
fn created_from(git_log: Option<&str>, mtime: Option<&str>, now: &str) -> String {
    if let Some(git) = git_log.and_then(first_commit_date) {
        return git.to_string();
    }
    if let Some(mt) = mtime {
        return mt.to_string();
    }
    now.to_string()
}

/// The file's first-adding commit author date in the workspace repo
/// (knowledge subtree). `None` when the repo or git is unavailable or
/// the path was never committed.
fn git_first_commit(workspace: &Path, rel: &str) -> Option<String> {
    if !workspace.join(".git").exists() {
        return None;
    }
    let out = std::process::Command::new("git")
        .current_dir(workspace)
        .args([
            "log",
            "--diff-filter=A",
            "--format=%aI",
            "--",
            &format!("knowledge/{rel}"),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    first_commit_date(&s).map(str::to_string)
}

/// File mtime as RFC3339 (created fallback when git knows nothing).
fn mtime_rfc3339(p: &Path) -> Option<String> {
    use std::time::UNIX_EPOCH;
    let md = std::fs::metadata(p).ok()?;
    let modt = md.modified().ok()?;
    let dur = modt.duration_since(UNIX_EPOCH).ok()?;
    chrono::DateTime::from_timestamp(dur.as_secs() as i64, dur.subsec_nanos())
        .map(|dt| dt.to_rfc3339())
}

fn resolve_paths(home: &Path) -> Result<Paths, String> {
    if !home.is_absolute() {
        return Err(format!("home must be an absolute path: {}", home.display()));
    }
    let oxios_home = home.join(".oxios");
    let oxi_config = home.join(".oxi").join("config.toml");
    let vault_dest =
        read_oxi_vault_path(&oxi_config, home).unwrap_or_else(|| home.join(".oxi").join("vault"));

    // Round-1 review #2: `~/.oxios/config.toml [kernel].knowledge_root`
    // is tier 1 of the kernel's resolution chain — if it points anywhere
    // else, oxios would read a vault this migration never wrote to and
    // the documents would vanish from the app (design §5.4: loud, never
    // silent divergence). Refuse and tell the user how to reconcile.
    if let Some(override_path) = read_kernel_knowledge_root(&oxios_home.join("config.toml"), home)
        && override_path != vault_dest
    {
        return Err(format!(
            "config conflict: ~/.oxios/config.toml sets kernel.knowledge_root to {} but the migration target resolves to {}. Both apps must share one vault — clear the override (or point it at the vault) and re-run.",
            override_path.display(),
            vault_dest.display()
        ));
    }

    Ok(Paths {
        workspace: oxios_home.join("workspace"),
        knowledge_src: oxios_home.join("workspace/knowledge"),
        oximemo_old: home
            .join("Library")
            .join("Application Support")
            .join("com.oximemo.app")
            .join("vault"),
        oxi_config,
        vault_dest,
        backups_root: oxios_home.join("backups"),
    })
}

/// Read `[vault].path` from `~/.oxi/config.toml` (best-effort; mirrors
/// the kernel's resolution chain: explicit path > default `~/.oxi/vault`).
fn read_oxi_vault_path(cfg: &Path, home: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(cfg).ok()?;
    let val: toml::Value = toml::from_str(&text).ok()?;
    let p = val.get("vault")?.get("path")?.as_str()?;
    if p.trim().is_empty() {
        return None;
    }
    Some(expand_home(p, home))
}

/// Read `[kernel].knowledge_root` from `~/.oxios/config.toml`
/// (best-effort; mirrors the kernel's tier-1 override).
fn read_kernel_knowledge_root(cfg: &Path, home: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(cfg).ok()?;
    let val: toml::Value = toml::from_str(&text).ok()?;
    let p = val.get("kernel")?.get("knowledge_root")?.as_str()?;
    if p.trim().is_empty() {
        return None;
    }
    Some(expand_home(p, home))
}

fn expand_home(p: &str, home: &Path) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(p)
    }
}

/// Recursively collect (rel, path) file entries under `root`, skipping
/// `.git` directories. Unreadable entries are collected as malformed —
/// nothing is silently skipped.
fn walk_files(
    root: &Path,
    out: &mut Vec<(String, PathBuf)>,
    malformed: &mut Vec<(PathBuf, String)>,
) {
    walk_files_rec(root, root, out, malformed);
}

/// `top` anchors the relative paths; `dir` is the recursion cursor.
fn walk_files_rec(
    top: &Path,
    dir: &Path,
    out: &mut Vec<(String, PathBuf)>,
    malformed: &mut Vec<(PathBuf, String)>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            malformed.push((dir.to_path_buf(), format!("cannot read directory: {e}")));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                malformed.push((
                    dir.to_path_buf(),
                    format!("directory entry unreadable: {e}"),
                ));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(e) => {
                malformed.push((path.clone(), format!("file type unreadable: {e}")));
                continue;
            }
        };
        if ft.is_dir() {
            walk_files_rec(top, &path, out, malformed);
        } else {
            let rel = path
                .strip_prefix(top)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, path));
        }
    }
}

fn build_plan(paths: &Paths, now_rfc3339: &str) -> Result<Plan, String> {
    let mut plan = Plan {
        oximemo: TreeStatus::NotPresent,
        knowledge: TreeStatus::NotPresent,
        actions: Vec::new(),
        conflicts: Vec::new(),
        malformed: Vec::new(),
        vault_config_needed: oxi_vault_table_absent(&paths.oxi_config),
        n_oximemo: 0,
    };

    // ── oximemo pre-unification default vault ──
    if paths.oximemo_old.is_dir() {
        let mut files = Vec::new();
        walk_files(&paths.oximemo_old, &mut files, &mut plan.malformed);
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let count = files.len();
        for (rel, src) in files {
            plan_oximemo_file(&mut plan, paths, &rel, src);
        }
        plan.oximemo = TreeStatus::Planned(count);
        plan.n_oximemo = plan.actions.len();
    } else if paths.vault_dest.is_dir() {
        plan.oximemo = TreeStatus::AlreadyMigrated;
    }

    // ── oxios knowledge tree ──
    if paths.knowledge_src.is_dir() {
        let mut files = Vec::new();
        walk_files(&paths.knowledge_src, &mut files, &mut plan.malformed);
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let count = files.len();
        for (rel, src) in files {
            plan_knowledge_file(&mut plan, paths, &rel, src, now_rfc3339);
        }
        plan.knowledge = TreeStatus::Planned(count);
    } else if paths.vault_dest.is_dir() {
        plan.knowledge = TreeStatus::AlreadyMigrated;
    }

    // ── cross-tree collision (round-1 review #1) ──
    // Both trees plan into `vault_dest.join(rel)`; the on-disk dest
    // checks cannot see a collision between the two SOURCE trees, and
    // apply would let the knowledge conversion silently overwrite the
    // oximemo one (then delete both sources). The same rel in both
    // trees is a MergeRequired hard error — never a silent overwrite.
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for action in &plan.actions {
        let rel = action.rel().to_string();
        if !seen.insert(rel.clone()) {
            plan.conflicts.push(format!(
                "{rel}: planned by BOTH the oximemo and knowledge source trees (MergeRequired; merge by hand or remove one side)"
            ));
        }
    }

    Ok(plan)
}

/// Plan one file from the oximemo old vault: `.md` notes with a v3
/// `+++` fence convert; everything else (machinery, `.html`, images)
/// moves verbatim.
fn plan_oximemo_file(plan: &mut Plan, paths: &Paths, rel: &str, src: PathBuf) {
    let dest = paths.vault_dest.join(rel);

    if rel.ends_with(".md")
        && let Ok(content) = std::fs::read_to_string(&src)
    {
        // Single read (round-1 review #5): re-reading between the fence
        // probe and the conversion raced concurrent rewrites into a
        // panic on `.expect`. Everything below derives from `content`.
        match split_v3_markdown(&content) {
            Err(e) => {
                plan.malformed.push((src, e));
                return;
            }
            Ok(None) => {} // no v3 fence — verbatim below
            Ok(Some((toml_text, body))) => {
                let raw: toml::Table = match toml::from_str(&toml_text) {
                    Ok(r) => r,
                    Err(e) => {
                        plan.malformed
                            .push((src, format!("invalid TOML frontmatter: {e}")));
                        return;
                    }
                };
                let v3_id = raw.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                let converted = match convert_v3_parts(&toml_text, &body) {
                    Ok(c) => c,
                    Err(e) => {
                        plan.malformed.push((src, e));
                        return;
                    }
                };
                if dest.exists() {
                    match v3_dest_matches(&dest, v3_id, &body) {
                        Ok(true) => {
                            plan.actions.push(FileAction::AlreadySame {
                                src,
                                rel: rel.to_string(),
                            });
                        }
                        Ok(false) => plan.conflicts.push(format!(
                            "{rel}: oximemo source and vault target both exist with different content (MergeRequired)"
                        )),
                        Err(reason) => plan
                            .conflicts
                            .push(format!("{rel}: vault target unreadable ({reason}); MergeRequired")),
                    }
                } else {
                    plan.actions.push(FileAction::ConvertToml {
                        src,
                        rel: rel.to_string(),
                        content: converted,
                    });
                }
                return;
            }
        }
    }

    plan_verbatim(plan, src, rel, &dest);
}

/// Plan one knowledge file: §6 classification decides verbatim vs
/// document conversion.
fn plan_knowledge_file(plan: &mut Plan, paths: &Paths, rel: &str, src: PathBuf, now: &str) {
    let dest = paths.vault_dest.join(rel);

    if matches!(classify(rel), Kind::System) {
        plan_verbatim(plan, src, rel, &dest);
        return;
    }

    let content = match std::fs::read_to_string(&src) {
        Ok(c) => c,
        Err(e) => {
            plan.malformed
                .push((src, format!("cannot read as UTF-8: {e}")));
            return;
        }
    };
    let (has_created, src_body) = match oxi_frontmatter::parse(&content, NoteFormat::Markdown) {
        Ok(Parsed::Memo { table, body }) => (table.contains_key("created"), body),
        Ok(Parsed::BodyOnly { body }) => (false, body),
        Err(e) => {
            plan.malformed
                .push((src, format!("malformed frontmatter: {e}")));
            return;
        }
    };

    let (created, created_src, hint) = if has_created {
        (String::new(), CreatedSource::Existing, None)
    } else {
        let git = git_first_commit(&paths.workspace, rel);
        let mt = mtime_rfc3339(&src);
        let created = created_from(git.as_deref(), mt.as_deref(), now);
        let src = if git.is_some() {
            CreatedSource::Git
        } else if mt.is_some() {
            CreatedSource::Mtime
        } else {
            CreatedSource::Now
        };
        let hint = Some(created.clone());
        (created, src, hint)
    };

    let conv = match convert_document(&content, hint.as_deref(), now) {
        Ok(c) => c,
        Err(e) => {
            plan.malformed.push((src, e));
            return;
        }
    };

    let mut created = created;
    if created_src == CreatedSource::Existing {
        // Report the carried value.
        created = extract_created(&content).unwrap_or_default();
    }

    if dest.exists() {
        match doc_dest_matches(&dest, &src_body) {
            Ok(true) => plan.actions.push(FileAction::AlreadySame { src, rel: rel.to_string() }),
            Ok(false) => plan.conflicts.push(format!(
                "{rel}: knowledge source and vault target both exist with different content (MergeRequired)"
            )),
            Err(reason) => plan
                .conflicts
                .push(format!("{rel}: vault target unreadable ({reason}); MergeRequired")),
        }
    } else {
        plan.actions.push(FileAction::ConvertDoc {
            src,
            rel: rel.to_string(),
            content: conv.content,
            created,
            created_src,
            id: conv.id,
            id_synthesized: conv.id_synthesized,
            had_fm: conv.had_fm,
        });
    }
}

fn extract_created(content: &str) -> Option<String> {
    match oxi_frontmatter::parse(content, NoteFormat::Markdown) {
        Ok(Parsed::Memo { table, .. }) => match table.get("created") {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Verbatim branch shared by both trees: byte-compare against the
/// target when one exists; identical means idempotent re-run.
fn plan_verbatim(plan: &mut Plan, src: PathBuf, rel: &str, dest: &Path) {
    if dest.exists() {
        match (std::fs::read(&src), std::fs::read(dest)) {
            (Ok(a), Ok(b)) if a == b => {
                plan.actions.push(FileAction::AlreadySame {
                    src,
                    rel: rel.to_string(),
                });
            }
            _ => plan.conflicts.push(format!(
                "{rel}: source and vault target both exist with different bytes (MergeRequired)"
            )),
        }
    } else {
        plan.actions.push(FileAction::MoveVerbatim {
            src,
            rel: rel.to_string(),
        });
    }
}

/// Target already holds the migrated form of this v3 note (same id and
/// body) — treat as done.
fn v3_dest_matches(dest: &Path, v3_id: &str, body: &str) -> Result<bool, String> {
    let text = std::fs::read_to_string(dest).map_err(|e| e.to_string())?;
    match oxi_frontmatter::parse(&text, NoteFormat::Markdown) {
        Ok(Parsed::Memo { table, body: b }) => {
            Ok(matches!(table.get("id"), Some(Value::Str(s)) if s == v3_id) && b == body)
        }
        Ok(Parsed::BodyOnly { .. }) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// Target already satisfies the document invariants (id/created/
/// updated present) with the same body — treat as done.
fn doc_dest_matches(dest: &Path, src_body: &str) -> Result<bool, String> {
    let text = std::fs::read_to_string(dest).map_err(|e| e.to_string())?;
    match oxi_frontmatter::parse(&text, NoteFormat::Markdown) {
        Ok(Parsed::Memo { table, body }) => Ok(table.contains_key("id")
            && table.contains_key("created")
            && table.contains_key("updated")
            && body == src_body),
        Ok(Parsed::BodyOnly { .. }) => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

/// `true` when `~/.oxi/config.toml` is missing or has no `[vault]`
/// table (the migration would write it).
fn oxi_vault_table_absent(cfg: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(cfg) else {
        return true;
    };
    match toml::from_str::<toml::Value>(&text) {
        Ok(v) => v.get("vault").is_none(),
        Err(_) => true,
    }
}

fn run(paths: &Paths, apply: bool, now_rfc3339: &str) -> Result<Outcome, String> {
    let plan = build_plan(paths, now_rfc3339)?;

    if !plan.malformed.is_empty() || !plan.conflicts.is_empty() {
        let mut msg = String::from("migration aborted; NOTHING was written:\n");
        if !plan.malformed.is_empty() {
            msg.push_str(&format!(
                "\n  malformed source files ({}):\n",
                plan.malformed.len()
            ));
            for (path, reason) in &plan.malformed {
                msg.push_str(&format!("    {}: {reason}\n", path.display()));
            }
        }
        if !plan.conflicts.is_empty() {
            msg.push_str(&format!(
                "\n  MergeRequired — source and target diverged ({}):\n",
                plan.conflicts.len()
            ));
            for c in &plan.conflicts {
                msg.push_str(&format!("    {c}\n"));
            }
            msg.push_str("\n  Merge the trees by hand (or remove one side) and re-run.\n");
        }
        return Err(msg);
    }

    if !apply {
        return Ok(Outcome { plan, apply: None });
    }
    let stats = apply_plan(paths, &plan)?;
    Ok(Outcome {
        plan,
        apply: Some(stats),
    })
}

fn apply_plan(paths: &Paths, plan: &Plan) -> Result<ApplyStats, String> {
    let mut stats = ApplyStats::default();
    // Capture before any mutation: the one-time git phases (history
    // import + workspace removal) must run exactly once — on the run
    // that actually moved the knowledge tree.
    let knowledge_existed = paths.knowledge_src.is_dir();

    // ── 1. Backup BEFORE any mutation (design §7: "the source tree is
    //    backed up before execution"). Backups land under the T18
    //    deny-protected `~/.oxios/backups/` root.
    std::fs::create_dir_all(&paths.backups_root)
        .map_err(|e| format!("cannot create {}: {e}", paths.backups_root.display()))?;
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    if paths.knowledge_src.is_dir() {
        let dst = paths.backups_root.join(format!("knowledge-{ts}"));
        backup_tree(&paths.knowledge_src, &dst)
            .map_err(|e| format!("backup to {} failed: {e}", dst.display()))?;
        stats.backup_dirs.push(dst);
    }
    if paths.oximemo_old.is_dir() {
        let dst = paths.backups_root.join(format!("oximemo-vault-{ts}"));
        backup_tree(&paths.oximemo_old, &dst)
            .map_err(|e| format!("backup to {} failed: {e}", dst.display()))?;
        stats.backup_dirs.push(dst);
    }

    // ── 2. Vault root + shared ecosystem config.
    std::fs::create_dir_all(&paths.vault_dest)
        .map_err(|e| format!("cannot create {}: {e}", paths.vault_dest.display()))?;
    let mut config_written = false;
    if plan.vault_config_needed {
        write_oxi_vault_config(paths)?;
        config_written = true;
    }

    // ── 3. Execute every planned action (oximemo tree first).
    for action in &plan.actions {
        match action {
            FileAction::MoveVerbatim { src, rel } => {
                let dest = paths.vault_dest.join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                copy_verbatim(src, &dest).map_err(|e| format!("move {rel} failed: {e}"))?;
                stats.moved_verbatim += 1;
            }
            FileAction::ConvertToml { rel, content, .. } => {
                let dest = paths.vault_dest.join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                atomic_write(&dest, content.as_bytes())
                    .map_err(|e| format!("convert {rel} failed: {e}"))?;
                stats.converted_toml += 1;
                if rel.ends_with(".html") {
                    stats.notes.push(format!(
                        "{rel}: v3 .html note moved verbatim (oximemo converts it on first open)"
                    ));
                }
            }
            FileAction::ConvertDoc { rel, content, .. } => {
                let dest = paths.vault_dest.join(rel);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
                }
                atomic_write(&dest, content.as_bytes())
                    .map_err(|e| format!("convert {rel} failed: {e}"))?;
                stats.converted_doc += 1;
            }
            FileAction::AlreadySame { .. } => {
                stats.skipped_identical += 1;
            }
        }
        // Remove the source half of the move.
        if let Some(src) = action_src(action) {
            let _ = std::fs::remove_file(src);
        }
    }

    // ── 4. Prune emptied source directories.
    prune_empty(&paths.knowledge_src);
    prune_empty(&paths.oximemo_old);

    // ── 5. Initialize the vault's oximemo.toml when absent.
    let oximemo_toml = paths.vault_dest.join("oximemo.toml");
    let mut oximemo_toml_created = false;
    if !oximemo_toml.exists() {
        atomic_write(&oximemo_toml, DEFAULT_OXIMEMO_TOML.as_bytes())
            .map_err(|e| format!("cannot write oximemo.toml: {e}"))?;
        oximemo_toml_created = true;
    }

    // ── 6+7. Git: history extraction (or fresh init) + workspace removal.
    let any_write = stats.moved_verbatim + stats.converted_toml + stats.converted_doc > 0
        || config_written
        || oximemo_toml_created;
    migrate_git(paths, &mut stats, knowledge_existed, any_write);

    Ok(stats)
}

fn action_src(action: &FileAction) -> Option<&Path> {
    match action {
        FileAction::MoveVerbatim { src, .. }
        | FileAction::ConvertToml { src, .. }
        | FileAction::ConvertDoc { src, .. }
        | FileAction::AlreadySame { src, .. } => Some(src),
    }
}

/// Minimal vault config; oximemo fills defaults for any missing
/// section (its config deserializes with `serde(default)`).
const DEFAULT_OXIMEMO_TOML: &str = "# Initialized by oxios-migrate-vault (vault unification).\n# oximemo fills in defaults for any missing section.\nschema_version = 3\n\n[general]\ntrash_retention_days = 30\n";

/// Recursive copy (backup). Includes `.git` — a backup that loses
/// history is not a backup.
fn backup_tree(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        // Symlinks are recreated as symlinks (round-1 review #7):
        // `fs::copy` follows them, so a dangling link would abort the
        // whole apply during backup. Mirrors `copy_verbatim`.
        let meta = std::fs::symlink_metadata(&from).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&from).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &to).map_err(|e| e.to_string())?;
            #[cfg(not(unix))]
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        } else if meta.is_dir() {
            backup_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Copy a file verbatim; symlinks are recreated as symlinks, regular
/// files go through `atomic_write` so no partial target can survive a
/// crash.
fn copy_verbatim(src: &Path, dest: &Path) -> Result<(), String> {
    let is_symlink = std::fs::symlink_metadata(src)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink {
        #[cfg(unix)]
        {
            let target = std::fs::read_link(src).map_err(|e| e.to_string())?;
            std::os::unix::fs::symlink(&target, dest).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    // Temp + rename (round-1 review #4): a crash mid-`fs::copy` leaves
    // a partial target that the re-run would hard-error on as
    // MergeRequired. `atomic_write` cannot produce a partial.
    let bytes = std::fs::read(src).map_err(|e| e.to_string())?;
    atomic_write(dest, &bytes).map_err(|e| e.to_string())
}

/// Write `~/.oxi/config.toml` `[vault]` (path + canonical brain space)
/// when absent — design §5.4.
fn write_oxi_vault_config(paths: &Paths) -> Result<(), String> {
    let vault_path = paths.vault_dest.display().to_string();
    if !paths.oxi_config.exists() {
        if let Some(parent) = paths.oxi_config.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        let body = format!("[vault]\npath = \"{vault_path}\"\nspace = \"personal\"\n");
        atomic_write(&paths.oxi_config, body.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", paths.oxi_config.display()))?;
        return Ok(());
    }
    let text = std::fs::read_to_string(&paths.oxi_config)
        .map_err(|e| format!("cannot read {}: {e}", paths.oxi_config.display()))?;
    let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| {
        format!(
            "{} is not valid TOML; [vault] not written: {e}",
            paths.oxi_config.display()
        )
    })?;
    if doc.get("vault").is_some() {
        return Ok(());
    }
    doc["vault"]["path"] = toml_edit::value(vault_path);
    doc["vault"]["space"] = toml_edit::value("personal");
    atomic_write(&paths.oxi_config, doc.to_string().as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", paths.oxi_config.display()))?;
    Ok(())
}

/// Post-order removal of directories that became empty after the move.
fn prune_empty(root: &Path) {
    if !root.is_dir() {
        return;
    }
    fn rec(dir: &Path) -> bool {
        let mut empty = true;
        for entry in std::fs::read_dir(dir).ok().into_iter().flatten().flatten() {
            let p = entry.path();
            if p.is_dir() {
                if !rec(&p) {
                    empty = false;
                }
            } else {
                empty = false;
            }
        }
        if empty {
            std::fs::remove_dir(dir).is_ok()
        } else {
            false
        }
    }
    rec(root);
}

/// Vault git import for an ENABLED `GitLayer` (whole-branch review
/// fix: extracted so the disabled-foreign-repo path in `migrate_git`
/// can skip the whole phase — walk/commit-all, straggler sweep, and
/// filtered-history merge — without disturbing the rest of the flow).
fn vault_git_import(
    layer: &oxios_kernel::GitLayer,
    paths: &Paths,
    stats: &mut ApplyStats,
    ws_git_exists: bool,
    knowledge_pending: bool,
    filtered: &Option<PathBuf>,
) {
    let mut rels = Vec::new();
    let mut malformed = Vec::new();
    walk_files(&paths.vault_dest, &mut rels, &mut malformed);
    let refs: Vec<&str> = rels.iter().map(|(r, _)| r.as_str()).collect();
    if !refs.is_empty()
        && let Err(e) = layer.commit_files(&refs, "vault unification: import migrated vaults")
    {
        stats.notes.push(format!("vault commit-all failed: {e}"));
    }
    // Straggler sweep (found in round-1 e2e): `commit_files`
    // commits file BYTES, so symlinks — dangling ones even
    // fail `Path::exists()` — stay untracked, and `git
    // merge` then refuses to overwrite the untracked file
    // coming from the imported history. Sweep everything
    // left over into a follow-up commit so the tree is
    // fully tracked pre-merge.
    let _ = std::process::Command::new("git")
        .current_dir(&paths.vault_dest)
        .args(["add", "--all"])
        .output();
    let staged = std::process::Command::new("git")
        .current_dir(&paths.vault_dest)
        .args(["diff", "--cached", "--name-only"])
        .output()
        .map(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty())
        .unwrap_or(false);
    if staged {
        let _ = std::process::Command::new("git")
            .current_dir(&paths.vault_dest)
            .args([
                "commit",
                "-q",
                "-m",
                "vault unification: track links and stragglers",
            ])
            .output();
    }
    stats.git_history = if filtered.is_some() {
        "fresh init + commit-all".to_string()
    } else if ws_git_exists && knowledge_pending {
        "fresh init + commit-all (history extraction unavailable — old history remains in the workspace repo)".to_string()
    } else if ws_git_exists {
        "fresh init + commit-all (knowledge not migrated this run; history import skipped)"
            .to_string()
    } else {
        "fresh init + commit-all (no workspace repo)".to_string()
    };

    // 3. Merge the filtered history under the migrated content.
    if let Some(tmp) = filtered.as_ref() {
        let tmp_str = tmp.display().to_string();
        let reset_ok = std::process::Command::new("git")
            .current_dir(&paths.vault_dest)
            .args(["reset"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let fetch_ok = std::process::Command::new("git")
            .current_dir(&paths.vault_dest)
            .args(["fetch", &tmp_str, "HEAD"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        let merge_ok = reset_ok
            && fetch_ok
            && std::process::Command::new("git")
                .current_dir(&paths.vault_dest)
                .args([
                    "merge",
                    "--allow-unrelated-histories",
                    "-X",
                    "ours",
                    "--no-edit",
                    "-m",
                    "vault unification: import knowledge/ history",
                    "FETCH_HEAD",
                ])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
        if merge_ok {
            stats.git_history = "knowledge/ history imported via git filter-repo".to_string();
        } else {
            let _ = std::process::Command::new("git")
                .current_dir(&paths.vault_dest)
                .args(["merge", "--abort"])
                .output();
            stats
                .notes
                .push("git history merge failed; kept fresh-init history (old history remains in the workspace repo)".to_string());
        }
    }
}

/// Git phases (design §7.4-7.5, brief steps 6-7):
///
/// 1. Extract the `knowledge/` subtree history from the workspace repo
///    (`git clone --no-local` + `git filter-repo --subdirectory-filter
///    knowledge`) into a scratch dir — git CLI is allowed once here;
///    GitLayer's no-CLI rule is a runtime property.
/// 2. Vault repo: init when missing, claim ownership through
///    `GitLayer::new_for_vault` (the marker keeps the post-migration
///    kernel layer enabled), then commit-all.
/// 3. Merge the filtered history in (`--allow-unrelated-histories -X
///    ours`) — content stays migrated, history is adopted. Any failure
///    logs and keeps the fresh-init fallback state.
/// 4. Workspace removal commit for the moved `knowledge` subtree.
fn migrate_git(paths: &Paths, stats: &mut ApplyStats, knowledge_existed: bool, any_write: bool) {
    let ws_git_exists = paths.workspace.join(".git").exists();
    let vault_pre_existing = paths.vault_dest.join(".git").exists();

    // Round-1 review #3: gating the one-time git phases on DISK presence
    // strands a run that crashed between the file moves and the git
    // phase — the knowledge dir is gone, so the removal commit (and the
    // history import) would be skipped forever, leaving unstaged
    // deletions. "Pending" therefore means: moved this run, OR still
    // tracked in the workspace HEAD (the removal commit never landed).
    // A completed migration removed the tree entry, so steady-state
    // re-runs stay no-ops.
    let knowledge_pending = knowledge_existed || knowledge_tracked_in_head(&paths.workspace);

    // 1. History material BEFORE the workspace removal commit — only
    //    while knowledge work is pending. Re-importing after a completed
    //    migration would merge against the already-imported history and
    //    hit modify/delete conflicts the `-X ours` strategy cannot
    //    resolve.
    let filtered = if ws_git_exists && knowledge_pending {
        prepare_filtered_history(paths, stats)
    } else {
        None
    };

    // 2. Vault repo init + ownership + commit-all. Also runs as a
    //    recovery path when a previous run moved files but died before
    //    the git phase (vault populated, no .git).
    if any_write || !vault_pre_existing {
        match std::process::Command::new("git")
            .current_dir(&paths.vault_dest)
            .args(["init", "-b", "main"])
            .output()
        {
            Ok(o) if o.status.success() => {}
            Ok(o) => stats.notes.push(format!(
                "git init -b main failed in vault: {}",
                String::from_utf8_lossy(&o.stderr)
            )),
            Err(e) => stats.notes.push(format!("git init failed in vault: {e}")),
        }
        let adopt = !vault_pre_existing;
        match oxios_kernel::GitLayer::new_for_vault(paths.vault_dest.clone(), true, adopt) {
            Ok(layer) => {
                if !layer.is_enabled() {
                    // Whole-branch review fix (P2): a DISABLED layer
                    // means the vault root holds a user-owned repo we
                    // must not adopt. The walk/commit, the straggler
                    // sweep (`git add --all` + commit), and the
                    // history merge below run raw git INSIDE that
                    // repo — they would sweep the user's uncommitted
                    // edits into our import commit, the exact harm
                    // the adoption gate exists to prevent. Report
                    // operator guidance and skip both phases; the
                    // repo is left byte-for-byte as we found it.
                    stats.notes.push(format!(
                        "vault git layer disabled ({}): foreign repo at {} left untouched — vault git import/commit skipped. To adopt the existing repository, set [git] adopt_foreign_repo = true and re-run; to start a fresh oxios-owned history, remove {}/.git and re-run.",
                        layer.disabled_reason().unwrap_or("foreign repo"),
                        paths.vault_dest.display(),
                        paths.vault_dest.display(),
                    ));
                    stats.git_history =
                        "skipped (foreign vault repo not adopted; set [git] adopt_foreign_repo = true to adopt)"
                            .to_string();
                } else {
                    vault_git_import(
                        &layer,
                        paths,
                        stats,
                        ws_git_exists,
                        knowledge_pending,
                        &filtered,
                    );
                }
            }
            Err(e) => stats.notes.push(format!("vault GitLayer failed: {e}")),
        }
    } else {
        stats.git_history =
            "skipped (vault already initialized; nothing migrated this run)".to_string();
    }

    // Drop the scratch clone.
    if let Some(tmp) = filtered.as_ref() {
        let _ = std::fs::remove_dir_all(tmp);
    }

    // 4. Workspace removal commit — one commit removing the whole
    //    `knowledge` subtree entry. Only while knowledge work is pending
    //    (see `knowledge_pending` above); otherwise gix would happily
    //    produce empty duplicate commits.
    if ws_git_exists && knowledge_pending {
        match oxios_kernel::GitLayer::new(paths.workspace.clone(), true) {
            Ok(layer) => match layer.remove_file(
                "knowledge",
                "vault unification: move knowledge/ into the shared vault",
            ) {
                Ok(info) => stats.workspace_removal = format!("commit {}", info.short_hash),
                Err(e) => {
                    stats.workspace_removal = format!("nothing committed ({e})");
                }
            },
            Err(e) => stats.workspace_removal = format!("workspace GitLayer failed: {e}"),
        }
    } else if !ws_git_exists {
        stats.workspace_removal = "no workspace repo".to_string();
    } else {
        stats.workspace_removal = "skipped (knowledge not migrated this run)".to_string();
    }
}

/// Clone the workspace repo and filter it down to the `knowledge/`
/// subtree. Returns the scratch path (under the deny-protected backups
/// root) on success; `None` (logged into `stats.notes`) falls back to
/// fresh-init.
fn prepare_filtered_history(paths: &Paths, stats: &mut ApplyStats) -> Option<PathBuf> {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let tmp = paths.backups_root.join(format!(".filter-repo-tmp-{ts}"));
    let tmp_str = tmp.display().to_string();
    let ws_str = paths.workspace.display().to_string();

    let clone = std::process::Command::new("git")
        .args(["clone", "--no-local", &ws_str, &tmp_str])
        .output()
        .ok()?;
    if !clone.status.success() {
        stats.notes.push(format!(
            "git clone of the workspace repo failed: {}",
            String::from_utf8_lossy(&clone.stderr)
        ));
        return None;
    }
    let filter_ok = std::process::Command::new("git")
        .current_dir(&tmp)
        .args([
            "filter-repo",
            "--subdirectory-filter",
            "knowledge",
            "--force",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        || std::process::Command::new("git-filter-repo")
            .current_dir(&tmp)
            .args(["--subdirectory-filter", "knowledge", "--force"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
    if !filter_ok {
        stats
            .notes
            .push("git filter-repo unavailable/failed — falling back to fresh init".to_string());
        let _ = std::fs::remove_dir_all(&tmp);
        return None;
    }
    Some(tmp)
}

/// `knowledge` still present in the workspace repo's HEAD tree — i.e.
/// the removal commit has not landed yet (crash between the file moves
/// and the git phase). Git CLI is allowed in this one-time binary.
fn knowledge_tracked_in_head(workspace: &Path) -> bool {
    if !workspace.join(".git").exists() {
        return false;
    }
    std::process::Command::new("git")
        .current_dir(workspace)
        .args(["ls-tree", "--name-only", "HEAD", "knowledge"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "knowledge")
        .unwrap_or(false)
}

fn describe(action: &FileAction, tree: &str) -> String {
    match action {
        FileAction::MoveVerbatim { rel, .. } => format!("[{tree}] move verbatim      {rel}"),
        FileAction::ConvertToml { rel, .. } => {
            format!("[{tree}] convert TOML->YAML {rel}")
        }
        FileAction::ConvertDoc {
            rel,
            id,
            id_synthesized,
            created,
            created_src,
            had_fm,
            ..
        } => {
            let what = if *had_fm {
                "extend RFC-022   "
            } else {
                "synthesize bare   "
            };
            let idpart = if *id_synthesized {
                format!("new id {id}")
            } else {
                format!("id {id}")
            };
            format!("[{tree}] {what} {rel}  ({idpart}, created from {created_src:?}: {created})")
        }
        FileAction::AlreadySame { rel, .. } => {
            format!("[{tree}] skip (identical)  {rel}")
        }
    }
}

fn print_report(paths: &Paths, outcome: &Outcome) {
    let plan = &outcome.plan;
    println!("oxios-migrate-vault — ecosystem vault unification (design §7)");
    println!("  oximemo old vault : {}", paths.oximemo_old.display());
    println!("  knowledge tree    : {}", paths.knowledge_src.display());
    println!("  vault (target)    : {}", paths.vault_dest.display());
    println!();
    println!("  oximemo tree  : {:?}", plan.oximemo);
    println!("  knowledge tree: {:?}", plan.knowledge);
    println!();
    for (i, action) in plan.actions.iter().enumerate() {
        let tree = if i < plan.n_oximemo {
            "oximemo  "
        } else {
            "knowledge"
        };
        println!("{}", describe(action, tree));
    }
    if plan.vault_config_needed {
        println!(
            "[config  ] write [vault] into {}",
            paths.oxi_config.display()
        );
    }
    println!();

    match &outcome.apply {
        None => {
            println!("dry-run: nothing was written. Re-run with --apply to execute.");
        }
        Some(stats) => {
            for b in &stats.backup_dirs {
                println!("backup : {}", b.display());
            }
            println!(
                "applied: {} verbatim moves, {} TOML conversions, {} document conversions, {} identical skips",
                stats.moved_verbatim,
                stats.converted_toml,
                stats.converted_doc,
                stats.skipped_identical
            );
            println!("git    : {}", stats.git_history);
            println!("ws-git : {}", stats.workspace_removal);
            for n in &stats.notes {
                println!("note   : {n}");
            }
            println!();
            println!(
                "Next: run BOTH apps once — oxios reindexes the vault on open and oximemo\nregisters the default vault + brain space. The migration is reversible from\nthe backups above."
            );
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let apply = match args.as_slice() {
        [] => false,
        [a] if a == "--apply" => true,
        _ => {
            eprintln!("usage: oxios-migrate-vault [--apply]");
            eprintln!("  (default) dry-run: print the full migration plan, touch nothing");
            eprintln!("  --apply   backup first, then execute the plan");
            std::process::exit(2);
        }
    };

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            eprintln!("cannot resolve the home directory");
            std::process::exit(1);
        }
    };
    let paths = match resolve_paths(&home) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("path resolution failed: {e}");
            std::process::exit(1);
        }
    };
    let now = chrono::Utc::now().to_rfc3339();

    match run(&paths, apply, &now) {
        Ok(outcome) => print_report(&paths, &outcome),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (Task 19 brief, step 1 — RED first)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("scratch home")
    }

    fn write(p: &Path, content: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    const V3_NOTE: &str = "+++\nid = \"x\"\ncreated_at = \"2026-07-28T10:15:03+09:00\"\nupdated_at = \"2026-07-28T10:15:03+09:00\"\nfavorite = false\nhash = \"b3:dead\"\ntags = []\n+++\nbody";

    #[test]
    fn classify_uses_migration_system_set() {
        assert!(matches!(classify("habits/Mood.md"), Kind::System));
        assert!(matches!(classify("brain/Rust.md"), Kind::Document));
        assert!(matches!(classify("config.json"), Kind::System));
    }

    #[test]
    fn converts_oximemo_toml_note() {
        let out = convert_toml_note(V3_NOTE).unwrap();
        assert!(out.starts_with("---\nid: x\ncreated: 2026-07-28T10:15:03+09:00"));
        assert!(!out.contains("hash:"));
        assert!(!out.contains("tags:"));
    }

    #[test]
    fn convert_toml_note_preserves_unknown_keys_and_app_tables() {
        let note = "+++\nid = \"x\"\ncreated_at = \"2026-07-28T10:15:03+09:00\"\nupdated_at = \"2026-07-28T10:15:03+09:00\"\naliases = [\"a\", \"b\"]\npin = 3\n\n[dream]\nvisited = true\n+++\nhello";
        let out = convert_toml_note(note).unwrap();
        assert!(out.contains("aliases: [a, b]"), "unknown array kept: {out}");
        assert!(out.contains("pin: 3"), "unknown scalar kept: {out}");
        assert!(out.contains("dream:"), "app table kept: {out}");
        assert!(out.contains("visited: true"), "app table value kept: {out}");
        assert!(out.ends_with("hello"));
        // Round-trips through the v4 grammar.
        assert!(oxi_frontmatter::parse(&out, oxi_frontmatter::NoteFormat::Markdown).is_ok());
    }

    #[test]
    fn convert_toml_note_rejects_malformed_sources() {
        // Missing required id.
        let no_id = "+++\ncreated_at = \"2026-07-28T10:15:03+09:00\"\nupdated_at = \"2026-07-28T10:15:03+09:00\"\n+++\nbody";
        assert!(convert_toml_note(no_id).is_err());
        // Invalid TOML.
        let bad_toml = "+++\nid = \n+++\nbody";
        assert!(convert_toml_note(bad_toml).is_err());
        // Stray core key colliding with the typed mapping.
        let stray = "+++\nid = \"x\"\ncreated_at = \"2026-07-28T10:15:03+09:00\"\nupdated_at = \"2026-07-28T10:15:03+09:00\"\ncreated = \"1999-01-01T00:00:00Z\"\n+++\nbody";
        assert!(convert_toml_note(stray).is_err());
        // Unclosed fence.
        let unclosed = "+++\nid = \"x\"\ncreated_at = \"2026-07-28T10:15:03+09:00\"\nupdated_at = \"2026-07-28T10:15:03+09:00\"\nbody";
        assert!(convert_toml_note(unclosed).is_err());
    }

    #[test]
    fn rfc022_note_keeps_block_and_gains_core_keys() {
        let note = "---\ntitle: Rust learnings\noxios:\n  author: me\n  quality: distilled\n  source: manual\n  needs_review: false\n---\n\n# Rust\n";
        let out =
            convert_document(note, Some("2020-01-01T00:00:00Z"), "2026-08-21T00:00:00Z").unwrap();
        assert!(out.had_fm, "source had a frontmatter block");
        assert!(out.id_synthesized, "id synthesized");
        assert!(out.content.starts_with("---\nid: "));
        assert!(
            out.content.contains("\ncreated: 2020-01-01T00:00:00Z\n"),
            "created from hint"
        );
        assert!(out.content.contains("\nupdated: 2026-08-21T00:00:00Z\n"));
        // The oxios: table and unknown keys survive.
        assert!(out.content.contains("oxios:\n  author: me\n  quality: distilled\n  source: manual\n  needs_review: false"), "{}", out.content);
        assert!(out.content.contains("title: Rust learnings"));
        // Body preserved verbatim.
        assert!(out.content.ends_with("---\n\n# Rust\n"));
    }

    #[test]
    fn bare_note_gets_full_block_with_created_hint() {
        let bare = "# just markdown\n\nsome body\n";
        let out =
            convert_document(bare, Some("2021-05-05T05:05:05Z"), "2026-08-21T00:00:00Z").unwrap();
        assert!(!out.had_fm);
        assert!(out.id_synthesized);
        assert!(out.content.starts_with("---\nid: "));
        assert!(out.content.contains("\ncreated: 2021-05-05T05:05:05Z\n"));
        assert!(out.content.contains("\nupdated: 2026-08-21T00:00:00Z\n"));
        assert!(
            out.content.ends_with("---\n# just markdown\n\nsome body\n"),
            "{}",
            out.content
        );
    }

    #[test]
    fn created_prefers_git_then_mtime_then_now() {
        assert_eq!(
            created_from(
                Some("2026-01-02T00:00:00Z\n2024-01-01T00:00:00Z\n"),
                Some("2025-06-06T00:00:00Z"),
                "2026-08-21T00:00:00Z"
            ),
            "2024-01-01T00:00:00Z",
            "oldest git line wins"
        );
        assert_eq!(
            created_from(None, Some("2025-06-06T00:00:00Z"), "2026-08-21T00:00:00Z"),
            "2025-06-06T00:00:00Z",
            "mtime fallback"
        );
        assert_eq!(
            created_from(Some(""), None, "2026-08-21T00:00:00Z"),
            "2026-08-21T00:00:00Z",
            "empty git output falls through to now"
        );
    }

    #[test]
    fn first_commit_date_picks_oldest_line() {
        assert_eq!(
            first_commit_date("2026-01-02T00:00:00+09:00\n2024-01-01T00:00:00+09:00\n"),
            Some("2024-01-01T00:00:00+09:00")
        );
        assert_eq!(first_commit_date(""), None);
        assert_eq!(first_commit_date("\n \n"), None, "whitespace-only is empty");
    }

    // ── fs-level planning ──────────────────────────────────────────────

    fn setup_knowledge_tree(home: &Path) {
        let kb = home.join(".oxios/workspace/knowledge");
        write(&kb.join("brain/Rust.md"), "# rust body\n");
        write(
            &kb.join("projects/rfc.md"),
            "---\ntitle: t\noxios:\n  author: me\n  quality: raw\n  source: hook\n  needs_review: false\n---\nbody\n",
        );
        write(&kb.join("Chat.md"), "inbox\n");
        write(&kb.join("config.json"), "{\"lang\": \"en\"}\n");
        write(&kb.join("img/logo.png"), "\u{89}PNG-not-really");
        write(&kb.join("habits/Mood.md"), "mood log\n");
    }

    #[test]
    fn plan_classifies_system_files_verbatim() {
        let home = scratch_home();
        setup_knowledge_tree(home.path());
        let paths = resolve_paths(home.path()).unwrap();
        let plan = build_plan(&paths, "2026-08-21T00:00:00Z").unwrap();

        let verbatim: Vec<&str> = plan
            .actions
            .iter()
            .filter(|a| matches!(a, FileAction::MoveVerbatim { .. }))
            .map(FileAction::rel)
            .collect();
        assert!(
            verbatim.contains(&"Chat.md"),
            "Chat.md verbatim: {verbatim:?}"
        );
        assert!(verbatim.contains(&"config.json"), "{verbatim:?}");
        assert!(verbatim.contains(&"img/logo.png"), "{verbatim:?}");
        assert!(verbatim.contains(&"habits/Mood.md"), "{verbatim:?}");

        let docs: Vec<&str> = plan
            .actions
            .iter()
            .filter(|a| matches!(a, FileAction::ConvertDoc { .. }))
            .map(FileAction::rel)
            .collect();
        assert!(docs.contains(&"brain/Rust.md"), "{docs:?}");
        assert!(docs.contains(&"projects/rfc.md"), "{docs:?}");

        // RFC-022 note keeps its block; bare note does not have one.
        for a in &plan.actions {
            if let FileAction::ConvertDoc { rel, had_fm, .. } = a {
                let expected = rel == "projects/rfc.md";
                assert_eq!(*had_fm, expected, "{rel} had_fm={had_fm}");
            }
        }
        assert!(plan.malformed.is_empty(), "{:?}", plan.malformed);
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
    }

    #[test]
    fn plan_reports_already_migrated_when_source_missing() {
        let home = scratch_home();
        // No oximemo old vault, no knowledge tree; vault exists already.
        write(
            &home.path().join(".oxi/vault/notes/there.md"),
            "---\nid: a\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\n---\nx\n",
        );
        let paths = resolve_paths(home.path()).unwrap();
        let plan = build_plan(&paths, "2026-08-21T00:00:00Z").unwrap();
        assert_eq!(plan.knowledge, TreeStatus::AlreadyMigrated);
        assert_eq!(plan.oximemo, TreeStatus::AlreadyMigrated);
        assert!(plan.actions.is_empty());
    }

    #[test]
    fn plan_flags_both_exist_divergence() {
        let home = scratch_home();
        setup_knowledge_tree(home.path());
        // Divergent target for the bare note (its converted form will not
        // equal this).
        write(
            &home.path().join(".oxi/vault/brain/Rust.md"),
            "different bytes entirely\n",
        );
        let paths = resolve_paths(home.path()).unwrap();
        let plan = build_plan(&paths, "2026-08-21T00:00:00Z").unwrap();
        assert!(
            plan.conflicts.iter().any(|c| c.contains("brain/Rust.md")),
            "{:?}",
            plan.conflicts
        );
    }

    #[test]
    fn malformed_source_is_collected_hard_error() {
        let home = scratch_home();
        let kb = home.path().join(".oxios/workspace/knowledge");
        // Malformed v4 frontmatter: unclosed fence.
        write(
            &kb.join("brain/bad.md"),
            "---\nid: x\ncreated: 1\nbody never closed",
        );
        let paths = resolve_paths(home.path()).unwrap();
        let plan = build_plan(&paths, "2026-08-21T00:00:00Z").unwrap();
        assert_eq!(plan.malformed.len(), 1, "{:?}", plan.malformed);
        assert!(plan.malformed[0].0.ends_with("brain/bad.md"));
        // The run must refuse to do anything, even with apply.
        let err = run(&paths, true, "2026-08-21T00:00:00Z").unwrap_err();
        assert!(err.contains("brain/bad.md"), "{err}");
        // And nothing was written to the vault.
        assert!(!paths.vault_dest.join("brain/bad.md").exists());
    }

    // ── apply / dry-run ────────────────────────────────────────────────

    #[test]
    fn dry_run_touches_nothing() {
        let home = scratch_home();
        setup_knowledge_tree(home.path());
        let paths = resolve_paths(home.path()).unwrap();
        let before: Vec<(PathBuf, String)> = {
            let mut v = Vec::new();
            collect(&paths.knowledge_src, &mut v);
            v
        };
        assert!(!before.is_empty());

        let outcome = run(&paths, false, "2026-08-21T00:00:00Z").unwrap();
        assert!(outcome.apply.is_none(), "dry-run must not apply");

        // Nothing was created anywhere.
        assert!(!paths.vault_dest.exists(), "vault must not exist");
        assert!(!paths.backups_root.exists(), "backups must not exist");
        assert!(
            !paths.oxi_config.exists(),
            "~/.oxi/config.toml must not exist"
        );

        // Sources are byte-identical.
        let after: Vec<(PathBuf, String)> = {
            let mut v = Vec::new();
            collect(&paths.knowledge_src, &mut v);
            v
        };
        assert_eq!(before, after);
    }

    #[test]
    fn apply_backs_up_before_mutating_and_converts() {
        let home = scratch_home();
        // oximemo old default vault with one v3 note + a machinery file.
        let old = home
            .path()
            .join("Library/Application Support/com.oximemo.app/vault");
        write(&old.join("notes/hello.md"), V3_NOTE);
        write(&old.join("TEMPLATE.md"), "# template\n");
        setup_knowledge_tree(home.path());

        let paths = resolve_paths(home.path()).unwrap();
        let outcome = run(&paths, true, "2026-08-21T00:00:00Z").unwrap();
        let stats = outcome.apply.expect("apply stats");

        // Backups were created and hold the ORIGINAL pre-mutation bytes
        // (proves backup ran before any conversion/removal).
        assert_eq!(stats.backup_dirs.len(), 2, "{:?}", stats.backup_dirs);
        let kb_backup = stats
            .backup_dirs
            .iter()
            .find(|d| d.to_string_lossy().contains("knowledge-"))
            .expect("knowledge backup");
        assert_eq!(
            fs::read_to_string(kb_backup.join("brain/Rust.md")).unwrap(),
            "# rust body\n",
            "backup holds the original bare note"
        );
        let old_backup = stats
            .backup_dirs
            .iter()
            .find(|d| d.to_string_lossy().contains("oximemo-vault-"))
            .expect("oximemo backup");
        assert_eq!(
            fs::read_to_string(old_backup.join("notes/hello.md")).unwrap(),
            V3_NOTE,
            "backup holds the original v3 note"
        );

        // Vault now holds converted content.
        let migrated = fs::read_to_string(paths.vault_dest.join("notes/hello.md")).unwrap();
        assert!(migrated.starts_with("---\nid: x\ncreated: 2026-07-28T10:15:03+09:00"));
        assert!(migrated.contains("favorite: false"));
        let rust = fs::read_to_string(paths.vault_dest.join("brain/Rust.md")).unwrap();
        assert!(rust.starts_with("---\nid: "), "{rust}");
        assert!(rust.ends_with("---\n# rust body\n"), "{rust}");

        // System files moved verbatim, still frontmatter-less.
        assert_eq!(
            fs::read_to_string(paths.vault_dest.join("Chat.md")).unwrap(),
            "inbox\n"
        );
        assert_eq!(
            fs::read_to_string(paths.vault_dest.join("config.json")).unwrap(),
            "{\"lang\": \"en\"}\n"
        );

        // Shared config + vault oximemo.toml written.
        let cfg = fs::read_to_string(&paths.oxi_config).unwrap();
        assert!(cfg.contains("[vault]"), "{cfg}");
        assert!(cfg.contains("space = \"personal\""), "{cfg}");
        assert!(paths.vault_dest.join("oximemo.toml").is_file());

        // Sources were pruned.
        assert!(!paths.knowledge_src.join("brain/Rust.md").exists());
        assert!(!old.join("notes/hello.md").exists());

        // The vault is a git repo after apply.
        assert!(paths.vault_dest.join(".git").exists());
    }

    /// git init + commit the knowledge tree so both apply runs traverse
    /// the REAL git phases (history import gating, `-X ours` merge,
    /// removal commit) — the exact path where round-1 caught re-import
    /// conflicts manually.
    fn commit_workspace(home: &Path) {
        let ws = home.join(".oxios/workspace");
        for args in [
            vec!["init", "-q", "-b", "main"],
            vec!["add", "-A"],
            vec!["commit", "-q", "-m", "initial knowledge"],
        ] {
            let out = std::process::Command::new("git")
                .current_dir(&ws)
                .args(&args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }

    /// Count `move knowledge/` removal commits in the workspace log.
    fn removal_commits(home: &Path) -> usize {
        let ws = home.join(".oxios/workspace");
        let out = std::process::Command::new("git")
            .current_dir(&ws)
            .args(["log", "--oneline"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains("move knowledge/"))
            .count()
    }

    #[test]
    fn second_apply_run_is_a_git_noop() {
        let home = scratch_home();
        setup_knowledge_tree(home.path());
        commit_workspace(home.path());
        let paths = resolve_paths(home.path()).unwrap();

        let first = run(&paths, true, "2026-08-21T00:00:00Z").unwrap();
        let stats1 = first.apply.expect("first apply");
        assert!(stats1.converted_doc > 0);
        assert!(
            stats1.workspace_removal.contains("commit"),
            "first run must land the removal commit: {}",
            stats1.workspace_removal
        );
        assert_eq!(removal_commits(home.path()), 1);

        // Re-run on the migrated state: no moves, no duplicate git work.
        let second = run(&paths, true, "2026-08-22T00:00:00Z").unwrap();
        let stats2 = second.apply.expect("second apply");
        assert_eq!(stats2.moved_verbatim, 0);
        assert_eq!(stats2.converted_toml, 0);
        assert_eq!(stats2.converted_doc, 0);
        assert_eq!(stats2.backup_dirs.len(), 0, "nothing left to back up");
        assert!(
            stats2.git_history.contains("skipped"),
            "git phases must not re-run: {}",
            stats2.git_history
        );
        assert!(
            !stats2.workspace_removal.contains("commit"),
            "workspace removal must not re-run: {}",
            stats2.workspace_removal
        );
        assert_eq!(
            removal_commits(home.path()),
            1,
            "no duplicate removal commits"
        );
    }

    // ── round-1 review covering tests ──────────────────────────────

    #[test]
    fn same_rel_in_both_sources_is_a_conflict() {
        let home = scratch_home();
        let old = home
            .path()
            .join("Library/Application Support/com.oximemo.app/vault");
        write(&old.join("brain/shared.md"), V3_NOTE);
        setup_knowledge_tree(home.path());
        write(
            &home
                .path()
                .join(".oxios/workspace/knowledge/brain/shared.md"),
            "knowledge side of the collision\n",
        );
        let paths = resolve_paths(home.path()).unwrap();
        let plan = build_plan(&paths, "2026-08-21T00:00:00Z").unwrap();
        assert!(
            plan.conflicts
                .iter()
                .any(|c| c.contains("brain/shared.md") && c.contains("BOTH")),
            "{:?}",
            plan.conflicts
        );
        // apply refuses, and NOTHING was written.
        let err = run(&paths, true, "2026-08-21T00:00:00Z").unwrap_err();
        assert!(err.contains("brain/shared.md"), "{err}");
        assert!(!paths.vault_dest.exists(), "no vault may be created");
        assert!(
            old.join("brain/shared.md").is_file()
                && paths.knowledge_src.join("brain/shared.md").is_file(),
            "both sources must survive untouched"
        );
    }

    #[test]
    fn kernel_knowledge_root_override_divergent_errors() {
        let home = scratch_home();
        write(
            &home.path().join(".oxios/config.toml"),
            "[kernel]\nknowledge_root = \"~/elsewhere\"\n",
        );
        let err = resolve_paths(home.path()).unwrap_err();
        assert!(
            err.contains("knowledge_root") && err.contains("elsewhere"),
            "{err}"
        );
        // And the divergent target is never consulted further.
        assert!(!home.path().join(".oxi").exists());
    }

    #[test]
    fn kernel_knowledge_root_override_matching_proceeds() {
        let home = scratch_home();
        write(
            &home.path().join(".oxios/config.toml"),
            "[kernel]\nknowledge_root = \"~/.oxi/vault\"\n",
        );
        let paths = resolve_paths(home.path()).unwrap();
        assert_eq!(paths.vault_dest, home.path().join(".oxi/vault"));
    }

    #[test]
    fn rerun_after_crash_before_git_phase_completes_removal() {
        let home = scratch_home();
        setup_knowledge_tree(home.path());
        commit_workspace(home.path());

        // Simulate the crash state: files already moved to the vault,
        // knowledge dir deleted, git phase never ran (knowledge is still
        // tracked in the workspace HEAD).
        let vault = home.path().join(".oxi/vault");
        let moved = copy_dir(&paths_knowledge(home.path()), &vault);
        assert!(moved > 0);
        fs::remove_dir_all(home.path().join(".oxios/workspace/knowledge")).unwrap();
        assert_eq!(removal_commits(home.path()), 0);

        let paths = resolve_paths(home.path()).unwrap();
        let outcome = run(&paths, true, "2026-08-21T00:00:00Z").unwrap();
        let stats = outcome.apply.expect("recovery apply");
        assert!(
            stats.workspace_removal.contains("commit"),
            "crash recovery must land the removal commit: {}",
            stats.workspace_removal
        );
        assert_eq!(removal_commits(home.path()), 1);
        // The migrated files survived the recovery run untouched.
        assert!(vault.join("brain/Rust.md").is_file());
    }

    #[test]
    fn disabled_foreign_vault_repo_is_left_untouched() {
        // Whole-branch review fix (P2): when `new_for_vault` returns
        // a DISABLED foreign layer (repo at the vault root, adoption
        // not opted in), the straggler sweep (`git add --all` +
        // commit) and the history merge ran anyway — INSIDE the
        // user's foreign repo, sweeping uncommitted edits. Both
        // phases must be skipped and the repo left untouched.
        let home = scratch_home();
        setup_knowledge_tree(home.path());
        commit_workspace(home.path());

        // Vault root holds a FOREIGN repo (plain git init — no oxios
        // ownership marker) with committed history and an
        // uncommitted user edit.
        let vault = home.path().join(".oxi/vault");
        let moved = copy_dir(&paths_knowledge(home.path()), &vault);
        assert!(moved > 0);
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .current_dir(&vault)
                .args(["-c", "user.email=t@t", "-c", "user.name=t"])
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        git(&["init", "-b", "main"]);
        git(&["add", "."]);
        git(&["commit", "-m", "user's own import"]);
        fs::write(vault.join("uncommitted-user-edit.md"), "uncommitted").unwrap();
        let head_before = git(&["rev-parse", "HEAD"]);

        // any_write=true enters the git phase; vault_pre_existing
        // with adopt=false yields the DISABLED foreign layer.
        let paths = resolve_paths(home.path()).unwrap();
        let mut stats = ApplyStats::default();
        migrate_git(&paths, &mut stats, true, true);

        // Repo untouched: no new commit landed, and the user's
        // uncommitted edit is still untracked (not swept).
        assert_eq!(
            git(&["rev-parse", "HEAD"]),
            head_before,
            "no commit may land in the foreign repo"
        );
        let status = git(&["status", "--porcelain"]);
        assert!(
            status.contains("?? uncommitted-user-edit.md"),
            "uncommitted edit was swept: {status}"
        );

        // Operator guidance: repo left untouched + how to adopt.
        assert!(
            stats.notes.iter().any(|n| n.contains("adopt_foreign_repo")),
            "adoption guidance missing: {:?}",
            stats.notes
        );
        assert!(
            stats.git_history.contains("skipped"),
            "git_history must report the skip: {}",
            stats.git_history
        );
    }

    #[test]
    fn copy_verbatim_leaves_no_temp_debris() {
        let home = scratch_home();
        let src = home.path().join("blob.bin");
        let bytes: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        fs::write(&src, &bytes).unwrap();
        let dest = home.path().join("out").join("blob.bin");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        copy_verbatim(&src, &dest).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), bytes, "byte-identical copy");
        let entries: Vec<_> = fs::read_dir(dest.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            entries,
            vec!["blob.bin".to_string()],
            "no temp debris: {entries:?}"
        );
    }

    #[test]
    fn backup_and_move_handle_dangling_symlinks() {
        let home = scratch_home();
        setup_knowledge_tree(home.path());
        // A dangling symlink (non-.md ⇒ system path ⇒ verbatim move).
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            "does-not-exist.txt",
            home.path().join(".oxios/workspace/knowledge/dangling.txt"),
        )
        .unwrap();

        let paths = resolve_paths(home.path()).unwrap();
        let outcome = run(&paths, true, "2026-08-21T00:00:00Z").unwrap();
        let stats = outcome.apply.expect("apply must survive the dangling link");
        assert!(!stats.backup_dirs.is_empty());

        // The backup recreated the link instead of following it.
        let backup_kb = stats
            .backup_dirs
            .iter()
            .find(|d| d.to_string_lossy().contains("knowledge-"))
            .unwrap();
        assert!(
            fs::symlink_metadata(backup_kb.join("dangling.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // And the vault holds the recreated link.
        #[cfg(unix)]
        {
            let v = paths.vault_dest.join("dangling.txt");
            assert!(fs::symlink_metadata(&v).unwrap().file_type().is_symlink());
            assert_eq!(
                fs::read_link(&v).unwrap().to_string_lossy(),
                "does-not-exist.txt"
            );
        }
    }

    fn paths_knowledge(home: &Path) -> PathBuf {
        home.join(".oxios/workspace/knowledge")
    }

    /// Test helper: recursively copy `src` into `dst`, returning the
    /// number of files copied.
    fn copy_dir(src: &Path, dst: &Path) -> usize {
        let mut n = 0;
        fs::create_dir_all(dst).unwrap();
        for entry in fs::read_dir(src).unwrap() {
            let entry = entry.unwrap();
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if from.is_dir() {
                n += copy_dir(&from, &to);
            } else {
                fs::copy(&from, &to).unwrap();
                n += 1;
            }
        }
        n
    }

    fn collect(root: &Path, out: &mut Vec<(PathBuf, String)>) {
        if !root.exists() {
            return;
        }
        for entry in fs::read_dir(root).unwrap() {
            let entry = entry.unwrap();
            let p = entry.path();
            if p.is_dir() {
                collect(&p, out);
            } else {
                out.push((p.clone(), fs::read_to_string(&p).unwrap_or_default()));
            }
        }
    }
}
