# Ecosystem Vault Unification — Design

> **Date:** 2026-08-20
> **Status:** Proposed (awaiting review)
> **Scope:** oxios, oximemo, oxibrain (cross-repo, oxios coordinates)
> **Approach:** B — converge on the oximemo file format; one shared vault at `~/.oxi/vault/`
> **Related:** oxibrain `doc/ECOSYSTEM.md` v1.0 (three-plane topology, C1–C8), ADR-010
> (daemon-hosted vault watch), RFC-003 (knowledge separation), RFC-022 (note provenance),
> RFC-047/048 (brain migration, foundation)

## 1. Motivation

The user's knowledge is scattered across per-app stores: oxios keeps a KnowledgeBase at
`~/.oxios/workspace/knowledge/` while oximemo keeps a vault at
`~/Library/Application Support/com.oximemo.app/vault/`. The ecosystem topology
(ECOSYSTEM.md v1.0) already made oxibrain the single durable-memory plane and reserved
`~/.oxi/` as the one installation root — but no vault subtree exists there yet. The goal:
**one vault, accumulated over a lifetime, shared by every oxi app, understood by the brain.**

Three facts from the cross-repo investigation make this cheap and safe now:

1. **oxibrain is done.** ADR-010 (accepted 2026-08-20) ships the daemon-hosted, 2
   s-debounced, incremental vault watcher. One `BrainClient::sync_run(dir, space)` call
   registers a pull source; the daemon re-adopts it after restarts from its `sources`
   table. The connector already scans `.md`/`.html` and excludes oximemo machinery
   (`oximemo.toml`, `_assets/`, `TEMPLATE.*`, dot-dirs, legacy `config.toml`).
2. **The oximemo format is already share-shaped.** Files are the source of truth; every
   persistent meta field (id, created_at, updated_at, hash, favorite, tags, deleted_at)
   lives in the `+++` TOML frontmatter; redb/tantivy are 100 % rebuildable caches; memo
   files are never locked (atomic-rename writes; per-op flock guards only the index
   files). Files without frontmatter parse as `BodyOnly` and are **invisible to the
   oximemo index** (`FileStore::read_memo` returns `Ok(None)`).
3. **The oxios KnowledgeBase is root-agnostic and self-contained.** All state is `.md`
   files plus `config.json`; every app feature (Chat.md inbox, journal, habits,
   checklists, nightly worker) is pure derived logic over those files. The KB is
   instantiated at exactly one place (`src/kernel.rs:151-157`).

## 2. Goals and invariants

**Goal:** `~/.oxi/vault/` is the single place where the user's knowledge markdown
accumulates. oxios and oximemo read and write the same files; oxibrain understands all
of it; each app keeps its own derived indexes.

Invariants — a design that breaks any of these is rejected:

1. **`.md` files are the only source of truth.** All persistent state lives inside the
   file (frontmatter included). Every index is a rebuildable projection.
2. **Frontmatter is the visibility boundary.** A file with `+++` frontmatter is a memo
   (first-class in oximemo). A file without it is `BodyOnly` — hidden from the oximemo
   index. No reserved-name registries are needed for cross-app machinery files.
3. **The brain is additive (C1).** With the daemon stopped, every app keeps full file
   functionality; only brain panels degrade. Ingestion has exactly one path: the daemon
   pull connector.
4. **Atomic writes only, never file locks.** Every `.md` write is tmp+fsync+rename.
   Locks exist only around each app's own index files.
5. **One frontmatter block per file.** The oxios RFC-022 YAML frontmatter is retired;
   its fields move into a namespaced `[oxios]` TOML table inside the single block.

## 3. Final architecture

```
~/.oxi/
├── foundation/v1/                    # unchanged
├── brain/                            # unchanged — oxibrain store, daemon is sole writer
│   └── oxibrain.sock
└── vault/                            # NEW — the source of truth for knowledge
    ├── oximemo.toml                  # vault config (owned by oximemo)
    ├── <folder>/<slug>.md            # user documents = memos (frontmatter required)
    │   +++
    │   id = "…"                      # UUIDv7
    │   created_at/updated_at/hash/favorite/tags
    │   [oxios]                       # optional oxios provenance (RFC-022 successor)
    │   author = "agent" | "user"
    │   source/quality/needs_review/saved_at
    │   +++
    ├── Chat.md, Later.md, Done.md…   # oxios app files — no frontmatter ⇒ BodyOnly
    ├── journal/, habits/, insights/, archive/
    ├── _assets/, .trash/, TEMPLATE.* # oximemo machinery
    └── .git/                         # vault-rooted repo (oxios GitLayer commits)

 oxios (kernel + web)        oximemo (GUI/CLI)           oxibrain daemon
   KnowledgeBase ──┐           Vault ──┐                  pull connector
   (root = vault)  │           (path = vault) │            (watch vault, 2 s debounce)
   backlink + git  │           redb + tantivy │            → episodes → projection
                   ▼           ▼
                the same .md files
```

