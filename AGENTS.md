# Oxios AGENTS.md

> Onboarding document for AI agents working on this codebase.
> Hand-written. Every sentence is intentional. Do not auto-regenerate.

## What

Oxios is an **Agent Operating System** in Rust. AI agents fork, exec, wait, kill — just like Unix processes.

**Stack:** Rust 2024 (edition 2024, MSRV 1.96), tokio async, serde, oxicode-sdk (crates.io).

```
User → Channel (Web/CLI/Telegram) → Gateway → Kernel
```

```
oxios/
├── crates/
│   ├── oxios-kernel/      # Supervisor, scheduler, brain connector, security, tools
│   ├── oxios-markdown/    # Knowledge base (VirtualFs, BacklinkIndex)
│   ├── oxios-ouroboros/   # Unified intent handling (assess → crystallize → execute → review)
│   ├── oxios-gateway/     # Channel-agnostic message hub
│   ├── oxios-mcp/         # MCP client (JSON-RPC 2.0 over stdio)
│   ├── oxibrain (external CLI+store)  # agent memory (per-op CLI + per-call stdio child) — separate repo
│   └── oxios-calendar/    # .ics-based calendar event management
├── src/                   # Binary: HTTP API server, CLI/Telegram channels, main()
│   ├── api/               # REST/WebSocket/SSE (was surface/oxios-web)
│   └── channels/          # In-process channels (was channels/oxios-{cli,telegram})
├── web/                   # React frontend (was surface/oxios-web/web)
├── share/                 # Default skills, config
└── docs/                  # Architecture, RFCs, design documents
```

**Dependencies:** `oxios → oxios-kernel → {oxios-ouroboros, oxios-markdown, oxios-calendar, oxios-mcp, oxicode-sdk}`, plus `oxibrain-client` (crates.io). `oxicode-sdk` is a crates.io dependency — never reimplement what it provides. Agent memory is served by the standalone `oxibrain` binary (oxibrain 0.10, agent-first CLI): the kernel's `BrainSession` runs agent ops as one-shot `oxibrain <op> --json -` processes (response envelope) and console-only native RPCs over a `serve --stdio` child spawned per call; a `BrainInstaller` manages the binary.

## Quick Facts

| Fact | Value |
|------|-------|
| **Language** | Rust 2021 + TypeScript 5 (frontend) |
| **License** | MIT |
| **Target** | `aarch64-apple-darwin` (macOS ARM64 — single target) |
| **CI** | `cargo fmt && clippy -D warnings && cargo test --workspace` (self-hosted macOS runner) |
| **Build** | `cargo build && cd web && bun run build` |
| **Test** | `cargo test --workspace` |

## Principles

- **Unix philosophy** — fork/exec/wait/kill for agents. Compose small pieces.
- **Intent-first** — assess every message; depth adapts to the task. assess → crystallize → execute → review.
- **No reimplementation** — reuse oxicode-sdk from crates.io.
- **Channel agnostic** — gateway doesn't care where messages come from.
- **No containers** — direct host execution. Security via AccessManager (RBAC + path sandboxing).

## Conventions

- **Language:** Code, comments, docs, commits — English. **Structural/tool output is English** (global product): CLI `--help`, status panels, banners, error messages, permission-denial reasons, and Telegram bot guidance/commands. **Agent conversational replies follow the user's language** (Korean for Korean users). **Web UI is bilingual** (Korean/English). The string *sources* in `oxios-gateway`/`oxios-kernel` (e.g. `error_classify.rs`, `gate.rs`) count as tool output → English.
- **Rust:** `anyhow` for apps, `thiserror` for libs. `#![warn(missing_docs)]` on public crates.
- **Naming:** Crates `oxios-<component>`, public API `verb_noun`.
- **Testing:** Unit tests in `#[cfg(test)] mod tests`. Integration tests in `tests/`.
- **Commits:** `<type>(<scope>): <description>` — scopes: kernel, ouroboros, gateway, web, cli, docs.

### Document Rules

**No analysis or progress files in the project root.** AI agents generate intermediate files during sessions. These belong in `docs/` with proper naming, or are deleted after use.

