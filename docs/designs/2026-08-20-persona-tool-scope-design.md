# Persona Execution Profiles — Design

**Date:** 2026-08-20
**Status:** Draft — rewritten after architecture review
**Depends on:** RFC-039 (persona completion), RFC-044 §8 (persona capability packs)
**Supersedes in part:** capability-template-based tool selection and the 2026-07-11 memory registration mechanism

## 1. Problem

Oxios needs domain-focused agents for coding, code review, research, writing,
security analysis, and operations. Each agent should see a small, coherent tool
surface without creating a separate application mode for every domain.

The current architecture has several independent selectors for prompt identity,
model routing, capability templates, tool registration, permissions, and UI
affordances. They can disagree. A persona may advertise tools that are not
registered, a registered tool may be permanently denied by a later access
layer, and a global persona switch may not affect an already pooled agent.

The previous draft proposed compiling editable persona tool packs directly into
an agent CSpace. That would conflate two distinct concepts:

- **Tool exposure:** what the model should see for this task.
- **Authority:** what the principal and agent are allowed to do.

A CSpace is authority: it contains kernel- or agent-issued capabilities. A
persona is a user-facing preset and must never mint authority. This design
separates the two while retaining CSpace as the kernel's capability currency.

## 2. Goals and non-goals

### Goals

1. A persona is a composable execution preset, not a security principal.
2. Tool profiles declare requested tools and capabilities; they do not issue
   capabilities.
3. A single trusted resolver produces the effective CSpace, active tool set,
   prompt metadata, UI affordances, diagnostics, and a stable fingerprint.
4. Every delegated authority is an attenuation of its parent authority.
5. Tool registration, prompt advertisement, and UI affordances are projections
   of one resolved execution profile.
6. Domain profiles expose a small per-turn tool set through progressive
   disclosure rather than registering an entire privileged catalog at once.
7. Persona selection is session- or turn-scoped. Global state is a default for
   new sessions only.
8. Tool and profile identities are typed, namespaced, versioned, and safe for
   dynamic providers such as MCP.

### Non-goals

- No dedicated coding, writing, or operations route. RFC-044's shared chat
  substrate remains the product model.
- No privileged controller agent. Kernel policy, not an LLM, issues authority.
- No replacement of RBAC, Permissions, ExecConfig, or approval gates. The
  resolver composes with them.
- No automatic authority increase when a new tool is installed or a profile is
  updated.
- No requirement that every candidate tool be present in every model turn.

## 3. Architectural invariants

The implementation must preserve these invariants:

1. **Persona non-authority:** editing or selecting a persona cannot increase
   authority.
2. **Bounded issuance:** an effective CSpace is minted only by the kernel from a
   trusted authority context.
3. **Monotone delegation:** `child_authority ⊆ parent_authority` for local
   delegation. Remote A2A additionally intersects the delegating principal's
   permitted A2A delegation scope, the target's accepted delegation authority,
   and both deployments' trust policies.
4. **One resolution result:** registry, prompt, UI, access context, and pool
   identity consume the same `ResolvedExecutionProfile`.
5. **Immutable execution identity:** a running turn never re-reads the global
   active persona or mutable profile records.
6. **Explicit promotion:** installing a tool or publishing a new profile
   revision does not alter an existing pinned profile, except through an
   explicitly declared dynamic provider contract bounded by its capability
   ceiling and recorded catalog dependency digest.
7. **Fail closed:** invalid, unknown, or unavailable profile requirements never
   disappear silently.
8. **No hidden tools:** every model-callable tool has a catalog descriptor and
   appears in the resolved active set.
9. **No false affordances:** a UI affordance is shown only when its required
   effective tool capabilities are available.
10. **Explainability:** every requested tool resolves to active, available on
    demand, approval-required, or unavailable with a machine-readable reason.

## 4. Conceptual model

### 4.1 Persona

A persona composes independent profiles:

