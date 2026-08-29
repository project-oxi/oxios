# Brain Chat Binding & Old Space Concept Removal — Design

- **Date:** 2026-08-29
- **Status:** Approved (approach A; full-block-when-unbound; project default brain; global default brain)
- **Relates to:** `2026-08-29-project-roots-persona-workbench-design.md` (owns Mount removal), `2026-08-29-brain-cli-op-cutover-design.md` (CLI contract preserved), RFC-050 (vault unification — seeding removed here)

## 1. Summary

Two coupled changes:

1. **Remove the old "space" concept** (workspace-management domain superseded by Projects). Only a thin `KernelDomain("space")` string chain and misnamed status fields remain; they are renamed to `project` or deleted.
2. **Make the brain's space a per-chat binding.** A chat connects to one brain space via a chat-UI picker. Connected → the agent's brain surface (skill, recall injection, CLI allowlist, kernel-side memory writes) operates against that space automatically. Unconnected → the brain surface is **fully off** (reads and writes). Projects can declare a default brain; a global default brain exists for non-project chats.

## 2. Approved Decisions

| Decision | Choice |
|---|---|
| Architecture | **A** — thread `brain_space` through the chat pipeline (persona precedent) + per-turn gating. No BrainTool reintroduction. |
| Unconnected chat | **Full block** — no reads (recall/search), no writes (ingest/remember/auto-memory), no skill, no `oxibrain` exec. |
| Project chats | Project carries `default_brain_space`; new chats in the project inherit it. |
| Non-project chats | Global default brain `[brain].default_space` (config), editable via `PATCH /api/config`. |
| Old brain config | `[brain].space`, ecosystem `~/.oxi/config.toml [vault].space` precedence (`resolve_space`), and vault seeding (`ensure_vault_root`) are **removed**. |
| Space identity | Space moves from boot-time config to a **per-op parameter** on `BrainSession`. |
| Old-space cleanup scope | Domain rename `"space"`→`"project"` + dead remnant deletion. **Mount removal is NOT here** (project-roots workstream). |

## 3. Current State (verified facts)

- Chat metadata pipeline precedent: `ChatRequest.persona_id` (`src/api/routes/chat.rs:60`) → gateway metadata extraction (`crates/oxios-gateway/src/gateway.rs:717-735`, call site `:741-756`) → `Directive`/`MsgCtx` (`crates/oxios-ouroboros/src/directive.rs:161`) → `ExecEnv.persona_id` (`directive.rs:104`) → agent assembly (`crates/oxios-kernel/src/agent_runtime.rs:358-404`). Session persistence: `Session.active_persona_id` (`crates/oxios-kernel/src/state_store.rs:171`, setter `:277`, write-through `chat.rs:298-300,354-356,1738-1741`; WS ingress `:1257-1266,1474`).
- Agent brain reach today: brain skill text (`share/default-skills/brain/SKILL.md`, extracted at boot `src/default_skills.rs:20-90`) + `oxibrain` pushed into exec allowlist at boot (`src/kernel.rs:2546-2550` `seed_oxibrain_exec_allowlist`). Structured exec: RBAC → bare-name → `is_binary_allowed` → metachar check → spawn with `env_clear` + fixed env; payload travels as `--json @scratch-file` args (`crates/oxios-kernel/src/tools/exec_tool.rs:282-341`). No stdin.
- Kernel-side brain consumers (all currently use the single boot space): system-prompt recall (`agent_runtime.rs:464-473`), compaction remember callback (`:1152-1153`), post-turn `PersistenceHook` (`crates/oxios-kernel/src/persistence_hook.rs:87-204`), system-prompt brain bullet (`:1759-1775`, `:1854`).
- `BrainSession` hardcodes `config.space` into every op payload and resource URI (`crates/oxios-kernel/src/brain/session.rs:153,175,196,214,250,257,266,277`). Space resolution precedence: `resolve_space` (`brain/mod.rs:36`), wired at boot (`src/kernel.rs:1255-1271`) and CLI brain status (`src/main.rs:1603`).
- Vault seeding: `brain/documents.rs::ensure_vault_root` (`:14`), boot call `src/kernel.rs:1298-1302` (detached), CLI call `src/main.rs:1656-1660`.
- Tools are registered **per turn** (`agent_runtime.rs:1008`, `register_arc` `:1135-1138`) — per-turn gating is structurally natural.
- Per-invocation env injection precedent: `OXICODE_SESSION_ID` in hook subprocess (`crates/oxios-kernel/src/hook_runner.rs:79-95`).
- Skill attach filter: `SkillManager::build_snapshot_for` (`crates/oxios-kernel/src/skill/manager.rs:98-147`).
- Old-space remnants: `KernelDomain("space")` grants (`capability/template.rs:215,246-247`), gated registration arm (`tools/registration.rs:408-411`, doc row `:328`), `KNOWN_DOMAINS`/`domain_description` (`tools/retrieval.rs:269,320`, tests `:550-565`), `ResourceRef::Space` variant with zero consumers (`capability/types.rs:185-189,216`, grant `template.rs:84-87`), `spaces_active` status field computed from projects (`src/api/routes/system.rs:129-130,262-265`), dead web i18n (`en/ko.json:2200-2216` `spaces` namespace, `:687` `common.spaces`, `:263,286,290` `chat.loadingSpaces*`/`spacesLabel`), unread web types (`web/src/types/index.ts:639-640`). `SpaceApi`/`SpaceManager`: fully gone.
- Brain tab UI: `web/src/routes/brain/index.tsx` (PageHeader + StatusBanner + BrainOverview), `web/src/components/brain/overview.tsx` (spaces table, current-space via `status.space` at `:100`), hooks `web/src/hooks/use-brain.ts` (no space param anywhere). Chat selector precedent: `web/src/components/chat/persona-picker.tsx` (`pick` `:174`, container `:346-351`), insertion point `web/src/components/chat/chat-input.tsx:831`.
- `PATCH /api/config` deep-merges and persists (`src/api/routes/system.rs:1957+`, PUT alias `:1682`); `BrainSection` is `#[serde(default)]` without `deny_unknown_fields` (`config.rs:90-91`) → legacy `space = "..."` keys parse harmlessly after field removal.
- Brain REST handlers (`src/api/routes/workspace.rs`): recall `:917`, search `:930`, timeline `:1001`, brief `:1064`, spaces `:1182`, space overview `:1194` — none accept a space parameter today.