| Type | Location | Example |
|------|----------|---------|
| RFC / design proposal | `docs/rfc-NNN-<topic>.md` | `rfc-014-agent-sandbox.md` |
| Architecture decision | `docs/ARCHITECTURE.md` | Append § section. No standalone files. |
| Design doc (UI, flow) | `docs/designs/` | `YYYY-MM-DD-<topic>-design.md` |
| Implementation result | `docs/archive/` | `<topic>-result.md` |
| Audit / review | `docs/production-audit/` | `YYYY-MM-DD-<topic>.md` |
| Temporary analysis | **Delete after use.** | Never create `*-analysis.md`, `fix-*.md`, `*-output.md`, `PROGRESS.md` in root. |

**Allowed root files:** `AGENTS.md`, `README.md`, `CHANGELOG.md`, `DESIGN.md`, `CONTRIBUTING.md`, `LICENSE`, `THIRD-PARTY-NOTICES.md`. Nothing else.

## File Locations

| Path | Purpose |
|------|---------|
| `~/.oxi/oxios/` | Oxios home (canonical; `$OXIOS_HOME` / `$OXI_HOME/oxios` override; legacy `~/.oxios/` read-only until `oxios migrate`) |
| `~/.oxi/oxios/config.toml` | Configuration |
| `~/.oxi/oxios/workspace/` | Agent working directory (sessions, skills) |
| `~/.oxi/spaces/<space>/vault/` | Shared user knowledge vault (oxi ecosystem) |
| `~/.oxi/oxios/backups/` | Backup tarballs (denied to agents) |
| `~/.oxi/oxicode/auth.json` | oxicode-cli credentials (canonical; legacy `~/.oxicode/auth.json` read-only fallback, separate from Oxios) |

## Architecture (summary)

See `docs/ARCHITECTURE.md` for the full reference (subsystems, data flow, dependency graph).

- **Kernel** (`oxios-kernel`) — intentionally monolithic single crate. Star topology around `AgentId`, `EventBus`, `StateStore`. No circular deps. Internal boundaries via `pub(crate)` + directory mod files. See ARCHITECTURE.md §10 for rationale.
- **KernelHandle** — Facade with 21 typed APIs (Agent, Security, Exec, Browser, MCP, A2A, State, Memory, Engine, …; full list in ARCHITECTURE.md §4). `ProjectApi` replaces the former `SpaceApi`; there is no `KnowledgeBaseApi` (knowledge surface is `KnowledgeLens`).
- **Supervisor** — Agent lifecycle: fork/exec/wait/kill.
- **Orchestrator** — Ouroboros protocol end-to-end. The "brain".
- **AgentRuntime** — Wraps oxicode-sdk tool-calling loop.
- **OxiosEngine** — Wraps oxicode-sdk's `Oxicode`. Provider/model resolution goes through `OxicodeBuilder`. The **catalog port** (`oxicode-sdk` `ModelCatalog`) is initialized once at boot (`OxiosEngine::init_file_catalog`, self-hosted under `~/.oxi/oxios/cache/`) and attached to every engine — including across hot-swaps — so `resolve_model` and Web UI introspection (`EngineApi`) consult dynamic models.dev metadata (live price/limit refresh, user overrides) before falling back to the static registry. Static free-fns (`get_provider_models`, etc.) remain as fallbacks (the static `model_db` is itself backed by the embedded models.dev snapshot, so it carries real prices).
- **Memory** — externalized to the standalone `oxibrain` binary (oxibrain 0.10 agent-first CLI; supersedes RFC-047/049 and the 0.8 stdio-session cutover). One process per operation: agent ops (`recall`/`ingest`/`search`/`brief`/…) run as one-shot `oxibrain <op> --json -` subprocesses; console-only native RPCs (`stats`, `document_history`, `pending_stats`, entity/timeline resources) ride a `serve --stdio` child spawned per call (`KernelHandle::brain: BrainApi`). Agents use the `brain` skill + allowlisted `oxibrain` CLI. The target space is a per-chat binding (`ExecEnv.brain_space`); chats with no binding have the brain surface off (`oxibrain` exec denied, no brain skill), while `[brain].default_space` and `Project.default_brain_space` seed new chats. Degradation contract: missing binary or failed op → empty results, turns still complete.
- **AccessManager** — OWASP RBAC + path sandboxing + Merkle audit trail.
- **Skill** — Unified system (RFC-009). Each skill = `SKILL.md` with YAML frontmatter.

