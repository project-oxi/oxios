# RFC-050: Ecosystem Vault Unification

> **Status:** Accepted · **Date:** 2026-08-22
> **Authors:** vault-unification plan (Tasks 1–20)
> **Supersedes:** the cross-repo plan recorded in `docs/designs/2026-08-20-vault-unification-design.md`
> **Implementation:** commit chain `9d510fb..a0994151c` on `feat/vault-unification`

## Context

Until 2026-08-19 the user's knowledge was split across per-app stores: oxios
kept its KnowledgeBase at `~/.oxios/workspace/knowledge/` while oximemo kept
its vault at `~/Library/Application Support/com.oximemo.app/vault/`. Two
trees, two indexes, no shared understanding — and the `oxibrain` daemon,
which is the single durable-memory plane of the ecosystem, had no path to
either of them as a continuous source.

The design doc (`docs/designs/2026-08-20-vault-unification-design.md`,
revision 2) makes this concrete:

1. The vault is a lifetime artifact that must outlive any single app
   (RFC-003).
2. Obsidian-class tooling is a latent requirement (oxios-markdown's
   `IGNORED_NAMES` already contains `.obsidian` and its backlink parser
   treats `[[wikilinks]]` as first-class).
3. The correct convergence target is a **restricted YAML subset** we own
   end-to-end, not oximemo's legacy `+++` TOML.

This RFC records the decision and the migration runbook that ships with it.

## Decision

The single vault path is **`~/.oxi/vault/`**. Every oxi app reads and
writes the same `.md` files there. The contract crate
**`oxi-frontmatter`** is the single source of truth for the format and for
the atomic-preserving write API.

### Why a new format instead of converging on oximemo's TOML `+++`

Obsidian (and the whole `---` frontmatter ecosystem) renders `+++` TOML as
body text, and if a user edits properties in Obsidian it writes `---` YAML
into the file regardless of our choice — a dual-format future by
construction. oxios's own RFC-022 frontmatter is already `---` YAML. The
correct convergence target is therefore a **restricted YAML subset** we own
end-to-end. The grammar (`SPEC.md` inside the `oxi-frontmatter` crate) is
normative; the parser is its reference implementation.

### Format — constrained YAML subset (grammar v2)

- `---` fences for Markdown notes; HTML-comment-wrapped for HTML notes.
- Flat `key: value`, nested maps to depth 2 (app tables).
- **Sequences in both forms**: flow `[a, b]` *and* block (`key:` followed
  by `  - item` lines). Obsidian's properties editor serializes lists as
  block sequences and actively converts flow→block on edit (verified
  2026-08-20). Canonical emission uses flow form.
- **Block scalars `|`** for multi-line strings. Obsidian multiline text
  properties use them. Canonical emission of a `\n`-containing string is
  the block-scalar form.
- Scalars: `true`/`false` ⇒ Bool. Everything else is a string (bare or
  quoted). Timestamps, numbers, dates, URLs pass through unquoted — they
  are valid bare strings.
- **Forbidden**: anchors, aliases, multi-document, YAML tags, complex
  keys, tabs, comments (`#` inside a value must be quoted), multi-line
  flow collections, empty values.
- **Edge policy**: UTF-8 only (BOM ⇒ error, not silent BodyOnly); CRLF
  accepted on parse, LF on canonical emission; duplicate keys ⇒ error;
  unclosed `---` ⇒ error with guidance ("a leading `---` must close a
  frontmatter block; for a horizontal rule use `***`"); empty block
  (`---\n---`) parses as an empty table.
- **Canonical emission**: keys `id, created, updated, favorite, deleted`
  first, then unknown keys in original order, then sub-tables
  alphabetically; strings quoted only when necessary (empty,
  leading/trailing space, contains any of `:#,[]{}&*!|>'"%@` or a tab, or
  would parse as Bool).
- **NoOp is semantic, not byte-level**: `write_document` on a semantically
  unchanged file returns `NoOp` and leaves foreign formatting untouched —
  it never "re-canonicalizes" idle files, because a byte rewrite would
  churn mtime and mint a brain episode.

### Schema v1

`id` (UUIDv7, required, immutable), `created`, `updated` (RFC3339,
required), `favorite` (optional), `deleted` (optional; presence =
soft-deleted), plus open app tables (`oxios:` per RFC-022). Deliberately
dropped from oximemo's prior schema: `hash` (derived — recomputed at walk
time) and `tags` (derived from body `#tags`; if present in frontmatter —
e.g. written by Obsidian — read and unioned, never rewritten). Grammar
evolution is additive-only: future versions must parse every v1 file.

