# Ecosystem Vault Unification — Design

> **Date:** 2026-08-20 · **Revision:** 2 (post-review + format redesign)
> **Status:** Proposed — amended after 3-way independent code review (oximemo / oxios /
> ecosystem-contract) and a fresh file-format decision (§4)
> **Scope:** oxios, oximemo, oxibrain (cross-repo, oxios coordinates)
> **Approach:** B — one shared vault at `~/.oxi/vault/`, converged on a **new neutral
> format** (`oxi-frontmatter`, constrained YAML subset), not on oximemo's legacy TOML
> **Related:** oxibrain `doc/ECOSYSTEM.md` v1.0 (three-plane topology, C1–C8), ADR-010
> (daemon-hosted vault watch), RFC-003 (knowledge separation), RFC-022 (note provenance —
> its `---` YAML `oxios:` block is natively conformant with the new format),
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
2. **The oximemo storage shape is share-ready — with two caveats the review exposed.**
   Files are the source of truth; memo files are never locked; files without frontmatter
   are invisible to the oximemo index (`BodyOnly`). Caveats: (a) every re-serialization
   today round-trips through the typed `Frontmatter` struct, dropping unknown keys at
   **nine** write sites (§4.1); (b) four of those sites use raw `std::fs::write` —
   non-atomic (§4.2). Both are fixed by this design.
3. **The oxios KnowledgeBase is root-agnostic and self-contained.** All state is `.md`
   files plus `config.json`; every app feature is pure derived logic over those files.
   The KB is constructed at **two** sites (`src/kernel.rs:132-136` in `handle()` and
   `:1508-1511` in `build()`, the latter feeding `PersistenceHook`) — both must switch
   together or the two halves of oxios split-brain on different roots.

**Why a new format instead of converging on oximemo's TOML `+++`:** the vault is a
lifetime artifact that must outlive any single app (RFC-003: knowledge exists
"independently of the agent OS"). Obsidian-class tooling is already a latent
requirement — oxios-markdown's `IGNORED_NAMES` contains `.obsidian` and its backlink
parser treats `[[wikilinks]]` as first-class. Obsidian (and the whole `---` frontmatter
ecosystem) renders `+++` TOML as body text, and if a user edits properties in Obsidian
it writes `---` YAML into the file regardless of our choice — a dual-format future by
construction. oxios's own RFC-022 frontmatter is already `---` YAML. The correct
convergence target is therefore a **restricted YAML subset** we own end-to-end (§4).

## 2. Goals and invariants

**Goal:** `~/.oxi/vault/` is the single place where the user's knowledge markdown
accumulates. oxios and oximemo read and write the same files; oxibrain understands all
of it; each app keeps its own derived indexes.

Invariants — a design that breaks any of these is rejected:

1. **`.md` files are the only source of truth.** All persistent state lives inside the
   file (frontmatter included). Every index is a rebuildable projection. Derived values
   (content hashes, tags) are recomputed, not stored.
2. **Frontmatter is the visibility boundary.** A file whose first line is `---` (with a
   conformant block) is a memo — first-class in oximemo. A file without it is
   `BodyOnly` — hidden from the oximemo index. Cross-app machinery files stay
   frontmatter-less by construction: the write path never synthesizes frontmatter onto
   the system set (§5.3.2). A malformed block is a **hard error**, never silently
   treated as body.
3. **The brain is additive (C1).** With the daemon stopped, every app keeps full file
   functionality; only brain panels degrade. Ingestion has exactly one path: the daemon
   pull connector.
4. **Atomic writes only, never file locks — as an enforced requirement, not an
   assumption.** Every `.md` write in every app goes through the format crate's
   atomic-preserving write. The four current raw-`fs::write` paths in oximemo
   (`vault.rs:482-483` in-place update, `:514-516` link propagation, `:563-565`
   tombstone, `:588-590` restore) are converted; direct `fs::write` to a vault `.md`
   is a contract violation. Locks exist only around each app's own index files.
5. **One frontmatter block per file, preserved by every writer.** Unknown keys and app
   tables (e.g. `oxios:`) survive every rewrite in every app, byte-stably.

## 3. Final architecture

