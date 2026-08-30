# Persona Chat Picker — Design

**Date:** 2026-08-22
**Status:** Implemented basis for review (user-approved autonomous run)
**Depends on:** RFC-039 (persona completion), RFC-044 §8 (persona capability flags), RFC-025 (mount system)
**Relationship to `2026-08-20-persona-tool-scope-design.md`:** that design
governs the future kernel capability/tool-profile resolver (Plan 1, not yet
implemented). This design is the UI-facing slice that can ship now and does
not conflict: persona remains a preset, never an authority mint. When Plan 1
lands, the persona record it consumes gains `tool_profile`; the session-scoped
selection introduced here is exactly the ingress snapshot trigger §8 of that
design calls for.

## 1. Problem

1. The chat composer only exposes model/role selection. The persona — the
   character that shapes the system prompt and the UI affordances — is only
   switchable **globally** (personas page, command palette), affecting every
   session on every channel at once.
2. Personas have no taxonomy: there is no way to distinguish a coding persona
   from a novel-writing persona, and no sub-categories inside writing.
3. Coding work needs workspace wiring (which mounts / project paths the agent
   operates on). That wiring exists (RFC-025 mounts, `@`-mention) but is not
   connected to persona selection.

## 2. Goals / non-goals

**Goals**

- Persona selection in the chat composer, **session-scoped** with global
  default fallback (`turn override > session persona > global active`).
- Persona categories: `normal`(일반, oxios 제어 포함), `coding`, `writing`
  (with genre sub-category: novel/scenario/essay/blog), `research`,
  `operations`, `general`(범용). Stored as free strings; UI groups unknown
  values under "other" — forward compatible with user-defined categories.
- Built-in writing personas for each genre + a built-in "일반" persona.
- Persona-scoped workspace defaults: a persona can carry
  `default_mount_ids` that the composer auto-attaches when selected
  (UI-level application only — the request still carries explicit
  `mount_ids`; the persona never forces authority).

**Non-goals**

- No kernel tool-profile resolver (Plan 1 territory). CSpace still comes
  from `resolve_cspace(persona_role)` → worker fallback.
- No separate routes per persona (RFC-044 §8.4: one shared chat substrate).
- No code-editing surface for the coding persona (explicitly excluded by
  the user).
- No removal of the global active-persona concept; it remains the default
  for new sessions and other channels (CLI/Telegram).

## 3. Data model

### 3.1 Persona fields (kernel, additive — schema v2 stays)

```rust
pub struct Persona {
    // … existing 9 fields unchanged …
    /// UI taxonomy bucket: normal | coding | writing | research |
    /// operations | general (free string; unknown → "other" in UI).
    #[serde(default = "default_category")]
    pub category: String,
    /// Writing sub-category: novel | scenario | essay | blog.
    /// None for non-writing personas.
    #[serde(default)]
    pub genre: Option<String>,
    /// Mount IDs the chat composer auto-attaches when this persona is
    /// selected (RFC-025 integration). Data only — never forces a grant.
    #[serde(default)]
    pub default_mount_ids: Vec<String>,
}
```

Backward compat: `#[serde(default)]` on all three — v2 `index.json` files
load unchanged; save rewrites with new fields. No schema bump.

### 3.2 Category assignment for defaults

| Persona | Category | Genre |
|---|---|---|
| normal (new) | normal | — |
| dev | coding | — |
| review | coding | — |
| writer | writing | — |
| novelist (new) | writing | novel |
| scenarist (new) | writing | scenario |
| essayist (new) | writing | essay |
| blogger (new) | writing | blog |
| research | research | — |
| ops | operations | — |
| security | operations | — |
| architect, mentor, planner | general | — |

"일반" (`normal`) is a real built-in persona — not a UI pseudo-entry — so it
flows through the single persona code path (prompt injection, capabilities,
session persistence). Its prompt is a short neutral assistant definition;
its role resolves to the worker template (full tool set incl. oxios control
tools), identical to every other persona today (`resolve_cspace` falls back
to worker for unknown roles).

## 4. Session-scoped selection (backend threading)

Precedence: `persona_id` on the turn (WS/HTTP payload) → session's
persisted `active_persona_id` is the durable record of the last override →
global active persona (unchanged default).

