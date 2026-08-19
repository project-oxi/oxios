# Persona Tool Scope — Design

**Date:** 2026-08-20
**Status:** Draft — awaiting review
**Depends on:** RFC-039 (persona completion), RFC-044 §8 (persona capability packs)
**Supersedes in part:** 2026-07-11 memory system overhaul §"unconditional registration" (mechanism only — see §10)

## 1. Problem

The intended model is "persona = preset for an agent execution", but tool availability
today has **three divergent sources of truth**, and none of them is the persona:

| # | Finding | Evidence |
|---|---------|----------|
| F1 | The runtime path (web/CLI/Telegram chat) registers only 9–13 tools per agent (8 always-on + exec + browse suite). The 30–35-tool flood lives on the `KernelToolProvider` bridge path, which is **dead code in production** — `OxiosKernelBridge::new` has no caller outside its own test. | `agent_runtime.rs:949` (runtime uses `register_tools_from_cspace_gated` only); `tools/kernel_bridge.rs:42-104`; workspace grep for `OxiosKernelBridge::new` |
| F2 | The system prompt advertises tools that are not registered. `memory_write`/`memory_read`/`knowledge`/kernel tools are listed in the prompt, but the gated registration path no-ops the `"memory"` arm and never registers knowledge or `ask_user` (bridge-path only). Agents attempt calls that cannot succeed. | `agent_runtime.rs:1788-1791` vs `tools/registration.rs:341` (`"memory" => {}`), `builtin/mod.rs:89-91` (memory only on bridge path), `token_maxing/maxer.rs:16` (documents ask_user bridge-only) |
| F3 | Persona `role` → CSpace template resolution never fires for default personas: template names are `worker/standard/operator/supervisor`, default persona roles are `developer/qa/researcher/…` — zero overlap, everything falls back to `worker` with a warning. The operator/supervisor tiers are unreachable from any persona. | `capability/resolve.rs:21-24,48-77`; `persona/mod.rs:109-421` |
| F4 | Pooled session agents freeze their prompt and tool set at build time; switching the active persona does not evict them, so subsequent turns can run with the previous persona's context. | `supervisor.rs` (AgentPool keyed by AgentId); persona prompt injected at build (`agent_runtime.rs:369-401`) |

External context: tool definitions are the dominant fixed per-turn cost (≈55K tokens
for 58 tool definitions) and tool-selection accuracy degrades measurably beyond
~20 tools. Persona scoping simultaneously fixes correctness (F2), reachability
(F3), and keeps every persona inside the 10–20-tool "safe zone" once tools are
properly supplied.

## 2. Goals / Non-goals

**Goals**

1. One source of truth for "which tools does this agent get": persona-declared
   tool scope, compiled to the existing capability currency (CSpace).
2. Persona completes as a four-axis profile: `system_prompt` + `role` (model
   routing) + `tool_scope` (tools) + `capabilities` (UI affordances).
3. Single tool-registration path; dead bridge deleted; prompt advertisement
   generated from the actual registry.
4. Delegation (A2A/subagent) carries the task's domain profile instead of
   inheriting the global active persona.

**Non-goals**

- No dedicated per-persona UI mode or route (RFC-044 §8.4 stance stands:
  capability packs on one shared chat substrate).
- No privileged "controller agent" / privilege-brokering broker. A2A stays a
  work-decomposition mechanism; escalation remains the job of approval gates
  and RBAC.
- Tool scope is **not** a security boundary. It narrows what the model sees;
  `AccessGate` (CSpace → RBAC → Permissions → ExecConfig) remains the
  enforcement stack, unchanged.

## 3. D1 — Capability-native tool packs

Packs are named **CSpace fragments**, not tool-name lists. A persona's scope
compiles to a CSpace; the registration match table stays the only
domain→tool mapping.

```rust
// crates/oxios-kernel/src/capability/packs.rs (new)
pub struct ToolPack {
    pub name: &'static str,                     // "coding", "writing", ...
    pub grants: &'static [(ResourceRefSpec, Rights)],
    pub description: &'static str,
}
```

```rust
// persona/mod.rs — Persona gains:
#[serde(default)]
pub tool_scope: Option<ToolScope>,

pub struct ToolScope {
    pub packs: Vec<String>,      // pack names, unioned
    pub exclude: Vec<String>,    // tool names, removed post-registration
}
```

**Compile semantics** (`capability::packs::compile(&ToolScope) -> CSpace`):

1. Union the grants of all named packs → capability set.
2. Unknown pack name → hard error at API/validation time, warn+ignore at
   legacy-load time (defensive).
3. `exclude` is **not** part of compilation — it filters registered tool
   names at the agent-build chokepoint (§4). Grants are capability-shaped,
   exclusions are tool-shaped; this mirrors Claude Code's
   `tools`/`disallowedTools` split and is what users edit.

**Resolution precedence** (`resolve_cspace`, `agent_runtime.rs:376-388`):

```
cspace_hint > persona.tool_scope (compiled) > recognized role name > worker
```

`tool_scope: None` preserves today's behavior exactly (schema-compat, §8).

