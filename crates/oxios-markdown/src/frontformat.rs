//! Vault write-side adapter for `oxi-frontmatter`.
//!
//! Once the `knowledge.rs` migration lands (Tasks 12/13/19), this
//! module will be the **sole** producer of `oxios:` frontmatter in
//! `oxios-markdown`; until then `knowledge.rs::note_write_with_meta`
//! still emits its own block and the two paths may overlap. The
//! frontformat module exists to consolidate the two responsibilities
//! §6 of the vault-unification design separates:
//!
//! 1. **System-path exclusion** — `Chat.md`, `Later.md`, `Done.md`,
//!    `Shop.md`, `Watch.md`, `Read.md`, `journal/`, `habits/`,
//!    `insights/`, `archive/`, `media/`, `img/`, and any non-`.md`
//!    file must NEVER carry frontmatter. Even if the caller passed
//!    body bytes that look like a memo, we treat them as raw bytes
//!    and write them through `atomic_write`.
//!
//! 2. **Memo-path merge-write** — for first-class documents
//!    (e.g. `brain/Rust.md`), `write_note` round-trips through
//!    `oxi-frontmatter::write_document` so the file's `id`,
//!    `created`, and `updated` invariants are kept consistent, and
//!    editor-supplied keys (Obsidian tags, aliases, etc.) are
//!    preserved.
//!
//! # Layering
//!
//! This module is a thin policy layer over `oxi-frontmatter`. It
//! does **not** re-implement merge logic — it routes every memo
//! write through `oxi-frontmatter::write_document`, which is the
//! canonical writer per the spec.
//!
//! Backed by `oxi-frontmatter` v0.1 (`grammar v2`). See
//! `oximemo-vault-unification/crates/oxi-frontmatter/SPEC.md` for
//! the underlying grammar.

use std::path::Path;

use oxi_frontmatter::{
    FrontmatterError, NoteFormat, Parsed, Synthesize, Table, Value, WriteOutcome, atomic_write,
    emit, parse, write_document,
};
use time::OffsetDateTime;

use crate::types::{
    CHAT_FILENAME, DIR_ARCHIVE, DIR_HABITS, DIR_INSIGHTS, DIR_JOURNAL, DIR_MEDIA, DONE_FILENAME,
    LATER_FILENAME, MD_EXT, NoteMeta, READ_FILENAME, SHOP_FILENAME, WATCH_FILENAME,
};

// ---------------------------------------------------------------------------
// Path hardening
// ---------------------------------------------------------------------------