```
~/.oxi/
├── foundation/v1/                    # unchanged
├── config.toml                       # shared — gains [vault] (canonical, §5.4)
├── brain/                            # unchanged — oxibrain store, daemon is sole writer
│   └── oxibrain.sock
└── vault/                            # NEW — shared user file space (ECOSYSTEM C5 v1.1)
    ├── oximemo.toml                  # vault config (owned by oximemo)
    ├── <folder>/<slug>.md            # user documents = memos (frontmatter required)
    │   ---
    │   id: 01991a2e-7c3f-7c91-9f3e-6b1a2e8f9c10     # UUIDv7, immutable
    │   created: 2026-07-28T10:15:03+09:00            # RFC3339
    │   updated: 2026-08-20T13:40:00+09:00
    │   favorite: false                                # optional
    │   deleted: 2026-08-01T09:00:00+09:00             # optional — presence = soft-delete
    │   oxios:                                         # optional app table (RFC-022 native)
    │     author: agent
    │     source: tool · quality: raw · needs_review: true
    │   ---
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
| File format (`oxi-frontmatter` spec + parser + atomic-preserving write) | `oxi-frontmatter` crate (new, §4) |
| Vault config file (`oximemo.toml`), memo indexing | oximemo |
| App-feature files (Chat.md, journal, habits, …) and their parsers | oxios (`oxios-markdown`) |
| Backlink/graph index, git history | oxios (per-app derived) |
| Understanding (episodes, entities, retrieval) | oxibrain daemon (sole writer of `~/.oxi/brain/`) |

**Contract amendment required (review F1):** ECOSYSTEM.md v1.0 C5 says "each subtree has
exactly one writer". This design's `vault/` has two writing apps, which violates the
letter while honoring the intent (the rule exists to prevent multi-writer corruption of
a durable store). ECOSYSTEM v1.1 (§5.2.3) must retitle C5 to *"One installation root,
one owner per subtree"* and add `vault/` as a **shared user file space** governed by
three disciplines: (1) every write goes through the `oxi-frontmatter` contract — atomic
tmp+fsync+rename, single frontmatter block, unknown-key-preserving serialization;
(2) no app places a derived index, cache, or lock inside `vault/` — derived state lives
in each app's own support directory; (3) per-file conflicts resolve last-writer-wins,
and cross-app visibility is by frontmatter convention, never by reserved names. Owned
subtrees keep exactly one writer: the daemon writes `brain/` and nothing else; hosts
never write `foundation/v1/`.

## 4. Contract crate — `oxi-frontmatter` (new, neutral)

Lives in the oximemo repo (pragmatic: the authoring app; relocation to its own repo
later is consumer-invisible via crates.io), under a **neutral name** — it is the
ecosystem's format, not oximemo's. `SPEC.md` inside the crate is the normative
contract; the parser is its reference implementation. Dependencies: none beyond
`uuid`, `time` (pure).

### 4.1 The format — constrained YAML subset

- **Markdown notes**: first line `---`, block of conformant lines, closing `---`.
  **HTML notes** (oximemo only): the same block wrapped in an HTML comment.
- **Allowed**: flat `key: value`; nested maps to depth 2 (for app tables); flow arrays
  of scalars `[a, b]`; scalars are bool / int / RFC3339 timestamp / string (bare or
  quoted). Canonical emission: unquoted where legal, keys in canonical order
  (`id`, `created`, `updated`, `favorite`, `deleted`, then unknown keys in original
  order, then app tables alphabetically).
- **Forbidden**: anchors/aliases, multi-document, YAML tags, block scalars, complex
  keys, tabs. Violations are a parse **error** surfaced to the caller — never a silent
  fallback to body (invariant 2). Obsidian's properties-UI output is plain flat
  scalars/arrays — inside the subset, absorbed as-is.
- **Schema v1**: `id` (UUIDv7, required, immutable), `created`, `updated` (RFC3339,
  required), `favorite` (optional), `deleted` (optional; presence = soft-deleted),
  plus open app tables (`oxios:` per RFC-022). **Deliberately dropped** from oximemo's
  current schema: `hash` (derived — recomputed at walk time; storing it violated
  invariant 1 and fed the dead doctor path, review minor) and `tags` (already derived
  from body `#tags` by `tags_of`; if present in frontmatter — e.g. written by
  Obsidian — it is read and unioned, never rewritten).

### 4.2 The single write API (review blocker fix)

The review enumerated **nine** re-serialization sites in oximemo — `create_note`
(`vault.rs:351`), `update_note` paths (`:459/:474/:482`), backlink propagation
(`:514-516`), delete tombstone (`:563-565`), `restore_memo` (`:588-590`), `move_note`
(`:874`), doctor-fix (`:1127`) — all typed, all unknown-key-dropping. The fix is
structural, not point-wise:

```rust
/// Merge-write: re-read the target file's frontmatter table (if any), apply the
/// typed mutation (id/created preserved, updated = now, favorite/deleted per
/// caller), keep every unknown key and app table, serialize canonically, write
/// atomically. Synthesizes a fresh block for new files only when `synth` allows.
/// Returns NoOp when nothing changed (body, favorite, deleted all equal) —
/// the updated bump is skipped too (invariant: no pointless churn).
pub fn write_document(
    path: &Path,
    body: &str,
    fmt: NoteFormat,
    mutations: Mutation,      // favorite / deleted overrides
    synth: Synthesize,        // Yes for create/new-doc, No elsewhere
    now: OffsetDateTime,
) -> Result<WriteOutcome, FrontmatterError>; // Written | NoOp
```

- The original unknown keys are obtained by **re-reading the file at write time** —
  in-memory `Memo` never carries them, so no side-channel exists (review blocker
  answer). `serialize_as`/`Frontmatter::from_memo` direct paths are removed from the
  public surface; all nine call sites convert.
- Round-trip law, tested: `write_document` with no mutations and identical body is a
  byte-level no-op.
- `atomic_write` (tmp+fsync+rename+dir-fsync) is the only writer — the four raw
  `std::fs::write` sites disappear with this conversion (§2-4).
- Reads: `parse(content, fmt) -> Result<Parsed>` where
  `Parsed = Memo { table, body } | BodyOnly { body }`, plus a typed view for
  indexing.

## 5. Per-repo changes

### 5.1 oximemo (format implementation owner — ships first)

1. Add `crates/oxi-frontmatter` (§4) with `SPEC.md`; convert all nine write sites to
   `write_document` (delivers preservation + atomicity together). Existing tests stay
   green; sync manifest hashes are recomputed at walk time (no stored `hash`).
2. Clean up the doctor hash-repair dead path (review: `read_memo` discards the stored
   hash, so the comparison at `vault.rs:1121-1122` can never fire).
3. Default vault path becomes `~/.oxi/vault` (`paths.rs:49-66`, `None` branch — the
   new path is a *default*, so the derived index stays at app-support `index/`, not
   `by-vault/`). One-time migration on open, **both-exist safe** (review major): old
   default exists ∧ new absent → move the **entire tree** (`.trash/`, `_assets/`,
   `oximemo.toml` included; the `index-fmt` marker lives in `index_dir` and does not
   move; vault-relative index records survive the move). **Both exist** → no silent
   skip: surface `VaultStatus::MergeRequired` that GUI/CLI must show, offering
   merge-into-vault or keep-old (§7 also handles it explicitly, so this path is mostly
   out-of-order updates). The same migration pass converts note frontmatter
   `+++`TOML→`---`YAML (§7.2).
4. When `[brain] enabled`, register the vault once on open via
   `BrainClient::connect_default()` + `sync_run(vault, space)` — **new code** (today
   registration is manual `oxibrain sync`; oximemo never calls `sync_run`). Space from
   the canonical source (§5.4). Unavailability is non-fatal (C1).
5. Watcher note (review question): `watcher.rs` forwards `.md` only; external `.html`
   edits are not reindexed. Accepted — oxios writes `.md` only. Documented.

### 5.2 oxibrain (minimal — the watcher is already done)

1. `scan_directory` exclusions (`markdown.rs:66-71`) gain `"Chat.md"` and `"Later.md"`
   **anchored to the vault root** (review F5: filename matches apply at any depth and
   would silently skip a user memo named `Chat.md` inside a folder). Test extends
   `scan_skips_template_and_config_files` (`markdown.rs:227-236`); CHANGELOG per
   convention. Everything else `.md` stays ingestible — journal/habits/insights/
   archive/Done.md are life-log knowledge, exactly what C2 exists for. `config.json`
   needs nothing (scanner accepts only `.md`/`.html`). oxibrain never parses
   frontmatter — it scans raw text, so the format change is invisible to it.
2. Deleted files do not propagate (no `SyncAction::Deleted`): accepted as the P1
   append-only design. **Privacy caveat documented** (review F6): a deleted note's
   episodes remain searchable until `redact`. Deferred follow-up (§10):
   `SyncAction::Deleted` → auto-`redact` behind a setting.
3. Ecosystem contract (review F1/F2): ship ECOSYSTEM.md v1.1 — the C5 amendment
   (§3) and `~/.oxi/config.toml` `[vault] space` as the canonical space source both
   apps read (per-app overrides warn loudly, never silently diverge — the review
   showed a two-space registration leaves the second space with one full pass and no
   watcher, which must be structurally unreachable silently).