## 4. Target Architecture

### 4.1 Binding model

- One oxibrain store (`--dir` unchanged), N spaces. A **chat binding** is `brain_space: Option<String>` — the space name, not a separate store.
- Resolution order for a **new** chat's initial binding:
  1. explicit picker selection at creation (none exposed initially → skip),
  2. project `default_brain_space` when the chat is created in a project context,
  3. global `[brain].default_space`,
  4. `None` (unconnected).
- The binding is persisted on the session; the picker can change it mid-chat and the **next turn** uses the new value (a turn captures the binding at assembly time; in-flight turns are unaffected).
- Global `brain.enabled = false` overrides everything: picker hidden, all bindings ignored, surface off.

### 4.2 Data model changes

| Site | Change |
|---|---|
| `Session` (`state_store.rs:148`) | + `brain_space: Option<String>`, `set_brain_space()`; expose in `SessionSummary` (`:914`) |
| `ChatRequest` (`chat.rs:26`) | + `brain_space: Option<String>` (validated, see §7) |
| Gateway meta | + `BRAIN_SPACE` key (`crates/oxios-gateway/src/meta.rs`), extraction at `gateway.rs:717-735` |
| `Directive` / `MsgCtx` / `ExecEnv` (`directive.rs:61,161`) | + `brain_space: Option<String>` (next to `persona_id` `:104`) |
| `Project` (`crates/oxios-kernel/src/project/mod.rs`, `project_db.rs`) | + `default_brain_space: Option<String>` (nullable column; default NULL) |
| `OxiosConfig::BrainSection` (`config.rs:90`) | `space` field **removed**; + `default_space: String` (empty = none), `#[serde(default)]` |
| `BrainConfig` (`brain/config.rs`) | `space` field **removed** — config is `{ dir }` only |
| `BrainSession` | every agent op (`recall`, `remember`, `search`, `brief`, `traverse`, `why`, `contradictions`) and resource URI takes `space: &str`; console ops (`get_entity`, `timeline`, `stats`, `space_overview`) likewise; `spaces/list`, `describe` stay global |
| `BrainStatusSnapshot` (`kernel_handle/brain_api.rs:36-38`) | drop `space` field + `BrainApi::space()` |

### 4.3 Gating matrix

| Surface | Binding = Some(space) | Binding = None |
|---|---|---|
| brain skill in snapshot (`build_snapshot_for` call in `agent_runtime`) | attached | **excluded** |
| system-prompt brain bullet | present, names the space | omitted |
| system-prompt recall (`:464-473`) | `recall(space)` | skipped |
| structured exec `oxibrain` (binary name) | allowed (must still pass allowlist) | **denied**: "No brain connected to this chat" |
| shell exec mentioning `oxibrain` | allowed | **denied** (token scan, same denial) |
| exec env | `OXIOS_BRAIN_SPACE=<space>` injected into the fixed env | not injected |
| compaction remember callback | writes to `space` | skipped |
| `PersistenceHook` post-turn remember | writes to `space` | skipped |
| Web @-mention brain search / recall | scoped to binding | no-op |