### The single write API

```rust
pub fn write_document(
    path: &Path,
    body: &str,
    fmt: NoteFormat,
    mutations: Mutation,      // favorite / deleted overrides
    synth: Synthesize,        // Yes for create/new-doc, No elsewhere
    now: OffsetDateTime,
) -> Result<WriteOutcome, FrontmatterError>; // Written | NoOp
```

- The original unknown keys are obtained by **re-reading the file at write
  time** — in-memory `Memo` never carries them, so no side-channel exists.
  All nine typed re-serialization sites in oximemo are converted.
- Round-trip law, tested: `write_document` with no mutations and identical
  body is a byte-level no-op.
- `atomic_write` (tmp+fsync+rename+dir-fsync) is the only writer — the
  four raw `std::fs::write` sites disappear with this conversion.

### Per-repo changes

**oximemo** (format implementation owner — ships first)

1. Add `crates/oxi-frontmatter` with `SPEC.md`; convert all nine write
   sites to `write_document` (delivers preservation + atomicity together).
2. Clean up the doctor hash-repair dead path.
3. Default vault path becomes `~/.oxi/vault` (`paths.rs:49-66`, `None`
   branch). One-time migration on open, both-exist safe.
4. When `[brain] enabled`, register the vault once on open via
   `BrainClient::connect_default()` + `sync_run(vault, space)`.
   Unavailability is non-fatal (C1).

**oxibrain** (minimal — the watcher is already done)

1. `scan_directory` exclusions (`markdown.rs:66-71`) gain `"Chat.md"` and
   `"Later.md"` **anchored to the vault root** (filename matches apply at
   any depth and would silently skip a user memo named `Chat.md` inside a
   folder). Everything else `.md` stays ingestible.
2. Deleted files do not propagate (no `SyncAction::Deleted`): accepted as
   the P1 append-only design. **Privacy caveat documented**: a deleted
   note's episodes remain searchable until `redact`.
3. ECOSYSTEM.md v1.1 ships — C5 retitled to *"One installation root, one
   owner per subtree"* with `vault/` as a **shared user file space**
   governed by three disciplines (atomic writes through
   `oxi-frontmatter`; no derived index inside `vault/`; last-writer-wins
   per file).

**oxios** (largest change — T11–T19)

1. **Config**: new `[kernel] knowledge_root` (default resolves through
   `~/.oxi/config.toml [vault] path`, itself defaulting to `~/.oxi/vault`;
   `expand_home` applied). Both KB construction sites
   (`src/kernel.rs:132-136` in `handle()` and `:1508-1511` in `build()`)
   switch to it, unified into one shared `Arc<KnowledgeBase>` built once
   in `build()`.
2. **Write path learns the format** (via `oxi-frontmatter`), with an
   explicit **system-file exclusion set**: no frontmatter synthesis for
   `SYSTEM_DIRS` ∪ `SYSTEM_FILES` ∪ {`habits/`, `config.json`} ∪
   non-`.md`. New non-system documents get synthesized frontmatter
   (UUIDv7 id, created, updated). Existing memos keep id/created and all
   unknown keys/app tables; `write_document` handles preservation and
   bumping.
3. **Vault watcher** — debounced (400 ms), read-only; refreshes the
   KnowledgeBase index on file change. `WatchGuard` joins on drop.
4. **Vault-rooted GitLayer** — `~/.oxi/vault/.git/` (oxios-owned). Curation
   goes through `knowledge_git`; foreign-repo adoption is gated by
   `[git] adopt_foreign_repo` and warns loudly when disabled.
5. **oxibrain registration** — `register_vault_source` retry 5 s → 60 s
   cap 10 min with dir recheck; `resolve_space` reads the canonical
   `~/.oxi/config.toml [vault] space` first, with per-app overrides
   warning loudly. `SyncOutcome` is logged at every step.
6. **Security + backup** — agent deny list now covers the `~/.oxi/` whole
   root and the `~/.oxios/` sensitive-subpath set (workspace/mount grants
   preserved); backups relocate to `~/.oxios/backups/` and are denied by
   construction. `OXIOS_HOME_DENY_SUBPATHS` is a single source of truth
   shared by web/run gates.
7. **One-time migration binary `oxios-migrate-vault`** — converts legacy
   oximemo v3 TOML frontmatter (`+++`) into v4 YAML (`---`) blocks via
   the shared frontmatter contract; imports git history; cross-tree
   collision pre-write refuses with `MergeRequired`; HEAD-tracked removal
   gated; atomic verbatim copy. Default invocation is a dry-run;
   `--apply` performs backup → move/convert → git history import.

