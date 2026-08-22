//! One-time ecosystem vault migration binary (Task 19, design §7 of
//! `docs/designs/2026-08-20-vault-unification-design.md`).
//!
//! Joins two pre-unification sources into the shared ecosystem vault:
//!
//! 1. The oximemo pre-unification default vault
//!    (`~/Library/Application Support/com.oximemo.app/vault`), moved with
//!    `+++`TOML → `---`YAML frontmatter conversion.
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
//! missing ∧ target exists) is tolerated and reported.

use std::path::{Path, PathBuf};

use oxi_frontmatter::{Table, Value};

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
    /// oximemo v3 `+++`TOML → v4 `---`YAML conversion.
    ConvertToml { src: PathBuf, rel: String },
    /// Knowledge document gaining/extending its v4 frontmatter block.
    ConvertDoc {
        src: PathBuf,
        rel: String,
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
}

/// All paths the migration resolves between (injected for tests).
#[derive(Debug, Clone)]
struct Paths {
    home: PathBuf,
    oxios_home: PathBuf,
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
    todo!()
}

/// Split a v3 markdown note into (TOML frontmatter text, body).
/// `Ok(None)` = no `+++` fence (system or foreign file).
fn split_v3_markdown(content: &str) -> Result<Option<(String, String)>, String> {
    todo!()
}

/// Map a parsed v3 TOML table onto the v4 table shape.
fn map_v3_to_v4(raw: &toml::Table) -> Result<Table, String> {
    todo!()
}

/// Convert a full v3 oximemo note (frontmatter + body) to v4 content.
fn convert_toml_note(content: &str) -> Result<String, String> {
    todo!()
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
    todo!()
}

/// Oldest (first) author-date line of `git log --diff-filter=A
/// --format=%aI` output; git lists newest first, so the first commit
/// that added the file is the **last** non-empty line.
fn first_commit_date(output: &str) -> Option<&str> {
    todo!()
}

/// created := git first-commit date | mtime | now.
fn created_from(git_log: Option<&str>, mtime: Option<&str>, now: &str) -> String {
    todo!()
}

fn resolve_paths(home: &Path) -> Result<Paths, String> {
    todo!()
}

fn build_plan(paths: &Paths, now_rfc3339: &str) -> Result<Plan, String> {
    todo!()
}

fn run(paths: &Paths, apply: bool, now_rfc3339: &str) -> Result<Outcome, String> {
    todo!()
}

fn main() {
    todo!()
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
                let expected = rel == &"projects/rfc.md";
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