Hard exec rule: `oxibrain` is denied unless the turn carries a brain binding — **independent of AllowlistMode** (covers the Permissive allow-all case). Applied in two places: `structured_exec` checks the resolved bare binary name (`exec_tool.rs:308`); `shell_exec` token-scans the `bash -c` command string for `oxibrain` (shell mode has no allowlist today — verified `exec_tool.rs:173-201` — so a substring gate is the pragmatic equivalent; same trust class as metachar blocking, absolute-path invocations included). Boot-time `seed_oxibrain_exec_allowlist` (`src/kernel.rs:2546-2550`) is **kept unchanged** (gated on `brain.enabled`) so allowlist behavior does not regress; the binding hard rule, not allowlist surgery, is the unconnected gate.

### 4.4 Data flow

```text
BrainPicker (chat-input) ──brain_space──▶ ChatRequest ──▶ gateway meta extract
   │                                          │
   │ persist on session (chat.rs :298 path)   ▼
   │                                    Directive.brain_space
   ▼                                          ▼
Session.brain_space                    MsgCtx → resolve_exec_env → ExecEnv.brain_space
                                              │
                    ┌─────────────────────────┼──────────────────────────┐
                    ▼                         ▼                          ▼
            agent_runtime turn assembly   ExecTool (oxibrain        PersistenceHook /
            (skill, bullet, recall,       deny/env)                 compaction callback
            callback space)                                         (space writes)
```

## 5. Component Changes

### 5.1 Kernel

- `crates/oxios-ouroboros/src/directive.rs`: `Directive.brain_space`, `ExecEnv.brain_space`, `MsgCtx` passthrough.
- `crates/oxios-gateway`: `meta::BRAIN_SPACE`, extraction + `handle_unified` pass-through (`gateway.rs:717-756`).
- `crates/oxios-kernel/src/orchestrator.rs`: thread binding into `MsgCtx` (`:518-528`) and `resolve_exec_env` (`:620-672`).
- `agent_runtime.rs`: gating per §4.3 (skill filter, bullet, recall, callback space, exec-tool construction carries binding, env injection).
- `exec_tool.rs`: `oxibrain` hard rule in `structured_exec` (bare-name check) and `shell_exec` (command token scan) + `OXIOS_BRAIN_SPACE` env injection into the fixed env when bound.
- `persistence_hook.rs`: space from binding; skip when `None`.
- `state_store.rs`, `project/` (column + `ProjectApi` update path `kernel_handle/project_api.rs:158-190` bundle), `config.rs` (`BrainSection`), `brain/{config,mod,documents,session}.rs` (space-per-op; delete `resolve_space`, `ensure_vault_root` + boot/CLI call sites), `src/kernel.rs` (boot rewiring `:1255-1302`; allowlist seeding kept), `src/main.rs` (brain status `:1603`, seeding `:1656-1660`).
- Old-space rename: `registration.rs:408-411,328` arm → `"project"`; `template.rs:215,246-247` grants; `template.rs:84-87` + `types.rs:185-189,216` `ResourceRef::Space` deletion; `retrieval.rs:269,320` + tests `:550-565`; `resolve.rs:117-119` test; stale docs (`template.rs:13,76-77,209-211`, `registration.rs:226`, `project/manager.rs:3`).

### 5.2 HTTP API

