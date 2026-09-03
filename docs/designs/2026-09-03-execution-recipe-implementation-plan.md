# Execution Recipe Coding Host Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Oxios consumes the released oxicode-sdk 0.81.0 behavior-pack API through an Oxios-owned `ExecutionRecipe` resolution path, with `coding-omp-v1` installable behind a pilot flag through the single gated registration path.

**Architecture:** One general `AgentRuntime` stays. A new `execution_recipe` kernel module adds: (1) a resolver that picks `general-v1` (default, zero behavior change) or `coding-omp-v1` per turn; (2) a `BehaviorToolInstaller` impl that wraps every canonical pack tool in the existing `GatedTool::with_approval` and registers it on the turn's existing `ToolRegistry` (last-write-wins replaces native same-name tools; native overlays reject pack duplicates with structured degradations); (3) a `RuntimeExtensionManager` that allocates/reuses/cleans the hashline snapshot store, persistent shell session, and eval kernels per `(project, session, workspace)` key. Design: `docs/designs/2026-08-31-execution-recipe-coding-host-design.md` + paired `/Volumes/MERCURY/PROJECTS/oxicode/docs/designs/2026-08-31-omp-compatible-behavior-pack-design.md`.

**Tech Stack:** Rust 2024 (MSRV 1.96), oxicode-sdk 0.81.0 (features `behavior`, `delegation`, `circuit-breaker`, `router`), oxicode-agent 0.81.0 (`PersistentShellSession`, `PythonEvalKernel`, `JavaScriptEvalKernel`), oxicode-hashline 0.81.0 (`InMemorySnapshotStore`), tokio, parking_lot.

**Spec:** docs/designs/2026-08-31-execution-recipe-coding-host-design.md

## Global Constraints