## Adding a New Tool

1. Define in `crates/oxios-kernel/src/tools/<name>_tool.rs` — implement `AgentTool` from `oxicode_sdk`
2. Add a `ToolDescriptor` to the kernel catalog in `capability/descriptor.rs` — `registered_name` must equal the tool's `name()` impl
3. Add the registration arm in `tools/registration.rs::register_from_resolved_profile` (keyed by descriptor id); turn-scoped tools also need an arm in `register_tools_from_cspace_gated`
4. If it wraps a KernelHandle API, add `*_api.rs` in `kernel_handle/`
5. Test: `oxios run --json "<command that triggers tool>"`

## Key Docs

| File | Read when |
|------|-----------|
| `docs/ARCHITECTURE.md` | Modifying kernel structure, adding modules, understanding subsystems |
| `DESIGN.md` | Oxi brand design system (UI tokens, color, typography) — canonical for frontend/UI work. Unix↔Oxios mapping lives in `docs/ARCHITECTURE.md` §8 |
| `docs/rfc-008-memory-consolidation.md` | Modifying memory system |
| `docs/rfc-009-skill-unification.md` | Modifying skill system |
| `docs/designs/2026-08-29-issues-milestones-todo-design.md` | Modifying project issues, milestones, or the session todo plan |
| `docs/rfc-010-clawhub-marketplace.md` | Marketplace feature |
| `docs/rfc-024-web-daemon-reliability.md` | Modifying web↔daemon delivery, SSE/WS, static asset serving, readiness |
| `docs/rfc-050-vault-unification.md` | Modifying the shared vault path, format (`oxi-frontmatter`), or migration binary |
| `docs/design-knowledge-ui.md` | Knowledge UI (frontend components, shortcuts, architecture) |
| `docs/channel-plugin-guide.md` | Adding a new channel |
| `docs/USER-GUIDE.md` | Changing user-facing features or CLI behavior |
| `docs/designs/2026-08-31-execution-recipe-coding-host-design.md` | Modifying turn execution behavior, tool registration, or coding recipe wiring |
| `share/default-config.toml` | Changing configuration options |

## Release

Two separate pipelines, two different triggers.

### Native binary — self-hosted macOS ARM64 runner

Tag push (`v*`) triggers `.github/workflows/release.yml`. Builds native
`aarch64-apple-darwin` binary (embeds the SPA via `include_dir!`), packages as
tarball + SHA256, creates GitHub Release.

```bash
git tag v1.2.0 && git push --tags   # → CI builds native binary
```

### crates.io — CI (automatic)

GitHub Actions `publish.yml` publishes all 7 crates in topological order.
`publish.yml` is dispatched by `release.yml` after a GitHub Release is created
(a Release made with GITHUB_TOKEN doesn't emit `release: published`, so
`release.yml` triggers it via `gh workflow run`).

Topological order:

