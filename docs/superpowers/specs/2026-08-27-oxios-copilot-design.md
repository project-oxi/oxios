# Oxios Copilot — Design Spec

Date: 2026-08-27
Status: Reviewed — amended after design review (amendment summary at end)

## Problem

Oxios has a "Quick Chat" widget (`QuickAsk`, ⌘J) — a global one-shot,
non-persisted chat overlay. It has no clear purpose distinct from the
normal chat page: it inherits whatever persona is globally active, and
that persona typically also carries general coding tools (exec/file/
browser). There is no dedicated, scoped way to ask Oxios about — or
directly control — its own configuration (projects, mounts, tasks,
personas, security, budget, resources, MCP servers, email/calendar
integrations, model settings) without also exposing a general-purpose
coding agent.

Separately, code investigation surfaced a real gap: several tool
implementations that *already exist* (`MountTool`, `TaskTool`,
`EmailTool`, `CalendarTool`, `MarketplaceTool`) are registered
unconditionally only in the CLI/background-agent tool-registration path
(`register_all_kernel_tools` in `tools/builtin/mod.rs`, invoked by
`OxiosKernelBridge`). The persona-driven chat path
(`register_tools_from_cspace_gated` in `tools/registration.rs`, invoked
by `agent_runtime.rs` for every chat/QuickAsk session) never wires these
domains in at all — no persona, however privileged, can reach them today.

## Goal

Retire the general-purpose "Quick Chat" concept. Replace it with an
**Oxios Copilot** — a dedicated built-in persona (`oxios`) scoped
*exclusively* to inspecting and controlling Oxios's own configuration,
reachable through the same lightweight one-shot overlay UX QuickAsk used
(kept for its ephemeral, always-one-tap-away qualities), but purpose-built
rather than a generic "ask anything" surface.

## Decisions

- **Tool scope**: control-only. No exec, no file ops, no browser. This is
  a settings copilot, not a second coding assistant.
- **Entry point**: reuse and rename the existing QuickAsk overlay
  (header button + ⌘J on desktop, tap on mobile) rather than building a
  new sidebar page — the mechanic (ephemeral one-shot dialog) already fits
  the new purpose and is already mobile-responsive.
- **Persistence**: ephemeral by default (matches QuickAsk today) — no
  session written unless the user explicitly promotes the conversation.
- **Promote-to-session**: kept. The existing "promote this exchange to a
  real session" action is retained, now tagging the created session with
  `persona_id: "oxios"`.
- **Scope of control tools**: build the missing wiring *and* the two
  genuinely missing tools (MCP server management, engine/model settings)
  in this pass, rather than shipping a partial v1. Existing typed
  `KernelHandle` APIs provide the operations (`mount_api.rs`, task store,
  `email_api.rs`, `calendar_api.rs`, `marketplace_api.rs`, `mcp_api.rs`,
  `engine_api.rs`); implementation adds CSpace registration and two thin
  `AgentTool` wrappers, never a raw configuration-file editor.
- **Architecture**: extend the existing CSpace/`ToolProfile` capability
  system (new `ToolProfile::Control` tier) rather than special-casing the
  `oxios` persona id or building a second, parallel tool-gating mechanism.
  Keeps the persona → tool_profile → CSpace → tool-registration pipeline
  as the single source of truth for what any persona can do.

## Non-goals

- No changes to `skills.sh` API-key provisioning (secret input stays
  Settings-page-only).
- No raw `config.toml` free-form editing — the copilot mutates only
  through the same typed `KernelHandle` APIs the REST routes already use.
- No changes to the cron→task consolidation work done earlier this
  session — disjoint file set.
- No migration of `quick_ask_model` config field name — display label
  changes only, schema untouched.
- No email/calendar tool wiring. Review finding: `EmailTool` is literally
  `send_email` (external SMTP side effect) and `CalendarTool` mutates
  user event data — both are *use* tools, not *settings* tools, and neither
  chat persona can reach them today. Integration *status* Q&A would need a
  new read-only tool; recorded as a future gap, not built here.

## Design

### 1. Backend — `ToolProfile::Control` + `oxios` persona