```rust
pub struct Persona {
    pub id: PersonaId,
    pub name: String,
    pub prompt_profile: PromptProfileRef,
    pub model_policy: ModelPolicyRef,
    pub tool_profile: ToolProfileRef,
    pub ui_profile: UiProfileRef,
    pub enabled: bool,
}
```

`role` is replaced by `model_policy`; a model-routing key is not an authority
role. Persona records contain references, not inline authority grants.

### 4.2 Tool profile

A tool profile is a versioned request specification:

```rust
pub struct ToolProfileSpec {
    pub id: ToolProfileId,
    pub revision: ProfileRevision,
    pub extends: Vec<ToolProfileRef>,
    pub capability_ceiling: Vec<CapabilityRequest>,
    pub include: Vec<ToolSelector>,
    pub exclude: Vec<ToolSelector>,
    pub activation: ToolActivationPolicy,
}

pub struct ToolProfileRef {
    pub id: ToolProfileId,
    pub revision: ProfileRevision,
}
```

Profiles pin exact revisions. Editing a profile creates a new revision. Existing
personas continue to reference their pinned revision until explicitly upgraded.
This prevents ambient authority and behavior changes.

Tool selectors are authoring syntax, not live queries by default. Publishing a
profile revision resolves tag/provider selectors to an immutable set of
namespaced tool IDs stored with that revision. A profile may opt into a dynamic
provider contract only by declaring the provider identity, accepted capability
ceiling, and compatibility policy explicitly. Dynamic binding still cannot
expand the bounding authority.

There is no raw `domains: Vec<String>` escape hatch. Long-tail tools use typed
capability requests or namespaced tool selectors.

### 4.3 Capability ceiling

Tool selection and authority requirements have one join rule. A profile's
`include` selectors choose tools. Each selected `ToolDescriptor` supplies the
exact capability requirements for that tool. `capability_ceiling` only narrows
the maximum resource scopes and rights those descriptor requirements may
request; a ceiling entry never issues a capability by itself.

Publishing a fixed profile revision fails when any selected tool requirement is
not representable within its capability ceiling. For an explicit dynamic
provider contract, a newly resolved tool that exceeds the declared ceiling is
classified unavailable and cannot enter the effective CSpace.

```rust
pub struct CapabilityRequest {
    pub resource: ResourceSelector,
    pub rights: Rights,
}

pub enum ResourceSelector {
    KernelDomain(KernelDomainId),
    Skill(SkillSelector),
    Space(SpaceSelector),
    Agent(AgentSelector),
    Exec(ExecMode),
    Browser(BrowserScope),
    A2a(A2aScope),
    Mcp(McpSelector),
    Fs(FsScope),
    WebSearch,
    Memory(MemorySelector),
    Knowledge(KnowledgeSelector),
}
```

Resource selectors compile to typed `ResourceRef` values only after policy
resolution. `ResourceRef` is extended with `Fs`, `WebSearch`, `Memory`, and
`Knowledge` variants. `KernelDomain` remains only for kernel resources that do
not have a more specific variant.

### 4.4 Tool descriptor and catalog

Every tool provider publishes self-describing descriptors:

```rust
pub struct ToolDescriptor {
    pub id: ToolId,
    pub provider: ToolProviderId,
    pub description: String,
    pub tags: Vec<ToolTag>,
    pub required_capabilities: Vec<CapabilityRequirement>,
    pub ui_affordances: Vec<UiAffordanceId>,
    pub activation_class: ActivationClass,
}
```

Tool IDs are namespaced and stable:

```text
kernel.fs.read
kernel.exec.shell
kernel.memory.search
kernel.persona.update
mcp.github.search_issues
skill.novel.outline
```

The runtime catalog is assembled from provider descriptors. There is no
separate hand-maintained `TOOL_CATALOG` name list. Built-in, feature-gated, MCP,
and skill-provided tools participate in the same catalog contract.