**Why fragments beat name lists:** a new kernel tool added under an existing
domain automatically reaches every pack granting that domain. Name lists would
require editing every pack on each tool addition — a second mapping table to
rot. Rights granularity (memory `R` vs `R|W`) comes free.

### 3.1 Initial pack catalog

| Pack | Grants (`ResourceRef`, `Rights`) | Resulting tools |
|------|----------------------------------|-----------------|
| `core-fs` | `Fs` `R\|W`, `WebSearch` `X` | read, write, edit, grep, find, ls, web_search, get_search_results |
| `coding` | `core-fs` + `Exec{shell}` `X\|R`, `Browser` `X`, `KernelDomain{"memory"}` `R\|W`, `KernelDomain{"knowledge"}`, `KernelDomain{"ask_user"}` `X` | 8 core + exec + browse×4 + memory×3 + knowledge + ask_user (+ SDK `subagent`) |
| `writing` | `core-fs` + `memory` `R\|W`, `knowledge`, `ask_user` `X` | 8 core + memory×3 + knowledge + ask_user |
| `research` | `core-fs` + `Browser` `X`, `memory` `R`, `knowledge`, `ask_user` `X` | 8 core + browse×4 + memory×2 (read, search) + knowledge + ask_user |
| `advisory` | `core-fs` + `memory` `R`, `knowledge`, `ask_user` `X` | 8 core + memory×2 (read, search) + knowledge + ask_user |
| `ops` | `core-fs` + `Exec{shell}` `X\|R`, `memory` `R\|W`, `knowledge`, `ask_user` `X` + `KernelDomain{cron}` `R\|W\|X`, `{resource}` `R\|W`, `{budget}` `R\|W`, `{security}` `R`, `{space}` `R\|W`, `{persona}` `R\|W`, `{agent}` `R\|W`, `{a2a}` `R\|W\|X`, `{mcp "*"}` `R\|X`, `{program}` `R\|W\|X` | coding-scale surface + all kernel-control tools |
| `kernel-control` | the `KernelDomain` set above only | project, kernel_agent, persona, cron, security, budget, resource, a2a×3, mcp |

Default persona assignment: `dev`/`review` → `coding`; `research` → `research`;
`writer` → `writing`; `architect`/`mentor`/`planner` → `advisory`;
`ops`/`security` → `ops`. Custom personas default to `None` (legacy behavior).
Per-persona `exclude` trims further (e.g. a novelist persona excluding
`grep`/`find`).

The SDK-registered `subagent` tool (zero-tool in-process reasoning hatch,
`agent_runtime.rs:1009`) stays universally present in v1 — it is the universal
decomposition escape hatch, carries no tools itself, and is out of scope for
exclusion.

## 4. D2 — Single registration path

1. **Delete the bridge.** Remove `OxiosKernelBridge` and its
   `KernelToolProvider` impl; dissolve `builtin::register_all_kernel_tools`
   into the one gated match table. `tool_names()` (30+5) is replaced by the
   `TOOL_CATALOG` static as the authoritative display inventory.
2. **Always-on tier → CSpace.** Introduce two `ResourceRef` variants —
   `Fs` and `WebSearch` (the enum currently has neither; `capability/types.rs:174-209`).
   Map them in the registration table; grant them in `worker()` and every
   default pack (`core-fs`). Remove the Layer-0 skip list
   (`gate.rs:333-342`) whose comment self-describes as a catch-22 workaround.
   Update RBAC `User` default policy (`rbac.rs:28-67`) to include
   `web_search`/`get_search_results` (currently fs tools only) so the default
   role is not silently narrowed.
3. **Fix the no-op arms, make them rights-aware.** `memory`, `knowledge`, and
   `ask_user` register from CSpace grants like every other domain. The `"mcp"`
   gated arm registers `McpToolWrapper` (currently bridge-only). Tools within
   a domain gate on the specific right they need (`memory_write` requires
   `WRITE`; `memory_read`/`memory_search` require `READ`), so a `READ`-only
   grant registers the read tools only — which is how the `research` and
   `advisory` packs end up with memory×2 instead of memory×3.
4. **Prompt from registry.** The hardcoded "Your tools" paragraph
   (`agent_runtime.rs:1788-1791`) is generated from the final
   `registry.names()` with one-liners from `TOOL_CATALOG` metadata.
   Advertisement/registration mismatch (F2) becomes structurally impossible.

**Token-maxing note:** `token_maxing/maxer.rs:16` currently relies on the
accident that ask_user is bridge-only. After D2 the maxer's task scope simply
omits `ask_user` (no pack granting it) — the invariant becomes explicit.

## 5. D3 — Persona as a four-axis profile

```
Persona = system_prompt   (who)      — unchanged
        + role            (model)    — model routing only (agent_runtime.rs:507-524)
        + tool_scope      (tools)    — new, compiles to CSpace
        + capabilities    (UI)       — RFC-044 §8.2, unchanged
```

`role` loses its (broken) CSpace duty. Model routing via
`engine.role_routing[role]` is untouched.