① oxios-markdown, oxios-mcp, oxios-ouroboros   (no oxios deps)
② oxios-calendar    → oxios-markdown
③ oxios-kernel      → {ouroboros, markdown, calendar, mcp}
④ oxios-gateway     → oxios-kernel
⑤ oxios (binary)    → {kernel, gateway, markdown, ouroboros, calendar}
```
`oxios-memory` was retired in RFC-047 (agent memory now lives in the `oxibrain` store (daemonless, separate repo)). The crates.io entry is deprecated, not yanked.

`oxios-web`/`oxios-cli`/`oxios-telegram` were merged into the binary as
in-process modules per RFC-026 — no separate crates to publish.
- **Kernel is intentionally monolithic.** See ARCHITECTURE.md §10. Do not propose splitting.
- **oxicode-sdk is crates.io only.** Never add as path dep. Never reimplement what it provides.
- **Kernel binary vs library.** `src/kernel.rs` (assembler) is in the binary crate, not `oxios-kernel`.
- **Agent lifecycle split.** `Supervisor` = low-level process. `AgentLifecycleManager` = full lifecycle (A2A, scheduling, permissions). Don't add lifecycle logic to Orchestrator.
- **Tool registration.** Catalog entry in `capability/descriptor.rs` (`registered_name` must match the tool's `name()` impl), registration arms in `tools/registration.rs` — `register_from_resolved_profile` for the execution-profile path, `register_tools_from_cspace_gated` for the turn path.
- **Issues are Oxios's own, not shared with oxicode.** Project issues live at `~/.oxi/oxios/projects/<id>/issues/` and are never written to a folder's `.oxicode/issues/`. The two products track different scopes — a terminal agent tracks the folder it was launched in, Oxios tracks a Project that may span zero or many folders — and converging them would reduce oxicode to a TUI skin over Oxios. We reuse the SDK *implementation* (`FileIssueStore`, `cas_retry`, `liveness`) without sharing its *data*. See `docs/designs/2026-08-29-issues-milestones-todo-design.md`.
- **Issue ownership identity.** `IssueApi::ownership_id` is `oxios-<pid>-<session>` and `AgentConfig.session_id` carries it into `ToolContext.session_id`. The SDK's assignment lock only protects anything when that identity is **non-empty** and backed by a `flock` this process holds (`IssueApi` keeps one `AliveGuard` per `(project, session)`). Do NOT set `AgentConfig.session_id` back to `None` — that was the pre-0.77 state and it made every claim instantly reclaimable, the same defect oxicode documents as #13. The session id must also be the *stable* turn key, not a per-request id; `ownership_id_is_stable_across_turns` pins it.
- **Milestones ride a reserved label.** `IssueMeta` is SDK-owned and drops unknown frontmatter keys on write, so membership is the `milestone:<slug>` label and the milestone's own record lives in `milestones.yaml`. Progress is always derived by listing labelled issues, never stored. Don't add a `milestone` field to the issue file — the next SDK write would delete it.
- **Always-on tools have two lists that must agree.** `registration::register_always_on` registers the tier; `gate::LAYER0_EXEMPT_TOOLS` exempts it from the Layer-0 CSpace check. A tool in the first but not the second is registered (so the LLM calls it) and then hard-denied on every call — the documented `web_search` catch-22. `registration::tests::always_on_tools_are_all_exempt_from_layer_zero` fails if they drift. The descriptor catalog in `capability/descriptor.rs` is a third, hand-maintained list; it advertises metadata and does not register anything.
- **Structured tool results ride one bus.** The SDK strips `AgentToolResult::metadata` at the agent-loop event boundary, so a tool that wants to send the Web UI more than text stashes a payload in `tools::StructuredResultBus` keyed by `tool_call_id`; the runtime completion callback publishes it on `KernelEvent::ToolExecutionFinished.results` → WS `tool_end.results` → the tool render. Consumers: `web_search`/`get_search_results`, `todo`, `issue`. Add to the bus rather than inventing a second channel.
- **Two knowledge systems.** Agent memory = `oxibrain` store (per-op CLI contract, see docs/designs/2026-08-29-brain-cli-op-cutover-design.md). User notes = KnowledgeBase (`.md` files, `~/.oxi/spaces/<space>/vault/` per RFC-050 + the unified home layout). The retired `~/.oxios/workspace/knowledge/` path was replaced by the shared vault — see `docs/rfc-050-vault-unification.md` for the migration runbook.
- **Unified skill model.** No separate `program/` module or `program.toml`. `SkillManager` handles everything. Each skill = `SKILL.md` + YAML frontmatter.
- **Feature gates.** Web, CLI, Telegram, browser, telemetry are feature-gated. Check `cargo build -p oxios --features <feature>`.
- **`--all-features` works.** oxicode-sdk 0.66.0 + wasmtime 24 migration — `cargo build/clippy --workspace --all-features` compiles. As of oxicode-sdk 0.45.x, `AgentConfig` gained `ttsr_engine`/`memory`/`todo`/`agent_pool` fields (all `#[serde(skip, default)]`); fill with `..Default::default()` to keep sites working. CI still uses per-crate features (`.github/workflows/ci.yml`) for precision. The prior wasm-sandbox `ResourceLimiter` regression (missing `table_growing` on wasmtime 24) is fixed in `crates/oxios-kernel/src/wasm_sandbox.rs`. As of oxicode-sdk 0.66.0, the project was renamed from `oxi` to `oxicode` (CHANGELOG §0.65.0 Breaking) — `oxi-sdk`/`oxi-ai`/`oxi-agent` → `oxicode-sdk`/`oxicode-ai`/`oxicode-agent`; `Oxi`/`OxiBuilder`/`OxiBrowserEngine` → `Oxicode`/`OxicodeBuilder`/`OxicodeBrowserEngine`; `OXI_*` → `OXICODE_*`; `~/.oxi/` → `~/.oxicode/`. oxibrowser-core bumped to 0.17. CI pulls oxicode-sdk from crates.io (no path dep, no separate `a7garden/oxicode` checkout — `Cargo.toml` `[patch.crates-io]` is intentionally uncommented).
- **Workspace deps.** `oxicode-sdk` must be in both `[workspace.dependencies]` (root `Cargo.toml`) AND `[dependencies]` in the crate using it.
- **Stdin blocking.** `oxios run --context-file -` reads stdin to EOF. Don't use with interactive input.
- **Chat turn identity.** One turn key — `session_id`, or `request_id` for a session's first message — is shared by `StreamingSinkRegistry`, `TurnRegistry`, and `ExecEnv.session_id`. Never introduce a second identifier for a turn.