### 4.5 Authority context

Authority originates outside the persona:

```rust
pub struct AuthorityContext {
    pub principal: PrincipalId,
    pub bounding_cspace: CSpace,
    pub rbac_policy: RbacPolicy,
    pub permissions: AgentPermissions,
    pub exec_config: ExecConfig,
    pub delegation_parent: Option<DelegatedAuthority>,
}
```

The bounding CSpace is derived from authenticated principal/session policy. A
persona cannot modify it.

### 4.6 Resolved execution profile

The trusted resolver returns one immutable result:

```rust
pub struct ResolvedExecutionProfile {
    pub snapshot: ExecutionProfileSnapshot,
    pub effective_cspace: CSpace,
    pub candidate_tools: Vec<ResolvedTool>,
    pub active_tools: Vec<ResolvedTool>,
    pub ui_affordances: Vec<UiAffordanceId>,
    pub diagnostics: Vec<ResolutionDiagnostic>,
    pub fingerprint: ExecutionProfileFingerprint,
}
```

`ResolvedTool` records one of:

```text
Active
AvailableOnDemand
RequiresApproval
Unavailable(reason)
```

Only `Active` tools are registered into the current model turn. Tools marked
`AvailableOnDemand` can be activated without increasing authority.

## 5. Resolution pipeline

The resolver is the single policy composition point:

```text
Persona snapshot
  + ToolProfile revision
  + Runtime Tool Catalog
  + AuthorityContext
  + Deployment policy
  + Turn intent/explicit activation
          |
          v
ExecutionProfileResolver
          |
          v
ResolvedExecutionProfile
```

Resolution is deterministic for identical inputs.

### 5.1 Steps

1. Load the exact pinned persona and profile revisions.
2. Expand profile inheritance and reject cycles.
3. Resolve tool selectors against the runtime catalog.
4. Read capability requirements from the selected tool descriptors and verify
   that every requirement fits the profile's capability ceiling.
5. Form the requested capability set as the union of selected descriptor
   requirements constrained by that ceiling. Ceiling entries unused by a
   selected tool do not enter the requested set.
6. Intersect requested capabilities with the bounding authority.
7. Apply RBAC, AgentPermissions, ExecConfig, runtime feature availability, and
   deployment policy.
8. Classify each tool. `Active` and `AvailableOnDemand` require every descriptor
   capability to be present in the effective CSpace. A missing requirement is
   `RequiresApproval` only when it is satisfiable from the approving
   principal's authority and the complete delegation chain; otherwise the tool
   is `Unavailable`.
9. Select the per-turn active set according to activation policy.
10. Derive UI affordances from the requested UI profile intersected with
    effective tool availability.
11. Produce a fingerprint over every behavior-affecting input.

The resolver does not maintain a parallel tool-name allowlist. The descriptor
is the authoritative tool-to-capability mapping; the profile ceiling can only
narrow it. All access layers are consulted through resolver policy adapters and
rechecked by `AccessGate` at invocation time.

### 5.2 Authority equation

For a normal session:

```text
effective authority
  = requested capability set
  ∩ principal/session bounding CSpace
  ∩ RBAC policy
  ∩ AgentPermissions
  ∩ ExecConfig
  ∩ deployment policy
```

For local delegation:

```text
child effective authority
  = child requested capability set
  ∩ parent effective authority
  ∩ deployment delegation policy
```

For remote A2A:

```text
remote effective authority
  = delegated request
  ∩ delegating principal's permitted A2A delegation scope
  ∩ target principal's accepted delegation authority
  ∩ target deployment policy
  ∩ A2A trust policy
```

The caller-side delegation scope is principal policy, not persona state. A
strong target may perform stronger work only when the authenticated delegating
principal was explicitly authorized to request that remote authority. A denied
capability is never converted into a kernel-issued capability merely because a
profile or remote task requested it.

### 5.3 Approval semantics