## 6. D4 — Delegation carries the domain profile

- `ExecEnv` gains `persona: Option<String>`; `AgentRuntime` consults
  `env.persona` before the global active persona (prompt, tool scope,
  capabilities resolution).
- A2A `TaskSpec`/`TaskDelegation` gains an optional `persona` field; the
  delegation handler (`src/kernel.rs:1747`, which currently discards
  `_from`/`_to`) threads it into `ExecEnv`. Delegating a coding task runs the
  sub-agent under the `coding` profile regardless of the global slot.
- **No privilege brokering:** a delegated agent runs with its own persona's
  scope and its own `AccessGate` context. Approval gates still fire. A domain
  agent cannot self-escalate by delegating to `ops` unless the deployment
  whitelists it — v1 ships no such whitelist; the user switches personas.

## 7. Agent pool invalidation (fixes F4)

`PersonaManager::set_active` already emits `PersonaUpdated` on the event bus.
The supervisor subscribes and **evicts pooled agents** whose build-time
persona differs from the new active persona (or: pool key becomes
`(AgentId, persona_id)`). Next turn rebuilds under the new scope — one-shot
rebuild cost, no steady-state overhead.

## 8. Persistence & API

- `personas/index.json` schema **v3**: personas gain `tool_scope`
  (`#[serde(default)]`). v2 files load as `None` (legacy behavior); no
  migration step needed. `persistence.rs` `SCHEMA_VERSION = 3`.
- `GET /api/personas` summaries include `tool_scope`;
  `PUT /api/personas/:id` accepts partial tool_scope updates. Validation:
  unknown pack → `400` with the pack catalog; unknown exclude name → `400`
  with `TOOL_CATALOG` names.
- New `GET /api/tool-packs` → pack catalog + tool display inventory
  (feeds the editor UI and the header chip).
- Remote RPC `persona.list` includes resolved pack names.

## 9. Web UI

Per the product direction: **no dedicated mode UI.**

- `/personas` edit dialog: "Tool scope" section — pack multi-select, exclude
  chips with autocomplete from the catalog.
- Chat header: scoped-tools chip (e.g. "17 tools", tooltip with the list) —
  makes the persona's effect visible without a mode switch.
- Existing capability affordances (diff-viewer, fan-out, terminal toggle)
  unchanged.

## 10. Supersessions & compatibility

- 2026-07-11 memory overhaul decided *unconditional* memory registration
  (implemented on the bridge path — which the runtime never calls, hence F2).
  This design supersedes the **mechanism**: memory is granted by every default
  pack instead of registered unconditionally. The spirit (default agents keep
  memory; brain recall stays system-level at `agent_runtime.rs:453`) is
  preserved; persona-level exclusion becomes an explicit user choice.
- RFC-044 §8.2 anticipated `allowed_tools` as part of the persona definition;
  this design delivers it capability-natively (packs → CSpace) rather than as
  a parallel name-allowlist mechanism.

## 11. Security considerations

- Scope narrows the **model-visible** surface (fewer tools to misuse under
  prompt injection, fewer tokens) but is not a sandbox. Enforcement stack
  unchanged: CSpace (Layer 0) → RBAC (Layer 1) → Permissions (Layer 2) →
  ExecConfig (Layer 3) → approval gates.
- A persona requesting `kernel-control` still hits per-action approvals for
  dangerous operations; nothing in this design grants rights the caller lacks.
- Delegation threading (D4) adds no authority transfer — the sub-agent's own
  context governs.

## 12. Resource impact

- **Daemon RAM:** neutral-to-slightly-down. CSpace/registry/prompt are
  per-execution objects that already exist; pack compile is a static-table
  union (microseconds). Tool instances share `Arc<KernelHandle>` state. Dead
  bridge code removed.
- **Token budget:** improved — persona tool sets land at 10–18 definitions,
  inside the measured safe zone, versus 35 if tools were properly supplied
  unscoped. Advertisement text now matches reality.
- **Persona switch:** one-shot agent rebuild (transient duplicate allocation).

## 13. Testing

- **Unit:** pack compile → exact capability set; multi-pack union; unknown
  pack error; exclude filtering; precedence (`hint > scope > role > worker`).
- **Golden counts:** per default persona, `registry.names()` equals the
  expected set (extend the pattern of
  `register_always_on_registers_eight_tools`, `registration.rs:367-420`).
- **Consistency invariant:** generated prompt tool section ≡ `registry.names()`.
- **Persistence:** v2 file loads with `None` scope; v3 roundtrip.
- **Regression:** token-maxing scope excludes `ask_user`; RBAC `User` can
  call web tools after policy update; empty-CSpace agent registers zero
  tools (today's 8 → 0 after skip-list removal is covered by pack grants).

## 14. Phasing

1. **Kernel (correctness core):** packs + compile + resolution wiring + D2
   unification + prompt-from-registry + defaults + schema v3 + API + pool
   eviction + tests.
2. **Web:** tool-scope editor section, catalog endpoint, header chip.
3. **Delegation:** `ExecEnv.persona` threading through the A2A handler and
   subagent runner.