`crates/oxios-kernel/src/persona/mod.rs`: add a fourth `ToolProfile`
variant:

```rust
pub enum ToolProfile {
    Minimal,
    Base,
    Code,
    Control,   // NEW — Oxios self-control only, no coding tools
}
```

`ToolProfile` serialization remains lowercase, so the new persisted value is
`"control"`. Update all three profile surfaces together:

- `ToolProfile::from_str_loose("control")` returns `ToolProfile::Control`;
- `Display` renders `"control"`;
- the persona API/UI profile enum recognizes `control` but never offers it
  as an editable choice for ordinary personas.

No snapshot migration is necessary: `PersonaManager::create_default_personas`
creates `oxios` when it is absent. Existing persisted personas retain their
current profiles; no pre-existing profile gains control tools.

`register_tools_from_cspace_gated` currently registers the always-on
read/write/edit/find/web-search tier before consulting the CSpace. That would
violate this persona's control-only boundary. Add its final, explicit
`include_always_on_tools: bool` parameter; `AgentRuntime` passes
`tool_profile != ToolProfile::Control`. When false, the function skips
`register_always_on_gated` entirely. `Control` therefore receives neither
the always-on file/web tools nor the CSpace-gated exec/browser/memory/knowledge
tools. The non-gated compatibility registration function is unchanged.

`crates/oxios-kernel/src/capability/template.rs`: new constructor,
independent of `worker()`/`code_profile()` (no exec, no browser):

```rust
/// **Control** — Oxios self-management only. No exec/browser/file tools,
/// no user-content tools (email send, calendar events).
///
/// Grants exactly the domains needed to inspect and mutate Oxios's own
/// configuration: projects/mounts, personas, tasks, security, budget,
/// resource limits, running agents, marketplace skills, MCP server
/// management, and model/engine settings.
pub fn control_profile() -> Self {
    let mut t = Self { caps: Vec::new() };
    let domains = [
        "space",        // ProjectTool + MountTool
        "persona",      // PersonaTool
        "task",         // TaskTool
        "security",     // SecurityTool
        "budget",       // BudgetTool
        "resource",     // ResourceTool
        "agent",        // KernelAgentTool
        "marketplace",  // MarketplaceTool
        "mcp_manage",   // McpManageTool (NEW)
        "engine",       // EngineTool (NEW)
    ];
    t.caps.extend(domains.map(|d| (
        ResourceRef::KernelDomain { domain: d.into() },
        Rights::READ | Rights::WRITE | Rights::EXECUTE,
    )));
    t
}
```

`crates/oxios-kernel/src/capability/resolve.rs::profile_template`: add
the `ToolProfile::Control => CapabilityTemplate::control_profile()` arm.

`crates/oxios-kernel/src/tools/registration.rs::register_tools_from_cspace_gated`:

1. Add `include_always_on_tools: bool` as its final parameter. If true,
   retain the existing `register_always_on_gated` call; if false, omit it.
   `agent_runtime.rs` passes `tool_profile != ToolProfile::Control`.
2. Extend the CSpace domain switch (the production path used by every
   chat/copilot session):
   - `"space"` → register `ProjectTool` **and** `MountTool` (currently only
     `ProjectTool`; mounts belong with projects);
   - `"task"` → register `TaskTool::from_kernel(kernel, agent_id)` when the
     task store is attached (mirrors the existing `Option`-returning guard
     in `tools/builtin/mod.rs`);
   - `"marketplace"` → register `MarketplaceTool::from_kernel(kernel)`;
   - `"mcp_manage"` → register new `McpManageTool::from_kernel(kernel)`
     wrapped in `GatedTool::with_approval` (see Safety);
   - `"engine"` → register new `EngineTool::from_kernel(kernel)` wrapped
     in `GatedTool::with_approval` (see Safety).

These cases live in the general switch, so any future persona assigned
`ToolProfile::Control` picks them up automatically — no `oxios`-specific
branching exists in capability resolution or tool registration.

New built-in persona in `crates/oxios-kernel/src/persona/mod.rs`
(`default_personas()`):