Approval is an explicit resolver decision, not a failed invocation accident.
`RequiresApproval` tools are described in the resolved profile but are not
registered as callable until approval succeeds. A single cataloged
`kernel.approval.request` primitive may request activation of a specific
candidate tool; it cannot issue authority.

Approved capabilities are minted from the approving principal's bounding
CSpace. Inside a delegated context they are additionally intersected with the
entire delegation chain. If the principal intends to broaden a parent
authority, approval occurs at that parent/session level and the child is
re-resolved; a child never becomes stronger than its current parent.

Approval creates a new snapshot with a bounded, attributable, time-limited
capability. Its capability ID, issuer, scope, and expiration are fingerprinted.
`AccessGate` checks expiration when the capability is presented, and expiration
invalidates any pooled runtime containing that capability. Snapshot immutability
therefore never bypasses live capability validity. Activation by itself does
not change authority. Permanently denied tools remain unavailable and are not
advertised as callable.

## 6. Tool profile vocabulary

Profiles are composed from narrow primitives rather than one broad `core-fs`
pack:

| Primitive | Requested surface |
|---|---|
| `fs-read` | read, grep, find, list |
| `fs-write` | write, edit |
| `web-search` | search, search-results |
| `browser-read` | navigate, extract, screenshot |
| `browser-interact` | click, type, upload, authenticated mutation |
| `system-observe` | bounded, structured inspection tools; never arbitrary shell |
| `shell-exec` | arbitrary shell and structured execution |
| `memory-read` | recall and search within authorized namespaces |
| `memory-write` | retain/update within authorized namespaces |
| `knowledge-read` | knowledge search/read |
| `knowledge-write` | knowledge mutation |
| `user-interaction` | ask-user/approval prompts |
| `delegation` | local subagent/A2A request primitives |
| `security-scan` | explicitly selected built-in, MCP, or skill scanner tools |
| `kernel-observe` | status, budget, resources, audit views |
| `kernel-mutate(scope)` | mutation of exact typed kernel resources and rights |

Domain presets compose primitives:

| Profile | Composition |
|---|---|
| `coding` | fs-read + fs-write + web-search + shell-exec + browser-read + memory-read/write + knowledge-read + user-interaction + delegation |
| `code-review` | fs-read + web-search + browser-read + memory-read + knowledge-read + user-interaction |
| `writing` | fs-read/write + web-search + memory-read/write + knowledge-read/write + user-interaction |
| `research` | fs-read + web-search + browser-read + memory-read + knowledge-read + user-interaction |
| `advisory` | fs-read + memory-read + knowledge-read + user-interaction |
| `security-audit` | fs-read + system-observe + web-search + security-scan + memory-read + knowledge-read + user-interaction |
| `operations` | fs-read/write + shell-exec + kernel-observe + kernel-mutate(cron, project, agent) + memory-read/write + user-interaction |

Default personas reference the matching exact profile revision:

```text
dev       → coding
review    → code-review
research  → research
writer    → writing
architect → advisory
mentor    → advisory
planner   → advisory
security  → security-audit
ops       → operations
```

Security analysis is not an operations profile. Review is not a write-enabled
coding profile.

New custom personas must select a tool profile explicitly. The API may suggest
`advisory`, but it never silently assigns an exec-bearing legacy default.

Legacy `worker/standard/operator/supervisor` capability templates are migrated
to named ToolProfile revisions and then removed. `cspace_hint` becomes a trusted
internal `tool_profile_override`; untrusted callers cannot use it to increase
authority.

## 7. Progressive tool disclosure

A profile defines the authorized candidate universe, not the set shown on every
turn. Large profiles such as operations use progressive disclosure.

Default activation policy:

- Keep the active model-visible set at or below 16 tools when possible.
- Always include tools explicitly required by the current directive.
- Keep recently used tools sticky for the session while relevant.
- Put remaining authorized candidates in `AvailableOnDemand`.
- Never hide a user-explicitly requested tool solely to satisfy the numeric
  target.