### 5.3 oxios (largest change)

1. **Config**: new `[kernel] knowledge_root` (default resolves through
   `~/.oxi/config.toml [vault] path`, itself defaulting to `~/.oxi/vault`;
   `expand_home` applied). **Both** KB construction sites switch to it —
   `src/kernel.rs:132-136` (`handle()`) and `:1508-1511` (`build()`, PersistenceHook) —
   preferably unified into one shared `Arc<KnowledgeBase>` built once in `build()`
   (review F2 split-brain).
2. **Write path learns the format** (via `oxi-frontmatter`), with an explicit
   **system-file exclusion set** (review F3/F4): no frontmatter synthesis for
   `SYSTEM_DIRS` ∪ `SYSTEM_FILES` ∪ {`habits/`, `config.json`} ∪ non-`.md`. This
   matters because system files funnel through the same `note_write` (`chat_append` →
   `knowledge.rs:546-561`, checklist `:694`, `set_config` `:667-669`, web PUT
   any-path) and would otherwise gain frontmatter — violating invariant 2 and
   breaking `habit_emoji` (`habits.rs:133-137` reads a bare emoji line).
   - New non-system documents get synthesized frontmatter (UUIDv7 id, created, updated).
   - Existing memos keep id/created and all unknown keys/app tables; `write_document`
     handles preservation and bumping.
   - **No-op precedence** (review F8): body, favorite, and deleted all unchanged →
     skip the write entirely *including* the `updated` bump — re-saving an unchanged
     editor buffer must not mint a brain episode.
   - RFC-022 frontmatter is **already conformant** (`---` YAML with an `oxios:`
     table). `parse_note_meta`/`format_frontmatter` (`knowledge.rs:832-874`) keep
     their `---` detection and gain id/created/updated awareness;
     **user-authored is redefined** as "frontmatter present without an `oxios:`
     table" (frontmatter alone no longer implies user-authored — every document now
     has id/created/updated). Contract consumers adjust on that definition:
     `knowledge_tool.rs:209-216` (write refusal), `persistence_hook.rs:223-231`
     (save skip), `knowledge_curation.rs:207`, `chat.rs:2696-2701`;
     `notes_needing_review` scans `oxios.needs_review`.
   - `note_restore` routes restored content through the same merge (review F9:
     restoring a pre-migration commit re-introduces frontmatter without id/created —
     merge: keep the live file's id if present, else synthesize; keep created, else
     first-commit date, else mtime).
3. **`KnowledgeBase::watch` (new)**: `notify`-based recursive watcher, ~400 ms debounce
   (mirrors oximemo `watcher.rs`); pure-Rust dep, consistent with oxios-markdown's
   purity (review confirms no kernel/AI deps conflict). On settled changes: reindex
   backlinks for changed paths and fire `on_file_change` so the git consumer commits
   external edits. Lifecycle (review F10): `watch()` returns a guard dropped at
   `Kernel` shutdown. Self-write double-fire is tolerated — remove-miss on
   already-removed paths logs at debug; identical content is absorbed by the I-3
   dedup.
4. **GitLayer vault instance + prefix fix** (review blocker): second `GitLayer::new`
   rooted at the vault (instance-level `enabled` exists, `git_layer.rs:227-251`). The
   rel-path computation changes at **all seven duplicated sites** — the consumer in
   `src/kernel.rs:149-214` and handlers `knowledge_routes.rs:833/875/918/1439/1476`:
   when `strip_prefix(kb_root, git_root)` yields an empty prefix (KB root == repo
   root) use the path **as-is**; never fall back to a literal `"knowledge"` segment
   (with the vault repo the fallback targets a deleted path and bails in
   `ensure_within_root` / commit-file-missing). The five handlers stop recomputing
   prefixes — rel-paths come from the knowledge GitLayer accessor.
5. **Lens de-duplication** (review F11): remove the **whole** event chain —
   `index_to_brain`, `lens_handle_event`, the `on_file_change` registration, channel,
   drain task, and `_callback_keepalive` (`knowledge_lens.rs:104-125, 323-350`) —
   leaving only `copilot_chat`/recall context search; a registered no-op loop waking
   on every change is unacceptable. Kernel bootstraps brain registration instead: on
   startup/brain-availability, `BrainConnection::sync_run(knowledge_root, space)`
   once (idempotent: source identity is the canonical vault path; verified
   `INSERT OR IGNORE` + blake3 source id). Space from §5.4.