```rust
Persona {
    id: "oxios".to_string(),
    name: "Oxios".to_string(),
    description: "Internal copilot for managing Oxios itself — projects, \
        mounts, tasks, personas, security, budget, marketplace skills, \
        MCP servers, and model settings.".to_string(),
    system_prompt: "You are the Oxios Copilot: an internal assistant for \
        managing this Oxios instance's own configuration. You have no \
        coding tools (no exec, file access, or browser) — only tools to \
        inspect and change Oxios's settings. If asked to write code, run \
        commands, or browse the web, explain that's outside your scope \
        and suggest switching to a coding persona instead.".to_string(),
    tool_profile: ToolProfile::Control,
    // other fields: built-in defaults matching the existing pattern
    // (category, default_mount_ids: vec![], etc.)
}
```

The `oxios` id is reserved, enforced at the **`PersonaManager` layer** (the
single choke point below both the REST routes and `PersonaTool`):
`update`/`delete` targeting `oxios` is rejected, protecting `tool_profile`,
`system_prompt`, and identity fields regardless of which caller attempts the
change. This is required because the copilot's own `PersonaTool` reaches the
manager directly — a REST-only guard would not stop it.

Accepted consequences, documented deliberately: the copilot can create,
update, and `set_active` *other* personas (that is its job as a settings
manager), including Control-tier ones — `from_str_loose("control")` accepts
the value for tool-created personas. It can never escalate *itself*: the
copilot session's persona is fixed client-side to `oxios`, and `oxios` cannot
be restyled. Existing built-in personas other than `oxios` retain their
current mutability.

### 2. New tools

**`McpManageTool`**
(`crates/oxios-kernel/src/tools/builtin/mcp_manage_tool.rs`, wraps
`KernelHandle::mcp` / `mcp_api.rs`). Distinct from the existing
`McpToolWrapper` (which proxies an already-configured server's exposed
tools) — this manages the server registry itself:

- `action: "list"` — configured servers: name, command, args, enabled
  state, and live connection state.
- `action: "add"` — `{ name, command, args?: string[], env?: Record<string,
  string> }`; register, initialize, and roll back registration when
  initialization fails, matching the existing REST handler.
- `action: "update"` — `{ name, command, args?: string[], env?: Record<string,
  string>, enabled: boolean }`; preserve the server name and reinitialize it,
  rolling back to the previous configuration if reinitialization fails.
- `action: "remove"` — `{ name }`; disconnect and remove the server.
- `action: "toggle"` — `{ name }`; enable or disable an existing server.
- `action: "test"` — `{ name }`; initialize/test a configured server and
  return its connection state plus cached tool count.

All successful `McpManageTool` actions return compact JSON encoded as
`AgentToolResult::success`; domain/API errors return
`AgentToolResult::error` with the API's actionable message. Unknown actions
return `ToolError`, following the existing builtin-tool convention.

**`EngineTool`**
(`crates/oxios-kernel/src/tools/builtin/engine_tool.rs`, wraps
`KernelHandle::engine` / `engine_api.rs`):

- `action: "current"` — `EngineApi::config()`: active default model,
  configured provider/credential status, `quick_ask_model`, and routing
  snapshot. Never returns an API key.
- `action: "list_models"` — `{ provider?: string, query?: string }`;
  `models(provider, query)` when a provider is supplied, otherwise
  `search_models(query)` when a non-empty query is supplied. A request with
  neither field returns the provider catalog via `providers()` instead of
  materializing every model.
- `action: "set_default_model"` — `{ model: "provider/model-id" }`;
  delegates to `set_model`, which validates the model/provider, persists it,
  and hot-swaps the runtime engine.
- `action: "set_copilot_model"` — `{ model?: "provider/model-id" }`;
  delegates to `set_quick_ask_model`. Omitting `model` clears the override,
  making the copilot inherit the default model.

`EngineTool` deliberately excludes credential writes/deletes, custom-provider
definition, raw provider options, and routing policy edits. Those settings
can contain secrets or have wider operational effects and remain explicit
Settings-page operations.

All successful `EngineTool` actions return compact JSON through
`AgentToolResult::success`; validation and persistence failures return
`AgentToolResult::error` with the cause. Unknown actions return `ToolError`.


### 3. Frontend — entry point & UX