## Migration runbook

This is what a human operator runs once. All steps are idempotent or
guarded; the runbook never silently skips a step.

### 1. Stop both apps and the brain

```bash
osascript -e 'tell application "oximemo" to quit'
launchctl bootout gui/$UID/com.oxi.oxibrain 2>/dev/null || true
```

### 2. Preview the migration

```bash
cargo run -p oxios --bin oxios-migrate-vault --release
```

The output is a JSON report listing every moved file, every converted
frontmatter block, and every collision. No files are written in preview
mode.

### 3. Apply

```bash
cargo run -p oxios --bin oxios-migrate-vault --release -- --apply
```

Steps performed, in order:

1. **Backup** the prior `~/.oxios/workspace/knowledge/` to
   `~/.oxios/backups/knowledge-YYYYMMDD-HHMMSS/` (symlink-safe; atomic
   copy via the migration binary's verbatim path).
2. **Move** the entire tree (`.trash/`, `_assets/`, `oximemo.toml`
   included) to `~/.oxi/vault/`.
3. **Convert** every `+++` TOML frontmatter block into a `---` YAML
   block via `oxi-frontmatter::parse` + canonical `emit`; synthesize a
   fresh `id` (UUIDv7), `created`, `updated` for files that don't carry
   one; preserve every unknown key and the `oxios:` app table.
4. **Import git history** — for each file that existed in the prior
   `~/.oxios/workspace/knowledge/.git/`, replay the commit graph into the
   new vault-rooted `~/.oxi/vault/.git/`. Files that were never tracked
   are skipped (first-commit date becomes the file's mtime).
5. **Verify** — re-walk the new vault with
   `oxi-frontmatter::parse` on every `.md`; `BodyOnly` files are
   invisible to the index but still on disk; **a malformed block is a
   hard error** (no silent BodyOnly fallback).

### 4. Restart the brain

```bash
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.oxi.oxibrain.plist
```

`BrainSupervisor` re-adopts the vault source from its `sources` table;
no manual `oxibrain sync` is needed.

### 5. Verify

```bash
scripts/vault-unification-smoke.sh
```

The smoke script initializes a temp `HOME`, builds the vault, exercises
the end-to-end flow (oximemo CLI creates a note → oxios HTTP API sees it
in tree/backlinks → oxios writes a doc → `oximemo reindex` lists it →
`oxibrain serve --daemon` + `sync_run` → `stats`/`search` reflect both →
root `Chat.md` absent from episodes → daemon stop → both file
operations still work), asserts at every step, and exits non-zero on
failure.

### 6. Roll back (if anything looks wrong)

```bash
# 1. stop everything
launchctl bootout gui/$UID/com.oxi.oxibrain 2>/dev/null
osascript -e 'tell application "oximemo" to quit'

# 2. restore from backup
rm -rf ~/.oxi/vault
mv ~/.oxios/backups/knowledge-YYYYMMDD-HHMMSS ~/.oxios/workspace/knowledge

# 3. restart
launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.oxi.oxibrain.plist
```

## Consequences

- **Two knowledge systems remain, on one file tree.** Agent memory =
  `oxibrain` daemon (RFC-047). User notes = `~/.oxi/vault/` (frontmatter
  required for first-class index visibility; `BodyOnly` is the visibility
  boundary, unchanged). The `~/.oxios/knowledge/` path that appears in
  older onboarding docs is **retired** — see the AGENTS.md correction in
  this changeset.
- **One format, three writers.** Every write goes through
  `oxi-frontmatter::write_document`. The contract is dependency-free
  (`uuid`, `time`, `indexmap`, `thiserror` only).
- **Atomic-only.** Raw `std::fs::write` to a vault `.md` is a contract
  violation.
- **Schema evolution is additive-only.** Future versions must parse
  every v1 file.
- **Vault root is `~/.oxi/vault/`.** The previous `~/.oxios/workspace/knowledge/`
  default is moved by the migration binary and the new `~/.oxios/knowledge/`
  path is **gone** — `AGENTS.md` and `CHANGELOG.md` reflect this.

## References

- Design: `docs/designs/2026-08-20-vault-unification-design.md` (revision 2)
- Plan: `docs/superpowers/plans/2026-08-20-vault-unification.md`
- Spec: `crates/oxi-frontmatter/SPEC.md` (oximemo repo)
- Related: RFC-003 (knowledge separation), RFC-022 (note provenance),
  RFC-047 (oxibrain migration), RFC-048 (Oxi Foundation), ADR-010
  (daemon-hosted vault watch), ECOSYSTEM.md v1.1 (C5 amendment).