6. **Security** (review): extend default `AgentPermissions::denied_paths`
   (`permissions.rs:69-75`) with `.oxi/**` — otherwise an agent with exec can write
   the vault directly, bypassing VirtualFs sandboxing, atomic writes, and frontmatter
   invariants.
7. **Backup** (review): the backup tar (`system.rs:2235-2241`) roots at `~/.oxios`
   with a stale `knowledge` member; post-migration it must include the vault (change
   tar root or add an explicit member).
8. **Migration routine** (§7), AGENTS.md path correction (`~/.oxios/knowledge/` is
   stale — the real path today is `~/.oxios/workspace/knowledge/`).

### 5.4 Shared vault config (review F2-ecosystem)

`~/.oxi/config.toml` (already the C5 shared config) gains:

```toml
[vault]
path = "~/.oxi/vault"   # default; both apps resolve this first
space = "personal"      # canonical brain space for vault ingestion
```

oxios's `knowledge_root` and oximemo's default vault resolve through it. Per-app
overrides remain possible for power users, but a mismatch triggers a loud warning —
never silent divergence.

## 6. Policy matrix

| File / dir | oximemo index | oxios app feature | brain ingest | git |
|---|---|---|---|---|
| `<folder>/<slug>.md` (frontmatter) | first-class memo | document editing | yes | yes |
| Chat.md, Later.md | hidden (BodyOnly) | inbox / working set | **no — new root-anchored exclusion** | yes |
| Done.md, Shop/Watch/Read.md | hidden | archive / lists | yes | yes |
| journal/, habits/, insights/, archive/, media/, img/ | hidden | life log | yes — core brain food | yes |
| config.json, _assets/, .trash/, TEMPLATE.*, oximemo.toml | machinery | — | no (non-`.md` or excluded) | yes |

`habits/` is part of the **migration-defined** system set — `SYSTEM_DIRS` today lacks
it (review F3), so the exclusion set is `SYSTEM_DIRS ∪ SYSTEM_FILES ∪ {habits/,
config.json}`.

## 7. Migration (one-time script)

1. Create `~/.oxi/vault/`, initialize `oximemo.toml`; write `~/.oxi/config.toml`
   `[vault]` if absent.
2. Move the existing oximemo default vault contents in (entire tree) **converting
   frontmatter `+++`TOML → `---`YAML**: map id/created_at/updated_at/favorite/
   deleted_at; drop `hash` (recomputed) and `tags` (body-derived; preserved verbatim
   if the user hand-wrote it). **Tolerate already-migrated state** (review Q): source
   missing ∧ target exists → report "already migrated" and continue.
3. Move `~/.oxios/workspace/knowledge/` contents in:
   - User documents (every `.md` outside the §6 exclusion set) get synthesized
     frontmatter — RFC-022 notes already carry a conformant `---` block, so only
     id/created/updated are added; bare notes get the full block. created prefers
     the file's first git commit date, falling back to mtime.
   - System files move verbatim, staying frontmatter-less.