- oxicode-sdk is a crates.io dependency only — NEVER a path dep on `/Volumes/MERCURY/PROJECTS/oxicode` (AGENTS.md).
- The pack never bypasses `AccessManager`, CSpace gating, approval, audit: every pack tool goes through `GatedTool::with_approval(tool, gate, context, approval_gate, event_bus, pending_approvals, pending_path_access)` — the only wrapper (`tools/gated_tool.rs:139`).
- One registration path: `register_tools_from_cspace_gated` (`tools/registration.rs:867`) remains the sole production path; pack install appends to the same fresh `ToolRegistry` (`agent_runtime.rs:925`, wired to the agent at `agent_runtime.rs:1199-1202`).
- A recipe narrows, never widens: a missing required capability is a structured preflight failure, not a substitution.
- Prompt order: immutable Oxios security text → persona/project → pack layers → (request context above persona stays put). Pack layers are injected directly after the Persona section (`agent_runtime.rs:2120-2125`), so the trailing "Execution Protocol"/"Hard Boundaries" security text always follows them.
- The turn key stays the existing shared `session_id` (`StreamingSinkRegistry`/`TurnRegistry`/`ExecEnv.session_id`). No second identifier.
- Always-on invariant: anything registered through `register_always_on*` must stay consistent with `LAYER0_EXEMPT_TOOLS` (`access_manager/gate.rs:313-340` = `["read","write","edit","grep","find","ls","web_search","get_search_results","todo"]`), guarded by `registration::tests::always_on_tools_are_all_exempt_from_layer_zero`. The pack installer extends this invariant with its own test rather than creating a competing list.
- Secrets/tokens never enter manifests or events; `behavior_tool_manifest` carries only tool names + implementation ids.
- English for code/comments/docs/commits. Commit format `<type>(<scope>): <description>`, scopes: kernel, ouroboros, docs.
- Gates before commit: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-features -- -D warnings`, `cargo check --workspace --all-features`, `cargo nextest run --workspace --no-fail-fast`, `cargo test --workspace --doc`. Web is untouched → no web gates.

## Verified SDK facts (oxicode 0.81.0, local registry)

- `behavior` is a cargo feature; module `oxicode_sdk::behavior` re-exports: `BehaviorPackResolver`, `ResolvedBehavior`, `BehaviorSessionServices`, `BehaviorToolInstaller` (trait: `fn install(&mut self, descriptor: &BehaviorToolDescriptor, tool: Arc<dyn AgentTool>) -> Result<(), BehaviorInstallError>`), `InstalledBehaviorManifest { packs, schema_version, tools: Vec<InstalledToolRecord{descriptor, exposed_name}>, degraded: Vec<DegradationRecord{feature, reason, affected_tools}>, prompt_layers, compatibility }`, `AgentConfigPatch { snapshot_store, lsp, ttsr_engine, url_resolver, subagent_runner, memory, todo, prompt_layers }`, `DegradationReason::{ServiceUnavailable, ExtensionUnavailable, DisabledByHost, HostRejected{tool, reason}}`.
- `BehaviorPackResolver::with_builtin_packs() -> Result<Self>` registers `coding-omp-v1`. `resolve(&[BehaviorPackId], &BehaviorSessionServices) -> Result<ResolvedBehavior>`. `ResolvedBehavior::install(&services, &mut dyn BehaviorToolInstaller) -> Result<InstalledBehaviorManifest, BehaviorInstallError>`.
- `BehaviorSessionServices::new(workspace_root: PathBuf)` + builders `with_snapshot_store/with_shell_session/with_eval_kernel/...`; `port_available(PortRequirementKind)`.
- `coding-omp-v1`: 16 tools — `read.file.v1`/`write.file.v1`/`edit.hashline.v1`/`bash.session.v1`/`grep.search.v1`/`find.search.v1`/`ls.fs.v1` are `.essential()`; `ast-grep.search.v1`, `ast-edit.write.v1`, `web-search.network.v1`, `search-results.cache.v1`, `todo.session.v1`, `subagent.delegation.v1`, `lsp.host.v1`, `eval.kernel.v2`, `debug.dap.v2` optional. HashlineState extension is REQUIRED (install fails without a snapshot store). Install loop: required port missing on essential → hard error; optional tool with missing port → skipped + degradation; installer rejection → essential ? hard error : `HostRejected` degradation.
- `ToolRegistry::register`/`register_arc` are map inserts — duplicate names overwrite (last write wins). `unregister(name) -> bool` exists.
- `oxicode_agent::runtime::{ShellSession (execute/cancel/reset), PersistentShellSession::new(workspace_root), EvalKernel, PythonEvalKernel::new(), JavaScriptEvalKernel::new(), DebugService, DapDebugService}`.
- `oxicode_hashline::snapshots::{SnapshotStore, InMemorySnapshotStore, InMemorySnapshotStoreOptions{max_paths, max_versions_per_path, max_total_bytes}}` (bounded, `Default`).

## Oxios seams (verified)

- `AgentRuntime::execute_inner` (`agent_runtime.rs:356-420`): persona resolved at L391, cspace at L405 → recipe resolution slots in right after L405.
- `run_agent` (`agent_runtime.rs:843`): workspace derivation L881, fresh registry L925, `AgentContext` L929, `AccessGate` L963-1008, `ApprovalGate` L1031-1038, `register_tools_from_cspace_gated(...)` L1053-1067, `generate_tool_section(&registry.names())` L1079, `AgentConfig{...}` L1087-1138, builder L1148-1152, `agent_tools.register_arc` L1199-1202.
- `ExecEnv` (`oxios-ouroboros/src/directive.rs:60-151`): `project_id`, `cspace_hint`, `persona_id`, `session_id: Option<String>`, ...
- `ExecutionResult` lives in `oxios-ouroboros` (same file, after ExecEnv) — add `recipe_id: Option<String>` with `#[serde(default)]`.
- `KernelEvent` (`event_bus.rs:30-446`) — single broadcast bus; `ToolExecutionFinished.results` carries `StructuredResultBus` payloads (`structured_results.rs:34-65`, stamped at `agent_runtime.rs:1362`).
- `OxiosConfig` (`config.rs:1236-1332`) — nested sections; `share/default-config.toml` documents defaults. `KernelHandle` composes facades (`kernel_handle/mod.rs:83-186`; constructor L191-209).
- `ToolProfile` = Base/Code/Minimal/Control (`persona/mod.rs`); `resolve_cspace(cspace_hint, tool_profile, agent_id)` (`capability/resolve.rs`), called at `agent_runtime.rs:405`.

## Out of scope (documented follow-ups, design rollout steps 6-8)