Repurpose the existing QuickAsk component tree rather than build new
scaffolding — it already has ephemeral WS streaming, tool-approval cards,
RAF-batched token rendering, and a mobile-responsive dialog. The header
button remains available on mobile; only its `⌘J` hint is desktop-only.

Rename (git mv, imports updated throughout):

| Old | New |
|---|---|
| `web/src/components/quick-ask/` | `web/src/components/oxios-copilot/` |
| `quick-ask-dialog.tsx` (`QuickAskDialog`) | `oxios-copilot-dialog.tsx` (`OxiosCopilotDialog`) |
| `web/src/stores/quick-ask.ts` (`useQuickAskStore`) | `web/src/stores/oxios-copilot.ts` (`useOxiosCopilotStore`) |
| `web/src/hooks/use-quick-ask-shortcut.ts` | `web/src/hooks/use-oxios-copilot-shortcut.ts` |

Referencing files updated: `app-layout.tsx`, `header.tsx`,
`command-palette/control.tsx`, `command-palette/new.tsx`.

Behavioral changes to the dialog/store:

1. **Persona locked**: today's QuickAsk payload carries no `persona_id` at
   all (`{ type, content, ephemeral, model }`) and silently inherits the
   globally active persona. The copilot store *adds*
   `persona_id: "oxios"` to every WS send. This surface never inherits.
2. **Model picker kept**: renamed store state
   `copilotModel`/`setCopilotModel` remains backed by
   `engineConfig.quick_ask_model` for config compatibility. Tool-calling
   quality varies by model, independently of the Control tool set.
3. **Empty state**: show the English title **“Oxios Copilot”**, subtitle
   **“Ask about or manage this Oxios instance.”**, and four action chips:
   “Add an MCP server”, “Show this month’s budget”, “List scheduled tasks”,
   and “Change the default model”. Chips send their literal text through the
   same copilot WS path; they are not static help links.
4. **Header trigger**: replace `Zap` with Lucide `Settings2`, retain the
   desktop `⌘J` hint, and rename `quickAsk.openAria` to
   `oxiosCopilot.openAria` (“Ask Oxios”).
5. **Promote-to-session kept**: `handlePromote` stays, but its seed request
   carries `persona_id: "oxios"` so the saved session keeps the Control tool
   set.

#### Composer restrictions

`ChatInput` gains a semantic `variant: "chat" | "copilot"` prop rather than
duplicated composer markup. `OxiosCopilotDialog` uses `"copilot"`, which:

- keeps the model picker, queue/stop controls, send control, and approval
  card flow;
- hides the persona picker because the persona is locked;
- hides role selection, file picker/drop handling, `@` context attachments
  (mount/knowledge/memory), model-parameter popover, and worktree fan-out;
- treats pasted/dropped files as plain editor content only; it never creates
  `AttachedFile` or `ContextAttachment` payloads.

The restricted composer makes the UI match the backend boundary: users see
only choices that the `Control` tool profile can actually use. Existing
full-chat callers retain `variant: "chat"` behavior unchanged.

No new mobile-specific layout — the existing `Dialog` overlay is already
responsive.

### 4. Data flow

Request path (mechanism unchanged from QuickAsk today, only the payload and
tool set differ):

1. `OxiosCopilotDialog` opens its own short-lived WS through `buildWsUrl`.
2. Every message sends
   `{ ephemeral: true, persona_id: "oxios", model: copilotModel, ... }`.
3. `chat.rs` resolves `"oxios"` through `state.kernel.persona.get("oxios")`,
   confirms it is enabled, and stores the id in message metadata.
4. `agent_runtime.rs` resolves `ToolProfile::Control` into
   `control_profile()` and calls `register_tools_from_cspace_gated` with
   `include_always_on_tools: false`. The registry receives exactly the
   control domains from §1.
5. `incoming_ephemeral == true` skips session persistence. Promotion calls
   `POST /api/chat/seed` with the captured exchange and
   `persona_id: "oxios"`. `SeedRequest` gains optional `persona_id`, uses
   the normal-chat enabled/unknown validation, and sets
   `Session::active_persona_id` before saving. The promoted exchange therefore
   opens as a durable Oxios-copilot session instead of falling back to the
   globally active persona.