Data flow: any app writes a file atomically → each app's own watcher refreshes its own
derived index (oximemo 300 ms, oxios 400 ms — new) → the daemon watcher ingests the
settled change incrementally (content-hash no-op for unchanged files) → every app reads
the whole knowledge space through brain search.

Who owns what:

| Concern | Owner |
|---|---|
| File format (frontmatter schema, hashing, atomic write) | `oximemo-format` crate (new, §4) |
| Vault config file (`oximemo.toml`), memo indexing | oximemo |
| App-feature files (Chat.md, journal, habits, …) and their parsers | oxios (`oxios-markdown`) |
| Backlink/graph index, git history | oxios (per-app derived) |
| Understanding (episodes, entities, retrieval) | oxibrain daemon (sole writer of `~/.oxi/brain/`) |

## 4. Contract crate — `oximemo-format` (new, lives in the oximemo repo)

Approach B means oximemo owns the file format. Ownership is enforced by extracting the
pure parts of `oximemo-core/src/store/files.rs` into a dependency-free crate that both
sides consume:

- `Frontmatter` schema v3 (id, created_at, updated_at, hash, favorite, tags,
  deleted_at) + parse/serialize for Markdown and HTML (comment-wrapped) forms.
- **Unknown-key-preserving update.** Today `serialize_as` re-serializes only the typed
  struct, which would silently drop an `[oxios]` table whenever oximemo edits a note
  (verified: `Frontmatter::from_memo` → `toml::to_string`). The preserving path parses
  the frontmatter as a `toml::Table`, mutates the known keys, and re-serializes the
  whole table. Without this, cross-app editing destroys metadata.
- `hash_memo` (normalized body+favorite, blake3, `b3:` prefix), `atomic_write`
  (tmp+fsync+rename+dir-fsync), `NoteFormat`.
- No redb/tantivy/fs2/notify dependencies — safe for `oxios-markdown` (which must stay
  pure) to depend on.

`oximemo-core` replaces its internal implementation with this crate; `oxios-markdown`
gains it as its format layer.

## 5. Per-repo changes

### 5.1 oximemo (format owner — ships first)

1. Extract `crates/oximemo-format` (§4); convert `oximemo-core` to consume it.
2. Default vault path becomes `~/.oxi/vault` (`paths.rs`). One-time auto-migration on
   open: if the old default vault exists and the new path does not, move the files and
   `reindex`. Custom `--vault` / `OXIMEMO_VAULT` users are unaffected. The derived
   index for the custom path already lands under `index/by-vault/<blake3-16>` —
   existing mechanism, no change.
3. When `[brain] enabled`, register the vault once on open via
   `BrainClient::connect_default()` + `sync_run(vault, space)` (idempotent;
   unavailability is non-fatal — C1).

### 5.2 oxibrain (minimal — the watcher is already done)

1. Add `"Chat.md"` and `"Later.md"` to `scan_directory` exclusions — high-churn inbox
   state should not become episodes (every settled checkbox toggle would otherwise run
   GGUF extraction). Everything else `.md` stays ingestible: journal/habits/insights/
   archive/Done.md are life-log knowledge, exactly the material C2 exists for
   ("a Tuesday routine, a note from March … concern the same entity").
   `config.json` needs nothing — the scanner only accepts `.md`/`.html`.
2. Deleted files still do not propagate (no `SyncAction::Deleted`): accepted as the
   P1 append-only design, unchanged from today's lens behavior. Episodes of deleted
   notes remain as history; explicit cleanup is `redact`, if ever needed.

### 5.3 oxios (largest change)

1. **Config**: new `[kernel] knowledge_root` (default `~/.oxi/vault`, `expand_home`
   applied). Single instantiation point `src/kernel.rs:151-157` switches to it.
2. **Write path learns the format** (via `oximemo-format`):
   - New documents get synthesized frontmatter (UUIDv7 id, created_at, updated_at,
     hash, favorite=false, tags=[]).
   - Existing memos keep id/created_at and the `[oxios]` table; every write bumps
     updated_at and recomputes hash.
   - No-op guard: if body and metadata are unchanged, do not write at all — a
     rewritten identical file would still look modified to the brain (whole-file
     content hash) and mint a pointless episode.
   - Legacy RFC-022 YAML frontmatter is converted to the `[oxios]` TOML table on
     first write (single frontmatter block per file).
3. **`KnowledgeBase::watch` (new)**: `notify`-based recursive watcher, ~400 ms debounce
   (mirrors oximemo `watcher.rs`). On settled external changes: reindex backlinks for
   the changed paths and fire `on_file_change` so the existing git consumer commits
   external edits too. Without this, oxios's in-memory backlink index goes stale on
   oximemo edits.
4. **GitLayer second instance** rooted at the vault. Knowledge routes
   (history/restore/diff) and the auto-commit consumer use the vault instance; the
   workspace repo keeps tracking sessions and the rest. `note_restore` keeps its
   callback-suppression (restore→commit causality loop guard).