/// Reject `rel` paths that could escape `root` when joined.
///
/// Mirrors `VirtualFs::safe_path` (fs.rs:95-117): rejects `..`
/// segments, leading `/` or `\` (absolute), and embedded null bytes.
/// Returns `Err(FsError::UnsafePath)` for any traversal attempt so the
/// caller cannot bypass the §6 system-path exclusion by passing
/// `../Chat.md` or `/etc/passwd` as a relative path.
fn assert_safe_rel(rel_path: &str) -> Result<(), FrontmatterError> {
    if rel_path.is_empty()
        || rel_path.starts_with('/')
        || rel_path.starts_with('\\')
        || rel_path.starts_with("..")
        || rel_path.contains("/../")
        || rel_path.contains("\\..\\")
        || rel_path.contains('\0')
    {
        return Err(FrontmatterError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unsafe path: {rel_path:?}"),
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// System-path exclusion set (§6 of the vault-unification design)
// ---------------------------------------------------------------------------

/// First path component that disqualifies a path from receiving
/// frontmatter, alongside the file-level constants in `types.rs`.
const SYSTEM_DIRS: &[&str] = &[
    DIR_ARCHIVE,
    DIR_JOURNAL,
    DIR_HABITS,
    DIR_INSIGHTS,
    DIR_MEDIA,
    "img",
];

/// Filenames that — **anchored to the vault root only** — must never
/// carry frontmatter. Review F5 of the design notes that filename
/// equality must be root-anchored so a user memo named `Chat.md`
/// inside a folder is not silently treated as a system file.
const SYSTEM_FILES_ROOT: &[&str] = &[
    CHAT_FILENAME,
    LATER_FILENAME,
    DONE_FILENAME,
    SHOP_FILENAME,
    WATCH_FILENAME,
    READ_FILENAME,
];

/// Returns `true` if `rel_path` must never carry frontmatter.
///
/// "System path" means ANY of:
///
/// 1. `rel_path` is not a `.md` file (raw bytes, images).
/// 2. The first path component is one of the SF-D-2 reserved
///    directories (`archive/`, `journal/`, `habits/`, `insights/`,
///    `media/`, `img/`).
/// 3. `rel_path` is exactly one of the inboxes / lists that the app
///    treats as "infrastructure" (`Chat.md`, `Later.md`, `Done.md`,
///    `Shop.md`, `Watch.md`, `Read.md`). The match is on the
///    full path so a user memo named `Chat.md` inside a folder is
///    NOT classified as system — see review F5.
///
/// A user note called `brain/Rust.md` returns `false` — first-class
/// documents always carry frontmatter.
pub fn is_system_path(rel_path: &str) -> bool {
    // Non-markdown files are always raw: the parser is markdown-only,
    // and images / non-md payloads have no frontmatter concept.
    if !rel_path.ends_with(MD_EXT) {
        return true;
    }

    let first = rel_path.split('/').next().unwrap_or(rel_path);

    // Root-anchored exact-match: the entire rel_path equals one of
    // the system filenames (no leading directory component).
    if SYSTEM_FILES_ROOT.contains(&rel_path) {
        return true;
    }

    // Directory match considers the first component only.
    if SYSTEM_DIRS.contains(&first) {
        return true;
    }

    false
}

/// Read RFC-022 [`NoteMeta`] from the `oxios:` table of a note.
///
/// Returns `None` when the file:
///
/// - has no frontmatter block at all (`BodyOnly` notes),
/// - has a frontmatter block but no `oxios:` key (user-authored
///   frontmatter, e.g. Obsidian tags — we never touch these).
///
/// **Malformed frontmatter** is a hard error propagated via
/// `Err(FrontmatterError::Parse)` — the oxi-frontmatter spec says
/// we don't silently repair, and the contract is the same here.
///
/// **Inside a valid `oxios:` map, individual fields that fail to
/// decode** (unknown enum variant, non-string scalar where a string
/// is required) fall back to the documented defaults: missing
/// `author` → `""`, missing `quality` → `Raw`, missing `source` →
/// `Hook`, missing `needs_review` → `false`. A malformed scalar
/// never prevents the surrounding note from being recognized.
pub fn read_note_meta(content: &str) -> Result<Option<NoteMeta>, FrontmatterError> {
    let parsed = parse(content, NoteFormat::Markdown)?;
    Ok(match parsed {
        Parsed::Memo { table, .. } => table_to_note_meta(&table),
        Parsed::BodyOnly { .. } => None,
    })
}

/// Read a note's body with any frontmatter block stripped (v4 grammar).
///
/// The body-only counterpart of [`read_note_meta`] for consumers that
/// need the markdown body — e.g. the curation scan feeding the LLM:
/// `Parsed::Memo` yields the verbatim body after the closing fence,
/// `BodyOnly` content is returned as-is. Malformed frontmatter is a
/// hard [`FrontmatterError::Parse`] — unlike the legacy bespoke
/// parser in `knowledge.rs`, which silently returned the full file
/// (frontmatter included) as the body on any parse miss.
pub fn read_note_body(content: &str) -> Result<String, FrontmatterError> {
    Ok(match parse(content, NoteFormat::Markdown)? {
        Parsed::Memo { body, .. } => body,
        Parsed::BodyOnly { body } => body,
    })
}

/// Serialize full file content with an `oxios:` table merged in.
///
/// `content` is whatever the caller wants the file to look like —
/// existing frontmatter, a plain body, or a complete note. The
/// returned `String` is the canonical form (starts with `---\n`,
/// contains an `oxios:` table, then the body).
///
/// Errors:
/// - [`FrontmatterError::Unemittable`] — the merged table contains
///   a shape the parser cannot re-read (e.g. an empty `Array`).
/// - [`FrontmatterError::Parse`] — the caller's `content` is
///   malformed frontmatter that we cannot safely merge.
pub fn with_oxios_table(content: &str, meta: &NoteMeta) -> Result<String, FrontmatterError> {
    let (incoming_table, body) = match parse(content, NoteFormat::Markdown)? {
        Parsed::Memo { table, body } => (table, body),
        Parsed::BodyOnly { body } => (Table::new(), body),
    };

    let mut merged = incoming_table;
    merge_note_meta(&mut merged, meta);

    Ok(emit(&merged, &body, NoteFormat::Markdown))
}

/// Write a note to `root / rel` using the §6 exclusion rule.
///
/// Path hardening: `rel` must be a vault-relative POSIX path
/// (no `..`, no leading `/`/`\`, no null). Otherwise we return
/// `Err(FrontmatterError::Io(InvalidInput))` without touching the
/// filesystem (mirrors `VirtualFs::safe_path`).
///
/// - **System path** (`Chat.md`, anything non-`.md`, anything inside
///   a SYSTEM_DIR) → raw `atomic_write`. We never synthesize
///   frontmatter. If the bytes match the existing file, we return
///   `WriteOutcome::NoOp` without touching the file.
///
/// - **Memo path** → routed through `oxi-frontmatter::write_document`
///   with the body's frontmatter pre-parsed:
///
///   * If `content` parses as `Memo{table, body}` (caller supplied a
///     frontmatter block), we write `body` to the file with the
///     **incoming table as base** — `write_document` carries its
///     `id`/`created`/unknown keys forward, and our `oxios:` map is
///     merged in alongside them. This is how editor-supplied tags
///     or aliases survive a knowledge-write pass.
///
///   * If `content` parses as `BodyOnly`, the whole `content` is
///     the body and the file's **existing table** becomes the base
///     (write_document reads it). This is the "missing
///     frontmatter ⇒ synthesize" path.
///
///   In both cases `write_document` synthesizes `id`/`created` if
///   the resulting table lacks them, bumps `updated` only on a real
///   write, and returns `WriteOutcome::NoOp` when the parsed form
///   is identical to what was on disk.
///
/// `now` is injected for testability.
pub fn write_note(
    root: &Path,
    rel: &str,
    content: &str,
    now: OffsetDateTime,
) -> Result<WriteOutcome, FrontmatterError> {
    assert_safe_rel(rel)?;

    let path = root.join(rel);
    if is_system_path(rel) {
        // Raw atomic write. NoOp on byte-identical content.
        let existing = std::fs::read(&path).ok();
        if existing.as_deref() == Some(content.as_bytes()) {
            return Ok(WriteOutcome::NoOp);
        }
        atomic_write(&path, content.as_bytes())?;
        return Ok(WriteOutcome::Written);
    }

    // Memo path. The incoming content's frontmatter block is the
    // *primary* source of truth for the merge base (the brief's
    // review-mandated behavior — see round-1 finding #1), but we
    // also carry forward unknown keys from the existing file so an
    // editor-supplied `custom_key: hello` survives a write that
    // happens after a previous edit added `legacy_key: kept`.
    match parse(content, NoteFormat::Markdown)? {
        Parsed::Memo {
            table: incoming_table,
            body,
        } => write_memo_with_incoming_table(&path, incoming_table, &body, now),
        Parsed::BodyOnly { body } => {
            // No frontmatter block at all in the caller's content:
            // the file's existing table is the merge base, and
            // write_document will synthesize id/created/updated if
            // the file is missing or BodyOnly.
            write_document(
                &path,
                &body,
                NoteFormat::Markdown,
                oxi_frontmatter::Mutation::default(),
                Synthesize::Yes,
                now,
            )
        }
    }
}

/// Merge-write a memo file where the caller supplied a frontmatter
/// block. The incoming table becomes the merge base; unknown keys
/// from the existing file are carried forward; `id` and `created`
/// are synthesized if either side lacks them; `updated` is set to
/// `now` only on a real write.
fn write_memo_with_incoming_table(
    path: &Path,
    incoming_table: Table,
    body: &str,
    now: OffsetDateTime,
) -> Result<WriteOutcome, FrontmatterError> {
    // Read the existing file once and reuse for both the merge
    // base and the NoOp probe. Missing file => empty table.
    let existing_parsed: Option<Parsed> = match std::fs::read(path) {
        Ok(b) => match std::str::from_utf8(&b) {
            Ok(s) => Some(parse(s, NoteFormat::Markdown)?),
            Err(_) => {
                return Err(FrontmatterError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("file at {} is not valid UTF-8", path.display()),
                )));
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(FrontmatterError::Io(e)),
    };

    let mut next_table: Table = match &existing_parsed {
        Some(Parsed::Memo { table, .. }) => table.clone(),
        _ => Table::new(),
    };
    for (k, v) in incoming_table {
        next_table.insert(k, v);
    }
    if !next_table.contains_key("id") {
        next_table.insert(
            "id".to_string(),
            Value::Str(uuid::Uuid::now_v7().to_string()),
        );
    }
    if !next_table.contains_key("created") {
        next_table.insert("created".to_string(), Value::Str(format_offset(now)));
    }
    // `updated` is deliberately NOT set here: the candidate table
    // keeps whatever `updated` the merged base carries (the on-disk
    // value overlaid by the incoming table), so the semantic-NoOp
    // probe below compares apples to apples. Mirrors
    // `oxi_frontmatter::write_document` step 3. The old code inserted
    // `updated = now` BEFORE the probe, so with a per-call clock
    // (production injects `now_utc()` per invocation) the probe
    // always differed and NoOp was unreachable for frontmatter-bearing
    // content — every unchanged editor re-save rewrote the file,
    // bumped `updated`, re-canonicalized foreign formatting, and
    // fired git auto-commit + brain episodes (whole-branch P1).

    // Semantic NoOp (round-2 fix): the probe must compare the
    // *incoming* body against the on-disk body, NOT reuse the
    // existing body in the emission. Otherwise a body edit can
    // return NoOp whenever the merged table happens to match the
    // on-disk table (e.g. when `now` equals the previous
    // `updated`) and the edit is silently dropped. NoOp only when
    // BOTH the table AND the incoming body are semantically
    // unchanged. The probe MUST use the *incoming* `body`, not the
    // existing body, otherwise a body change can return NoOp whenever
    // the merged table happens to equal the on-disk table.
    let same = match &existing_parsed {
        Some(Parsed::Memo { table: t, body: b }) => {
            let probe = emit(&next_table, body, NoteFormat::Markdown);
            match parse(&probe, NoteFormat::Markdown) {
                Ok(Parsed::Memo {
                    table: t2,
                    body: b2,
                }) => t == &t2 && b == &b2,
                _ => false,
            }
        }
        _ => false,
    };
    if same {
        return Ok(WriteOutcome::NoOp);
    }

    // Real write — bump `updated` to `now` ONLY now (after the probe
    // passed), mirroring `write_document` step 6: a true NoOp never
    // bumps it.
    next_table.insert("updated".to_string(), Value::Str(format_offset(now)));

    let new_bytes = emit(&next_table, body, NoteFormat::Markdown).into_bytes();
    atomic_write(path, &new_bytes)?;
    Ok(WriteOutcome::Written)
}

/// Format an `OffsetDateTime` as an RFC3339 string. Cannot fail for
/// well-formed inputs — panicking is the loud-but-safe choice.
fn format_offset(t: OffsetDateTime) -> String {
    t.format(&time::format_description::well_known::Rfc3339)
        .expect("RFC3339 formatting of OffsetDateTime cannot fail")
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Convert an `oxios:` table to [`NoteMeta`].
///
/// Returns `None` if the table has no `oxios` key — caller treats
/// that as "user-authored, no agent provenance". Unparseable scalar
/// fields default per the documented fallback rules on [`read_note_meta`].
fn table_to_note_meta(table: &Table) -> Option<NoteMeta> {
    let oxios = table.get("oxios")?;
    let Value::Map(map) = oxios else {
        return None;
    };

    let author = get_str(map, "author").unwrap_or_default();
    let quality = get_str(map, "quality")
        .and_then(|s| parse_quality(&s))
        .unwrap_or(crate::types::NoteQuality::Raw);
    let source = get_str(map, "source")
        .and_then(|s| parse_source(&s))
        .unwrap_or(crate::types::NoteSource::Hook);
    let needs_review = get_bool(map, "needs_review").unwrap_or(false);
    let session_id = get_str(map, "session_id");
    let message_index = get_usize(map, "message_index");
    let saved_at = get_str(map, "saved_at");

    Some(NoteMeta {
        author,
        source,
        quality,
        needs_review,
        session_id,
        message_index,
        saved_at,
    })
}

/// Embed `meta` as an `oxios:` row in `table`. Existing non-`oxios`
/// keys are preserved (id, created, updated, tags, etc.).
fn merge_note_meta(table: &mut Table, meta: &NoteMeta) {
    let mut inner = Table::new();
    inner.insert("author".to_string(), Value::Str(meta.author.clone()));
    inner.insert(
        "source".to_string(),
        Value::Str(source_str(&meta.source).to_string()),
    );
    inner.insert(
        "quality".to_string(),
        Value::Str(quality_str(&meta.quality).to_string()),
    );
    inner.insert("needs_review".to_string(), Value::Bool(meta.needs_review));
    if let Some(sid) = &meta.session_id {
        inner.insert("session_id".to_string(), Value::Str(sid.clone()));
    }
    if let Some(idx) = meta.message_index {
        inner.insert("message_index".to_string(), Value::Str(idx.to_string()));
    }
    if let Some(ts) = &meta.saved_at {
        inner.insert("saved_at".to_string(), Value::Str(ts.clone()));
    }
    table.insert("oxios".to_string(), Value::Map(inner));
}

fn get_str(map: &Table, key: &str) -> Option<String> {
    match map.get(key)? {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

fn get_bool(map: &Table, key: &str) -> Option<bool> {
    match map.get(key)? {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn get_usize(map: &Table, key: &str) -> Option<usize> {
    match map.get(key)? {
        Value::Str(s) => s.parse().ok(),
        _ => None,
    }
}

fn parse_quality(s: &str) -> Option<crate::types::NoteQuality> {
    match s {
        "raw" => Some(crate::types::NoteQuality::Raw),
        "curated" => Some(crate::types::NoteQuality::Curated),
        "refined" => Some(crate::types::NoteQuality::Refined),
        _ => None,
    }
}

fn parse_source(s: &str) -> Option<crate::types::NoteSource> {
    match s {
        "hook" => Some(crate::types::NoteSource::Hook),
        "tool" => Some(crate::types::NoteSource::Tool),
        "ui" => Some(crate::types::NoteSource::Ui),
        "dream" => Some(crate::types::NoteSource::Dream),
        _ => None,
    }
}

fn source_str(s: &crate::types::NoteSource) -> &'static str {
    match s {
        crate::types::NoteSource::Hook => "hook",
        crate::types::NoteSource::Tool => "tool",
        crate::types::NoteSource::Ui => "ui",
        crate::types::NoteSource::Dream => "dream",
    }
}

fn quality_str(q: &crate::types::NoteQuality) -> &'static str {
    match q {
        crate::types::NoteQuality::Raw => "raw",
        crate::types::NoteQuality::Curated => "curated",
        crate::types::NoteQuality::Refined => "refined",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NoteQuality;
    use oxi_frontmatter::parse;
    use time::macros::datetime;

    #[test]
    fn system_paths_are_excluded() {
        for p in [
            "Chat.md",
            "Later.md",
            "Done.md",
            "Shop.md",
            "journal/2026.08 August.md",
            "habits/Mood.md",
            "insights/2026 Habits.md",
            "archive/Done.md",
            "config.json",
            "img/x.png",
        ] {
            assert!(is_system_path(p), "{p} should be a system path");
        }
        assert!(
            !is_system_path("brain/Rust.md"),
            "first-class memo must NOT be a system path"
        );
        // Review F5: a user memo named "Chat.md" inside a folder is
        // NOT a system path (root-anchored filename match).
        assert!(
            !is_system_path("personal/Chat.md"),
            "filename equality is root-anchored; personal/Chat.md is a memo"
        );
    }

    #[test]
    fn unsafe_rel_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let now = datetime!(2026-08-21 00:00 UTC);
        for bad in [
            "../Chat.md",
            "/etc/passwd",
            "\\Windows\\System32",
            "ok/\x00/bad",
        ] {
            let err = write_note(tmp.path(), bad, "body", now).unwrap_err();
            let msg = format!("{err}");
            assert!(
                msg.contains("unsafe path"),
                "{bad} should be rejected; got {msg}"
            );
        }
    }

    #[test]
    fn legacy_rfc022_is_native_and_meta_roundtrips() {
        let legacy = "---\noxios:\n  author: agent\n  quality: raw\n---\nbody";
        let meta = read_note_meta(legacy)
            .expect("legacy RFC-022 must parse")
            .expect("oxios: present");
        assert_eq!(meta.author, "agent");
        assert_eq!(meta.quality, NoteQuality::Raw);

        let out = with_oxios_table(legacy, &meta).expect("emit must succeed");
        assert!(
            out.starts_with("---\n") && out.contains("oxios:"),
            "canonical form must carry ---\\noxios:; got: {out:?}"
        );
        // Round-trip: reparse the canonical form and recover the meta.
        let reparsed = read_note_meta(&out)
            .expect("canonical form must parse")
            .expect("oxios: present");
        assert_eq!(reparsed.author, "agent");
        assert_eq!(reparsed.quality, NoteQuality::Raw);
    }

    #[test]
    fn user_authored_means_no_oxios_table() {
        assert!(
            read_note_meta(
                "---\nid: a\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\n---\nbody"
            )
            .unwrap()
            .is_none(),
            "frontmatter without `oxios:` key is user-authored"
        );
        assert!(
            read_note_meta("plain body, no frontmatter")
                .unwrap()
                .is_none(),
            "no-fence content returns None"
        );
    }

    #[test]
    fn read_note_body_strips_frontmatter_and_hard_fails_on_malformed() {
        // Memo with an oxios: table ⇒ body only, fence gone.
        let body = read_note_body(
            "---\nid: a\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:00:00Z\noxios:\n  author: agent\n  needs_review: true\n---\n# Curate me\n",
        )
        .expect("memo must parse");
        assert_eq!(body, "# Curate me\n");
        assert!(!body.contains("---"), "frontmatter must be stripped");

        // BodyOnly content passes through verbatim.
        assert_eq!(
            read_note_body("plain body, no frontmatter").unwrap(),
            "plain body, no frontmatter"
        );

        // Malformed frontmatter is a hard error — never silently
        // returned as the body (the legacy bespoke parser did that,
        // feeding frontmatter to the curation LLM).
        assert!(
            read_note_body("---\nfoo: [unclosed\n---\nbody").is_err(),
            "malformed frontmatter must be a hard parse error"
        );
    }

    #[test]
    fn write_note_synthesizes_and_preserves() {
        let tmp = tempfile::tempdir().unwrap();
        let now = datetime!(2026-08-21 00:00 UTC);

        // New doc: write_note must rewrite the file with a canonical
        // frontmatter block (id/created/updated synthesized).
        let rel = "brain/Rust.md";
        let outcome =
            write_note(tmp.path(), rel, "# Rust\n\nOwnership rules.", now).expect("write_note");
        assert_eq!(outcome, WriteOutcome::Written);
        let bytes = std::fs::read(tmp.path().join(rel)).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.starts_with("---\n"),
            "must have frontmatter; got: {text}"
        );
        assert!(text.contains("id:"), "must synthesize id; got: {text}");
        assert!(
            text.contains("created:"),
            "must synthesize created; got: {text}"
        );
        assert!(
            text.contains("updated:"),
            "must synthesize updated; got: {text}"
        );
        assert!(
            text.contains("Ownership rules."),
            "body preserved; got: {text}"
        );

        // Rewrite with identical body content → NoOp.
        let outcome2 = write_note(tmp.path(), rel, "# Rust\n\nOwnership rules.", now).unwrap();
        assert_eq!(outcome2, WriteOutcome::NoOp);

        // Pre-seed the file with an editor block carrying an unknown
        // key, then write new content with a different unknown key:
        // both must survive and the body must not start with a fence.
        let _ = std::fs::write(
            tmp.path().join(rel),
            "---\nid: pre-existing-id\nlegacy_key: kept\n---\n# Rust\n\nOwnership rules.\n",
        );
        let editor_input =
            "---\ntags: [rust, design]\ncustom_key: hello\n---\n# Rust\n\nOwnership rules.\n";
        let outcome3 = write_note(tmp.path(), rel, editor_input, now).unwrap();
        assert_eq!(outcome3, WriteOutcome::Written);
        let text2 = std::fs::read_to_string(tmp.path().join(rel)).unwrap();

        // Parse the written file and assert the table is exactly what
        // we expect — strong check (no string-contains footgun).
        let parsed = parse(&text2, NoteFormat::Markdown).expect("written file must parse");
        let Parsed::Memo { table, body } = parsed else {
            panic!("written file must have frontmatter; got: {text2}")
        };
        // Existing-file keys carry forward when absent from incoming.
        assert!(
            table.contains_key("id"),
            "pre-existing id must remain; got table keys: {:?}",
            table.keys().collect::<Vec<_>>()
        );
        assert!(
            table.contains_key("legacy_key"),
            "pre-existing foreign key must carry forward; got table keys: {:?}",
            table.keys().collect::<Vec<_>>()
        );
        // Incoming keys overwrite/land alongside existing.
        assert!(
            table.contains_key("tags"),
            "editor-supplied tags key must survive; got table keys: {:?}",
            table.keys().collect::<Vec<_>>()
        );
        assert!(
            table.contains_key("custom_key"),
            "editor-supplied custom_key must survive; got table keys: {:?}",
            table.keys().collect::<Vec<_>>()
        );
        // No oxios: key — write_note does NOT add it (that's
        // with_oxios_table's job, which uses write_note under the
        // hood but adds the oxios: row first).
        assert!(
            !table.contains_key("oxios"),
            "write_note must NOT add an oxios: row; got table keys: {:?}",
            table.keys().collect::<Vec<_>>()
        );
        // Body must NOT start with a fence line (review #2:
        // contains() alone would pass if the key leaked into the body).
        assert!(
            !body.starts_with("---"),
            "body must not start with a fence; got body: {body:?}"
        );
        assert!(
            body.contains("Ownership rules."),
            "body must contain the user content; got: {body}"
        );
    }

    #[test]
    fn write_note_system_path_is_raw_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let now = datetime!(2026-08-21 00:00 UTC);

        let rel = "Chat.md";
        let content = "free-form chat log, no frontmatter expected\n";
        let outcome = write_note(tmp.path(), rel, content, now).unwrap();
        assert_eq!(outcome, WriteOutcome::Written);
        let bytes = std::fs::read(tmp.path().join(rel)).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert_eq!(text, content, "system path gets raw bytes");
        assert!(!text.starts_with("---\n"), "no frontmatter synthesized");

        // Identical rewrite → NoOp.
        let outcome2 = write_note(tmp.path(), rel, content, now).unwrap();
        assert_eq!(outcome2, WriteOutcome::NoOp);

        // config.json is also a system path (non-.md short-circuit).
        let cfg = "{\"k\":1}";
        let outcome3 = write_note(tmp.path(), "config.json", cfg, now).unwrap();
        assert_eq!(outcome3, WriteOutcome::Written);
        let cfg_bytes = std::fs::read(tmp.path().join("config.json")).unwrap();
        assert_eq!(cfg_bytes, cfg.as_bytes());
    }

    /// Round-2 covering test. When the merged table is semantically
    /// equal to the on-disk table (e.g. the editor rewrites the
    /// frontmatter but the merge produces the same key set with the
    /// same `updated`), the `NoOp` decision MUST depend on the body
    /// too: a different incoming body has to result in
    /// `WriteOutcome::Written`, never a silent drop. The early
    /// round-2 prototype reused the existing body in the probe,
    /// which let any table-equivalent rewrite return NoOp even when
    /// the body differed.
    #[test]
    fn write_note_body_change_is_not_a_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let now = datetime!(2026-08-21 00:00 UTC);

        let rel = "brain/Rust.md";

        // Seed the file with a known table T and body "old". Use a
        // frontmatter block so write_memo_with_incoming_table sees a
        // Memo parsed (not a fresh synthesize). The `brain/` parent
        // directory needs to exist because write_note does not
        // create intermediate dirs (path hardening responsibility
        // lives in the caller).
        std::fs::create_dir_all(tmp.path().join("brain")).unwrap();
        let seed = "---\nid: pre-existing-id\ntags: [keep]\n---\nold body\n";
        std::fs::write(tmp.path().join(rel), seed).unwrap();

        // The incoming content has the SAME frontmatter keys plus
        // the same `tags` value (so the merged table equals T
        // semantically) but a DIFFERENT body. The probe must
        // compare the incoming body "new" against the file body
        // "old"; they differ, so a write must occur.
        let incoming = "---\nid: pre-existing-id\ntags: [keep]\n---\nnew body\n";
        let outcome1 = write_note(tmp.path(), rel, incoming, now).expect("first write");
        assert_eq!(
            outcome1,
            WriteOutcome::Written,
            "body change must produce Written, never a silent NoOp"
        );

        // The file body must be the incoming body, not the old one.
        let after_first = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        let parsed1 = parse(&after_first, NoteFormat::Markdown).expect("file parses");
        let Parsed::Memo {
            table: t1,
            body: b1,
        } = parsed1
        else {
            panic!("expected Memo after first write; got BodyOnly; file: {after_first}")
        };
        assert_eq!(b1, "new body\n", "body must reflect the incoming content");

        // `updated` must be bumped to `now` (otherwise the table
        // would already equal a prior state and the next call would
        // be a NoOp by accident).
        assert!(
            t1.contains_key("updated"),
            "updated must be present on a real write; got keys: {:?}",
            t1.keys().collect::<Vec<_>>()
        );

        // A SECOND identical write must now be NoOp: same body,
        // same merged table, same updated timestamp.
        let outcome2 = write_note(tmp.path(), rel, incoming, now).expect("second write");
        assert_eq!(
            outcome2,
            WriteOutcome::NoOp,
            "second identical write must be NoOp"
        );

        // The file body must STILL be the incoming content (the
        // NoOp did not silently strip it).
        let after_second = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        assert_eq!(after_second, after_first, "NoOp must not modify the file");
    }

    /// Whole-branch review fix (P1 — NoOp ordering): production calls
    /// inject a fresh `now_utc()` on every invocation, so the covering
    /// tests here deliberately use DIFFERENT `now` values across calls —
    /// the earlier tests injected one identical `now`, which masked the
    /// ordering bug (the old code inserted `updated = now` into the
    /// merge base BEFORE the semantic-NoOp probe, so any clock advance
    /// made the probe differ and every unchanged editor re-save rewrote
    /// the file, bumping `updated`, re-canonicalizing foreign
    /// formatting, and firing git auto-commit + brain episodes).
    #[test]
    fn write_note_noop_survives_advancing_clock() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("brain")).unwrap();
        let rel = "brain/Rust.md";
        let now1 = datetime!(2026-08-21 00:00 UTC);
        let now2 = datetime!(2026-08-21 09:30 UTC);
        let now3 = datetime!(2026-08-22 14:10 UTC);

        let incoming =
            "---\nid: fixed-id\ncreated: 2026-08-20T00:00:00Z\ntags: [keep]\n---\nstable body\n";
        assert_eq!(
            write_note(tmp.path(), rel, incoming, now1).unwrap(),
            WriteOutcome::Written
        );
        let after_first = std::fs::read_to_string(tmp.path().join(rel)).unwrap();

        // Unchanged editor re-save: the incoming content is exactly what
        // is on disk, but the wall clock moved on (per-call now_utc()).
        // Must be NoOp and leave the file byte-identical.
        assert_eq!(
            write_note(tmp.path(), rel, &after_first, now2).unwrap(),
            WriteOutcome::NoOp,
            "unchanged re-save with an advanced clock must be NoOp"
        );
        let after_resave = String::from_utf8(std::fs::read(tmp.path().join(rel)).unwrap()).unwrap();
        assert_eq!(
            after_resave, after_first,
            "NoOp must leave the file byte-identical (no updated bump, no re-canonicalization)"
        );

        // Changed body with a further-advanced clock ⇒ Written with
        // `updated` bumped to the new `now`, id/created preserved.
        let edited = after_first.replace("stable body", "edited body");
        assert_eq!(
            write_note(tmp.path(), rel, &edited, now3).unwrap(),
            WriteOutcome::Written
        );
        let after_edit = std::fs::read_to_string(tmp.path().join(rel)).unwrap();
        let parsed = parse(&after_edit, NoteFormat::Markdown).expect("file must parse");
        let Parsed::Memo { table, body } = parsed else {
            panic!("edited file must have frontmatter; got: {after_edit}")
        };
        assert_eq!(body, "edited body\n", "body must reflect the edit");
        assert_eq!(
            table.get("updated"),
            Some(&Value::Str(format_offset(now3))),
            "real write must bump updated to the injected now"
        );
        assert_eq!(
            table.get("id"),
            Some(&Value::Str("fixed-id".to_string())),
            "id must carry forward"
        );
        assert_eq!(
            table.get("created"),
            Some(&Value::Str("2026-08-20T00:00:00Z".to_string())),
            "created must carry forward"
        );
    }
}