A namespaced platform primitive such as `kernel.tools.search` can search and
activate tools from `AvailableOnDemand`. Activation changes exposure only:

```text
authority before activation == authority after activation
```

`subagent` is not a universal hard-coded exception. Local delegation is a
cataloged primitive included by profiles that request the `delegation`
capability. Profiles may omit it.

The 16-tool value is an operational default, not a security invariant. Golden
tests verify profile membership and resolver classification; they do not freeze
a misleading global tool count.

## 8. Session and turn identity

Global persona state is a user preference used only when creating a session.
Execution never consults it after ingress.

Precedence at session ingress:

```text
turn override > session persona > user default persona > product default
```

The selected mutable records are immediately converted into an immutable
snapshot:

```rust
pub struct ExecutionProfileSnapshot {
    pub persona_id: PersonaId,
    pub persona_revision: ProfileRevision,
    pub tool_profile: ToolProfileRef,
    pub model_policy_revision: ProfileRevision,
    pub ui_profile_revision: ProfileRevision,
    pub authority_fingerprint: AuthorityFingerprint,
    pub catalog_dependency_digest: CatalogDependencyDigest,
}
```

Every chat turn, CLI execution, Telegram request, scheduled task, token-maxing
job, and delegated task receives an explicit snapshot. `AgentRuntime` never
falls back to `PersonaManager::active_persona_id` internally.

Changing the user default affects new sessions only. Changing a session persona
creates a new snapshot for the next turn.

## 9. Delegation and A2A

The model authors only the task and requested profile:

```rust
pub struct ModelDelegationRequest {
    pub task: TaskSpec,
    pub requested_profile: ToolProfileRef,
}

pub struct KernelDelegationEnvelope {
    pub request: ModelDelegationRequest,
    pub caller_session: SessionId,
    pub parent_authority: DelegatedAuthorityHandle,
    pub parent_fingerprint: ExecutionProfileFingerprint,
}
```

`DelegatedAuthorityHandle` is an opaque, unforgeable kernel-sealed reference
minted with the parent's resolved profile. The kernel injects the handle after
authenticating the caller and looks up the authority from kernel state. The
model never supplies serialized authority; `parent_fingerprint` is only a
consistency check.

The kernel resolves the child profile. A model may request `operations`; the
resolver rejects or attenuates it when the parent and caller-side delegation
scope do not contain the required authority.

Local subagents always receive a subset of the parent's effective authority.
Remote A2A additionally applies the delegating principal's permitted scope, the
target's accepted delegation authority, and the A2A trust policy. This prevents
a weak agent from using a stronger remote principal as a confused deputy.

A specialized Oxios control agent may exist as an ordinary domain agent for UX
or planning, but it is never an authority broker. Kernel control operations
remain normal catalog tools subject to typed capabilities and approvals.

## 10. Unified tool registration

Tool providers implement one registration contract:

```rust
pub trait ToolProvider {
    fn descriptors(&self) -> &[ToolDescriptor];
    fn instantiate(&self, id: &ToolId, context: &ToolContext)
        -> Result<Arc<dyn AgentTool>>;
}
```

The runtime registry is built only from `ResolvedExecutionProfile.active_tools`.
The dead `OxiosKernelBridge` path and duplicate registration tables are removed.
Feature-gated providers are absent from the runtime catalog when unavailable;
profile resolution reports the missing provider rather than silently dropping
the request.

Each descriptor's `required_capabilities` is the authoritative tool-to-policy
mapping. Rights-aware distinctions such as memory read versus memory write,
browser read versus browser interaction, and kernel observe versus mutate are
encoded in descriptors rather than bespoke domain arms.

Existing `AgentPermissions.allowed_tools` name lists are replaced by policy
rules over namespaced tool IDs, providers, tags, or capability requirements.
Compatibility adapters may exist during migration but are not an enduring
source of truth.