4. Git history: extract the `knowledge/` subtree from the workspace repo with
   `git filter-repo --subdirectory-filter knowledge` into the new vault repo (git CLI
   allowed once here — GitLayer's no-CLI rule is a runtime property). Fallback:
   fresh init; the old workspace repo is preserved, so old history remains
   reachable there.
5. Commit the removal in the workspace repo; leave legacy `knowledge:lens` brain
   episodes in place (immutable ledger; consolidation absorbs the duplication).
6. Reindex both apps; run `sync_run` once; verify with the smoke test below.

Dry-run mode reports every planned move/conversion before touching disk; the source
tree is backed up before execution.

## 8. Concurrency, failure, performance

- **Concurrent writes**: atomic renames give last-writer-wins per file. The oxios web
  API's ETag (`If-Match`) optimistic concurrency compares whole-file content
  (frontmatter included) — verified consistent, unaffected.
- **Daemon down**: files, editing, git, backlinks all live; brain panels degrade (C1).
  On daemon restart, watchers self-restore from the `sources` table.
- **Performance**: backlink reindex touches changed paths only; the oximemo derived
  index for the new default vault stays at app-support `index/` (namespacing applies
  only to custom vaults); brain ingestion is 2 s-debounced with content-hash no-ops.
- **Episode quality** (verified): every real edit is a new episode (C4); A→B→A edits
  produce three episodes — consistent with the occurrence-chain design; semantic
  duplication folds at the assertion/entity layer; GGUF extraction runs once per
  settled change. The no-op guard (§5.3.2) keeps *pointless* episodes out.

## 9. Testing

Per-repo gates (fmt/clippy/test) plus one integration smoke on a shared temp vault:

1. oximemo creates a memo → oxios watcher refreshes backlinks/graph; web UI sees it.
2. oxios writes a document → oximemo `reindex` shows it as a memo with a stable id.
3. **Preservation matrix** (review blocker test): editing a note through every
   oximemo path — update, rename, backlink propagation, soft-delete, restore, move —
   and doctor-fix, preserves unknown keys and the `oxios:` table.
4. **Obsidian interop**: a properties-UI-shaped edit (flat scalars + array, quoted
   where Obsidian quotes) parses, and tags survive as read-union.
5. **Malformed frontmatter** (`---` block violating the subset) → hard error from
   `parse`, surfaced by both apps; never ingested as body by oximemo.
6. Brain watcher turns both apps' changes into episodes; root `Chat.md`/`Later.md`
   stay excluded while `<folder>/Chat.md` is ingested.
7. With the daemon stopped, both apps remain fully functional for file operations.
8. No-op re-PUT leaves the file untouched (mtime/content/episode count unchanged).
9. Git: knowledge commit/history/restore/diff work with the vault-rooted repo
   (empty-prefix rel-path unit test included).
10. Backup tar contains the vault.
11. Agent exec cannot write `~/.oxi/**` under default permissions.

## 10. Risks and deferred items

- **R1** — format conversion bugs during migration (both oximemo TOML and oxios
  synthesis): mitigated by dry-run report + backup; reversible.
- **R2** — we own the parser (hundreds of lines + tests). Accepted deliberately: the
  restricted subset makes it tractable, and owning it is what makes the preservation
  and canonical-round-trip guarantees enforceable. YAML comments are not preserved
  (canonical serializer rewrites); documented — Obsidian's properties UI writes no
  comments, so no practical conflict.
- **R3** — oximemo folder counts include BodyOnly files (walk-based): cosmetic.
- **Deferred**: `SyncAction::Deleted` → auto-redact (privacy follow-up, gated);
  backlink-parser unification; hiding frontmatter in the oxios web editor UX;
  per-path brain ingestion rate limits; oximemo watcher `.html` support; TS mirror
  of the frontmatter parser if a web consumer appears (SPEC.md is the contract).

## 11. Sequencing

1. **oximemo**: `oxi-frontmatter` crate (SPEC + parser + `write_document`) +
   nine-site conversion + default-path migration with format conversion + brain
   registration. (Contract ships first.)
2. **oxibrain**: root-anchored exclusion-list addition + ECOSYSTEM.md v1.1 (C5
   amendment, `[vault] space` canonicalization). (Tiny, parallel with 1.)
3. **oxios**: config root (both KB sites), format-aware writes with the exclusion set
   and no-op precedence, watcher, vault GitLayer + seven-site prefix fix, lens chain
   removal + boot registration, `.oxi/**` deny, backup member, migration routine.
   (Depends on published `oxi-frontmatter`.)
4. **Docs**: ECOSYSTEM v1.1 (with 2); RFC-050 records the migration; oximemo
   DESIGN.md updated; AGENTS.md path correction.

## 12. Amendment log

**Revision 2 — review + format redesign (2026-08-20).** From three independent
reviewers (oximemo / oxios / ecosystem): preservation reaches all nine
re-serialization sites via the single merge-write API with re-read-at-write sourcing
(blocker); git rel-path empty-prefix rule + seven call sites enumerated (blocker);
both KB construction sites switch (split-brain); system-file exclusion set defined
(`habits/` missing from `SYSTEM_DIRS`); no-op precedence fixed; `note_restore` merge;
`.oxi/**` agent deny; ECOSYSTEM v1.1 C5 amendment text; canonical `[vault] space`;
both-exist migration surfaces `MergeRequired`; raw `fs::write` sites converted;
doctor dead-path cleanup; root-anchored brain exclusions; lens chain fully removed;
watcher lifecycle/self-fire tolerance; backup member; deleted-notes privacy caveat;
already-migrated tolerance; `.html` watcher gap documented. Format decision: converged
target changed from oximemo's TOML `+++` to the new neutral `oxi-frontmatter`
constrained-YAML subset — Obsidian/`---`-ecosystem compatibility, RFC-022 native
conformance, derived fields (`hash`, `tags`) removed from the file (§1, §4).