Unchanged: WS protocol, chunk parsers (`parseChunk`, `applyContentChunk`),
tool-approval flow (`ToolApprovalCard`), RAF token batching, and
reconnect/replay dedup (`_resolvedApprovalIds`).

### 5. Rename scope & i18n

- i18n: `quickAsk.*` keys → `oxiosCopilot.*` in `en.json`/`ko.json`,
  including new empty-state/suggestion-chip copy. Chip text is localized —
  English chips in `en.json`, Korean chips in `ko.json`.
- Settings page label "Quick Ask Model" → "Oxios Copilot Model" (display
  text only; the underlying `quick_ask_model` config field name is
  unchanged — no schema migration).
- `chat.rs` comments and the `POST /api/chat/seed` request contract update
  from "QuickAsk" to "Oxios Copilot"; the generic `ephemeral` flag name
  stays as-is.
- No RFC doc — this spec is sufficient given the scope; inline
  doc-comments cover `ToolProfile::Control`, its registration parameter,
  and the new tool domains per repo convention.

## Non-scope confirmation

Disjoint from the cron→task consolidation and other web redesign work
completed earlier in this session — no overlapping files.

## Safety and verification

- The `Control` CSpace intentionally excludes all general file, web, exec,
  browser, memory, knowledge, A2A, subagent, and skill execution tools —
  and, per review amendment, user-content tools (`send_email`, calendar
  events). The `include_always_on_tools` registration parameter is the
  enforcement point for the previously unconditional file/web tier.
- **Approval reality (verified against `registration.rs`):** kernel-domain
  tools register *bare* today — no `GatedTool`, no per-call approval. Only
  the always-on tier and exec are approval-wrapped. Therefore: the two NEW
  tools (`McpManageTool`, `EngineTool`) register via
  `GatedTool::with_approval`, so destructive control operations (remove
  server, change default model) prompt through the existing
  `ToolApprovalCard` flow that the dialog already renders. Existing domains
  (`persona`, `security`, `agent` kill, …) stay bare — matching status quo
  for today's Base personas, avoiding a behavior change for unrelated
  personas. In particular, agent `kill` runs without a prompt, same as
  today; this is an accepted, documented risk.
- The retained agent-management tool exposes only its existing `list`,
  `budget`, and `kill` actions — it cannot spawn agents.
- `McpManageTool` validates the existing server lifecycle and rolls back an
  unsuccessful initialize/update, matching the REST route's contract.
  `EngineTool` delegates validation/hot-swap/persistence to `EngineApi`;
  neither tool exposes credentials.
- Backend tests cover: `Control` serialization/parsing/display; CSpace has
  only the declared control domains; a control registration contains the
  approved control tools and excludes every general tool; each new domain
  case; `SeedRequest` validation/persistence of `active_persona_id`; reserved
  `oxios` persona update/delete rejection; and action-schema/error paths for
  both new tools.
- Frontend tests cover: fixed `persona_id: "oxios"` WS payload; restricted
  composer affordances; promotion seed payload carrying `persona_id`; renamed
  i18n labels/suggestions; and header/⌘J plus command-palette opening.
  Browser smoke verification covers desktop and a narrow mobile viewport.

## Review amendments (2026-08-27)

1. **Dropped `email` + `calendar` domains.** Verified tool reality:
   `EmailTool` = `send_email` (external side effect), `CalendarTool` =
   event CRUD (user content). Both violate the settings-copilot boundary;
   wiring them would grant a locked persona a channel no chat persona has
   today. Integration-status Q&A noted as a future gap.
2. **Corrected the approval claim.** Kernel-domain tools register bare
   (no `GatedTool`) — the original Safety section overstated approval
   coverage. New tools get `GatedTool::with_approval`; existing domains
   keep status-quo bare registration.
3. **Persona reservation enforcement moved to `PersonaManager`** (choke
   point below REST *and* `PersonaTool`), with accepted consequences for
   `set_active`/create of other personas documented.
4. **Data-flow phrasing fixed**: the copilot *adds* `persona_id` to a
   payload that has none today (there is no explicit fallback to remove).