## 11. Prompt and UI projection

Agent SDK tool schemas remain the machine-callable source of tool descriptions.
If the system prompt includes a human-readable capability summary, it is
rendered from `ResolvedExecutionProfile.active_tools` and does not repeat a
separate static catalog.

Existing capability retrieval and kernel manifest prompt sections must consume
the same resolved profile. They may describe skills, resources, or non-tool
capabilities, but cannot advertise tools outside the active set.

UI affordances are derived as follows:

```text
effective UI affordances
  = requested UiProfile affordances
  ∩ resolved tool capabilities
  ∩ runtime feature availability
```

The chat header displays the current turn/session effective profile:

```text
Active: 11 tools
Available on demand: 6
Approval required: 1
Unavailable: 2
```

The detail view explains each unavailable or approval-required item. No
separate coding route is introduced; existing diff, fan-out, terminal, artifact,
and search components appear when supported by the resolved profile.

## 12. Agent pool and conversation state

Agent runtimes are session-isolated. Reuse requires both identity and behavior
to match:

```text
runtime reusable
  iff stored_session_id == requested_session_id
  and stored_fingerprint == requested_fingerprint
```

The fingerprint covers:

- exact persona and sub-profile revisions;
- effective authority, including approval capability IDs and expirations;
- active tool IDs and descriptor revisions;
- model policy;
- descriptor revisions and provider availability used by this resolution;
- behavior-affecting deployment policy.

Conversation state is stored separately from the runtime/tool registry. A
profile change rebuilds the session's agent runtime and rehydrates the same
conversation state when policy allows. Global persona changes do not broadcast
pool-wide evictions.

Descriptors, provider factories, and immutable kernel handles may be shared.
Instantiated tools and their `ToolContext` never cross session boundaries.
Providers declare state scope (`Stateless`, `Session`, or `Turn`); browser,
exec environment, credentials, cwd, and other stateful instances are created at
the declared session/turn boundary. Profile updates or capability expiration
produce new fingerprints and invalidate affected session runtimes.

## 13. Persistence and API

Persona persistence schema v3 stores profile references instead of inline tool
rights. Existing v2 personas are migrated transactionally to explicit legacy
profile revisions; `None` does not carry permanent semantic meaning.

Tool profiles are first-class versioned records. Built-in revisions are
immutable. Editing a custom profile creates a new revision and updates referring
personas only through an explicit operation.

Required APIs:

- `GET /api/tool-catalog` — runtime descriptors and provider availability.
- `GET /api/tool-profiles` — profile revisions and compositions.
- `POST /api/tool-profiles` — create a validated custom revision.
- `PUT /api/personas/:id` — update profile references with optimistic
  concurrency (`revision`/ETag).
- `GET /api/sessions/:id/execution-profile` — requested and resolved views,
  fingerprint, and diagnostics.
- `PUT /api/sessions/:id/persona` — select the next-turn persona snapshot.

Validation is transactional:

- unknown tool/profile/provider → reject;
- inheritance cycle → reject;
- invalid rights/resource combination → reject;
- UI affordance without a satisfiable tool capability → reject or require an
  explicit conditional declaration;
- unavailable optional provider → retain the requirement and report it as
  unavailable;
- unknown exclusion → reject, never warn-and-ignore.

Malformed persisted profiles are quarantined and surfaced through diagnostics.
They do not silently compile to a broader or narrower profile.

## 14. Security properties

- Persona editing changes requested behavior, never the bounding authority.
- Tool activation cannot create capabilities.
- Delegation is monotone, uses a kernel-sealed parent handle, and carries an
  auditable authority chain.
- Pinned revisions prevent silent profile growth, except for explicitly
  declared dynamic provider contracts bounded by their capability ceiling and
  recorded in `catalog_dependency_digest`.