```
WS payload {persona_id?}
  → chat.rs recv task: validate (exists + enabled, else error frame)
    → msg.metadata["persona_id"]
  → gateway.rs: read metadata → handle_unified(…, persona_id, …)
  → orchestrator handle_unified → MsgCtx.persona_id
  → resolve_exec_env → ExecEnv.persona_id
  → agent_runtime execute_inner:
        persona = env.persona_id (enabled) ?? global active
        → persona_prompt (## Persona section)
        → persona_role (CSpace resolution)
  → persist path: session.active_persona_id = persona_id (WS persist_session
    + HTTP POST /api/chat), so GET /api/sessions/:id reflects it
```

- Unknown/disabled override id at execute time falls back to global active
  (fail-open to today's behavior); the WS layer rejects invalid ids up front
  (mirroring model validation).
- CLI/Telegram/unspecified web turns: no `persona_id` in metadata → global
  active — exactly current behavior.
- `MsgCtx`/`ExecEnv` (oxios-ouroboros) gain `persona_id: Option<String>`
  with `#[serde(default, skip_serializing_if)]`, matching sibling fields.
- HTTP `POST /api/chat` `ChatRequest` gains `persona_id: String` (default
  empty) threaded identically.

## 5. Web UI

### 5.1 Chat store (`web/src/stores/chat.ts`)

- `activePersonaId: string | null` — persisted (sticky across reloads,
  same policy as `activeModelId`).
- `setActivePersona(id | null)`; `sendMessage` payload gains
  `persona_id: activePersonaId ?? ''`.
- `loadSession` rehydrates `activePersonaId` from the session's
  `active_persona_id` (session record wins on reopen — durable truth).

### 5.2 PersonaPicker (`web/src/components/chat/persona-picker.tsx`)

- Pill next to `ModelPickerContainer` in the composer toolbar: persona icon
  + current name (or "자동" when inheriting the global persona).
- Popover: first row "자동 (전역: <name>)" clears the override; then personas
  grouped by category with localized category headers and genre badges.
  Disabled personas hidden; capability chips not repeated (composer already
  reflects them).
- Selecting a persona with `default_mount_ids` auto-attaches those mounts
  (dedup against current `activeMountIds`, primary-first ordering preserved)
  with a toast. Removal stays possible via existing mount chips.
- Follows the `ModelPicker`/`ModelPickerContainer` split: raw props component
  (testable) + container wiring store + queries.
- Reuses `['personas']` and `['persona','active']` react-query caches —
  `usePersonaCapabilities` invalidation semantics unchanged.

### 5.3 Persona affordances

`usePersonaCapabilities` already resolves session persona → capability set
→ gates terminal/diff-viewer/worktree-fanout. With session-level selection
persisted on the session record, the existing hook picks the override up
after the first completed turn; before that, the local store selection is
the UI truth (picker state itself). No hook changes required.

### 5.4 Personas management page

- Cards grouped under category section headers; category + genre badges.
- Create dialog: category select + genre select (visible when category is
  writing).
- Edit dialog: category/genre selects + default-mounts multi-select
  (populated from `useMounts`, writes `default_mount_ids`).
- `Persona` TS type + API request/response types gain the three fields.

### 5.5 i18n

ko/en keys: picker labels, category names, genre names, editor labels.
`i18n-coverage.test.ts` enforces parity.

## 6. API surface

- `GET /api/personas` (`PersonaSummary`), `GET /api/personas/:id`,
  `POST /api/personas`, `PUT /api/personas/:id` gain
  `category`/`genre`/`default_mount_ids`.
- No new endpoints. Session persona exposure already exists
  (`GET /api/sessions/:id` → `active_persona_id`).

## 7. Testing

- Rust unit: default persona categories/genres; persistence roundtrip with
  new fields; `MsgCtx`/`ExecEnv` serde roundtrip; WS-layer validation
  covered by existing patterns (manual/browser).
- Web: typecheck, Biome, unit tests (`stores.test.ts` etc.), i18n coverage.
- Browser: persona picker open/select/send flow against the running daemon;
  personas page grouping; edit dialog fields.

## 8. Security posture

Persona selection changes prompt text and UI affordances only — it cannot
mint authority (CSpace resolution unchanged; mounts still granted explicitly
per request through the existing RFC-025 path; `default_mount_ids` is a UI
convenience that adds chips the user can remove). Validation rejects
unknown/disabled persona ids at ingress. This matches the tool-scope design
invariant "persona non-authority".