## Local CI Gates (pre-commit cleanup)

Run before committing a non-trivial change:

1. Checkpoint the tree before edits.
2. `cargo fmt --all -- --check` · `cargo clippy --workspace --all-features -- -D warnings` · `cargo check --workspace --all-features` · `cargo nextest run --workspace --no-fail-fast` · `cargo test --workspace --doc`.
3. `web/`: frozen bun install, typecheck, tests, Biome lint, build.
4. `cargo audit` + `cargo deny check` — cargo-deny 0.20 requires modern advisory values (`unmaintained = "all"|"workspace"|"transitive"|"none"`) and SPDX-valid license identifiers.
5. Fix real warnings rather than hiding them; rerun all gates; commit the cleaned tree.

## oxicode-sdk upgrade

When bumping `oxicode-sdk`/`oxicode-agent`:

- **Source of truth is the local oxicode repo** (`/Volumes/MERCURY/PROJECTS/oxicode`) — read the actual trait/struct definitions there, don't guess from compiler errors: `oxicode-ai/src/circuit_breaker.rs` (`CircuitBreaker`, `DefaultCircuitBreaker`, `BreakerError`/`BreakerState`), `oxicode-agent/src/mcp/spawn.rs` (`SpawnValidator`), `oxicode-sdk/src/lib.rs` (`#[cfg(feature)]` + `#[oxi_unstable]` re-export gates), `oxicode-sdk/Cargo.toml` `[features]`, `CHANGELOG.md` Breaking/Removed sections.
- **Feature-gate gotcha:** oxicode-sdk's unstable features (`browser`, `delegation`, `subagent`, `circuit-breaker`, …) are often EMPTY — they only gate oxicode-sdk's own re-exports and do NOT enable features on oxicode-agent/oxicode-ai. Check each underlying crate's own features separately.
- **Procedure:** bump root `[workspace.dependencies]` + kernel dep → `cargo update -p oxicode-sdk -p oxicode-agent` → `cargo check -p oxios-kernel` and fix the REAL errors empirically (design docs predict wrong counts) → gates: `cargo check/clippy/test --workspace --all-features`, rustfmt only on files you touched (don't reformat pre-existing drift) → Rust-only bumps skip the web gate unless TS changed.
- Common breakages: removed types (`ProviderPool`, `ProviderCircuitBreaker`, …), typed `SdkError` returns (wrap `Ok(...?)`), feature-gated re-exports.
- Known wiring: `LLM_CIRCUIT_BREAKER` (agent_runtime.rs) is metrics-only — real gating is `resilience/health.rs`; `oxios-mcp` has its own McpClient (direct spawn) and does not use the SDK MCP transport, so `SpawnValidator` has no consumer here — don't wire it as dead code.