5. **Lens de-duplication**: delete `knowledge_lens::index_to_brain` (it duplicates the
   daemon pull connector with a different source identity — double ingestion). The
   kernel bootstraps brain registration instead: on startup/brain-availability, call
   `sync_run(knowledge_root, space)` once (space defaults to `personal` — both apps'
   existing brain space configuration; registration is idempotent because the source
   identity is the canonical vault path). Lens keeps copilot context search.
6. **Migration routine** (§7), AGENTS.md path correction (`~/.oxios/knowledge/` is
   stale — the real path today is `~/.oxios/workspace/knowledge/`).

## 6. Policy matrix

| File / dir | oximemo index | oxios app feature | brain ingest | git |
|---|---|---|---|---|
| `<folder>/<slug>.md` (frontmatter) | first-class memo | document editing | yes | yes |
| Chat.md, Later.md | hidden (BodyOnly) | inbox / working set | **no — new exclusion** | yes |
| Done.md, Shop/Watch/Read.md | hidden | archive / lists | yes | yes |
| journal/, habits/, insights/, archive/ | hidden | life log | yes — core brain food | yes |
| config.json, _assets/, .trash/, TEMPLATE.*, oximemo.toml | machinery | — | no (non-`.md` or excluded) | yes |

## 7. Migration (one-time script)

1. Create `~/.oxi/vault/`, initialize `oximemo.toml`.
2. Move the existing oximemo default vault contents in (already format-conformant).
3. Move `~/.oxios/workspace/knowledge/` contents in:
   - User documents (every `.md` outside the oxios system set — exactly the
     `SYSTEM_DIRS`/`SYSTEM_FILES` constants in `oxios-markdown/src/fs.rs`) get
     synthesized frontmatter; created_at prefers the file's first git commit date,
     falling back to mtime. Existing YAML `oxios:` frontmatter converts to the
     `[oxios]` table.
   - System files (Chat.md, Later.md, Done.md, Shop/Watch/Read.md, `journal/`,
     `habits/`, `insights/`, `archive/`, `media/`, `img/`, `config.json`) move
     verbatim, staying frontmatter-less.
4. Git history: extract the `knowledge/` subtree from the workspace repo with
   `git filter-repo --subdirectory-filter knowledge` into the new vault repo (git CLI is
   allowed once in the migration script — GitLayer's no-CLI rule is a runtime property).
   Fallback: fresh init; the old workspace repo is preserved, so old file history
   remains reachable there.
5. Commit the removal in the workspace repo; leave legacy `knowledge:lens` brain
   episodes in place (immutable ledger; consolidation absorbs the duplication).
6. Reindex both apps; run `sync_run` once; verify with the smoke test below.

Dry-run mode reports every planned move/frontmatter synthesis before touching disk;
the source tree is backed up before execution.

## 8. Concurrency, failure, performance

- **Concurrent writes**: atomic renames give last-writer-wins per file. The oxios web
  API's ETag (`If-Match`) optimistic concurrency compares file content — unaffected.
- **Daemon down**: files, editing, git, backlinks all live; brain panels degrade (C1).
  On daemon restart, watchers self-restore from the `sources` table.
- **Performance**: backlink reindex touches changed paths only; oximemo's derived index
  lives in its app-support directory (namespaced per vault — existing mechanism);
  brain ingestion is 2 s-debounced with content-hash no-ops.

## 9. Testing

Per-repo gates (fmt/clippy/test) plus one integration smoke on a shared temp vault:

1. oximemo creates a memo → oxios watcher refreshes backlinks/graph; web UI sees it.
2. oxios writes a document → oximemo `reindex` shows it as a memo with a stable id;
   editing it in oximemo preserves the `[oxios]` table.
3. Brain watcher turns both changes into episodes; Chat.md stays excluded.
4. With the daemon stopped, both apps remain fully functional for file operations.
5. No-op write produces no file change (mtime/content unchanged → no new episode).

## 10. Risks and deferred items

- **R1** — frontmatter synthesis bugs during migration: mitigated by dry-run report +
  backup; migration is reversible by restoring the backup.
- **R2** — oximemo folder counts include BodyOnly files (walk-based counts): cosmetic,
  accepted.
- **Deferred**: unifying the backlink parsers (oximemo `wiki.rs` vs oxios
  `parser.rs` — both are derived projections, coexistence is legal); hiding frontmatter
  in the oxios web editor UX; per-path brain ingestion rate limits. All follow-up
  material, not in this design.

## 11. Sequencing

1. **oximemo**: `oximemo-format` extraction + preserving update + default-path
   migration + brain registration. (Contract ships first.)
2. **oxibrain**: exclusion-list addition. (Tiny, parallel with 1.)
3. **oxios**: config root, format-aware writes, watcher, vault GitLayer, lens
   de-duplication, migration routine. (Depends on published `oximemo-format`.)
4. **Docs**: ECOSYSTEM.md v1.1 adds the `vault/` subtree to the C5 layout;
   RFC-050 records the migration; oximemo DESIGN.md updated.
