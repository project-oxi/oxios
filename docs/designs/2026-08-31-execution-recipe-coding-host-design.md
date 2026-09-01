# Execution Recipes and the OMP-Compatible Coding Host

**Status:** Proposed  
**Date:** 2026-08-31  
**Owners:** Oxios maintainers  
**Depends on:** a released \`oxicode-sdk\` behavior-pack API (see the paired
[Oxicode design](../../../oxicode/docs/designs/2026-08-31-omp-compatible-behavior-pack-design.md))

## Summary

Oxios will consume portable Oxicode behavior packs through a single,
policy-preserving **execution recipe** resolution path. The first recipe,
\`coding-omp-v1\`, makes a Web, CLI, Telegram, scheduled, or API request use
the same underlying coding behavior without making any channel responsible for
tool composition.

This is not a separate coding harness or a second \`AgentRuntime\`. Oxios keeps
one general runtime and selects its behavior declaratively:

\`\`\`text
channel message / scheduled job
  → Ouroboros assessment + persona + project context
  → ExecutionRecipeResolver
  → ResolvedExecution
  → RuntimeExtensionManager (when requested)
  → SDK behavior-pack installer
  → existing Oxios policy wrappers and registry
  → existing AgentRuntime → KernelEvent → channel UI
\`\`\`

The recipe decides **what behavior is requested**. Oxios policy decides
**whether and how it may execute**. The pack never bypasses
\`AccessManager\`, CSpace gating, approval, project boundaries, quotas, or the
existing event stream.

## Context

Oxios already delegates the model tool-calling loop to \`oxicode-sdk\`, but its
current \`AgentRuntime\` creates a fresh agent/tool registry per directive and
registers a CSpace-selected Oxios tool set. This is intentional for a general
agent operating system, yet it means an Oxios coding turn is not automatically
the same composition as an Oxicode CLI coding turn.

Several important differences exist today:

- the production path is \`register_tools_from_cspace_gated\`; the
  \`register_from_resolved_profile\` helper has no production caller;
- \`AgentRuntime\` does not currently attach a hashline snapshot store, LSP
  provider, URL resolver, or TTSR engine when it constructs \`AgentConfig\`;
- the standard Oxios \`exec\` tool has host security semantics and is not the
  SDK Bash tool;
- the current Oxios subagent runner creates a text-only child with no tools;
- multiple products and channels are already expected to share the same kernel,
  while their user interfaces remain intentionally different.

The answer is not to make Web UI emulate a terminal. Coding quality follows the
behavioral composition—tools, state, retries, extension lifetimes, and
delegation—not the renderer.

The corresponding portable behavior contract is defined in
\`/Volumes/MERCURY/PROJECTS/oxicode/docs/designs/2026-08-31-omp-compatible-behavior-pack-design.md\`.
Oxios consumes released \`oxicode-sdk\` packages from crates.io only; it never
takes a path dependency on the adjacent Oxicode checkout.

## Goals

1. Let any Oxios channel select the same \`coding-omp-v1\` behavior for the
   same project/session context.
2. Preserve the existing Kernel, Supervisor, AgentLifecycleManager,
   AccessManager, CSpace, and event transport responsibilities.
3. Make it mechanically difficult for a pack-installed tool to bypass
   Oxios authorization, audit, approval, and structured result delivery.
4. Create coding state only for sessions that request a coding recipe and
   clean it up through the normal lifecycle.
5. Surface installed/degraded behavior to the Web UI and other channels without
   inventing a second event bus.
6. Keep general-purpose personas free to compose other packs and overlays.
## Non-goals

- Replacing the monolithic \`oxios-kernel\` with new crates or a parallel
  agent-runtime architecture.
- Giving a behavior pack ownership of the filesystem sandbox, RBAC, approval,
  secrets, scheduling, or user/project persistence.
- Making every conversation a coding session.
- Replacing existing issue ownership, Brain memory, project storage, or
  channel-specific rendering.
- Directly launching or embedding the OMP binary.
- Promising all OMP features before the Oxicode compatibility ledger marks them
  equivalent.

## Design decision

Introduce \`ExecutionRecipe\` as an Oxios-owned declaration that selects
portable behavior packs and Oxios-specific overlays. Resolve it once before
each turn, use it to prepare optional runtime extensions, then install the
selected SDK packs through a policy-wrapping installer.

The core types are conceptually:

\`\`\`rust
pub struct ExecutionRecipe {
    pub id: RecipeId,                    // e.g. "coding-omp-v1"
    pub base_packs: Vec<BehaviorPackId>,
    pub overlays: Vec<RecipeOverlayId>,
    pub required_capabilities: CapabilityRequirements,
    pub extension_policy: ExtensionPolicy,
    pub prompt_policy: PromptPolicy,
}

pub struct ResolvedExecution {
    pub recipe: ExecutionRecipe,
    pub packs: Vec<ResolvedBehavior>,
    pub cspace: ResolvedCapabilitySpace,
    pub policy: Arc<OxiosPolicyEnvelope>,
    pub workspace: WorkspaceBinding,
    pub degradation: Vec<Degradation>,
}

pub struct ToolInstallContext {
    pub resolved: Arc<ResolvedExecution>,
    pub structured_results: Arc<StructuredResultBus>,
    pub turn: TurnIdentity,
}
\`\`\`

The exact Rust names may differ, but the ownership must not:

| Layer | Owns |
|---|---|
| Oxicode behavior pack | canonical tool semantics, extension requirements, prompt layers, compatibility status |
| Recipe resolver | selecting packs/overlays from persona, request, project, and scheduling context |
| Oxios policy envelope | AccessManager checks, CSpace, path rules, RBAC, approvals, quota/timeout limits, audit context |
| Runtime extension manager | allocation/reuse/cleanup of host adapters and session-scoped state |
| AgentRuntime | one SDK agent loop with the final \`AgentConfig\` and final gated registry |
| Channels/Web UI | request submission and rendering of the same Kernel events |

A recipe may narrow the behavior requested by a persona or project. It never
widens the capability set already granted by policy. A missing required
capability produces a structured preflight failure; it is not silently replaced
with an unsafe tool.

## Recipe selection

\`ExecutionRecipeResolver\` belongs inside the current kernel composition, near
persona/profile and capability resolution—not in the Gateway or any channel.

The resolver combines four inputs in a fixed precedence order:

1. explicit, authorized request selection;
2. project recipe/default coding binding;
3. persona default;
4. system default \`general-v1\`.

A scheduled or automated run carries the same project/persona metadata and
therefore resolves the same recipe as an interactive Web request. A channel
must never send an arbitrary recipe identifier directly to a tool registry.

The resolver emits a fully materialized \`ResolvedExecution\` before tool
registration. It includes the chosen SDK pack versions, disabled optional
features, policy-relevant descriptors, canonical workspace roots, and a
compatibility/degradation report. This is persisted as turn metadata so a
completed coding run can be diagnosed or reproduced.

Example policies:

| Request class | Resolved recipe | Result |
|---|---|---|
| General knowledge task | \`general-v1\` | normal Oxios profile; no coding extensions |
| Project coding request | \`coding-omp-v1\` | SDK coding pack plus Oxios gates and coding session extensions |
| Scheduled code review | \`coding-omp-v1 + git-review-v1\` | same tool behavior, non-interactive approval policy may narrow mutations |
| Coding persona outside a bound workspace | \`coding-omp-v1\` preflight failure | no inferred broad filesystem access |

## One registration path

Oxios must not add a second, parallel tool registry for packs. The existing
registry/CSpace path is the sole production path and must be refactored to
accept canonical SDK tools from a \`BehaviorToolInstaller\`.

The target sequence is:

\`\`\`text
SDK pack emits (descriptor, canonical tool)
  → Oxios validates descriptor against ResolvedExecution + CSpace
  → Oxios wraps canonical tool in GatedTool / audit / approval adapter
  → Oxios installs the wrapped tool in the current registry
  → AgentRuntime attaches that registry to one SDK Agent
\`\`\`

This replaces the current manual list only at the composition seam. It does not
replace \`ToolDescriptor\` catalog ownership or duplicate KernelEvent delivery.

### Descriptor reconciliation

Oxicode's \`BehaviorToolDescriptor\` describes portable semantics. Oxios's
\`capability::descriptor::ToolDescriptor\` describes product capability and UI
metadata. Neither replaces the other.

Add an explicit mapping keyed by stable SDK implementation identity. Resolution
must fail in development/test configurations when a required behavior tool has
no Oxios mapping, has a mismatched exposed name, or declares a side-effect
class inconsistent with its Oxios descriptor. This prevents the current
failure mode where a tool is registered but inevitably denied because its
Layer-0 exemption, catalog descriptor, and registration tier drift apart.

The always-on tool invariant remains: a tool registered through
\`register_always_on\` must also be treated consistently by
\`LAYER0_EXEMPT_TOOLS\`. The new installer must extend the existing test rather
than create a competing list.

### Native Oxios tools and canonical SDK tools

Some Oxios tools remain product-native: Brain, projects, calendar, schedules,
A2A, browser lifecycle, and other kernel APIs. They are recipe overlays or
always-on services, not substitutions for a pack's canonical coding tool.

Where \`coding-omp-v1\` requires a process or file operation, the selected
implementation must preserve the pack behavior while being wrapped by Oxios.
For example, it may use an SDK persistent-shell implementation under an Oxios
\`exec\` policy adapter; it must not expose an ungated SDK Bash tool next to the
existing secure \`exec\` tool under the same model-visible name.

## Runtime extensions and session lifecycle

Add a general \`RuntimeExtensionManager\` to the kernel. A coding session is
its first consumer, not a special lifecycle subsystem.

\`\`\`text
RuntimeExtensionManager
  ├── HashlineExtension
  ├── LspExtension
  ├── ShellExtension
  ├── EvalExtension
  ├── DebugExtension
  ├── TtsrExtension
  └── DelegationExtension
\`\`\`

It receives the \`ResolvedExecution\` and creates only requested extensions.
Its session key is a stable tuple:

\`\`\`text
(project binding, stable chat session id, canonical workspace roots, worktree identity)
\`\`\`

The current turn key must remain the existing shared \`session_id\` (or the
first-request fallback) used by \`StreamingSinkRegistry\`, \`TurnRegistry\`,
and \`ExecEnv\`. A separate per-request key would break persistence and tool
ownership. The extension key may include the canonical workspace/worktree to
prevent a session from retaining shell/LSP state after an authorized workspace
switch.

Extension lifetimes:

| Extension | Create | Retain | Tear down |
|---|---|---|---|
| Hashline | coding session start | stable session/worktree | session end, workspace change, explicit reset |
| LSP | first LSP-capable turn | workspace/worktree | idle lease expiry, workspace change, lifecycle kill |
| Shell/Eval | first corresponding tool use | coding session/worktree | explicit reset, cancellation policy, session end |
| Debug | explicit debug request | debug target | target termination, session end, lifecycle kill |
| TTSR | turn creation | one turn | turn completion |
| Delegation | child lifecycle request | child only | child completion/cancel/kill |

Every extension must support cancellation and bounded cleanup. Supervisor and
\`AgentLifecycleManager\` remain responsible for lifecycle authority; the
extension manager owns only resources allocated on behalf of a resolved
execution.

The existing \`SubagentRunner\` must not be treated as coding-equivalent while
it creates text-only children. For a coding recipe, delegation is routed through
\`AgentLifecycleManager\` so child agents inherit an explicitly narrowed
recipe, policy envelope, workspace binding, cancellation chain, and tool
registration. The text-only runner can remain an intentional general-task
fallback, but it cannot satisfy the coding pack's delegation requirement.

## Agent configuration

\`AgentRuntime\` continues to construct one \`oxicode_sdk::Agent\` per turn.
The difference is that it receives a validated configuration contribution from
the resolved pack and extensions:

\`\`\`text
current AgentConfig defaults
  + recipe prompt layers
  + HashlineExtension snapshot store
  + LspExtension provider
  + TtsrExtension engine
  + approved URL/session/delegation ports
  + current model, workspace, timeout, session id, todo, and budgets
  = final AgentConfig
\`\`\`

All product-defined limits are applied after pack requirements are interpreted.
For example, a pack can require persistent shell support, but it cannot change
Oxios's maximum runtime or make shell mode enabled where the profile/security
policy denies it.

Agent lifecycle snapshots and hashline snapshots remain distinct SDK services;
their fields and telemetry must use distinct names.

## Prompt layering

Packs can contribute canonical prompt layers, but Oxios retains the final prompt
assembly order and its existing persona/system/security instructions. The
required order is:

1. immutable Oxios security and execution-policy instructions;
2. resolved persona/project instructions;
3. behavior-pack prompt layers;
4. request-specific context.

A pack cannot add language that overrides security instructions or claims tools
that preflight marked unavailable. The resolved prompt-layer identifiers are
recorded in turn metadata, while prompt content continues to follow existing
privacy/logging policy.

## Events, UI, and automation

The Web UI is not a special execution path. It renders the same events that
CLI, Telegram, and automation receive.

No new event transport is introduced. Pack installation and extension lifecycle
events are represented through existing Kernel events and tool execution
events. Tools that need structured UI payloads continue to use
\`StructuredResultBus\` keyed by \`tool_call_id\`, which flows through
\`KernelEvent::ToolExecutionFinished.results\` to WebSocket/SSE consumers.

Recommended additional structured event payloads are:

- \`execution_recipe_resolved\`: recipe, pack versions, compatible/degraded
  capabilities, workspace identity;
- \`runtime_extension_status\`: started, reused, stopped, failed, reset;
- \`behavior_tool_manifest\`: model-visible names and implementation ids,
  excluding secrets;
- \`policy_decision\`: allow/audit/approval/deny classification, with existing
  audit redaction rules.

The Web UI may show a compact “Coding environment” panel, but it must be
derived from these events. It must not own session state or construct tools in
the browser.

Automation uses the same resolver. An unattended schedule can set an approval
policy that denies or queues mutations, yet it still receives the same coding
pack and trace quality for allowed work.

## Security invariants

The following must be covered by tests and code review invariants:

1. No SDK tool from a behavior pack reaches an \`Agent\` without an Oxios gate
   wrapper, except explicitly documented read-only primitives that still pass
   the Layer-0 rule.
2. A recipe cannot expand CSpace or bypass \`AccessManager\`.
3. Workspace, worktree, and project bindings are canonicalized before extension
   allocation or path authorization.
4. Child recipes may only retain or narrow parent authority.
5. A denied/approval-required pack capability is visible to the model and UI as
   structured degradation, not silently replaced.
6. Secrets, tokens, and private command output do not enter manifests or
   structured events.
7. \`kill\` and cancellation release child/extension resources even when a
   model turn is interrupted.

## Testing strategy

Tests are layered so quality is asserted at the correct boundary.

| Layer | Proves |
|---|---|
| Oxicode pack fixtures | canonical behavior, state semantics, and OMP compatibility claims |
| Oxios installer tests | every canonical tool is mapped, gated, and registered through the sole path |
| Oxios extension tests | session reuse/reset/cancellation/workspace change behavior |
| Kernel integration tests | resolved recipe reaches one AgentRuntime and normal Kernel events |
| Channel tests | Web/CLI/Telegram render the same structured state; no channel composes tools |
| Security tests | path escape, denied process/network, approval, child narrowing, cleanup |

The Oxios integration fixture runner uses scripted models and temporary project
workspaces. It must assert final diffs, installed manifests, policy trace, and
event flow; it must not assert terminal formatting.

A compatibility status in an Oxios UI is read from the SDK pack manifest. Oxios
cannot turn an SDK \`Partial\` claim into \`Equivalent\` by passing a local
smoke test.

## Migration and rollout

1. **SDK prerequisite:** wait for the released Oxicode behavior-pack API and
   \`coding-omp-v1\` manifest. Do not introduce a local clone of the API.
2. **Resolver:** add execution-recipe resolution alongside existing persona and
   capability resolution, initially with \`general-v1\` only.
3. **Installer seam:** refactor the current CSpace registration path so both
   existing native tools and pack-provided canonical tools flow through one
   gated installer.
4. **Extension manager:** introduce hashline/LSP/TTSR adapters first, with
   explicit degradation reporting until the SDK supplies persistent shell,
   eval, DAP, and typed delegation behavior.
5. **Coding recipe pilot:** enable \`coding-omp-v1\` for explicitly selected
   project sessions behind a feature flag. Preserve the current profile as a
   rollback recipe.
6. **Lifecycle delegation:** move coding child execution onto
   \`AgentLifecycleManager\`; retain the text-only runner only for recipes that
   declare it sufficient.
7. **Channel presentation:** render existing structured events in Web first,
   then ensure CLI/Telegram/API observe the same contract.
8. **Defaulting:** make a persona/project select the coding recipe by default
   only after pack fixtures and Oxios security/integration gates pass.

This ordering avoids the false milestone of displaying more coding buttons
before the behavior and policy path are actually equivalent.

## Alternatives considered

### A dedicated Oxios coding harness

Rejected. A second loop would duplicate \`AgentRuntime\`, scheduling,
streaming, lifecycle, and policy behavior. The correct separation is a
general runtime plus a selected execution recipe.

### Free-form LangChain-style graphs controlled entirely by Oxios

Rejected as the default composition model. Free composition is valuable for
future general-agent recipes, but reassembling every coding primitive in each
graph would reproduce tool and state drift. Recipes may compose packs and
overlays; the OMP-compatible coding pack remains opinionated and versioned.

### Install the SDK's full builtin registry directly

Rejected. It bypasses or complicates Oxios's CSpace/AccessManager gate,
duplicates names with native tools, and leaves no reliable proof of descriptor
coverage.

### Reimplement OMP features within Oxios

Rejected. Canonical coding behavior belongs upstream in Oxicode. Oxios should
host it safely, not fork it.

### Treat Web UI terminal emulation as the solution

Rejected. Terminal access can be useful, but it does not establish hashline,
LSP, persistent execution, retries, or tool-capable delegation. Those are
runtime properties addressed by this design.

## Acceptance criteria

The design is ready to implement when an explicit coding request can produce a
\`ResolvedExecution\` that:

- selects a released \`coding-omp-v1\` pack and records its version/status;
- installs every required canonical tool through an Oxios policy wrapper;
- attaches only the extensions declared by the pack and permitted by policy;
- runs through the existing \`AgentRuntime\`, \`KernelEvent\`, and
  \`StructuredResultBus\` paths;
- routes coding subagents through lifecycle-managed, narrowed child execution;
- reports unavailable pack features honestly without silently changing the
  behavior contract;
- leaves ordinary non-coding work on the same general runtime without paying
  for coding extensions.

Implementation planning begins only after review of this design and its paired
Oxicode contract.