- Lifecycle-managed coding delegation (step 6): pilot rejects `subagent.delegation.v1` with an honest `HostRejected` degradation; text-only `OxiosSubagentRunner` remains the general-task fallback.
- Web "Coding environment" panel (step 7): kernel emits the structured events; UI rendering is a separate surface task.
- Defaulting personas/projects to the coding recipe (step 8): requires pilot gates passing first.
- LSP host discovery, DAP service construction, TTSR rule-registry content: extensions degrade with structured records until host-side implementations exist.

---

### Task 1: Bump oxicode-sdk 0.77.0 → 0.81.0 (+ `behavior` feature)

**Files:**
- Modify: `Cargo.toml:134` (workspace dep)
- Modify: `crates/oxios-kernel/Cargo.toml:43-46`

**Interfaces:**
- Produces: workspace resolves oxicode-sdk/oxicode-agent/oxicode-ai 0.81.0; `oxicode_sdk::behavior` module available under the `behavior` feature.

- [ ] **Step 1: Bump versions**

Root `Cargo.toml`:
```toml
oxicode-sdk = { version = "0.81.0", features = ["behavior", "delegation", "circuit-breaker", "router"] }
```
`crates/oxios-kernel/Cargo.toml`:
```toml
oxicode-agent = "0.81.0"
oxicode-ai = "0.81.0"
```

- [ ] **Step 2: Update lockfile and check**

Run: `cargo update -p oxicode-sdk -p oxicode-agent -p oxicode-ai && cargo check -p oxios-kernel`
Expected: resolves 0.81.0; compiles with no errors (0.78–0.81 changelogs are Added/Fixed only — no Breaking sections). Fix any real errors empirically; do not guess from design docs.

- [ ] **Step 3: Full-workspace check (all features)**

Run: `cargo check --workspace --all-features`
Expected: PASS. Fix real errors; `..Default::default()` already covers `AgentConfig` growth.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock crates/oxios-kernel/Cargo.toml
git commit -m "chore(kernel): bump oxicode-sdk to 0.81.0 with behavior-pack feature"
```

---

### Task 2: `execution_recipe` module — types + resolver + config

**Files:**
- Create: `crates/oxios-kernel/src/execution_recipe/mod.rs`, `types.rs`, `resolver.rs`
- Modify: `crates/oxios-kernel/src/lib.rs` (module declaration near other kernel modules)
- Modify: `crates/oxios-kernel/src/config.rs` (new `ExecutionRecipeConfig` section in `OxiosConfig`)
- Modify: `share/default-config.toml`
- Modify: `crates/oxios-ouroboros/src/directive.rs` (`ExecutionResult.recipe_id`)
- Test: `#[cfg(test)] mod tests` inside `resolver.rs`

