# Changelog

All notable changes to this project are documented in this file.

and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).


## [Unreleased]

## [1.42.0] - 2026-08-17

### Added
- **RFC-048: Oxi Foundation integration** — versioned Foundation filesystem
  contract (`~/.oxi/foundation/v1`) that owns non-secret profile metadata,
  OS Keychain credential locators, and the immutable shared package lock.
  Foundation is not a provider proxy and never shells out to an external
  worker — `OxiosEngine` stays the embedded `oxicode_sdk::Oxicode`.
  - New `foundation` module in `oxios-kernel`: bootstrap, profile,
    packages, migrate, resolver (keychain-backed
    `CredentialSource::FoundationKeychain` via the optional
    `foundation-keychain` feature).
  - `FoundationProfileResolver` wired into `OxiosEngine` for role-based
    model resolution.
  - New CLI commands: `foundation status|bootstrap|register|migrate`,
    `brain consolidate` (delegates to the oxibrain daemon).
  - `knowledge_dream` → `knowledge_curation` rename (deprecated alias
    kept for compatibility).
  - Integration tests: `foundation_bootstrap`, `foundation_profiles`,
    `foundation_packages`, `foundation_credential_migration`.
  - Docs: `INDEX.md`, `ARCHITECTURE.md`, `README.md`, `getting-started.md`
    updated; RFC-003, RFC-041, RFC-047 carry RFC-048 supersession links.
- **Foundation packages surface as skills (RFC-048 §4)** — wire the
  read-only Foundation package registry into the live product.
  - `SkillManager::with_foundation(home)` attaches the registry;
    `init()` loads verified packages between bundled defaults and user
    skills (precedence: bundled < Foundation < user/workspace). Packages
    are gated per-entry through `PackageLock::import` so one bad digest
    never hides healthy packages. Archives are parsed fully in memory;
    the Foundation tree stays read-only (`set_enabled` rejects Foundation
    skills).
  - `SkillSource::Foundation` + `FoundationPackageInfo` provenance
    (id/version/digest/persona) recorded on `SkillEntry` and exposed via
    `SkillSnapshot.foundation_packages` for the audit trail.
  - `build_snapshot_for(persona)` keeps prompt construction selective:
    persona-hinted packages contribute only to matching personas;
    `PersonaManager::compatible_foundation_packages` exposes the same
    filter.
  - `apply_to_template` maps package requirements through the reviewed
    requirement table into `CapabilityTemplate`; `AccessGate`/RBAC still
    make every decision. Gate tests cover CSpace-lacking and
    permission-denied paths; `brain.query` maps to read-only Brain access.
  - Boot wiring (`src/kernel.rs`) + Web API exposes `source` /
    `foundation` fields.
  - CLI channels label for `CredentialSource::FoundationKeychain`.
  - Clippy cleanups in foundation (collapsible ifs, `contains`,
    redundant closure, unused fixture params, stray doc line).

### Fixed
- Workspace `clippy` gate: new Foundation integration tests now declare
  `#![allow(clippy::unwrap_used)]` so `.unwrap()` / `.unwrap_err()` in
  tests is idiomatic (matching the existing test convention). Keeps
  `cargo clippy --workspace --all-targets -- -D warnings` green.

## [1.41.0] - 2026-08-16

### Added
- **RFC-043 complete: task management end-to-end** —
  - **Verify gate**: `TaskStore::set_verify` persists
    `verify_enabled`/`verify_requirement`/`verify_max_iterations`/
    `verify_verifier_agent_id`; a run with the gate armed is checked by a
    separate verifier conversation and, on FAIL, repaired with the
    verifier's feedback (bounded by one shared deadline). Runs finalize as
    `verified`; the web run history shows a verified badge. `PUT
    /api/tasks/:id/verify` now persists (it previously echoed its input).
  - **`task` agent tool** (Phase 2): create, create_batch, list, view,
    edit, update_status, set_schedule, set_verify, run, add_comment,
    delete. `run` is fire-and-forget through the shared runner; agent
    creations/comments are stamped with the agent + session id.
  - **Failure fuse** (Phase 5): three consecutive scheduled/heartbeat
    failures pause a task (`paused`, no reschedule) with a fuse note in
    `last_error`; manual-run failures never count.
  - **Comments & dependencies** (Phase 1 remainder): store CRUD + HTTP
    routes + web UI (thread, add/remove with cycle-guard errors surfaced);
    dependency graph (d3-force) in the task detail dialog; auto-run tick
    defers tasks whose dependencies aren't all completed.
  - **Edit + batch**: `PUT /api/tasks/:id` partial update, `POST
    /api/tasks/batch`, task edit dialog in the web UI.
  - **Cron → task migration** (Phase 5): `POST /api/tasks/migrate-cron`
    with `dryRun` preview; "Migrate to tasks" action on /cron-jobs.
    Copy-based — cron jobs are left in place.
  - Task execution consolidated into `oxios-kernel`'s
    `task::runner::execute_task_run` (manual endpoint, auto-run tick, and
    agent tool share one path); `KernelHandle` owns the TaskStore.

### Fixed
- Task API wire format: task DTOs now serialize camelCase, matching the
  web types that shipped reading camelCase (multi-word fields such as
  `verifyEnabled`, `nextRunAt`, and `createdAt` previously surfaced as
  `undefined` in the web UI; create/update params with multi-word keys
  were silently dropped).
- Mount auto-promotion scanner is now stopped during graceful shutdown
  (Promo-6 wiring); stale TODOs resolved in the WS done-chunk
  `tool_calls` path and `oxios-markdown::should_split_checklist`.