- Chat: `chat.rs` request field, validation, session persist, WS ingress (`:1257-1266,1474`), write-through (`:1738-1741`).
- Brain handlers (`workspace.rs`): all **scoped** handlers (`recall :917`, `search :930`, `timeline :1001`, `brief :1064`, `space overview :1194`, plus entity/why/contradictions/stats as consumed by the tab) now **require** `?space=` (400 when absent — the UI always sends an explicit selection). `spaces` (`:1182`) and `status` stay global; `status` loses `space`.
- `system.rs`: `spaces_active` → `projects_active` (or drop; nothing consumes it — choose rename for the health card's future use).
- Global default brain: `PATCH /api/config {"brain":{"default_space": "..."}}` (existing deep-merge path; no hot-reload semantics needed — read only at chat creation).

### 5.3 Web

- `stores/chat.ts`: `PersistedState.activeBrainSpace: string | null` (default `~:851`, `partialize :2051`, `setActiveBrainSpace` next to `setActivePersona :1445`, rehydrate `loadSession :1373`, hard-reset `~:1503`); `sendMessage` payload `brain_space` next to `persona_id` (`:1196-1207`).
- New `components/chat/brain-picker.tsx`: popover pill mirroring persona-picker; roster via `useBrainSpaces()`; states: unconnected ("도구 꺼짐" label), connected (`<name>`), "(기본)" suffix when value came from project/global default; project-context init from project default.
- `chat-input.tsx:831`: `<BrainPickerContainer />` beside PersonaPicker; `routes/chat.tsx` store wiring.
- Project settings (`components/project/edit-project-dialog.tsx` + create): "기본 브레인" select (uses `useBrainSpaces`, persists via project update API).
- Brain tab (`routes/brain/index.tsx`): space `Select` between StatusBanner and Overview (state via `validateSearch` param, precedent `routes/brain/entity.tsx:18-21`); scoped hooks get `space` param (`use-brain.ts` search `:50`, overview `:191`); spaces table rows navigate to the selected space; header hosts a "기본 브레인" select that PATCHes `/api/config`.
- i18n: add `chat.brain.*` (label, toolsOff, defaultTag), `brain.selectSpace`, `brain.defaultSpace`; **delete** dead keys: `spaces` namespace (en/ko `:2200-2216`), `common.spaces` (`:687`), `chat.loadingSpaces`/`loadingSpacesShort`/`spacesLabel` (`:263,286,290`); types `index.ts:639-640` cleanup.

### 5.4 Skill & config

- `share/default-skills/brain/SKILL.md`: replace "read `[brain] space` from `~/.oxios/config.toml`" with "the connected space is stated in the system prompt and exported as `$OXIOS_BRAIN_SPACE`; include it in every payload".
- `share/default-config.toml`: `[brain]` section — remove `space`, add `default_space = ""` with comment (empty = unconnected default).

## 6. Error Handling

- **Binding validation at set time:** `handle_chat` / WS ingress validate `brain_space` against `spaces/list`; unknown → 400 with a clear message (UI resets the picker). When the brain is unavailable (disabled, binary missing), validation is skipped and the binding persists — the turn surface denies/no-ops gracefully per §4.3 (degradation contract unchanged).
- **Stale binding on resume:** a space deleted in oxibrain after binding → ops return not-ok envelopes; the chat surfaces a brain error state; re-picking fixes. No auto-fallback to another space (silent rebinding is worse than a visible error).
- **Missing `?space=` on scoped brain endpoints:** 400 (contract: the tab always selects explicitly).
- **Unconnected + user asks agent to remember:** the agent has no brain skill/bullet; if it still attempts `oxibrain`, exec denies with the connect hint — the error text must be user-actionable.

## 7. Testing

- Kernel unit: directive/gateway threading; exec deny (unbound, both allowlist modes) / allow (bound); runtime gating (skill excluded, bullet+recall skipped, callback+hook space writes vs skip); `BrainSession` payload space threading; `resolve_space`/`ensure_vault_root` deletion fallout; domain rename tests (`resolve.rs:117-119`, `retrieval.rs:550-565`); project column round-trip.
- API tests: binding validation (known/unknown/unavailable-brain), session persist + write-through, scoped endpoints `?space=` required, config PATCH of `brain.default_space`.
- Web (vitest + msw): store field persist/rehydrate; picker states (none/default/connected); msw handlers for `?space=`; dead-key removal (no dangling `t('spaces.` references).
- E2E smoke: (1) connected chat — `ingest` lands in the bound space (`describe`/spaces overview confirms), recall injection present in system prompt path; (2) unconnected chat — `oxibrain` exec denied via both structured and shell paths (`bash -c "oxibrain ..."`), no brain skill in context, no post-turn memory row; (3) brain tab — switch space, scoped overview/search reflect selection; (4) project default inherited by new project chat; global default inherited by non-project chat.

## 8. Out of Scope

- Mount system removal (project-roots workstream owns it; `MountTool` keeps its current registration — after the rename it rides the `"project"` domain arm).
- oxibrain-side changes: none required (space is already a payload field; `describe`/`spaces/list` unchanged).
- Cross-space queries, space CRUD from the Web UI (creation stays oxibrain-CLI/admin-side).
- `~/.oxi/config.toml` ecosystem `[vault].space` removal inside the oxibrain repo (we only stop reading it).

## 9. Migration & Compatibility

- Legacy `space = "..."` under `[brain]` in user configs: silently ignored (`serde(default)`, no `deny_unknown_fields` — verified). Users who relied on it set `[brain] default_space` once (documented in CHANGELOG).
- Existing sessions: `brain_space` defaults to `NULL` (unconnected) — consistent with the full-block decision; users re-connect per chat or set a default.
- `documents.toml` vault root rows already seeded remain (user-editable file; kernel no longer writes it). Vault knowledge remains reachable via KnowledgeBase; brain-side vault ingestion becomes explicit (`admin index` / `ingest`).
- Clean-break precedent applies to `~/.oxios` test data; no schema migration beyond the nullable project column.