**Interfaces:**
- Produces (consumed by Tasks 4-6):
```rust
// execution_recipe/types.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeId(pub String); // "general-v1" | "coding-omp-v1"
pub const GENERAL_V1: &str = "general-v1";
pub const CODING_OMP_V1: &str = "coding-omp-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecipeSource { Explicit, ProjectBinding, PersonaDefault, SystemDefault }

#[derive(Debug, Clone)]
pub struct ResolvedExecution {
    pub recipe: RecipeId,
    pub pack_ids: Vec<oxicode_sdk::behavior::BehaviorPackId>, // empty for general-v1
    pub source: RecipeSource,
    pub services: CodingServiceFlags, // which host services the pilot may wire
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct CodingServiceFlags {
    pub shell: bool,
    pub eval: bool,
    // lsp / debug / ttsr / delegation: always false in the pilot — structured degradations.
}

// execution_recipe/resolver.rs
pub struct ExecutionRecipeResolver { cfg: crate::config::ExecutionRecipeConfig }
pub struct RecipeRequest<'a> {
    pub project_id: Option<uuid::Uuid>,
    pub explicit: Option<RecipeId>,   // reserved; config-originated only
    pub persona_id: Option<&'a str>,
}
impl ExecutionRecipeResolver {
    pub fn new(cfg: crate::config::ExecutionRecipeConfig) -> Self;
    pub fn resolve(&self, req: RecipeRequest<'_>) -> ResolvedExecution;
}
```
- Config (`config.rs`, near the other section structs; wire into `OxiosConfig`):
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutionRecipeConfig {
    pub enabled: bool,           // resolver active; general-v1 default
    pub coding_pilot: bool,      // master switch for coding-omp-v1 selection
    pub coding_shell: bool,      // persistent shell session service
    pub coding_eval: bool,       // persistent python/js eval kernels
    pub project_bindings: std::collections::HashMap<String, String>, // project uuid -> recipe id
}
impl Default for ExecutionRecipeConfig { /* enabled: true, rest false/empty */ }
```

- [ ] **Step 1: Write failing resolver tests** — precedence matrix: (a) no inputs → `general-v1` + `SystemDefault` + empty packs; (b) project binding + `coding_pilot=false` → `general-v1` (binding ignored); (c) project binding + `coding_pilot=true` + known id → `coding-omp-v1` + `ProjectBinding` + `pack_ids=[coding-omp-v1]` + services from flags; (d) binding to unknown id → `general-v1` (never fails a turn on a typo); (e) `enabled=false` → always `general-v1`.
- [ ] **Step 2: Run** `cargo nextest run -p oxios-kernel execution_recipe` — expect compile failure (module missing).
- [ ] **Step 3: Implement** types + resolver per interfaces above; `resolve` reads flags into `CodingServiceFlags` only when the coding recipe is selected.
- [ ] **Step 4: Config plumbing** — add `ExecutionRecipeConfig` to `OxiosConfig` + `share/default-config.toml`:
```toml
[execution_recipe]
enabled = true
coding_pilot = false
coding_shell = true
coding_eval = true
# project_bindings = { "<project-uuid>" = "coding-omp-v1" }
```
- [ ] **Step 5: ExecutionResult field** — `crates/oxios-ouroboros/src/directive.rs`: add `#[serde(default)] pub recipe_id: Option<String>` to `ExecutionResult`; fix every struct-literal construction site (`supervisor.rs:407,458,568`, `coordinator.rs` tests, `agent_runtime.rs:709`) with `recipe_id: None` or `..Default::default()` as the site allows.
- [ ] **Step 6: Run tests** — `cargo nextest run -p oxios-kernel execution_recipe -p oxios-ouroboros` PASS.
- [ ] **Step 7: Commit** — `feat(kernel): execution recipe types and resolver (general-v1 default)`

---

### Task 3: `RuntimeExtensionManager` — hashline/shell/eval adapters with lifecycle

**Files:**
- Create: `crates/oxios-kernel/src/execution_recipe/extensions.rs`
- Test: `#[cfg(test)] mod tests` in the same file