- **Telegram instant connect from the Web UI** — Settings → Telegram now has
  a connection card: paste the @BotFather token, press Connect, and the
  bot starts immediately (no daemon restart). New `POST /api/channels/{name}/connect`
  and `POST /api/channels/{name}/disconnect` endpoints drive the gateway's
  runtime register/unregister and persist `channels.enabled` after a
  successful start; `GET /api/channels` reports availability, enabled,
  running, token source, and live channel `info` (e.g. the connected
  bot's username).
  The card walks first-time users through the flow: @BotFather token
  issuance steps when no token is stored, a "validating token and starting
  the bot" progress label while connecting, and a post-connect hint naming
  the bot to message in Telegram.
- The Telegram plugin now resolves the bot token via the credential store
  (env var → `~/.oxios` store → shared `~/.oxicode` store — same resolution
  the Secrets page displays) and validates it with a one-shot `getMe` call:
  invalid tokens fail fast with Telegram's own error (e.g. `Unauthorized`),
  transient network problems still boot with retries.
- `channels.telegram.api_base` config (default `https://api.telegram.org`)
  for self-hosted Bot API servers; `Channel::status()` /
  `Gateway::channel_status()` expose live channel introspection.

### Changed
- Bumped `oxibrain-client` to 0.2 (client API unchanged; daemon-side
  extraction quality fixes — multi-type entity objects, relaxed subject
  types, `ANTHROPIC_BASE_URL` override, local GGUF extraction).


## [1.40.0] - 2026-08-15

### Changed (Breaking — RFC-047)
- **`oxios-memory` removed; agent memory now lives in the standalone
  `oxibrain` daemon** (~13,600 LOC retired, big-bang cutover).
  - `oxios-kernel` connects to the daemon over a Unix-domain socket
    (`[brain]` config: `enabled` / `socket_path` / `space`; default socket
    `~/.oxi/brain/oxibrain.sock`, default space `personal`).
  - Degradation contract: when the daemon is unreachable every memory
    operation returns empty and agent turns complete normally; the kernel
    logs a warning and reconnects on the next call.
  - Agent memory tools (`memory_write` / `memory_read` / `memory_search`),
    the persistence hook, compaction summaries, and knowledge-lens indexing
    all route through the brain (`recall` / `ingest` / `search` / `get_entity`).
  - SONA (trajectory pattern engine) and the embedding traits were rehomed
    into `oxios-kernel` (`memory_agent::sona`, `embedding`).
  - `KernelDatabase` (SQLite WAL) replaces `MemoryDatabase` for the mount /
    project tables — data moves to `~/.oxios/workspace/kernel.db`
    (forward-only; `memory.db` is preserved untouched).
  - `sqlite-memory` feature removed; `rusqlite` is now non-optional.
- **API surface** — `/api/memory/*` replaced by `/api/brain/*`
  (`status`, `recall`, `search`, `entity/{id}`, `timeline`, `why/{id}`,
  `contradictions`, `stats`). `MemoryApi` facade and AgentApi memory methods
  deleted; `KernelHandle::brain: BrainApi` is the new surface.
- **Web UI** — the Memory panel is replaced by the Brain panel (`/brain`):
  overview (availability + episode/entity/statement/contradiction counts),
  hybrid search, entity drill-down (beliefs/timeline/provenance), and the
  contradiction inbox. Dashboard, chat `@memory` mentions, and settings
  (`[brain]` section) updated.
- **CLI** — `oxios brain status|ingest|ask` talks to the daemon directly
  (no kernel needed). `oxios brain export` is unsupported — use the
  `oxibrain` CLI.
- **Metrics** — `oxibrain_available` gauge + `oxibrain_recall_total` counter
  replace the old memory metrics.

### Prerequisites / notes
- `oxibrain-client` v0.1.0 (published to crates.io on 2026-08-15) is now a
  normal registry dependency — the former path deps and the CI sibling-repo
  checkout are gone.
- Data migration (one-time, optional):
  1. Stop the oxibrain daemon so the CLI can take the write lock.
  2. `oxibrain import-oxios --source ~/.oxios/workspace/memory.db --space personal`
  3. `oxibrain reproject` — rebuilds `fts_word`/`fts_ngram` projections from
     the episode ledger. Without this step search misses any episodes
     imported in step 2 (the projection is a derived index, not part of
     the ledger).
  4. `oxibrain reextract --space personal` — runs the LLM extractor.
  5. Restart the daemon. `memory.db` is never touched.
- `crates/oxios-memory` is removed from the workspace; the crates.io entry
  will be deprecated (not yanked) by the release engineer.

## [1.39.0] - 2026-08-11

### Changed
- **oxicode-sdk 0.66.0 → 0.73.0** — adopted the latest SDK release.
  oxicode-sdk 0.72 dropped its browsing re-exports (BrowseTool,
  BrowseSessionTool, OxicodeBrowserEngine, the `browsing_tools()` /
  `native_browser_tools()` factories, and the `.browsing()` / `.native_browser()`
  AgentBuilder methods). The `browser` / `native-browser` features of
  `oxicode-sdk` are gone. oxios already depended on oxibrowser-core directly
  (RFC-046), so no call-site churn — the type-level surface (`BrowseProgress`,
  `BrowseProgressCallback`, `ToolExecutionMode`, `AgentTool`, `StreamDelta`,
  `SubagentRunner`, `ForkResult`, `ContentBlock`, `ImageContent`) is still
  re-exported and used as before. 0.73.0 added `BrowseProgress::PdfExported`,
  surfaced end-to-end via the new `BrowserEvent::PdfExported` event from
  oxibrowser-core 0.21.
- **oxibrowser 0.20 → 0.21 / oxibrowser-core 0.20 → 0.21** — picked up
  `Tab::print_to_pdf` (a Rust API, not just CDP) and WebAssembly 1.0 (MVP)
  support via the wasmi ↔ boa_engine bridge (fuel-metered: 10M-instruction
  budget per Store; infinite WASM loops trap as `RuntimeError` instead of
  hanging the JS thread). Pages using `<script>` modules that load WASM now
  run.

### Added
- **Browse event mapping** — `BrowserEvent::PdfExported` is now mapped to
  `BrowseProgress::PdfExported` in the kernel's event-drain loop, so
  per-tab progress callbacks (and the UI card label "PDF ready — …") fire
  on `Tab::print_to_pdf` calls. Tab-id extraction covers the new event too.

## [1.38.1] - 2026-08-07

### Fixed
- **`cargo install` build failure** — `utoipa-swagger-ui`'s build script
  downloaded Swagger UI assets from GitHub at compile time, so `cargo install
  oxios` failed with `curl: (28) … Couldn't connect to server` whenever
  GitHub was unreachable (timeout, firewall, restricted network). Enabled
  the `vendored` feature on `utoipa-swagger-ui` so the Swagger UI assets are
  embedded at build time — zero network access required at build **and**
  runtime. Fixes the 1.38.0 crates.io install.

## [1.38.0] - 2026-08-07

### Added
- **Persona capability editing (web)** — capabilities render as badges on
  roster cards and are editable as toggle chips in the edit dialog (en/ko).

### Changed
- **New app icon** — replaced the legacy purple mark with the oxiOS
  terminal-window icon: web favicon (`/favicon.png`) + apple-touch-icon,
  sidebar brand mark, README header logo, repo-root `icon.png`, and the
  mobile companion app icons (Expo `assets/`). Also fixed fixed-path static
  asset serving — root files like `/favicon.svg` previously 500'd because
  `static_handler` required a `{*path}` capture; it now falls back to the
  request URI.

### Fixed
- **Persona system audit** — `capabilities` are now serialized on all three
  persona read endpoints (`PersonaSummary`, `GET /:id`, `GET /active`);
  previously the field never reached the client, so every capability-gated
  affordance (diff viewer, worktree fanout, terminal toggle) was unreachable.
  Also: `persist()` writes the declared schema version (2, was hardcoded 1);
  the agent-tool security-review comment matches its fail-open behavior;
  `set_active` rolls back the slot and propagates persist IO errors; and
  unimplemented capability flags were dropped from default personas so
  declared == implemented.
- **CLI** — extracted the CLI argument schema to `src/cli.rs`.

## [1.37.0] - 2026-08-06

### Added
- **Remote access & mobile companion (RFC-044)** — `oxios --remote` server with
  Tailscale-aware endpoint enumeration, pairing QR offer, Noise_XX encrypted
  WS transport with frame gate + backpressure, RPC dispatch
  (`status.get`, version-gated), persistent DeviceRegistry with
  hashed-at-rest tokens, and an Expo companion app scaffold (Phase 2).
- **Persona capabilities (RFC-044 §8.2)** — `capabilities: string[]` on
  personas (schema v2) surfaced to the web UI; capability-pack components
  and per-session persona resolution gate optional affordances (diff viewer,
  worktree fanout, terminal toggle).
- **Worktree fan-out, diff & merge (RFC-044 Phase 4)** — `/api/*` endpoints
  for fan-out worktree results with compare/merge panel in the web UI.
- **oxicode-sdk 0.66 router** — `RouterProvider` + synthetic models,
  router config template/profile models in the engine API, TOML
  deserialization wired into kernel startup, effective default model
  resolution at boot validation.
- **Lifecycle hooks** — `CommandHookRunner` for SDK lifecycle hooks, wired
  into `OxiosEngine` + `build_with_routing` and config.toml.
- **Stream deltas** — SDK `MessageUpdate` stream delta handling fixes live
  text streaming.
- **oximemo / oxiline app-module integration** — opt-in `memo` and `timeline`
  cargo features; web-UI live Connect toggle with runtime-swap facade;
  oximemo-core/oxiline-core now pulled from crates.io (path deps dropped);
  rusqlite 0.34 → 0.40.

### Changed
- **oxicode-sdk 0.64 → 0.66 (kernel)** — Followed the upstream `oxi → oxicode`
  rename (CHANGELOG §0.65.0 Breaking). Crate deps: `oxi-sdk`/`oxi-ai`/`oxi-agent`
  → `oxicode-sdk`/`oxicode-ai`/`oxicode-agent` (0.66.0). Identifiers:
  `Oxi`/`OxiBuilder`/`OxiBrowserEngine` → `Oxicode`/`OxicodeBuilder`/
  `OxicodeBrowserEngine`; the `OxiosEngine.oxi` field name is preserved
  (oxios-internal). Env vars: `OXI_*` → `OXICODE_*`. Config paths:
  `~/.oxi/auth.json` → `~/.oxicode/auth.json` (no backward-compat read — the
  shared `oxicode-cli` home is now the only fallback store). Bumped
  `oxibrowser` 0.16 → 0.17 (its core crate moved with the SDK).
  Workspace check / clippy / test build all green; oxicode-sdk 0.66.0 brings
  the hooks system (Claude Code-compatible `HookRunner` port), shake
  compaction, typed `StreamDelta`, and an LSP `reload`/`capabilities`/
  `request` action set — not yet wired in oxios.

### Fixed
- **CI gate (web)** — dropped an unused `agentId` prop in `WorktreeComparePanel`;
  `BlockStream` tests now wrap renders in `QueryClientProvider`.
- **RFC-044 hardening** — DeviceRegistry atomic write + error propagation,
  order-preserving endpoint dedup (Tailscale-first), transient accept-error
  tolerance, identity 0600 perms, `--remote` implies foreground.
- **Engine boot** — effective default model resolved at boot validation;
  `RouterConfig` manual `Default` matching serde defaults.

## [1.36.0] - 2026-08-04

### Added
- **Code workspace mode (kernel/gateway/web)** — Full in-browser IDE as the
  4th top-level mode (⌃4). New `code` persona + `coder` CSpace (shell exec only).
  Backend: PTY manager, code session & change tracker, checkpoint system,
  `CodeApi` facade, `/api/code/*` REST + WebSocket terminal, coder CSpace
  resolution, file-system search/read/write/move endpoints, and agent
  messaging endpoint. Frontend: file explorer, code editor with tabs,
  agent conversation panel, resizable panels, project canvas visualization,
  change review system with diff viewer and accept/reject, quick file open
  (⌘P) with fuzzy search, and workspace layout/persistence.

## [1.35.0] - 2026-08-02

### Added
- **Scheduled tasks & cron (kernel/gateway/web)** — Added a `run_goal`
  primitive and `TaskStore` lifecycle to the kernel, wired a task
  run/schedule API and cron auto-start in the gateway, and built a task
  schedule config UI with a detail drawer and run history on the web.
  Cron auto-start is now enabled by default.
- **Unified asset store (web)** — Central binary storage with a metadata
  index. Browse, upload, preview (image/audio/video), and delete assets
  with per-asset title/tag metadata.

### Changed
- **oxi-sdk 0.58 → 0.64 (kernel)** — Migrated to oxi-sdk 0.64 / oxi-agent 0.64.
  oxi-sdk 0.64 moved the unstable re-export surface behind opt-in cargo
  features (R3), so the workspace now enables `browser`, `delegation`, and
  `circuit-breaker` explicitly. The LLM circuit breaker
  (`agent_runtime::LLM_CIRCUIT_BREAKER`) adopts the new `CircuitBreaker` trait
  + `DefaultCircuitBreaker` reference impl (R6) — replacing the
  `ProviderCircuitBreaker`/`CircuitBreakerConfig` types removed in 0.61 — and
  the `llm_circuit_breaker_state` metric now reflects the breaker's real
  state machine (Closed/HalfOpen/Open) instead of "last call errored". The
  dead rate-limited `ProviderPool`/`pooled_provider` path
  (`provider_rpm`, never set > 0 in production) is removed; agent construction
  is now a single `AgentBuilder` path. `OxiosEngine::resolve_model`/
  `create_provider` wrap the SDK's new typed `SdkError` (R7) into `anyhow`.
  SpawnValidator (R6) is intentionally not wired: `oxios-mcp` spawns via its
  own `McpClient` and does not use the SDK MCP transport, so the trait has no
  consumer today; see `docs/designs/2026-08-02-oxi-sdk-0.64-migration-design.md`
  §부록 D.

### Fixed
- **Frontend CI (web)** — Removed an unused `useMemo` import (tsc TS6133),
  applied `biome --write` format/import-sort, and suppressed
  `lint/a11y/useMediaCaption` on audio/video asset previews. Frontend gate
  (typecheck/lint/test/build) is green again.

## [1.34.0] - 2026-07-31

### Changed
- **Oxi design system (web)** — Adopted the canonical oxi design system: unified
  design tokens (colors, typography, spacing, theme switching), replaced raw
  palette colors and UI primitives with design-system tokens, and swapped
  DESIGN.md for the canonical oxi design-system spec. Web UI is now styled
  entirely from the token hierarchy.

### Chore
- **Project identity** — Migrated a7garden/oxios references to
  project-oxi/oxios and unified the standard project-oxi MIT license.

## [1.33.1] - 2026-07-30

### Fixed
- **Biome lint/format (web)** — Applied biome-safe fixes to `markdown-editor.tsx`
  and `autocompletion-override-conflict.test.ts`.

## [1.33.0] - 2026-07-30

### Changed
- **Knowledge editor (web)** — Replaced @uiw/react-codemirror with
  @atomic-editor/editor for the knowledge-base markdown editor. AtomicEditor
  provides Obsidian-style inline live preview, WYSIWYG editable tables,
  async wiki links with autocomplete, image blocks, read-only mode, and
  smart edit helpers. Deleted 11 files (−2552 lines).

## [1.32.0] - 2026-07-29

### Added
- **Knowledge editor typography (web)** — Redesigned markdown editor with improved
  typography and inline title editing, featuring a dedicated note-title component
  and biome-formatted CodeMirror integration.

### Fixed
- **Knowledge backlinks API (web)** — Registered missing backlinks API route
  and hardened SPA fallback.
- **Approval config persistence (security)** — Approval configuration is now
  persisted to disk on every PATCH operation.

## [1.31.1] - 2026-07-28

### Changed
- **Thinking block visual tier (web)** — reasoning now renders flat as muted
  marginalia (no fill, border, or left-rail) instead of a recessed aside, so
  the flow-of-thought reads as the agent's internal monologue rather than a
  container like a tool card. Settled thoughts recede to 60% opacity to fade
  into the rhythm between tool cards. Drops the now-unused `ScrollArea` import.

### Fixed
- **Duplicate i18n key (web)** — removed the duplicate `sectionNotifications`
  entry in the Korean locale (`ko.json`) flagged by biome; both occurrences
  held the identical value, so no behavior change.

## [1.31.0] - 2026-07-28

### Added
- **Context compression (kernel)** — `CompressionService` with LLM streaming
  summary for compressing conversation history; `CompressionApi` facade on
  `KernelHandle` with compression endpoint `POST /api/sessions/:id/compress`
  and auto-trigger on context threshold (P3a).
- **Knowledge in search (web)** — `KnowledgeBrowser` component for viewing
  read-only knowledge notes; `SearchView` with tab bar (Web/Knowledge tabs);
  `KnowledgeView` portal panel type; Web results toggle in Knowledge Copilot;
  Web tab in ⌘K SearchModal.
- **Search & Browse panel (web)** — `POST /api/search` and `POST /api/browse`
  endpoints; `SearchPanel` store with manual search and browse cache;
  `BrowseRender` component for markdown rendering; auto-open panel on
  `web_search`/`browse` tool calls; i18n keys.
- **BrowseTool (kernel)** — wired oxicode-sdk `BrowseTool` for headless page
  content reading.
- **Virtualized chat list (web)** — migrated chat message list to `virtua`
  for smooth rendering of large histories.
- **Animated Thinking title (web)** — shiny animated title while reasoning
  streams.

### Fixed
- **Multi-turn streaming boundary (web)** — fixed turn-boundary targeting and
  duplicate reasoning card rendering in streaming responses.
- **clippy & biome fixups** — resolved warnings across kernel and web.

### Changed
- **CompressedGroup (web)** — refactored as controlled collapse toggle.
- **Chat row model (web)** — pure chat row model for virtualized list.
- **CI** — migrated all runners to self-hosted macOS ARM64; pinned
  `bun-version` 1.3.14.
- **Deps** — updated `Cargo.lock` for `oxibrowser` dependency.

## [1.30.0] - 2026-07-28

### Added
- **Block-stream timeline (web)** — assistant turns now render as
  interleaved block-stream timelines: reasoning segments, tool calls,
  and text are each displayed in their full original order instead of
  being concatenated into a single blob (P3a; see
  `docs/designs/2026-07-27-block-stream-chat-design.md`).
- **Persisted reasoning (kernel)** — positioned reasoning segments are
  stored per-message so the web UI can faithfully reconstruct the
  interleaved timeline on reopen (P3b).
- **YAML frontmatter rendering (web)** — knowledge note frontmatter now
  renders as Obsidian-style property cards instead of enlarged body text.
  Editing the note's H1 title inline renames the knowledge entry via the
  frontmatter-aware title API.
- **HTML knowledge support** — `.html` files are treated as read-only
  knowledge entries.

### Fixed
- **Web UI sync drift** — the daemon only re-synced web UI at 03:00 daily,
  so a frequently-restarted host never caught up. Web-dist sync is now
  owned by a single `web_dist::sync` core (compare `version.json` →
  download to staging dir → atomic publish), run both on startup (throttled
  to once/hour) and on the existing daily schedule.
- **Rust lint**: trailing blank line in `cron.rs`, empty line after `#[cfg(test)]`
  attribute in `blacklist.rs`.
- **Web lint**: import ordering and bracket-to-dot notation fixes across
  6 test files.

### Changed
- **StreamProcessor** reworked for block-stream chat model
- **Gateway**: `active_web_dist.rs`, `gateway.rs`, `gateway_behavior` test
- **Kernel**: cron scheduler, Cargo.toml updates
- **API**: bridge and plugin modules updated
- **CI**: workflow updates
- **Refactor(web)**: removed legacy ActivityTimeline, activity/toolCalls,
  and reasoning fields (P3b cleanup)

## [1.29.0] - 2026-07-27

### Added
- **Embedded web UI (oxios)** — the SPA is now baked into the binary via
  `include_dir!` at build time, eliminating the first-run download and the
  separate `web-dist.zip` release asset for the binary-only distribution.
- **Interactive path-access cards (kernel)** — Mount / temp-allow / deny
  cards for interactive file-system access decisions.

### Fixed
- **Approval state sharing (kernel)** — `approval_config` Arc is now shared
  across both `KernelHandle` instances, preventing stale approval state.
- **Approval re-evaluation (kernel)** — manual-mode approval grants are
  honored and re-evaluated on policy change.
- **Web-dist sync (gateway)** — unified sync with eager startup check so
  the daemon always serves the correct version.
- **Frontend lint**: biome unsafe optional chain fix.
- **Module declaration order**: fmt fix.

### Changed
- **Version bumps**: oxios-kernel and oxios-gateway to 1.29.0.
- **Dep version specs**: updated for gateway and binary.

## [1.28.0] - 2026-07-27

### Added
- **Tool approval mode system (RFC-035)** — lobehub-style 3-mode approval
  (manual / allow-list / auto-run) crossed with a 3-tier tool policy
  (Auto / OnDemand / Always). A new `ApprovalGate` runs after the existing
  4-layer `AccessGate` and decides whether a tool call auto-runs or surfaces
  an approval card. Security-blacklist patterns (`rm -rf /`, `sudo *`, fork
  bomb, etc.) always escalate to `Always` and prompt regardless of mode.
  `[security.approval]` config section + `/api/security/approval` HTTP API +
  Web UI dropdown (chat input), "remember" checkbox (allow-list), and
  settings panel. Removes the bespoke exec-only shell approval (root cause
  of the "every exec prompts" complaint). Design:
  `docs/designs/2026-07-27-approval-mode-system-design.md`.
- **Portal document preview (web)** — clicking a saved-document chip in
  chat now opens a read-only rendered preview in the portal pane instead
  of navigating away from the chat. Toggling re-surfaces or peeks off the
  view (matching artifact-card affordances); an "Edit in Knowledge Base"
  action with a leave-chat warning covers edits.
- **Inline document title (web)** — the note's first H1 renders as an
  Obsidian-style inline title in the live-preview editor (larger than
  in-body headings; the leading `#` is hidden).
- **In-input thinking indicator (web)** — the activity/thinking status row
  moved from a full-width bar above the input into the input box itself,
  fixing the misalignment with centered chat content.

### Fixed
- **Long-tail WebSocket reconnect (web)** — after the 5-attempt fast
  exponential backoff (~31 s) the client gave up permanently, stranding
  the tab at `connected = false` (disabled input, stuck reconnect banner)
  until a full page refresh. Now continues with a steady 10 s long-tail
  retry until the socket opens.
- **Stuck "Thinking" block (web)** — the reasoning spinner could spin
  forever across three paths: reasoning lifecycle markers treated as
  terminal (corrupting the stream), a reasoning→tool transition never
  closing reasoning, and an abnormal socket close mid-reasoning leaving
  the spinner pinned.

### Changed
- **Chat input toolbar (web)** — collapsed the three disconnected strips
  below the chat input (in-box toolbar, detached action bar, keyboard-hint
  row) into a single in-box toolbar and removed the no-op action bar whose
  toggles were never wired to send.

## [1.27.1] - 2026-07-27

### Fixed
- **Layer-0 tool gate (kernel)** — `web_search`, `get_search_results`, and `ls`
  were absent from the default agents' Layer-0 allowed-tools list, leaving them
  dead-registered. Restored so default agents can actually invoke these tools.
- **Chat input placeholder (web)** — the editor placeholder was captured once at
  mount and never updated; it now refreshes when the WebSocket connection state
  changes, fixing the stuck "연결 대기 중…" hint after connect.
- **Agent step visibility (web)** — intermediate agent steps are now surfaced
  during the gap before the chat response streams, so the UI no longer appears
  frozen while the model is working.
- **Activity holder (web)** — added a LobeHub-style activity indicator pinned
  above the chat input to signal in-flight work.
- **Turn metadata (web)** — dropped the execute-phase badge and relocated the
  turn-duration readout to hover to declutter the message row.
- **Chat turn styling (web)** — removed avatars and now distinguish turns by
  alignment plus a faint tint instead.
- **User message styling (web)** — stripped the user message to plain
  right-aligned text with no bubble for a cleaner conversation layout.

## [1.27.0] - 2026-07-25

### Fixed
- **Workspace publish cycle** — Removed `oxios-gateway` and `oxios-ouroboros`
  from `oxios-kernel`'s `[dev-dependencies]`, breaking a `kernel → gateway → kernel`
  dependency cycle that blocked crates.io publishing. The gateway-mock integration
  test that required it was removed.
- **Crates.io publish topology** — Restored strict topological publish order
  (markdown → mcp → ouroboros → memory → calendar → kernel → gateway → oxios) in
  the publish workflow so each crate's internal dependencies are on the registry
  before the dependent is published.
- **Mention-search render loop (Web)** — `searchMentions` listed `useMutation`
  results and the `roles` array as `useCallback` deps; since `useMutation` returns
  a fresh object each render and `roles` is rebuilt every parent render, the
  callback was recreated every render — re-running the mention effect, whose
  `setMentionResults([])` (new array ref) never bailed, producing a
  `setState → render` loop. Inputs are now mirrored in a ref and read inside a
  stable callback; the results-clear bails out when already empty.

## [1.26.0] - 2026-07-25

### Changed
- **Runtime and dependency maintenance** — upgraded the Wasmtime stack, refreshed
  policy configuration, and removed an obsolete route.
- **Release gate hardening** — feature-gated the GGUF-only `Path` import so the
  documented default-feature Clippy gate remains warning-free.

## [1.25.0] - 2026-07-23

### Fixed
- **`--workspace --all-features` compile gate** — three pre-existing
  regressions blocked by wasmtime 24 migration and the `ResponseMeta`
  rework:
  - `crates/oxios-kernel/src/wasm_sandbox.rs`: `StoreLimiter` now
    implements `wasmtime::ResourceLimiter` v24 contract — adds
    `table_growing` (defer to wasmtime default table cap), while
    `memory_growing` continues to enforce `WasmConfig::max_memory_bytes`.
  - `src/kernel.rs`: imports `std::path::Path` alongside the existing
    `PathBuf` — the embedding/gguf `cfg` branches at lines 930/1200
    reference `Path::new(&config.kernel.workspace)`.
  - `src/channels/telegram/format.rs`: three tests referenced
    `ResponseMeta` fields removed in the gateway rework
    (`interview_ambiguity`, `mode`). Switched to `..Default::default()`
    against a new `Default` impl on `ResponseMeta` so future field
    additions keep the tests compiling.
- **`telegram::with_api_base` dead-code lint** — public builder for
  local Bot API servers; marked `#[allow(dead_code)]` until config
  wiring lands.

### Docs
- AGENTS.md `--all-features` note refreshed to reflect the wasm-sandbox
  fix.

## [1.24.1] - 2026-07-19

### Fixed
- **Daemon startup panic** — `TaskStore::init_schema` called
  `tokio::sync::Mutex::blocking_lock()` from inside the runtime
  (`WebSurface::start`), panicking at every `oxios restart` with
  *"Cannot block the current thread from within a runtime."* Schema
  initialization now runs on the raw `Connection` before the async
  mutex wraps it; `init_schema` is a free function and `new()` takes
  `Connection` directly.
- **`POST /api/tasks` deadlock** — `create_task` held the connection
  lock across `self.get_task_by_id().await`, re-entering the same
  non-reentrant `tokio::sync::Mutex`. The lock is now scoped to the
  INSERT so it releases before the read.
- **OAuth test assertion** — `host_tools::oauth::tests` asserted
  `json.contains("user_code")` but `DeviceCodeResponse` serializes
  under `#[serde(rename_all = "camelCase")]` as `userCode`. This was
  a deterministic pre-existing failure on clean main that would have
  blocked the release CI gate.


## [1.24.0] - 2026-07-19

### Added
- **Dynamic model catalog (oxi-sdk 0.56.0)** — Models.dev-powered catalog with
  live price/limit refresh and user overrides. Provider resolution fixes for
  previously broken providers.
- **Task management (RFC-043)** — SQLite-backed task lifecycle with CRUD,
  scheduling (cron + heartbeat), verify pipeline, and REST API.
- **Appearance settings** — New Web UI component for theme/layout preferences.
- **ETag caching** — Conditional request support (`If-None-Match` / `304`)
  for non-immutable static assets in the web dashboard.

### Changed
- **Cron schedule croner → cron** — Switched cron parser for better
  compatibility with standard cron expressions.

### Fixed
- **WebSurface struct** — Restored missing struct definition in `plugin.rs`
  that prevented compilation with `web` feature.
- **Clippy** — All warnings resolved across kernel (derivable_impls,
  if_same_then_else, collapsible_if, useless_format) and binary
  (collapsible_if, await_holding_lock, print_literal, dead_code).

## [1.23.3] - 2026-07-15

### Fixed
- **Rate limiting** — Default `max_requests_per_minute` raised from 120 to 600
  (local-first single-user server; ample headroom for the ~20 frontend polling
  queries). Rate limiting can now be disabled entirely by setting the value to
  `0`. The web client no longer retries queries on HTTP 429, avoiding amplified
  throttling under load.

## [1.23.2] - 2026-07-13

### Fixed
- **TreeResponse visibility** — `TreeResponse` enum was private but used in a
  public route handler, preventing binary compilation. Combined with the
  v1.23.1 `guardian_heartbeat` fix, `cargo install oxios` now compiles.

## [1.23.1] - 2026-07-13

### Fixed
- **Compile error in binary crate** — `guardian_heartbeat` variable was
  referenced but never created in `src/main.rs` (deleted during audit
  remediation refactoring). This made `cargo install oxios` fail to compile.

## [1.23.0] - 2026-07-13

### Added
- **Host integrations (RFC-041)** — Cross-platform host CLI scanner (replaces
  `which`-only `has_bin`), OAuth device-code broker (first: GitHub `gh`), and
  provisioner for first-time SkillInstallSpec execution.
- **Persona manager improvements** — Async `set_active` with reseed callback
  for the intent engine's system prompt.
- **Daemon supervision (RFC-040)** — Multi-source liveness interpretation
  (pidfile + lock + port probe), stale pidfile cleanup, orphan detection.
- **Recursive knowledge filetree** — Move, folder creation, and file upload
  in the web knowledge sidebar.

### Fixed
- **Security: MCP spawn chokepoint** — Enforced single spawn path + environment
  variable sanitization (F-1).
- **Security: auth default** — Shipped `auth_enabled` default flipped to `true` (F-13).
- **Integrity: ClawHub hash re-verification** — Previous origin hash logged
  before overwrite; A2A circuit-breaker ordering fixed (F-12, F-15).
- **Recovery: error-recovery paths** — Replaced hard `expect()` calls with
  graceful error handling (F-4).
- **WebSocket keepalive** — Corrected deadline calculation in `select!` loop.
- **Knowledge routes** — Removed unreachable catch-all on exhaustive match.
- **DaemonManager visibility** — `cleanup()` made `pub` for integration tests.
- **Frontend** — TS strict mode errors (`noUncheckedIndexedAccess`), biome lint,
  a11y fixes (`useSemanticElements`, `useFocusableInteractive`), settings
  consistency test, and biome 2.5.3 config migration.
- **Clippy** — 9 warnings resolved across kernel (single_match, collapsible_if,
  type_complexity, derivable_impls, question_mark, manual_unwrap_or_default).
- **CI** — `--no-verify` for publish (OOM on heavy crates), `--allow-dirty` for
  Cargo.lock modifications, non-portable `[patch]` paths commented out.

### Changed
- **Memory recall perf** — Removed redundant embedding clone (F-9).
- **Build profiles** — Dev `opt-level` 2→1 for faster incremental compiles;
  new `dist` profile with thin-LTO for release binaries.
- **Supply chain** — cargo-deny baseline config added (F-7 partial).
- **Dependencies** — Added `flate2` + `tar` for host-tool tar.gz downloads.

## [1.22.0] - 2026-07-11

### Added
- **Memory system overhaul** — New embedding API module (`embedding/api.rs`),
  hyperbolic distance/Möbius operations with improved numerical stability,
  SQLite store backfill optimization, and proptest coverage.
- **Knowledge editor redesign** — Configurable editor preferences store
  (`editor-prefs.ts`), settings popover component, and 6 component updates
  across the knowledge editor suite.
- **ResourceMonitor async safety** — `record_snapshot()` now runs on a
  `spawn_blocking` thread to prevent async runtime stalls during directory walks.

### Fixed
- **Frontend CI (v1.21.0 known issues)** — Typecheck null-safety in
  `use-tab-shortcuts.ts`, `chunkToActivity` phase-chunk handling in
  `stores.test.ts`, and biome organizeImports across 7 files.
- **Credential suffix fallback** — Single-element for-loop clippy fix in
  `-coding-plan` auth token resolution.
- **Git layer** — Collapsible if-let chain (clippy `collapsible_if`).
- **Resource monitor disk walk** — Skip `target/`, `node_modules/`, `.git/`,
  `dist/` in `walk_dir_size` to prevent 90s+ hangs on large repos.
- **chat.rs** — Missing function closing brace in `kernel_event_to_ws_chunk`.
- **knowledge_routes.rs** — Borrow-after-move in file diff handler.
- **Persona ordering** — Canonical priority sort (dev-first) instead of
  alphabetical ID sort in `list_enabled()`.
- **ExecutionResult serialization** — `skip_serializing_if = "Vec::is_empty"`
  on `tool_calls` field.
- **Hyperbolic proptests** — Same-dimension vector generation to prevent
  cross-dimension assertion failures.

### Removed
- **Playwright e2e suite** — 8 spec files + config removed; replaced by
  vitest unit/integration tests (279 passing).

## [1.21.0] - 2026-07-09

### Removed
- **RFC-038: Interactive PTY/terminal subsystem** — Complete removal of the terminal feature shipped in 1.20.0. Deleted `oxios-kernel/src/pty/` (manager, session, error, config, mod), `kernel_handle/pty_api.rs`, `src/api/routes/terminal.rs` (ticket, stream, sessions, pty/start), `KernelHandle::PtyApi` field + 17th constructor arg, `PtyConfig`/`PtySize` from `OxiosConfig`, `portable-pty` dependency, `/api/terminal/stream` auth exemption, `web/src/components/terminal/`, `web/src/routes/terminal.tsx`, `web/src/lib/ws-client.ts` (terminal-only transport), `isTerminal` branch in app-layout, `/terminal` sidebar nav, `ghostty-web` dep, `ptySection` settings UI, terminal/pty i18n keys, `[pty]` blocks from default + user configs, and `docs/rfc-038-interactive-terminal.md`. The user-facing `/terminal` route, the PTY-aware settings panel, and the kernel-side PTY manager are gone with no replacement in this release.

### Known Issues
- **Pre-existing frontend CI failures (carried over from main, NOT introduced by 1.21.0)** — `web/` has three failing gates on the `frontend` CI job at this tag:
  - `bun run typecheck` — `src/hooks/use-tab-shortcuts.ts:32` TS2532 (`Object is possibly 'undefined'`).
  - `bun run test` — `src/__tests__/stores.test.ts` test "appendActivityToMessages creates a placeholder when no assistant exists" fails because `chunkToActivity` lacks a `case 'phase'` branch (real bug: server-emitted phase chunks silently drop to `null`). To be fixed in a follow-up release.
  - `bun run lint` — 7 biome `assist/source/organizeImports` errors across `knowledge-home.tsx`, `quick-ask-dialog.tsx`, `math-fold-extension.ts`, `mermaid-extension.ts`, `table-fold-extension.ts`.
  These were red on main before RFC-038 merged; this release does not introduce them but does not fix them either. The CI `frontend` job will fail at this tag; cut a v1.21.1 patch with the fixes.

## [1.20.0] - 2026-07-08

### Added
- **RFC-039: Persona system completion** — Full persona persistence via `StateStore::durable_write` under `~/.oxios/state/personas/index.json`, `PersonaManager::load_from_state_store` / `persist`, `PersonaConfig.default_persona_id` now honored at boot (previously ignored), and `PersonaApi::set_active_with_persist` for idempotent `PUT /api/personas/active {id}`. HTTP create/update/delete routes and the `PersonaTool` agent-tool path auto-persist after every mutation. Active persona system prompt re-seeds the intent engine on runtime switch.
- **effective_role model resolution** — The active persona's `role` participates in model resolution (`agent_runtime.rs:496`) so `engine.role_routing[persona_role]` fires, closing a gap where only the WS client's per-message role hint was consulted (RFC-039 §3.5).
- **RFC-038: PTY/terminal subsystem** — `PtyConfig` + `PtyManager` + `PtyApi` + KernelHandle wiring, terminal routes + ghostty-web UI, settings UI + hot reload.
- **Audit remediation (backlog)** — Remaining findings across backend, refactoring, design, and LOW-priority items.

### Fixed
- **RFC-038** — Drop `AuditSink` dep, use tracing audit; fix terminal.rs Message types; `AuditEvent::ToolAccess` + `take_writer` API.
- **UI/UX audit, 1st pass (21 High)** — Close-dialog dup, KPI threshold colors, tab URL sync, card accessibility, action/label alignment.
- **UI/UX audit, 2nd pass** — Chart legends, security approvals queue, cron timeline, knowledge inbox mode signal.
- **UI/UX audit, 3rd pass** — Memory tab, activity timeline, delete confirmation, budget guidance labels.
- **UI/UX audit, 4th pass (MED/LOW)** — 75 items across all screens.
- **Regression fix** — Restore `cost.periodSpend` key, remove `monitoringOnly` duplication.

### Changed
- **Dependencies** — Add `portable-pty 0.8` dep.
- **Refactor** — Chat activity merge into single helper; `live-activity` cleanup.
- **Docs** — UI/UX audit report: 19 screens, 96 findings (21H/47M/28L).
- **Style** — `git_layer::delete_tag` clippy fix (cmp_owned).

### Removed
- **`PersonaConfig.max_concurrent_personas`** — Removed unused dead field (only one persona active at a time; `PersonaManager` has a single slot). Existing `config.toml` values are silently ignored via `#[serde(default)]`.
- **"Multiple personas active simultaneously" docstring** — Removed from `Persona` struct doc (single-slot design; multi-persona v2 requires a separate RFC).

## [1.19.0]
- **CORS origins editor (Web)** — Removed the redundant CORS origins editor (`cors-origins-editor`, `cors-validator`); CORS is enforced by the gateway.

## [1.18.0] - 2026-07-05

### Added
- **RFC-035 Phase A** — oxi-sdk 0.53.0: `KernelEvent::CompactionTriggered` makes provider-reported token accounting observable end-to-end via the EventBus; resolves the 3-4× heuristic drift that made `Threshold(0.8)` auto-compaction a silent no-op.
- **RFC-035 Phase B+C** — agent-loop tool-result eviction (gap 1) and sub-agent delegation (gap 3) via `max_tool_result_bytes`, `subagent_runner`, and `subagent_depth`.

### Changed
- Bumped `oxi-sdk`/`oxi-agent` 0.53.0 → 0.54.0.
- Codebase-wide `cargo fmt` pass; the `agent_log_db` TTL prune test is now deterministic (anchored to `Utc::now()`) instead of hardcoded calendar dates that expired 2026-07-01.

## [1.17.0] - 2026-06-30

### Fixed
- **CI frontend lint conformance** — Resolved biome `noAssignInExpressions` in the emoji/math CodeMirror fold-extension regex loops (refactored the `while ((m = re.exec(text)))` scan to a `for` loop so `continue` stays safe) and auto-applied the remaining `useTemplate`, `useOptionalChain`, import-sorting, and formatter fixes across the web app. The CI `frontend` job (lint/typecheck/test/build) is green again.

### Changed
- **Version bump to 1.17.0** — All workspace crates updated to 1.17.0 (package + internal dependency versions); Cargo.lock re-synced.

## [1.15.0] - 2026-06-29

### Added
- **Skill management UI** — Create, edit, and import skills with Claude-grade UX (`.skill` file import, inline editor).
- **Unified model picker** — Role + model selection merged into a single `ModelPicker` component.
- **Role editor** — Inline role config editor in the settings engine panel.
- **Cron schedule editor** — Visual cron expression builder and schedule management UI.
- **Token-maxing billing model** — Final wiring of `billing_model` sourced from live provider snapshot.

### Changed
- **Chat input shell refactor** — Complete UX overhaul of the chat input area and model picker layout.
- **Unified streaming orchestration (RFC-033)** — Ouroboros streaming architecture refactor for consistent event flow.
- **Batch workspace changes** — Pending workspace dependency updates.

### Fixed
- **Chat + sidebar hardening (RFC-032)** — Audit-driven fixes for chat state and sidebar reliability.
- **Agent completion status** — Successful agents now correctly marked `Completed` instead of `Idle`.

## [1.16.0] - 2026-06-30

### Added
- **Calendar integration** — Event creation, editing, deletion, conflict detection, iCalendar import/export, journal bridge, and cron bridge for scheduling.
- **Command palette** — ⌘K command palette for quick navigation and actions.
- **Skill forge tool** — Agent-facing tool for skill generation, validation, packaging, and benchmarking.

## [1.14.0] - 2026-06-28

### Added
- **Live activity transparency** — Real-time agent progress display through SSE streaming_sink and live-activity bar, showing thinking/reasoning/phase transitions to the user.
- **MCP server edit dialog** — Inline dialog for editing MCP server config (command, args, env, enable/disable).
- **Mount edit dialog** — Inline dialog for editing mount names.
- **Persona edit dialog** — Inline dialog for editing persona config.
- **Cron job management UI** — List view and edit dialog for cron jobs.
- **Kernel streaming sink** — `StreamingSinkRegistry` for agent-to-client real-time transparency.
- **i18n updates** — EN/KO translations for all new UI components.

### Changed
- **Gateway rework** — Improved message routing and event bus architecture.
- **Typing indicator** — Real-time update improvements for WebSocket-based chat.
- **State store & session nav** — Enhanced session navigation and state persistence.
- **Workspace members** — Added `oxios-ouroboros` and `oxios-gateway` to workspace members for full CI coverage across all published crates.

### Fixed
- **Clippy warnings** — 7 lint fixes across kernel, MCP, and API routes (collapsible if, clone on copy, len_without_is_empty, unnecessary get-then-check).

## [1.13.2] - 2026-06-28

### Changed
- **Structural/tool-output localization** — All tool and structural output (CLI `--help`, error messages, permission-denial reasons, Telegram & gateway messages, config output) is now English, aligning with the AGENTS.md convention that structural output is English for a global product. Agent conversational replies still follow the user's language.

## [1.13.1] - 2026-06-27

### Fixed
- **Calendar enabled by default** — `CalendarConfig.enabled` now defaults to `true` (serde field default + `Default` impl); fresh installs no longer see calendar 503s in the web UI.
- **Disabled-subsystem 503 handling (web)** — react-query no longer retries a permanent "subsystem not available" 503; the notification-center schedule widget shows a friendly "enable in settings" prompt when a subsystem (calendar/email) is disabled.

## [1.13.0] - 2026-06-26

### Added
- **RFC-031: token-maxing mode** — Unattended work within configured windows, gated to subscription-only providers (never API-credit plans). Adds `token_maxing/` (QuotaTracker, BudgetGuard, WorkPlanner, TokenMaxingSession), a `TokenMaxingApi` with REST routes under `/api/token-maxing/*`, assembler wiring, a config block, and a web provider-status panel + session report.
- **Cost aggregation** — Per-provider token/cost tracking through the kernel, with ZAI + Minimax subscription quota fetchers powering token-maxing's eligibility detection.
- **Cost & quota REST endpoints** — Spend/cost/quota surface for the new cost dashboard.
- **Unified agent monitor + cost views** — Redesigned web IA: A2A, agent-groups, approvals, events collapsed into a single Agents view; new cost dashboard with spend limits.
- **Budget config block** — `share/default-config.toml` ships a `[budget]` section.
- **Persona security review** — Hardened persona creation/edit paths.

### Changed
- Web: biome formatting + lint conformance (import ordering, `Number.isNaN`, React Flow a11y) across cost/agent-monitor/budget/engine views.
- Workspace `cargo fmt` applied across the release range.

### Fixed
- **Chat WebSocket join-set** — Leaked join handles on the chat stream are now correctly awaited.

## [1.12.0] - 2026-06-25

### Added
- **RFC-030: runtime task supervision** — A single-process task supervisor now owns a root `CancellationToken` for cooperative shutdown, replacing the standalone `scheduler` module. Gateway/web surfaces wire their graceful-shutdown path to the supervisor token instead of listening to `ctrl_c` independently, giving a single shutdown signal source. Adds a `tokio-util` `CancellationToken`, drops the `[scheduler]` config section, and threads shutdown through the infra/system routes and metrics.
- **Web notification center** — Slide-over notification panel backed by a dedicated store, surfaced from the header bell.
- **Calendar UI refresh** — A mini-calendar popover trigger in the header replaces the standalone `/calendar` and `/scheduler` routes; sidebar nav and i18n (en/ko) updated.

## [1.11.0] - 2026-06-25

### Changed
- **Removed explicit chat/ouroboros mode toggle** — The web UI no longer exposes a manual chat/spec (Ouroboros) mode toggle. Intent detection is now automatic: the orchestrator interviews when intent is unclear, instead of requiring the user to switch modes. Removes the specMode store state, toggle button, ⌘⇧M shortcut, placeholder variants, the stream-chunk mode field, and the backend MODE meta constant.

## [1.10.1] - 2026-06-25

### Security
- **RUSTSEC-2026-0185 (quinn-proto)** — Bumped `quinn-proto` 0.11.14 → 0.11.15 via a lockfile-only `cargo update`, fixing a remote memory-exhaustion vector from unbounded out-of-order stream reassembly. The vulnerability was reachable transitively via `reqwest → quinn → quinn-proto`. No API or behavior change; `cargo audit` now reports zero vulnerabilities.

## [1.10.0] - 2026-06-25

### Added
- **RFC-029: execution resilience** — OTP-style recovery layered on the existing Unix supervisor: snapshot/restore, `SupervisorPolicy` + `RestartBackoff`, and `ModelSwitched` lifecycle events (adopted from oxicode-sdk). A bounded recovery ladder runs on provider failure: L0 execute → L1 restart (same model) → L2 snapshot+restore-with-new-model → L3 compact-or-larger → L4 A2A delegate → L5 terminal `ResilientFailure`. Backed by error classification (`FailureClass`), a shared `AttemptBudget`, and a per-provider circuit breaker (`ProviderHealthRegistry`) that replaces the global `LLM_CIRCUIT_BREAKER`.

### Fixed
- **P0: provider errors now propagate as `Err`** — `run_agent` previously swallowed provider failures as `Ok(success:false)`, burying them in `ExecutionResult.output`. It now returns `Err`, so the lifecycle boundary and the recovery ladder can react. `ExecutionResult` carries `failure_class` + `restore_state` so the class/state survive even when a caller returns `Ok(success:false)`.

### Changed
- `oxios-ouroboros` gains a resilience bridge for directive-level recovery; the orchestrator wires `RecoveryCoordinator` behind a read lock and falls back to the direct lifecycle when unconfigured.

## [1.9.0] - 2026-06-24

### Added
- **RFC-027: single-path intent pipeline** — Ouroboros type reorg consolidating the intent (assess → crystallize → execute → review) flow into a single path; orchestrator/agent-runtime/gateway migrated to root-level ouroboros types.
- **RFC-024: web↔daemon reliability** — full SP1–SP4 close: message ordering + replay buffer, atomic web-dist swap (no 404 window), subsystem readiness gate (503 until warm, Degraded counts as ready), and client-side WS keepalive/resume.

### Fixed
- **Chat WebSocket connects when auth is disabled** — v1.8.1's F3 token hardening blocked the WS in the default no-auth config (no login UI exists to set a token), leaving chat stuck on "재연결 중". The frontend now reads `auth_enabled` from `/api/status` (newly exposed) and connects without credentials when auth is off.
- **Auth-enabled browser WebSocket** — `/api/chat/stream` no longer fails the upgrade under `require_auth` (browsers cannot attach a Bearer header to a WebSocket); authentication is enforced by the handler via the `?ticket=` query param.
- **Memory HTTP API wired to the MemoryManager** — list/get/stats/pin/delete previously read the legacy category state-store while `create` wrote to the SQLite MemoryManager, so the memory page was always empty and mutations 404'd. All five handlers now use the MemoryManager (via four new `AgentApi` methods), and four missing routes are registered (`dream/status`, `dream/reports`, `{id}/pin`, `DELETE {name}`). Response shapes match the frontend `MemoryDetail`/`MemoryStats` types.
- **Memory overview renders in production builds** — recharts 3.x `BarChart`/`PieChart` threw `TypeError: t is not a function` when bundled by rolldown (vite v8); replaced with a dependency-free CSS bar.
- **Web lint** — auto-fixed pre-existing biome violations (`useLiteralKeys`, `organizeImports`) that failed the v1.8.1 release CI.

### Changed
- Kernel/orchestrator/gateway refactored to root-level ouroboros types; legacy five-phase integration tests dropped.

## [1.7.1] - 2026-06-22

### Changed
- **Cargo.lock update** — Lockfile refresh to include the correct dependency resolution for the v1.7.0 release.

## [1.8.1] - 2026-06-22

### Changed
- **oxi-sdk 0.37.1 → 0.45.1.** Workspace dependency bumped. `oxi-agent`'s `AgentConfig` gained four `#[serde(skip, default)]` fields (`ttsr_engine`, `memory`, `todo`, `agent_pool`); the single construction site in `crates/oxios-kernel/src/agent_runtime.rs::run_agent` now ends with `..Default::default()`. Catalog-port (0.37.0), `ask` tool rename (0.40.0), edition-2024 lift (0.41.x), and `resolve_model_from_id` catalog fallback (0.45.0) are all additive; no source-level behavior change for oxios.
- **ProjectManager schema initialization** — `ProjectManager::new` now calls `ensure_project_schema` to bootstrap the project database tables, mirroring `MountManager`'s startup behavior.


## [1.8.0] - 2026-06-22

### Added — RFC-028: Web UI Delivery
- **AgentStopped `success` flag (SP-1a)** — `KernelEvent::AgentStopped` now carries `success: bool`. `sanitize_event` serializes it as `agent_stopped.success` on the SSE wire. The supervisor emits `result.success` on the Ok path and `false` on kill/terminate. `#[serde(default)]` keeps older consumers working.
- **Completion notifications (SP-1b)** — `use-global-events.ts` handles `agent_stopped` events: `success:true` → "Task Completed" (success severity), `success:false` → "Task Failed" (warning). Cross-event dedup suppresses `agent_stopped(success:false)` when `agent_failed` was already emitted within 30s.
- **Notification persistence (SP-1c)** — Zustand `persist` middleware stores unread notifications (max 30) in `localStorage` under `oxios-notifications`. Read notifications are transient.
- **Desktop notifications + sound (SP-1d)** — New `desktop-notify.ts` (Notification API, background-tab only) and `sound.ts` (Web Audio oscillator, severity-distinct tones). Integrated into `use-global-events`.
- **Notification preferences (SP-1e)** — Client-side toggles for desktop notifications, sound, completion sound, and error sound in a new Settings → Notifications section. Stored in `localStorage`.
- **Declarative config sections (SP-2a)** — Six config sections now editable in Settings: `calendar`, `otel`, `agent_log`, `resource_monitor`, `browser`, `budget`. All use the existing declarative field-defs framework; no backend changes needed.
- **Secrets API (SP-2b)** — `GET/PUT/DELETE /api/secrets[/{key}]` and `GET /api/secrets/{key}/source`. Stores credentials in `~/.oxicode/auth.json` via `CredentialStore`, never in `config.toml` plaintext. Responses are masked (`has_value`, `source`, `preview`).
- **Secrets UI (SP-2c)** — Settings → Secrets section with per-key password inputs, source badges, and masked previews.
- **Trace trajectory join (SP-3a)** — `GET /api/agents/{id}/trace` now merges session trajectory steps with `agent.tool_calls` (deduped by `tool_call_id`). Trace steps carry a `kind` field (`tool` | `memory` | `reasoning`) for future expansion.
- **UI polish (SP-4)** — Shadow tokens added (`--shadow-sm/md/lg`) with dark-mode alpha 0.2–0.4 vs light 0.04–0.08. Background raised to `oklch(0.99 0 0)` for card elevation. `focus-visible` added to header/sidebar buttons. Global `<kbd>` styling.

### Changed
- `CredentialStore` gains `delete()` and `resolve_secret()` methods for non-provider key management.
- `settings.tsx` `buildPayload` now parses `multiline` fields as JSON (for `browser.engine`); form population JSON.stringifies multiline object values.
- `SectionIconKey` union extended with 8 new icon keys; `section-icons.tsx` `ICON_MAP` updated.
- Settings consistency test updated to include `secrets` and `notifications` custom sections.
## [1.6.1] - 2026-06-21

### Fixed
- **Web daemon startup reliability** — Hardened `oxios start` / `oxios serve` against silent failure modes (RFC-024 territory):
  - Pre-spawn port guard detects an orphaned oxios process still holding the port past a stale/missing pidfile, so the spawned daemon's bind no longer fails silently while the readiness probe reports success against the old listener.
  - A readiness-probe miss now surfaces the daemon log tail and fails the start instead of printing a misleading "started".
  - `oxios serve` refuses to start a daemon whose web assets could not be obtained (it would have served 503 on every web request); CLI/Telegram-only configs with the web surface disabled are unaffected.
  - `web_dist` auto-download from GitHub Releases now retries with a bounded backoff so a transient network blip or rate-limit does not strand the daemon.
  - Unit tests added for `port_in_use` and the startup guards.

## [1.6.0] - 2026-06-21

### Added
- **Interview wizard a11y / keyboard** — Roving focus for option groups (ArrowLeft/Right on `single_choice` auto-selects like a native radiogroup), Space to focus-and-select, Shift+Enter inserts a newline in `free_text`, and `role="group"` / `aria-pressed` / `aria-label` on option buttons so screen readers announce selection state and group semantics. The `keyboardHint` strings (en/ko) are updated to reflect the new bindings. A new test file covers the keyboard + selection behavior across `single_choice`, `multi_choice`, and `free_text` kinds.

### Changed
- **Refactor: live model resolution via `ModelResolver` port** — All LLM-bound phases now read the live, post-hot-swap engine default through a new `ModelResolver` trait (`oxios-ouroboros::ModelResolver`) instead of capturing a frozen model id at construction. This eliminates the divergence where interview / seed / evaluate / evolve used a boot-time model while execute re-resolved via the engine handle, and surfaces a bad model id at the first phase call instead of silently at execute.
  - `OuroborosEngine::new` now takes `Arc<dyn ModelResolver>` and resolves the live default + provider at the start of every LLM-bound phase. Tests use a new `StaticModelResolver` helper.
  - `EngineHandle` (kernel) implements `ModelResolver`; `OxiosEngine` gains a provider cache that survives across reads within one generation and is cleared on `swap`.
  - `EngineApi::set_model` validates the new model BEFORE persisting (rejects unknown models / unconfigured providers), so a Web UI "switch succeeded" is truthful and a bad model id no longer surfaces only at execute time.
  - `AgentRuntime`, `PersistenceHook`, `KnowledgeDream`, `KnowledgeLens` drop their frozen `model_id` fields and resolve live on each call.
  - Boot-time validation: a broken configured model now fails the daemon fast instead of silently at every curation run (`KnowledgeDream`, `KernelBuilder`).

### Fixed
- **Clippy: clear pre-existing lints on v1.5.2** — A clippy upgrade since v1.5.2 surfaced 38 mechanical lints (in `option_map_unit_fn`, `field_reassign_with_default`, `items_after_test_module`, `needless_borrows_for_generic_args`, `nonminimal_bool`, `ptr_arg`, `useless_conversion`, `cloned_ref_to_slice_refs`, `unused_imports`, and `dead_code`). All are addressed without behavior change. `cargo clippy --workspace --all-targets -- -D warnings` (the documented quality gate) now passes locally and matches CI.

## [1.5.1] - 2026-06-17

### Fixed
- **Security: wasmtime-wasi RUSTSEC-2026-0182** — Upgraded the `wasmtime` / `wasmtime-wasi` dependency from 22 to 24.0.10 (the backport release that fixes the WASIp1 `fd_renumber` resource leak). `cargo audit` now reports zero vulnerabilities. `wasm-sandbox` is still an optional, non-default feature, so default builds were unaffected, but the published `oxios-kernel` now resolves to the patched transitive dependency.

## [1.5.0] - 2026-06-17

### Added
- **`oxios update` overhaul** — Progress bars for all three update stages (web UI download with byte/speed/ETA, zip extraction file count, `cargo install` spinner that reflects the live compile line) and automatic daemon restart after a successful update so the new binary/web UI takes effect immediately. A `--no-restart` flag opts out, and restart only fires when the daemon is already running.

### Fixed
- **Web i18n (Korean UI)** — Restored 189 translation keys that were missing from both `en.json` and `ko.json` (mounts, projects, email, knowledge UI, chat/questionnaire, agents/sessions, dataTable, shared common/settings), which had been rendering as raw `section.key` strings in the UI.
- **`oxios update`** — A daemon restart failure no longer masks a successful update; it now warns and points at `oxios start` for manual recovery instead of exiting as a failure.
- **Web i18n polish** — `questionnaire.count` singular/plural ("1 questions"), mounts rescan terminology consistency, and removal of a dead duplicate `chat.questionnaire.*` namespace.

## [1.4.0] - 2026-06-16

### Added
- **RFC-024 web↔daemon reliability** — Atomic static-asset distribution with content-hash references, hard timeouts on all SSE/WS streams, and a readiness gate (SP4) so the web surface only serves after the kernel is fully initialized. Gateway gains a SP1/SP2 reliability layer.
- **RFC-025 Mount + Project system** — Unified notion of host directories mounted into the workspace as first-class Project bundles:
  - Mount core + Workspace Context injection (Phase 1) and Project bundle layer + agent enrichment (Phase 2/3).
  - Frontend Mount UI with detection badge and Project bundle rendering.
  - Phase 4 Mount rescan; Phase 5 frequent-path auto-promotion to Mounts.
  - Project-tree sidebar with drag-to-reparent and data migration.
- **Mobile responsive design (Web)** — Full responsive redesign (Phases 1–5) across chat, control, browse, and settings surfaces.
- **Settings UX overhaul (Web)** — Range sliders, full tool checklist (replacing the allowed_tools tag-input), CORS editor, and field-control polish.

### Changed
- **Version bump to 1.4.0** — All crates updated to 1.4.0; web `package.json` aligned to 1.4.0.
- **Rust 2024 edition + oxi-sdk 0.35.0** — Workspace migrated to edition 2024 and bumped to oxi-sdk 0.35.0 (native-browser fix).
- **wasm-sandbox wasmtime 22 migration** — Resolved `WasiCtx`, `fuel_remaining`, `define_wasi`, and `Memory::read` API drift; `cargo build/clippy --workspace --all-features` now passes cleanly.
- **Iconography (Web)** — Replaced emoji across the UI with lucide-react icons.

### Fixed
- **RFC-025 review pass** — Fixed all critical, major, and minor issues identified in the review across the stack (remaining substantive bugs, last design issues).
- **Settings** — Phantom memory changes from a non-existent field key; `dream_interval_hours` slider max reduced from 168h to 72h; settings shell flex layout-break on narrow screens.
- **Web** — Accidental text selection on interactive UI chrome.
- **Frontend provider catalog** — Missing provider models added to the fallback catalog.

## [1.3.0] - 2026-06-13

### Added
- **Agent History Log** — Persistent agent records survive daemon restarts.
  - Dual-tier storage: filesystem JSON (source of truth, `state/agents/<id>.json`) + SQLite query index (`state/agent_log.db`) with FTS5 full-text search.
  - `AgentLogDb` query engine: filtering (status, date range, session/project/seed), sorting (cost, duration, tokens, name), pagination, search across agent name / error / tool names / tool outputs.
  - `KernelHandle::reindex()` rebuilds the SQLite index from filesystem JSON at any time. SQLite is optional via the `sqlite-memory` feature; falls back to filesystem scan when disabled.
- **`AgentStatus::Completed`** — New terminal status for agents that finish successfully; integrated into the agent stats aggregation (`Idle`/`Stopped`/`Completed` → `completed`).
- **RFC-015 knowledge/memory separation** — Distinguished agent memory (`MemoryManager`) from user knowledge notes (`KnowledgeBase`), clarifying the two-system boundary.
- **RFC-016 autonomous persistence** — Agent-generated notes persist with provenance metadata automatically.
- **RFC-022 knowledge provenance, quality metadata & dream curation** — Notes carry `source` (Hook/Agent) and `quality` (Raw/Reviewed) frontmatter; dream consolidation curates based on quality.
- **Interactive interview wizard (Web)** — Multi-round Ouroboros interview UI with Q&A preserved across turns, typing indicator, and structured question rendering.
- **Chat & dashboard redesign (Web)** — Redesigned chat (tool-name transparency, session titles, keyboard shortcuts) and dashboard (agent status, system health, live activity feed, approvals queue).

### Changed
- **Version bump to 1.3.0** — All crates updated to 1.3.0.
- **Interview multi-turn context** — Original user message and prior Q&A are now included in interview context so the LLM understands follow-up rounds.
- **Evaluation semantics** — `evaluation_passed` modelled as `Option<bool>` end-to-end (gateway → web → frontend) for correct null semantics.
- **Async-trait restoration** — Replaced manual `Pin<Box<...>>` boilerplate with the `async-trait` macro in the kernel.

### Fixed
- **Test compile & clippy** — Resolved incomplete `agent_log_db` module (added `AgentStatus::Completed` variant, completed `parse_status` mapping) and cleared all `clippy -D warnings` lints in the new code.
- **Agent stats SQL NULL handling** — `SUM(CASE …)` / `AVG(…)` / `MIN`/`MAX` aggregates now wrapped in `COALESCE` and read as `Option`, so stats queries succeed on empty/all-NULL tables.
- **i18n** — Added missing `common.justNow` / `minutesAgo` / `hoursAgo` translation keys.
- **Frontend provider catalog** — Added missing provider models to the frontend fallback catalog.

## [1.1.0] - 2026-06-06

### Added
- **OxiBrowser Observability v0.12 — Phases 3 & 4** — Real-time tool progress flows from the oxi-agent loop through oxios-kernel → oxios-web → frontend.
  - `KernelEvent::ToolExecutionProgress` variant + `agent_runtime` forwarding of `AgentEvent::ToolExecutionUpdate { partial_result }`
  - oxios-web converts the new event into a `tool_progress` WS chunk (and SSE event)
  - Frontend: `StreamChunk.tool_progress` → `ChatActivity.tool_call` with `progress` and `isRunning: true`; `tool_start` sets `isRunning: true`, `tool_end` clears it
  - `ActivityCard` renders a `Loader2` spinner for running tool calls and shows the latest progress text inline
- **OxiBrowser Observability v0.12 — Phase 5 (tab-id propagation)** — Browser tab id propagation through kernel → web → frontend, enabling concurrent tab distinction in the chat transparency timeline.
  - `KernelEvent::ToolExecutionProgress` gains `tab_id: Option<Uuid>` (optional, serde skip-if-none for back-compat).
  - WS/SSE events include `tab_id`; frontend `ActivityCard` shows a short tab-id badge.
  - Audit-action detail string appends `:tab=<id>` when tab is known.
- **RFC-018 b.1: Memory extraction** — `chunking`, `normalizer`, `hyperbolic` modules extracted from `oxios-kernel::memory` to new `oxios-memory` leaf crate.
  - Back-compat: `use oxios_kernel::chunk_fixed` etc. all continue to work.
- **oxios-calendar** — New `.ics`-based calendar event management crate (parse, query, CRUD).
- **Email subsystem** — SMTP-based email sending integration (`leitner`), template management, sent history, provider config.
- **Calendar CLI** — `oxios calendar` subcommand with `list`, `add`, `delete`, `search`, `import`, `export`.
- **Email CLI** — `oxios email` subcommand with `setup`, `test`, `history`, `templates`.
- **Email & Calendar REST API** — Full CRUD endpoints on `/api/email/*` and `/api/calendar/*`.

### Changed
- **Version bump to 1.1.0** — All crates updated to 1.1.0 for first crates.io publication.
- **Memory re-export layer** — `oxios-kernel` re-exports the moved memory types so downstream crates (web, gateway) require no source changes.
- **Release profile applied** — `[profile.release]` with `lto = "thin"`, `codegen-units = 1`, `strip = true`, `panic = "abort"`, `opt-level = 3`. Binary size ~50 MB.
- **CI workflow hardened** — Workflow-level `permissions: contents: read`; `cargo-audit` uses `taiki-e/install-action`; target cache key includes `${{ github.sha }}`.
- **Release workflow permissions** — Read-only default; release job keeps `contents: write`.

### Fixed
- **TSC errors** — All 96 pre-existing + 3 v0.12-scope TypeScript errors cleared to 0.
- **Clippy warnings** — 14 warnings in binary crate (`src/main.rs`, `src/kernel.rs`, `src/web_dist.rs`) resolved.
- **CI formatting drift** — `cargo fmt` inconsistencies across kernel, web, and binary crate rectified.
- **CI clippy feature flag** — Fixed `browser` feature not existing on core crates in CI workflow.
- **Dead-code warning** — `WebDistResult::Embedded` marked `#[allow(dead_code)]`.

### Removed
- **Legacy `share/default-programs/`** — Superseded by `share/default-skills/` per RFC-009.

### Release Infrastructure
- **Publish order** — `release.yml` updated: `oxios-memory` and `oxios-calendar` added to publish sequence in correct dependency order.

## [1.0.2] - 2026-05-31

### Changed

- **Version bump to 1.0.2** — All crates updated: oxios, oxios-kernel, oxios-markdown, oxios-ouroboros, oxios-gateway, oxios-mcp, oxios-web, oxios-cli, oxios-telegram
- **Path dependencies updated** — All internal workspace dependencies now reference 1.0.2

### Notes

- This release prepares crates for publication to crates.io
- Web UI dist should be published to GitHub Releases separately

## [0.5.0] - 2026-05-30

### Added

#### Architecture Review Implementation (RFC-013~020)

- **Gateway Event-Driven** (RFC-013) — `tokio::select!` + shared `mpsc` channel replacing polling loop. Semaphore-bounded concurrency (32). Per-channel `tokio::spawn` receive tasks with graceful shutdown
- **Channel UX Unification** (RFC-014) — Shared `format.rs` module (CLI/Telegram/Web). `ErrorKind` classification (`error_classify.rs`). Typed `ResponseMeta` (session_id, space_id, seed_id, phase, evaluation_passed, duration_ms). `ChannelFormatter` trait
- **Security Model Integration** (RFC-015) — 4-layer `AccessGate` (CSpace → RBAC → Permissions → ExecConfig) with short-circuit evaluation. `AuditSink` for policy decision recording. `AgentContext` (who/why/where) tracking. `GatedTool` wrapper for permission enforcement
- **Proactive Recall & SONA** (RFC-020) — Activated proactive recall at session start and topic transitions. SONA learning engine: trajectory recording, pattern distillation, embedding-based similarity
- **Ouroboros Evolution Loop** (RFC-019) — Full evaluate + evolve cycle connected. `should_evaluate()`, structured evaluation with caching, LLM-based seed evolution with max iteration control

#### Memory Infrastructure (RFC-012)

- **SQLite Memory Store** — Persistent memory backend replacing in-memory-only storage
- **GGUF Embedding Provider** — Local embedding via llama-gguf (replacing MLX for cross-platform support)
- **PageRank** — Importance scoring via link graph analysis
- **Hyperbolic Embeddings** — Hierarchical memory representation
- **Flash Attention** — Efficient context window utilization
- **Auto Memory Bridge** — Automatic memory operations during agent execution

#### Observability & Routing

- **Observability Module** — `Tracer`, `CostTracker`, `AuditLog` for production monitoring
- **Model Routing** — `EngineConfig` + `RoutingControl` for complexity-based model selection
- **ProviderPool** — Rate limiting across LLM providers
- **AgentPool** — Session persistence for multi-turn conversations without re-creation
- **StructuredOutput** — Evaluation result parsing with typed output

#### Frontend

- **i18n** — English and Korean support with react-i18next
- **Session Prune API** — `DELETE /api/sessions/prune` for stale session cleanup

#### Coordination

- **Middleware Pipeline** — Audit logging middleware for agent execution
- **Coordination Module** — Multi-agent coordination primitives

### Changed

- **oxi-sdk 0.22.0 → 0.23.0** — Removed direct `oxi-ai` deps, use `oxi_sdk::Oxi` via `OxiBuilder`
- **Agent Runtime** — Uses `Agent::run_streaming()` instead of deprecated `AgentLoop`
- **Kernel Re-exports** — 33 dead re-exports moved to `sdk_exports` module
- **Web surface promotion** — `channels/oxios-web` → `surface/oxios-web` (first-class citizen)
- **Frontend auth** — `getToken()` / `api-client` / `sse-client` unified to `useAuthStore` (single source of truth)
- **Config UX** — `toml_edit`-based `config set` (comment-preserving). Added `config list`, `config reset` subcommands
- **Clippy** — 82 → 0 warnings across entire workspace
- **Version bumped** to `0.5.0`

### Fixed

- **MutexGuard across await** in `sona.rs` — potential deadlock eliminated
- **agent_id RBAC bug** — `can_access_path_in_workspace` now receives real `AgentId` instead of random UUID
- **ExecTool production connection** — `with_exec_tool()` properly wired in kernel assembly
- **SQLite deadlocks** in memory tests + CJK BM25 tokenization support
- **Engine credential injection** — `validate_key` improvement for multi-provider setup
- **Release workflow** — Path corrected from `channels/oxios-web` to `surface/oxios-web`
- **`ko-KR` hardcoded locale** → browser default locale in chat UI

### Removed

- **`reasoning_bank.rs`** — Unused module (RFC-017)
- **`rvf_store.rs`** — Unused module (RFC-017)
- **`lateral.rs` / `regression.rs`** in ouroboros — Superseded by integrated evolution loop
- **`oxicode-ai` direct dependency** — All provider construction via `oxicode-sdk`
- **280+ missing_docs warnings** — Resolved across kernel crate

## [0.4.0] - 2026-05-25

### Added

#### Tiered Memory System (RFC-008)

- **3-Tier Memory** (`memory/mod.rs`) — Hot (always loaded, ~3K tokens), Warm (on-demand), Cold (compressed archive)
- **Dream Process** (`memory/dream.rs`) — 4-phase background consolidation: Orient → Gather Signal → Consolidate → Prune & Index. Supports checkpointing for crash recovery.
- **Auto-Classification** (`memory/auto_classify.rs`) — Infers `MemoryType` (Fact, Decision, Episode, Knowledge, etc.) from content patterns
- **Auto-Protection** (`memory/auto_protect.rs`) — Automatically promotes protection level based on access frequency, session appearances, and user corrections
- **Decay Engine** (`memory/decay.rs`) — Ebbinghaus-inspired forgetting curve with protection-aware rate adjustment
- **Compaction Tree** (`memory/compaction.rs`) — 5-level compression: Raw → Daily → Weekly → Monthly → Root
- **ROOT Index** (`memory/root_index.rs`) — O(1) topic lookup so agents know what they know without scanning
- **Proactive Recall** (`memory/proactive.rs`) — Automatically injects relevant memories at session start and topic transitions
- **Auto Memory Bridge** (`memory/auto_memory_bridge.rs`) — Bridge between agent runtime and memory subsystem for automatic memory operations
- **Memory Types**: Conversation, Session, Fact, Episode, Knowledge, Skill, Preference, Decision, UserProfile
- **Protection Levels**: None → Low → Medium → High → Permanent (auto-calculated)

#### Unified Skill System (RFC-009)

- **SkillManager** (`skill.rs`) — Unified skill manager replacing `SkillStore` + `ProgramManager` + `HostToolValidator`
- **SKILL.md Frontmatter** — All metadata in YAML frontmatter (no separate `program.toml`)
- **4-Dimensional Requirements** — `bins`, `anyBins`, `env`, `config` checks per skill
- **Install Specs** — Automatic dependency installation: brew, node, go, uv, download
- **Skill Eligibility** — Per-skill status: Ready, NeedsSetup, Disabled with missing requirements details
- **Skill Source Hierarchy** — agent-specific > workspace > global user > bundled
- **Skill Snapshot** — XML prompt injection for agent initialization

### Changed

- **Memory system** upgraded from flat vector store to tiered memory with Dream-time consolidation
- **Skills and Programs merged** into a single unified Skill model
- Version bumped to `0.4.0`

### Removed

- **`program/` module** — replaced by unified `SkillManager` in `skill.rs`
- **`ProgramManager`** — merged into `SkillManager`
- **`SkillStore`** — merged into `SkillManager`
- **`HostToolValidator`** (`host_tools.rs`) — replaced by per-skill `check_requirements()`
- **`program.toml` format** — all metadata now in SKILL.md YAML frontmatter
- **`.programs/` directory** — skills migrated to `share/default-skills/`
- **Programs API endpoints** — merged into `/api/skills`
- **Host Tools API endpoint** — deprecated, functionality in skill eligibility checks

## [0.2.0-alpha] - 2026-05-03

### Added

#### AIOS-Inspired Kernel Extensions

- **AgentScheduler** (`scheduler.rs`) — Priority-based task scheduler with:
  - Priority queue (Critical > High > Normal > Low)
  - Rate-limit-aware admission control
  - Max concurrent task enforcement
  - Zombie task detection and automatic reaping
  - API endpoints: `GET /api/scheduler/stats`, `GET /api/scheduler/tasks`

- **ContextManager** (`context_manager.rs`) — 3-tier context hierarchy:
  - **Active tier**: In-memory, in-context (configurable tokens)
  - **Cache tier**: In-memory, not in-context (LRU entries)
  - **Archive tier**: Compressed on disk (unlimited)
  - Automatic demotion when active tier fills up

- **AccessManager** (`access_manager.rs`) — OWASP-inspired security:
  - Tool access control (allow-list per agent)
  - Path sandboxing (glob patterns for allowed/denied paths)
  - Network restrictions (disabled by default)
  - Execution limits (time and memory)
  - Audit logging (timestamp, agent, action, resource, decision)
  - API endpoints: `GET /api/audit`, `GET/PUT /api/permissions/:agent`

#### Programs System

- **ProgramManager** (`program.rs`) — OS-level installable applications:
  - Install/uninstall programs from directories, git, or tarball URLs
  - Enable/disable programs
  - Host requirements validation
  - Program metadata parsing (program.toml)
  - API endpoints:
    - `GET /api/programs`, `POST /api/programs`
    - `GET /api/programs/:name`, `DELETE /api/programs/:name`
    - `POST /api/programs/:name/enable`, `POST /api/programs/:name/disable`
    - `GET /api/programs/:name/host-requirements`

- **SkillStore** (`skill.rs`) — Markdown-based instruction templates:
  - CRUD operations for skills
  - Storage in `~/.oxios/workspace/skills/`
  - API endpoints: `GET /api/skills`, `POST /api/skills`, `DELETE /api/skills/:name`

#### MCP & Host Tools

- **McpBridge** (`mcp.rs`) — Model Context Protocol awareness:
  - MCP server registration
  - Tool capability enumeration
  - Protocol handshake support
  - API endpoints: `GET /api/mcp/servers`, `POST /api/mcp/servers`

- **HostToolValidator** (`host_tools.rs`) — Minimal container validation:
  - Required vs optional host tool distinction
  - Presence checking via `which`
  - Full host environment audit
  - API endpoint: `GET /api/host-tools`

#### Seeds & Evaluation API

- `GET /api/seeds/:id/evolution` — Track seed evolution lineage with parent links and evaluation scores
- **ExecutionMetadata** (`oxios-ouroboros`) — Per-seed execution tracking:
  - Execution count and rolling average score
  - Success rate calculation
  - User-defined tags for categorization

#### Configuration Enhancements

- `[scheduler]` section — Max concurrent, rate limit, zombie timeout
- `[context]` section — Active/cache/archive tier configuration
- `[security]` section — Audit log size, default tool allowlists
- `[persona]` section — Default persona and concurrent persona limits

#### Persona System

- **PersonaManager** + **PersonaStore** (`persona_manager.rs`, `persona_store.rs`) — Multiple AI characters:
  - Three default personas: Dev, Review, Research
  - Per-persona system prompts and personality traits
  - Active persona switching for orchestrator

#### State & Sessions

- **StateStore** (`state_store.rs`) — Extended with Session management:
  - `SessionId`, `UserMessage`, `AgentResponse`, `Session` types
  - Full conversation history persistence
  - Path traversal protection

### Changed

- Kernel module structure expanded from core modules to include AIOS extensions
- API routes reorganized to group related endpoints logically
- Version bumped to `0.2.0-alpha` across all crates
- `Seed::new()` now includes `execution_metadata` field

### Fixed

- `parking_lots` typo corrected to `parking_lot` in persona modules
- `Deserialize` import added to `state_store.rs`
- `OxiosConfig` default initialization includes all config sections
- Tuple element count mismatch in `init_kernel` callers
- `mut` warning in `PersonaManager::with_defaults`

## [0.1.0-alpha] - 2026-05-03

### Added

- **Core kernel** (`oxios-kernel`) with supervisor, event bus, and state store
- **Ouroboros protocol** (`oxios-ouroboros`) — spec-first workflow:
  interview → seed → execute → evaluate → evolve
- **Gateway** (`oxios-gateway`) with channel-agnostic message routing
- **Web dashboard** (`oxios-web`) with chat, control, and browse panels
- **Removed** container layer — replaced with direct ExecTool execution
- **Host Exec Bridge** for secure macOS command execution
- **Skill system** for markdown-based agent instruction templates
- **CLI** with `run`, `status`, `config`, `pkg`, `agent`, `daemon` subcommands
- **38 tests** (25 unit + 13 integration)
- **7006 lines** of Rust code across 27 source files
- **1761 lines** of HTML for the web dashboard