- Namespaced IDs prevent built-in, MCP, and skill tool collisions.
- Approval-issued capabilities are bounded by the approving principal and full
  delegation chain, attributable, time-limited, and checked at presentation.
- UI and prompt projections cannot claim permanently denied tools are callable.
- Dynamic providers must publish capability requirements before their tools can
  enter the catalog.

## 15. Resource behavior

The design intentionally optimizes model context rather than daemon memory.
Descriptors, provider factories, immutable profile revisions, and kernel
handles are shared. Per-turn resolved sets contain references and small decision
records. Stateful tool instances remain session- or turn-local.

Candidate profiles may contain many tools, but the active model-visible set is
normally at most 16. This bounds tool-definition tokens without requiring a
privileged controller agent. Activation and resolution are deterministic table
operations; they do not require another model call.

Profile switches rebuild only behavior-dependent runtime state. Conversation
history remains session-owned and is rehydrated rather than duplicated
indefinitely.

## 16. Testing and verification

### Unit properties

- profile inheritance is deterministic and cycle-safe;
- profile revision pinning is stable;
- unknown selectors and invalid rights fail closed;
- requested authority never exceeds the bounding CSpace;
- delegated authority is always a subset of parent and caller delegation scope;
- approval inside a child cannot exceed the current delegation chain;
- activation changes exposure but not authority;
- expired capabilities fail at presentation and invalidate affected runtimes;
- UI affordances require their effective tool capabilities;
- fingerprints include approval identity/expiry and remain stable for identical
  non-temporal inputs.

### Property-based tests

- for arbitrary parent/profile inputs, `child_cspace ⊆ parent_cspace`;
- adding a deny policy cannot add an active tool;
- removing a capability cannot change a tool from unavailable to active;
- catalog insertion cannot alter a pinned profile unless the profile explicitly
  selects a dynamic provider contract permitting it;
- two sessions with the same profile fingerprint never share stateful tool
  instances;
- prompt tool IDs, registry tool IDs, and UI active tool IDs are identical
  projections of the resolved profile.

### Integration scenarios

1. A code-review persona can inspect but cannot mutate files or execute shell.
2. A security persona cannot acquire operations authority through A2A.
3. An operations profile exposes only task-relevant tools, then activates an
   authorized on-demand tool without changing CSpace.
4. A persona switch rebuilds runtime tools while preserving conversation state.
5. Two concurrent sessions use different personas without global interference.
6. Updating a profile creates a new revision; a pinned session remains
   unchanged.
7. An unavailable MCP provider produces diagnostics and no phantom prompt tool.
8. Approval grants a bounded temporary tool capability and records its issuer.

## 17. Migration and supersession

1. Introduce namespaced tool descriptors and the runtime catalog alongside
   compatibility adapters.
2. Convert `worker/standard/operator/supervisor` templates into pinned built-in
   ToolProfile revisions.
3. Migrate default personas to explicit domain profile references.
4. Resolve every ingress into an immutable execution snapshot; stop runtime
   reads of global active persona.
5. Introduce the resolver and make registry, prompt, UI, and AccessGate consume
   its result.
6. Add progressive activation and delegation attenuation.
7. Remove compatibility templates, duplicate tool-name allowlists, dead bridge
   registration, and legacy `None` semantics.

The migration is complete only when no execution path can register a tool,
advertise a tool, display a tool affordance, or mint an agent capability without
passing through `ExecutionProfileResolver`.

## 18. Decision summary

- Personas reference profiles; they do not own authority.
- Tool profiles are typed, namespaced, versioned requests.
- The kernel resolver intersects requests with existing authority.
- A2A passes attenuated authority and immutable profile snapshots.
- One resolved profile drives tools, prompt, UI, security context, and pooling.
- Large profiles use progressive disclosure instead of a controller agent or a
  permanently large tool surface.
- Shared chat remains the only interaction substrate; persona-specific UI is
  composed from resolved affordances.