**Interfaces:**
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExtensionKey {
    pub project_id: Option<uuid::Uuid>,
    pub session_id: String,        // the shared turn key
    pub workspace: std::path::PathBuf, // canonicalized at run_agent
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionStatus { pub extension: String, pub state: String /* started|reused|degraded|stopped */, pub detail: Option<String> }

#[derive(Clone)]
pub struct CodingExtensions {
    pub snapshot_store: Arc<dyn oxicode_sdk::SnapshotStore>,
    pub shell_session: Option<Arc<dyn oxicode_agent::runtime::ShellSession>>,
    pub eval_kernels: Vec<Arc<dyn oxicode_agent::runtime::EvalKernel>>,
    pub statuses: Vec<ExtensionStatus>,
}

#[derive(Default)]
pub struct RuntimeExtensionManager { sessions: parking_lot::HashMap<ExtensionKey, CodingExtensions> }
impl RuntimeExtensionManager {
    /// Create-or-reuse per key. Hashline is always allocated (pack requires it).
    /// shell/eval only when `flags` say so. lsp/debug/ttsr/delegation are recorded
    /// as degraded statuses (no host implementation in the pilot).
    pub fn acquire(&self, key: ExtensionKey, flags: CodingServiceFlags) -> CodingExtensions;
    /// Drop the session's extensions: shell cancel + async reset is best-effort
    /// (spawned task), snapshots/kernels dropped with the Arcs.
    pub fn release(&self, key: &ExtensionKey);
    pub fn release_session(&self, session_id: &str); // workspace-change/session-end cleanup
}
```

- [ ] **Step 1: Failing tests** — (a) same key twice → identical `Arc` pointer for snapshot store (`Arc::ptr_eq` on a downcast helper or track generation counter — simplest: assert `statuses` says `reused` on the second acquire and the manager holds one entry); (b) different workspace → new entries; (c) `release` then acquire → fresh (`started`); (d) flags.shell=false → `shell_session` is `None` + degraded status recorded; (e) `release_session` clears all keys with that session.
- [ ] **Step 2: Run** `cargo nextest run -p oxios-kernel runtime_extension` — expect failure.
- [ ] **Step 3: Implement** — hashline: `InMemorySnapshotStore::with_options(InMemorySnapshotStoreOptions::default())` (check the exact constructor name in oxicode-hashline 0.81.0 `snapshots.rs`; use whatever bounded constructor exists); shell: `PersistentShellSession::new(workspace.clone())`; eval: `PythonEvalKernel::new()` + `JavaScriptEvalKernel::new()`.
- [ ] **Step 4: Run tests** PASS.
- [ ] **Step 5: Commit** — `feat(kernel): runtime extension manager with hashline/shell/eval adapters`

---

### Task 4: `GatedBehaviorInstaller` — descriptor reconciliation + gated pack install

**Files:**
- Create: `crates/oxios-kernel/src/execution_recipe/installer.rs`
- Test: `#[cfg(test)] mod tests` in the same file

**Interfaces:**
```rust
/// Static reconciliation table: SDK implementation id -> Oxios policy.
struct PackToolPolicy {
    exposed_name: &'static str,
    action: PackAction,
    side_effect: oxicode_sdk::behavior::SideEffectClass, // expected; mismatch = programming error
}
enum PackAction {
    /// Canonical pack tool replaces the native same-name tool for this turn.
    InstallShadowNative,
    /// New model-visible name; no native collision.
    Install,
    /// Oxios overlay stays; pack tool rejected with a structured degradation.
    KeepNativeOverlay(&'static str), // reason
}

static PACK_TOOL_MAP: &[PackToolPolicy] = &[
    // id, exposed, action, expected side effect
    p("read.file.v1", "read", InstallShadowNative, ReadOnly),
    p("write.file.v1", "write", InstallShadowNative, Mutating),
    p("edit.hashline.v1", "edit", InstallShadowNative, Mutating),
    p("bash.session.v1", "bash", Install, ProcessSpawning),
    p("grep.search.v1", "grep", InstallShadowNative, ReadOnly),
    p("find.search.v1", "find", InstallShadowNative, ReadOnly),
    p("ls.fs.v1", "ls", InstallShadowNative, ReadOnly),
    p("ast-grep.search.v1", "ast_grep", Install, ReadOnly),
    p("ast-edit.write.v1", "ast_edit", Install, Mutating),
    p("web-search.network.v1", "web_search", KeepNativeOverlay("oxios kernel web_search overlay (managed provider + structured results)"), Networked),
    p("search-results.cache.v1", "get_search_results", KeepNativeOverlay("oxios kernel overlay"), ReadOnly),
    p("todo.session.v1", "todo", KeepNativeOverlay("oxios session todo overlay (StructuredResultBus)"), Mutating),
    p("subagent.delegation.v1", "subagent", KeepNativeOverlay("lifecycle-managed coding delegation lands in rollout step 6; text-only runner is not coding-equivalent"), ProcessSpawning),
    p("lsp.host.v1", "lsp", Install, ReadOnly),
    p("eval.kernel.v2", "eval", Install, ProcessSpawning),
    p("debug.dap.v2", "debug", Install, ProcessSpawning),
];

pub struct GatedBehaviorInstaller<'a> {
    registry: &'a ToolRegistry,
    gate: Arc<AccessGate>,
    agent_context: AgentContext,
    approval_gate: Option<Arc<crate::approval::ApprovalGate>>,
    event_bus: Option<crate::event_bus::EventBus>,
    pending_approvals: Option<Arc<crate::tools::PendingToolApprovals>>,
    pending_path_access: Option<Arc<crate::tools::PendingPathAccess>>,
    pub degraded: Vec<oxicode_sdk::behavior::DegradationRecord>,
}
impl oxicode_sdk::behavior::BehaviorToolInstaller for GatedBehaviorInstaller<'_> {
    fn install(&mut self, descriptor: &BehaviorToolDescriptor, tool: Arc<dyn AgentTool>)
        -> Result<(), BehaviorInstallError> { /* see steps */ }
}
```
Install logic per descriptor:
1. Look up `PACK_TOOL_MAP` by `descriptor.id`. Missing / `exposed_name` mismatch / `side_effect` mismatch → `Err(BehaviorInstallError::HostRejected{..})`-style hard error — reconciliation failure must fail the install loudly (design §Descriptor reconciliation). Use whatever `BehaviorInstallError` variant fits; if none fits, wrap the failure into the variant the SDK install loop surfaces for installer rejections of essential tools — essential tools with rejected install abort the pack, which is the required loud failure.
2. `KeepNativeOverlay` → `Err(BehaviorInstallError` rejection reason `)` with the overlay reason (SDK converts optional rejections into `DegradationReason::HostRejected`); if such a tool were ever essential, the pack fails — correct.
3. `InstallShadowNative`/`Install` → wrap: `GatedTool::with_approval(...)` — note `GatedTool<T>` is generic over the concrete tool; for `Arc<dyn AgentTool>` use the `Arc<dyn AgentTool>`-compatible form (verify: if `GatedTool` requires `T: AgentTool` concrete, wrap via a newtype or check whether `GatedTool<Arc<dyn AgentTool>>` works — `Arc<dyn AgentTool>` itself implements `AgentTool` in oxicode-agent if there is a blanket impl; if not, add a small `ArcTool` newtype in installer.rs implementing `AgentTool` by delegation). Register with `registry.register_arc(gated)`.

- [ ] **Step 1: Failing test — reconciliation covers the pack** — iterate `BehaviorPackResolver::with_builtin_packs()?.pack(&coding_omp_v1)` descriptors; assert every id is in `PACK_TOOL_MAP` exactly once, names and side-effect classes match. This is the fail-loud drift guard (design: resolution must fail in dev/test when a mapping is missing).
- [ ] **Step 2: Failing test — Layer-0 consistency** — every `InstallShadowNative`/`Install` exposed name that is read-only must be in `LAYER0_EXEMPT_TOOLS`; every name in `LAYER0_EXEMPT_TOOLS` that the pack shadows must be declared `InstallShadowNative` (write/edit/todo are mutating but already Layer-0-exempt by product decision — encode exactly the shipped exemptions: `read,write,edit,grep,find,ls,web_search,get_search_results,todo`).
- [ ] **Step 3: Failing test — install + gate** — fake gate (deny-all), build services with snapshot store, resolve+install into a fresh `ToolRegistry`; assert: `read/write/edit/grep/find/ls/bash/ast_grep/ast_edit` registered (10 of 16 — web_search/get_search_results/todo/subagent rejected → degraded), every registered tool is wrapped (call through the deny-all gate → denial result, never the raw tool output), `degraded` records carry `HostRejected` with the overlay reasons.
- [ ] **Step 4: Failing test — required extension fails loudly** — install with `BehaviorSessionServices::new(tmp)` (no snapshot store) → `Err` (HashlineState required) — proves preflight failure instead of silent degradation.
- [ ] **Step 5: Implement** the installer + map.
- [ ] **Step 6: Run** `cargo nextest run -p oxios-kernel execution_recipe` PASS.
- [ ] **Step 7: Commit** — `feat(kernel): gated behavior tool installer with descriptor reconciliation`

---

### Task 5: Wire the resolver into the turn (general-v1 = zero behavior change)

**Files:**
- Modify: `crates/oxios-kernel/src/agent_runtime.rs` (`execute_inner` L405 area, `run_agent` signature L843 + body)
- Modify: `crates/oxios-kernel/src/kernel_handle/mod.rs` (hold `ExecutionRecipeResolver`)
- Test: `crates/oxios-kernel/tests/execution_recipe_integration.rs` (create; follow existing `tests/` conventions)

**Interfaces:**
- Consumes: `ExecutionRecipeResolver::resolve` (Task 2), `ResolvedExecution`.
- Produces: `run_agent(..., resolved: &ResolvedExecution)` param; `ExecutionResult.recipe_id = Some(recipe.0)` on the coding path (`None` → serializes as absent for general turns).

- [ ] **Step 1: Read `register_tools_from_cspace_gated` body** (`registration.rs:867-1018`) and confirm where the always-on set is registered inside it (needed by Task 6 ordering).
- [ ] **Step 2: Hold the resolver on KernelHandle** — field `execution_recipe: crate::execution_recipe::ExecutionRecipeResolver`, built in the constructor from `config.execution_recipe.clone()`; refresh on config reload only if a reload hook exists for sibling sections (mirror how `SearchConfig` snapshots are taken — snapshot per turn is fine).
- [ ] **Step 3: Resolve in `execute_inner`** right after `resolve_cspace` (L405):
```rust
let resolved_execution = self.kernel_handle.execution_recipe.resolve(RecipeRequest {
    project_id,
    explicit: None,
    persona_id: persona_id.map(str::to_string).as_deref(),
});
```
(Adapt to however execute_inner reaches KernelHandle — it takes `&AgentRuntimeConfig` + `self`; `AgentRuntime` holds a `KernelHandle` — check field name at `agent_runtime.rs:212-227`.)
- [ ] **Step 4: Thread into `run_agent`** as a parameter; in `run_agent`, `general-v1` short-circuits: no pack install, registry unchanged. Stamp `recipe_id` onto the `ExecutionResult` built at L709.
- [ ] **Step 5: Integration test** — with the default config (no pilot), run the registration seam for a turn and assert the registry names equal the pre-change baseline (record baseline names from `main` in the test as an explicit expected list OR assert no `bash`/`ast_grep`/`ast_edit` tools present and no recipe event emitted). Keep the harness light: construct only what `run_agent` needs, or assert at the resolver+registry level if `run_agent` needs a live engine.
- [ ] **Step 6: Run** `cargo nextest run -p oxios-kernel` PASS (full crate — the param change touches callers).
- [ ] **Step 7: Commit** — `feat(kernel): resolve execution recipe per turn (general-v1 no-op path)`

---

### Task 6: coding-omp-v1 pilot — install, config patch, prompt layers, events

**Files:**
- Modify: `crates/oxios-kernel/src/agent_runtime.rs` (`run_agent` between L1067 and L1079; `AgentConfig` L1087-1138; `build_system_prompt_inner` L2050)
- Modify: `crates/oxios-kernel/src/event_bus.rs` (two `KernelEvent` variants)
- Modify: `crates/oxios-kernel/src/execution_recipe/extensions.rs` (emit statuses through the caller)
- Test: `crates/oxios-kernel/tests/execution_recipe_integration.rs`

**Interfaces:**
```rust
// KernelEvent additions (serde-compatible struct variants, matching file style):
ExecutionRecipeResolved {
    exec_id: Uuid, agent_id: AgentId,
    recipe: String, packs: Vec<String>, source: String,
    workspace: PathBuf,
    tools: Vec<ToolManifestEntry { name: String, implementation: String }>, // no secrets
    degraded: Vec<DegradedFeature { feature: String, reason: String, affected_tools: Vec<String> }>,
    compatibility_target: String,
},
RuntimeExtensionStatus { exec_id: Uuid, agent_id: AgentId, extension: String, state: String, detail: Option<String> },
```
- AgentConfig patch application (policy-validated): only fields the host itself put into `BehaviorSessionServices` are accepted back from `patch` (`snapshot_store`, `lsp`, `ttsr_engine` — the pilot supplies only `snapshot_store`), so `agent_config.snapshot_store = resolved patch value`; prompt layer bodies appended after Persona via a new `pack_prompt_layers: &[(String, String)]` param on `build_system_prompt_inner` rendered as `## Behavior Pack (<layer id>)`.

- [ ] **Step 1: Failing integration test** — config with `coding_pilot=true` + a project binding; at the seam level (extract a testable `fn install_coding_recipe(registry, gate_bundle, resolved, extensions, workspace) -> Result<InstalledBehaviorManifest>` from `run_agent` if the full `run_agent` harness is too heavy): assert manifest tools registered, native overlays intact (`web_search` present = oxios version, `bash` present = pack), `KernelEvent::ExecutionRecipeResolved` + `RuntimeExtensionStatus` received on a subscribed broadcast handle, degradations list subagent/web_search/get_search_results/todo (+lsp/debug/ttsr when flags off), and `generate_tool_section` output contains `bash` and `ast_grep` names.
- [ ] **Step 2: Run** — expect failure (seam missing).
- [ ] **Step 3: Implement `run_agent` coding path** — order inside `run_agent`:
  1. after `register_tools_from_cspace_gated` (L1067) and BEFORE `generate_tool_section` (L1079): `if !resolved.pack_ids.is_empty()` → `kernel_handle.runtime_extensions.acquire(ExtensionKey { project_id, session_id: session_id.clone().unwrap_or_else(|| exec_id.to_string()), workspace: workspace.canonicalize() }, resolved.services)`;
  2. `BehaviorSessionServices::new(workspace.clone())` + `.with_snapshot_store(ext.snapshot_store.clone())` + shell/eval per ext;
  3. resolve via a `BehaviorPackResolver::with_builtin_packs()?` held once (build per call is fine — resolver construction is cheap and side-effect free; note: `SearchCache` inside the pack is per-resolver, so per-turn cache is acceptable for the pilot, matching the existing per-turn `SearchCache::new()` at L926);
  4. `resolved_behavior.install(&services, &mut installer)?` — `?` = structured preflight failure (turn fails before any model call);
  5. apply the validated `AgentConfigPatch` fields to `agent_config` (snapshot_store etc.) and pass prompt layers into `build_system_prompt_inner`;
  6. emit `KernelEvent::ExecutionRecipeResolved` + one `RuntimeExtensionStatus` per `ExtensionStatus` on `approval_event_bus`;
  7. log `tracing::info!(recipe, tools, degraded, "coding recipe installed")`.
- [ ] **Step 4: Session cleanup** — workspace-change/session-end: call `runtime_extensions.release_session(&session_id)` where the kernel already tears down turn state (locate the session-end path via `TurnRegistry`/`StreamingSinkRegistry` cleanup; if none exists for chat sessions, document that extension state is bounded (LRU snapshot store, one shell per key) and release on process exit — record the decision in the module docs).
- [ ] **Step 5: Run integration + crate tests** PASS.
- [ ] **Step 6: Smoke test (behavioral proof)** — `cargo build` then `oxios run --json "<trigger a coding task with the pilot bound to a test project>"`; observe the `execution_recipe_resolved`-derived log line and the turn completing with pack tools. If a live model call is not available in this environment, prove at the seam level (Step 1 test) and state exactly that in the report.
- [ ] **Step 7: Commit** — `feat(kernel): coding-omp-v1 pilot behind execution_recipe.coding_pilot`

---

### Task 7: Docs + full local CI gates

**Files:**
- Modify: `AGENTS.md` (Key Docs table row for the design + plan), `CHANGELOG.md` (Unreleased/next-version entry)
- Modify: `docs/ARCHITECTURE.md` (append short §: execution recipe seam ownership per design table)

- [ ] **Step 1: Docs** — CHANGELOG entry under the unreleased heading describing: SDK 0.81 bump, execution recipe resolver, gated behavior installer, runtime extension manager, coding pilot flag (default off). ARCHITECTURE § mapping the five ownership rows to concrete symbols. AGENTS.md Key Docs row: `docs/designs/2026-08-31-execution-recipe-coding-host-design.md` — "Modifying turn execution behavior, tool registration, or coding recipe wiring".
- [ ] **Step 2: Gates** — run in order, fix and rerun until green:
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-features -- -D warnings
cargo check --workspace --all-features
cargo nextest run --workspace --no-fail-fast
cargo test --workspace --doc
```
- [ ] **Step 3: Commit** — `docs(kernel): execution recipe architecture notes and changelog`

---

## Self-review notes

- Spec coverage: rollout steps 1-5 mapped (Task 1 = step 1 prerequisite; Task 2 = step 2; Tasks 4+5+6 = step 3; Task 3 = step 4; Task 6 = step 5 pilot). Steps 6-8 explicitly out of scope with honest degradations.
- Security invariants (design §Security invariants) → Task 4 tests 1-4 + Task 6 ordering (pack install after native, before prompt tool section; structured failure before any model call).
- Type consistency: `ResolvedExecution` produced by Task 2 consumed by Tasks 5-6; `ExtensionKey`/`CodingExtensions` produced by Task 3 consumed by Task 6; `GatedBehaviorInstaller` produced by Task 4 consumed by Task 6.
