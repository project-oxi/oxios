# Persona Execution Profiles — Kernel Implementation Plan (Plan 1)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the kernel core of `docs/designs/2026-08-20-persona-tool-scope-design.md`: typed capability vocabulary, self-describing tool catalog, versioned tool profiles, the ExecutionProfileResolver, a single registration path, persona integration with immutable snapshots, persistence v3, and catalog/profile APIs.

**Architecture:** New additive modules under `crates/oxios-kernel/src/capability/` (descriptor, profile, resolver, builtin_profiles) define the request language; a new registration entry point consumes `ResolvedExecutionProfile`; the final tasks cut `agent_runtime` over to the resolver, delete the dead bridge, and migrate personas/persistence/API. Every task leaves the workspace compiling and tests green.

**Tech Stack:** Rust 2024 (edition 2024, MSRV 1.96), tokio, serde, oxicode-sdk (crates.io only), cargo-nextest.

## Global Constraints

- Rust 2024, MSRV 1.96, single target `aarch64-apple-darwin`.
- `oxios-kernel` is intentionally monolithic — new modules go in `crates/oxios-kernel/src/capability/`, no new crates.
- `#![warn(missing_docs)]` applies: every new public item gets a doc comment.
- English for all code, comments, commits. Commit scope: `kernel` (e.g. `feat(kernel): ...`).
- oxicode-sdk from crates.io only — never a path dep.
- Tests: unit tests in `#[cfg(test)] mod tests` within the touched file. Run with `cargo nextest run -p oxios-kernel` (fallback `cargo test -p oxios-kernel` if nextest missing).
- Do NOT run `cargo fmt` on files you did not touch (pre-existing drift). Format only files you create/modify.
- No new external dependencies. Fingerprints use hand-rolled FNV-1a 64-bit (deterministic across builds).
- Gate commands after each task: `cargo check -p oxios-kernel --all-features` then the task's test filter. Full gates only at Task 12.
- Serde on new public types uses `#[derive(Debug, Clone, Serialize, Deserialize)]` with `#[serde(default)]` on Option/Vec fields for forward compat.

## File Structure

| File | Responsibility |
|---|---|
| `crates/oxios-kernel/src/capability/types.rs` | `ResourceRef` gains `Fs`, `WebSearch`, `Memory`, `Knowledge` variants |
| `crates/oxios-kernel/src/capability/descriptor.rs` (new) | `ToolId`, `ToolProviderId`, `ToolTag`, `ToolContractVersion`, `CapabilityRequirement`, `ActivationClass`, `ToolDescriptor`, static kernel catalog |
| `crates/oxios-kernel/src/capability/profile.rs` (new) | `ToolSelector`, `CapabilityRequest`, `ResourceSelector`, `ToolProfileSpec`, `ToolActivationPolicy`, `DynamicProviderContract`, `ProviderCompatibilityPolicy`, `ProfileRevision`, `ToolProfileRef`, selection algebra, publish validation |
| `crates/oxios-kernel/src/capability/resolver.rs` (new) | `ResolvedTool`, `ResolutionStatus`, `UnavailabilityReason`, `ExecutionProfileFingerprint`, `ResolvedExecutionProfile`, `resolve()` |
| `crates/oxios-kernel/src/capability/builtin_profiles.rs` (new) | 7 domain presets + legacy worker/standard/operator/supervisor expressed as profiles |
| `crates/oxios-kernel/src/capability/mod.rs` | re-exports |
| `crates/oxios-kernel/src/tools/registration.rs` | `register_from_resolved_profile` — ID→constructor table, rights-aware arms |
| `crates/oxios-kernel/src/tools/kernel_bridge.rs` | **deleted** in Task 9 |
| `crates/oxios-kernel/src/tools/builtin/mod.rs` | `register_all_kernel_tools` dissolved in Task 9 |
| `crates/oxios-kernel/src/agent_runtime.rs` | resolver cutover + prompt-from-registry |
| `crates/oxios-kernel/src/access_manager/gate.rs` | Layer-0 `ResourceRef` mapping, skip-list removal |
| `crates/oxios-kernel/src/access_manager/rbac.rs` | `User` policy gains web tools |
| `crates/oxios-kernel/src/persona/mod.rs` | `Persona.tool_profile` field |
| `crates/oxios-kernel/src/persona/persistence.rs` | schema v3 |
| `src/api/persona_routes.rs` + `src/api/routes/mod.rs` | tool_profile CRUD + catalog/profile endpoints |

---

### Task 1: ResourceRef capability variants

**Files:**
- Modify: `crates/oxios-kernel/src/capability/types.rs` (enum at lines 174-209, Display at 211-224)
- Test: same file `#[cfg(test)] mod tests` (append at end)

**Interfaces:**
- Produces: `ResourceRef::Fs`, `ResourceRef::WebSearch`, `ResourceRef::Memory`, `ResourceRef::Knowledge` (all unit variants) used by Tasks 3, 4, 6.

- [ ] **Step 1: Write the failing test** (append inside `mod tests` in types.rs)

```rust
#[test]
fn resource_ref_new_variants_display_and_serde_roundtrip() {
    use super::ResourceRef;
    assert_eq!(ResourceRef::Fs.to_string(), "fs");
    assert_eq!(ResourceRef::WebSearch.to_string(), "web_search");
    assert_eq!(ResourceRef::Memory.to_string(), "memory");
    assert_eq!(ResourceRef::Knowledge.to_string(), "knowledge");
    for r in [
        ResourceRef::Fs,
        ResourceRef::WebSearch,
        ResourceRef::Memory,
        ResourceRef::Knowledge,
    ] {
        let json = serde_json::to_string(&r).expect("serialize");
        let back: ResourceRef = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, r);
    }
    // KernelDomain and new variants coexist: a Memory grant is distinct from KernelDomain{"memory"}.
    assert_ne!(
        ResourceRef::Memory,
        ResourceRef::KernelDomain { domain: "memory".into() }
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p oxios-kernel resource_ref_new_variants`
Expected: FAIL — no variant `Fs` found.

- [ ] **Step 3: Implement** — add to `pub enum ResourceRef` (after the `Mcp` variant):

```rust
    /// Filesystem primitive tools (read/write/edit/grep/find/ls).
    Fs,
    /// Web search tools (web_search/get_search_results).
    WebSearch,
    /// Agent memory (oxibrain-backed recall/retain).
    Memory,
    /// User markdown knowledge base.
    Knowledge,
```

Add Display arms in `impl fmt::Display for ResourceRef`:

```rust
            ResourceRef::Fs => write!(f, "fs"),
            ResourceRef::WebSearch => write!(f, "web_search"),
            ResourceRef::Memory => write!(f, "memory"),
            ResourceRef::Knowledge => write!(f, "knowledge"),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p oxios-kernel resource_ref_new_variants`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/oxios-kernel/src/capability/types.rs
git commit -m "feat(kernel): add Fs/WebSearch/Memory/Knowledge ResourceRef variants"
```

---

### Task 2: Tool descriptors and the kernel catalog

**Files:**
- Create: `crates/oxios-kernel/src/capability/descriptor.rs`
- Modify: `crates/oxios-kernel/src/capability/mod.rs` (add `pub mod descriptor;`)

**Interfaces:**
- Consumes: `ResourceRef`, `Rights` from Task 1 / types.rs.
- Produces (used by Tasks 3, 4, 6, 11):
  - `pub struct ToolId(pub Arc<str>)` with `ToolId::new("kernel.fs.read")`, `Display`, `Hash`, `Eq`
  - `pub struct ToolProviderId(pub Arc<str>)`
  - `pub struct ToolTag(pub Arc<str>)`
  - `pub struct ToolContractVersion { pub major: u32, pub minor: u32 }`
  - `pub struct CapabilityRequirement { pub resource: ResourceRef, pub rights: Rights }`
  - `pub enum ActivationClass { Always, IntentMatched, OnDemand, ApprovalOnly }`
  - `pub struct ToolDescriptor { pub id: ToolId, pub provider: ToolProviderId, pub contract_version: ToolContractVersion, pub description: &'static str, pub tags: &'static [ToolTag], pub required_capabilities: &'static [CapabilityRequirement], pub ui_affordances: &'static [ToolTag], pub activation_class: ActivationClass, pub registered_name: &'static str }`
  - `pub fn kernel_catalog() -> &'static [ToolDescriptor]`
  - `pub fn find_descriptor(id: &ToolId) -> Option<&'static ToolDescriptor>`

All serde-deriving where serialized (ToolId, ToolContractVersion, CapabilityRequirement at minimum).

**Descriptor table contents** (exact — `registered_name` must equal the name the tool registers under today; `Rights` constructors are `Rights::READ`, `Rights::WRITE`, `Rights::EXECUTE`, combined with `|`):

| id | registered_name | rights requirement | activation_class | tags |
|---|---|---|---|---|
| kernel.fs.read | read | Fs READ | Always | fs |
| kernel.fs.grep | grep | Fs READ | Always | fs |
| kernel.fs.find | find | Fs READ | Always | fs |
| kernel.fs.ls | ls | Fs READ | Always | fs |
| kernel.fs.write | write | Fs WRITE | Always | fs |
| kernel.fs.edit | edit | Fs WRITE | Always | fs |
| kernel.web.search | web_search | WebSearch EXECUTE | Always | web |
| kernel.web.results | get_search_results | WebSearch EXECUTE | Always | web |
| kernel.exec.run | exec | Exec{mode:"shell"} EXECUTE | IntentMatched | exec |
| kernel.browse.open | browse | Browser EXECUTE | OnDemand | browser |
| kernel.browse.extract | browse_extract | Browser EXECUTE | OnDemand | browser |
| kernel.browse.session | browse_session | Browser EXECUTE | OnDemand | browser |
| kernel.browse.script | browse_script | Browser EXECUTE | OnDemand | browser |
| kernel.memory.read | memory_read | Memory READ | IntentMatched | memory |
| kernel.memory.search | memory_search | Memory READ | IntentMatched | memory |
| kernel.memory.write | memory_write | Memory WRITE | IntentMatched | memory |
| kernel.knowledge.read | knowledge | Knowledge READ | IntentMatched | knowledge |
| kernel.knowledge.write | knowledge_write | Knowledge WRITE | OnDemand | knowledge |
| kernel.ask_user.ask | ask_user | KernelDomain{"ask_user"} EXECUTE | IntentMatched | user-interaction |
| kernel.persona.update | persona | KernelDomain{"persona"} WRITE | ApprovalOnly | kernel-mutate |
| kernel.project.update | project | KernelDomain{"space"} WRITE | ApprovalOnly | kernel-mutate |
| kernel.agent.update | kernel_agent | KernelDomain{"agent"} WRITE | ApprovalOnly | kernel-mutate |
| kernel.cron.update | cron | KernelDomain{"cron"} WRITE | ApprovalOnly | kernel-mutate |
| kernel.security.query | security | KernelDomain{"security"} READ | OnDemand | kernel-observe |
| kernel.budget.query | budget | KernelDomain{"budget"} READ | OnDemand | kernel-observe |
| kernel.resource.query | resource | KernelDomain{"resource"} READ | OnDemand | kernel-observe |
| kernel.a2a.delegate | a2a_delegate | A2a EXECUTE | IntentMatched | delegation |
| kernel.a2a.send | a2a_send | A2a EXECUTE | IntentMatched | delegation |
| kernel.a2a.query | a2a_query | A2a EXECUTE | IntentMatched | delegation |

(Long-tail bridge-only tools — mount/task/marketplace/calendar/memo/timeline/email/image_gen/screenshot/skill_forge — get descriptors in Task 9 when their registration arms are added.)

- [ ] **Step 1: Write failing tests** in `descriptor.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_requirements_nonempty() {
        let catalog = kernel_catalog();
        assert!(catalog.len() >= 28);
        let mut ids: Vec<_> = catalog.iter().map(|d| d.id.to_string()).collect();
        let total = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), total, "duplicate ToolId in catalog");
        for d in catalog {
            assert!(
                !d.required_capabilities.is_empty(),
                "{} must declare required_capabilities",
                d.id
            );
            assert!(!d.registered_name.is_empty());
        }
    }

    #[test]
    fn find_descriptor_resolves_by_id() {
        let id = ToolId::new("kernel.fs.read");
        let d = find_descriptor(&id).expect("kernel.fs.read in catalog");
        assert_eq!(d.registered_name, "read");
        assert!(matches!(d.activation_class, ActivationClass::Always));
    }
}
```

- [ ] **Step 2: Run** `cargo nextest run -p oxios-kernel catalog_ids_are_unique` → FAIL (module missing).
- [ ] **Step 3: Implement** descriptor.rs with the types above and the static table (a `static CATALOG: &[ToolDescriptor] = &[...]`; construct `ResourceRef` values inline; `Rights::READ` etc. are `const fn`-callable or use `Rights(Rights::READ.bits())` — check `types.rs` bitflags usage in template.rs and follow that pattern). Add `pub mod descriptor;` to capability/mod.rs.
- [ ] **Step 4: Run** both tests → PASS. Then `cargo check -p oxios-kernel --all-features` → clean.
- [ ] **Step 5: Commit** `feat(kernel): tool descriptor vocabulary and kernel catalog`

---

### Task 3: Tool profiles — selectors, ceilings, validation

**Files:**
- Create: `crates/oxios-kernel/src/capability/profile.rs`
- Modify: `crates/oxios-kernel/src/capability/mod.rs` (`pub mod profile;`)

**Interfaces:**
- Consumes: Task 2 descriptor types, Task 1 `ResourceRef`.
- Produces (used by Tasks 4, 5, 11):

```rust
pub struct ProfileRevision(pub u64);
pub struct ToolProfileRef { pub id: Arc<str>, pub revision: ProfileRevision }

pub enum ToolSelector {
    Tool(ToolId),
    ProviderTag { provider: ToolProviderId, tag: ToolTag },
}

pub struct CapabilityRequest { pub resource: ResourceSelector, pub rights: Rights }

pub enum ResourceSelector {
    KernelDomain(Arc<str>),
    Skill(Arc<str>),
    Space(uuid::Uuid),
    Agent(crate::types::AgentId),
    Exec { mode: Arc<str> },
    Browser,
    A2a,
    Mcp { server: Arc<str> },
    Fs,
    WebSearch,
    Memory,
    Knowledge,
}

impl ResourceSelector {
    /// Compile to the typed ResourceRef this selector governs.
    pub fn to_resource_ref(&self) -> ResourceRef;
    /// True when `req.resource` requirements of a descriptor are within this selector's scope.
    pub fn covers(&self, resource: &ResourceRef) -> bool;
}

pub enum ProviderCompatibilityPolicy {
    ExactContract(ToolContractVersion),
    CompatibleMajor { contract: Arc<str>, major: u32 },
}

pub struct DynamicProviderContract {
    pub provider: ToolProviderId,
    pub selector: ProviderToolSelector,
    pub capability_ceiling: Vec<CapabilityRequest>,
    pub compatibility: ProviderCompatibilityPolicy,
}

pub enum ProviderToolSelector { AllTools, Tag(ToolTag) }

pub struct ToolActivationPolicy {
    pub max_active_tools: usize,
    pub required_core: Vec<ToolId>,
    pub sticky_turns: u32,
}

pub struct ToolProfileSpec {
    pub id: Arc<str>,
    pub revision: ProfileRevision,
    pub extends: Vec<ToolProfileRef>,
    pub capability_ceiling: Vec<CapabilityRequest>,
    pub include: Vec<ToolSelector>,
    pub exclude: Vec<ToolSelector>,
    pub dynamic_providers: Vec<DynamicProviderContract>,
    pub activation: ToolActivationPolicy,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("tool {0} is not in the catalog")]
    UnknownTool(ToolId),
    #[error("capability requirement of {tool} exceeds the profile ceiling: {req}")]
    CeilingExceeded { tool: ToolId, req: String },
    #[error("required core tool {0} is not selected")]
    CoreNotSelected(ToolId),
    #[error("required core ({0} tools) exceeds max_active_tools {1}")]
    CoreTooLarge(usize, usize),
}
```

`covers` semantics: `Fs` covers `ResourceRef::Fs`; `WebSearch` covers `ResourceRef::WebSearch`; `Memory`/`Knowledge` likewise; `KernelDomain(name)` covers `ResourceRef::KernelDomain{domain}` with equal name; `Exec{mode}` covers `Exec{mode}` equal; `Browser` covers `Browser`; `A2a` covers `A2a`; `Mcp{s}` covers `Mcp{server}` equal or `s == "*"`; `Skill`/`Space`/`Agent` analogous.

Key functions:

```rust
impl ToolProfileSpec {
    /// Resolved post-exclusion selected set: (include ∪ dynamic) − exclude.
    /// `exclude` always wins; evaluation is set-based, order-independent.
    pub fn select_tools(&self, catalog: &[ToolDescriptor]) -> Vec<&'static ToolDescriptor>;

    /// Publish-time validation per design §4.2/§4.3/§5.1 step 4.
    pub fn validate_publish(&self, catalog: &[ToolDescriptor]) -> Result<(), ProfileError>;
}
```

`select_tools` implementation shape: expand each `ToolSelector` against the catalog (exact ID match, or provider+tag match), union, then remove everything matched by any exclude selector. `validate_publish`: for each selected descriptor and each `CapabilityRequirement`, there must exist a ceiling `CapabilityRequest` whose `resource.covers(&req.resource)` and whose `rights.contains(req.rights)`; `required_core` ⊆ selected IDs; `required_core.len() <= max_active_tools`.

- [ ] **Step 1: Failing tests** (profile.rs `mod tests`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::{kernel_catalog, ActivationClass, ToolId};

    fn sel(id: &str) -> ToolSelector { ToolSelector::Tool(ToolId::new(id)) }

    #[test]
    fn exclude_always_wins_over_include() {
        let spec = ToolProfileSpec {
            id: "test".into(), revision: ProfileRevision(1), extends: vec![],
            capability_ceiling: vec![
                CapabilityRequest { resource: ResourceSelector::Fs, rights: Rights::READ | Rights::WRITE },
            ],
            include: vec![
                sel("kernel.fs.read"), sel("kernel.fs.write"),
            ],
            exclude: vec![sel("kernel.fs.write")],
            dynamic_providers: vec![],
            activation: ToolActivationPolicy { max_active_tools: 16, required_core: vec![], sticky_turns: 2 },
        };
        let selected = spec.select_tools(kernel_catalog());
        let ids: Vec<_> = selected.iter().map(|d| d.id.to_string()).collect();
        assert!(ids.contains(&"kernel.fs.read".to_string()));
        assert!(!ids.contains(&"kernel.fs.write".to_string()), "exclude must win");
    }

    #[test]
    fn validate_rejects_ceiling_violation_and_missing_core() {
        let mut spec = ToolProfileSpec {
            id: "t".into(), revision: ProfileRevision(1), extends: vec![],
            // Fs READ only — but exec is included → its Exec EXECUTE req exceeds ceiling.
            capability_ceiling: vec![
                CapabilityRequest { resource: ResourceSelector::Fs, rights: Rights::READ },
            ],
            include: vec![sel("kernel.fs.read"), sel("kernel.exec.run")],
            exclude: vec![],
            dynamic_providers: vec![],
            activation: ToolActivationPolicy { max_active_tools: 16, required_core: vec![ToolId::new("kernel.browse.open")], sticky_turns: 0 },
        };
        let err = spec.validate_publish(kernel_catalog()).unwrap_err();
        assert!(matches!(err, ProfileError::CeilingExceeded { .. }), "got: {err}");

        spec.capability_ceiling.push(CapabilityRequest {
            resource: ResourceSelector::Exec { mode: "shell".into() },
            rights: Rights::EXECUTE,
        });
        let err = spec.validate_publish(kernel_catalog()).unwrap_err();
        assert!(matches!(err, ProfileError::CoreNotSelected(_)), "got: {err}");

        spec.include.push(sel("kernel.browse.open"));
        spec.capability_ceiling.push(CapabilityRequest { resource: ResourceSelector::Browser, rights: Rights::EXECUTE });
        spec.validate_publish(kernel_catalog()).expect("valid after fixes");
    }
}
```

- [ ] **Step 2: Run** `cargo nextest run -p oxios-kernel exclude_always_wins` → FAIL.
- [ ] **Step 3: Implement** profile.rs per interfaces. Use `thiserror` (already a kernel dep).
- [ ] **Step 4: Run tests + `cargo check -p oxios-kernel --all-features`** → PASS/clean.
- [ ] **Step 5: Commit** `feat(kernel): tool profile specs with selector precedence and ceiling validation`

---

### Task 4: ExecutionProfileResolver

**Files:**
- Create: `crates/oxios-kernel/src/capability/resolver.rs`
- Modify: `capability/mod.rs` (`pub mod resolver;`)

**Interfaces:**
- Consumes: Tasks 1-3.
- Produces (used by Tasks 5, 7, 11):

```rust
pub enum UnavailabilityReason {
    CeilingExceeded,
    MissingCapability { resource: ResourceRef, rights: Rights },
    ProviderUnavailable,
}

pub enum ResolutionStatus {
    Active,
    AvailableOnDemand,
    RequiresApproval,
    Unavailable(UnavailabilityReason),
}

pub struct ResolvedTool {
    pub descriptor: &'static ToolDescriptor,
    pub status: ResolutionStatus,
}

pub struct ExecutionProfileFingerprint(pub u64);

pub struct ResolvedExecutionProfile {
    pub profile_id: Arc<str>,
    pub profile_revision: ProfileRevision,
    pub agent_id: crate::types::AgentId,
    pub tools: Vec<ResolvedTool>,
    /// requested ∩ bounding, as kernel-issued capabilities.
    pub effective_cspace: CSpace,
    pub fingerprint: ExecutionProfileFingerprint,
}

impl ResolvedExecutionProfile {
    pub fn active_tool_descriptors(&self) -> Vec<&'static ToolDescriptor>;
    pub fn active_registered_names(&self) -> Vec<String>;
}

/// Resolve a profile against a catalog and a bounding authority.
///
/// Classification (design §5.1):
/// * requirement outside ceiling → Unavailable(CeilingExceeded), never approval-eligible;
/// * all requirements within effective CSpace → Active for Always/IntentMatched
///   descriptors, AvailableOnDemand for OnDemand, RequiresApproval for ApprovalOnly;
/// * requirement satisfiable from `approval_grantable` (predicate over ResourceRef+Rights,
///   default None) but absent from the effective CSpace → RequiresApproval;
/// * otherwise → Unavailable(MissingCapability).
pub fn resolve(
    profile: &ToolProfileSpec,
    bounding: &CSpace,
    agent_id: crate::types::AgentId,
    approval_grantable: Option<&dyn Fn(&ResourceRef, Rights) -> bool>,
) -> ResolvedExecutionProfile;
```

Resolution algorithm (implement exactly):
1. `selected = profile.select_tools(kernel_catalog())`.
2. `bounding_can(res, rights) = bounding.can(res, rights)`.
3. For each selected descriptor, for each `CapabilityRequirement`:
   - ceiling check: some ceiling entry covers resource AND rights ⊇ required → else status = Unavailable(CeilingExceeded), skip remaining reqs.
   - authority check: `bounding_can(req.resource, req.rights)` → if false: approval-eligible iff `approval_grantable` predicate says true → RequiresApproval, else Unavailable(MissingCapability).
4. If all reqs satisfied → status by `activation_class` (Always|IntentMatched → Active, OnDemand → AvailableOnDemand, ApprovalOnly → RequiresApproval).
5. `effective_cspace`: for every tool that reached Active/AvailableOnDemand/RequiresApproval, insert `Capability::kernel(req.resource.clone(), req.rights)` for each satisfied requirement into a fresh `CSpace::new(agent_id)`. (RequiresApproval tools enter the CSpace only after approval lands — a later re-resolve produces them as Active/OnDemand; do not insert their caps here.)
6. Fingerprint: FNV-1a 64 over canonical string: `profile_id|revision|` + sorted `id:contract_major.minor:status_discriminant` + sorted effective capability strings (`resource.to_string():rights.bits()`).

```rust
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |h, b| (h ^ *b as u64).wrapping_mul(FNV_PRIME))
}
```

- [ ] **Step 1: Failing tests**:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::{kernel_catalog, ToolId};
    use crate::capability::profile::*;
    use crate::capability::{CSpace, Capability, ResourceRef, Rights};
    use crate::types::AgentId;

    fn worker_bounding(agent_id: AgentId) -> CSpace {
        let mut c = CSpace::new(agent_id);
        c.insert(Capability::kernel(ResourceRef::Fs, Rights::READ | Rights::WRITE));
        c.insert(Capability::kernel(ResourceRef::WebSearch, Rights::EXECUTE));
        c.insert(Capability::kernel(ResourceRef::Exec { mode: "shell".into() }, Rights::EXECUTE));
        c.insert(Capability::kernel(ResourceRef::Browser, Rights::EXECUTE));
        c.insert(Capability::kernel(ResourceRef::Memory, Rights::READ | Rights::WRITE));
        c.insert(Capability::kernel(ResourceRef::Knowledge, Rights::READ));
        c.insert(Capability::kernel(ResourceRef::KernelDomain { domain: "ask_user".into() }, Rights::EXECUTE));
        c
    }

    fn coding_like_profile() -> ToolProfileSpec {
        ToolProfileSpec {
            id: "test-coding".into(), revision: ProfileRevision(1), extends: vec![],
            capability_ceiling: vec![
                CapabilityRequest { resource: ResourceSelector::Fs, rights: Rights::READ | Rights::WRITE },
                CapabilityRequest { resource: ResourceSelector::WebSearch, rights: Rights::EXECUTE },
                CapabilityRequest { resource: ResourceSelector::Exec { mode: "shell".into() }, rights: Rights::EXECUTE },
                CapabilityRequest { resource: ResourceSelector::Browser, rights: Rights::EXECUTE },
                CapabilityRequest { resource: ResourceSelector::Memory, rights: Rights::READ | Rights::WRITE },
                CapabilityRequest { resource: ResourceSelector::Knowledge, rights: Rights::READ },
                CapabilityRequest { resource: ResourceSelector::KernelDomain("ask_user".into()), rights: Rights::EXECUTE },
            ],
            include: vec![
                ToolSelector::Tool(ToolId::new("kernel.fs.read")),
                ToolSelector::Tool(ToolId::new("kernel.fs.write")),
                ToolSelector::Tool(ToolId::new("kernel.web.search")),
                ToolSelector::Tool(ToolId::new("kernel.exec.run")),
                ToolSelector::Tool(ToolId::new("kernel.memory.read")),
                ToolSelector::Tool(ToolId::new("kernel.memory.write")),
                ToolSelector::Tool(ToolId::new("kernel.ask_user.ask")),
            ],
            exclude: vec![],
            dynamic_providers: vec![],
            activation: ToolActivationPolicy { max_active_tools: 16, required_core: vec![], sticky_turns: 2 },
        }
    }

    #[test]
    fn active_tools_land_in_cspace_and_names() {
        let agent = AgentId::new_v4();
        let resolved = resolve(&coding_like_profile(), &worker_bounding(agent), agent, None);
        let names = resolved.active_registered_names();
        for expected in ["read", "write", "web_search", "exec", "memory_read", "memory_write"] {
            assert!(names.contains(&expected.to_string()), "missing {expected} in {names:?}");
        }
        assert!(names.contains(&"ask_user".to_string())); // IntentMatched → Active
        assert!(resolved.effective_cspace.can(&ResourceRef::Fs, Rights::WRITE));
    }

    #[test]
    fn ceiling_exceeded_is_unavailable_even_with_authority() {
        let agent = AgentId::new_v4();
        let mut p = coding_like_profile();
        p.capability_ceiling.retain(|c| !matches!(c.resource, ResourceSelector::Exec { .. }));
        let resolved = resolve(&p, &worker_bounding(agent), agent, None);
        let exec = resolved.tools.iter().find(|t| t.descriptor.registered_name == "exec").unwrap();
        assert!(matches!(exec.status, ResolutionStatus::Unavailable(UnavailabilityReason::CeilingExceeded)));
    }

    #[test]
    fn missing_authority_is_unavailable_not_escalated() {
        let agent = AgentId::new_v4();
        let mut bounding = worker_bounding(agent);
        bounding.retain(|c| c.resource != ResourceRef::Memory);
        let resolved = resolve(&coding_like_profile(), &bounding, agent, None);
        for t in &resolved.tools {
            if t.descriptor.registered_name.starts_with("memory") {
                assert!(matches!(t.status, ResolutionStatus::Unavailable(UnavailabilityReason::MissingCapability { .. })));
            }
        }
    }

    #[test]
    fn fingerprint_is_stable_and_input_sensitive() {
        let agent = AgentId::new_v4();
        let a = resolve(&coding_like_profile(), &worker_bounding(agent), agent, None);
        let b = resolve(&coding_like_profile(), &worker_bounding(agent), agent, None);
        assert_eq!(a.fingerprint, b.fingerprint);
        let mut p2 = coding_like_profile();
        p2.exclude.push(ToolSelector::Tool(ToolId::new("kernel.exec.run")));
        let c = resolve(&p2, &worker_bounding(agent), agent, None);
        assert_ne!(a.fingerprint, c.fingerprint);
    }
}
```

- [ ] **Step 2: Run** `cargo nextest run -p oxios-kernel fingerprint_is_stable` → FAIL.
- [ ] **Step 3: Implement** resolver.rs. `CSpace::retain` exists (types.rs:337). `Rights` needs `.bits()` — if not public, use `format!("{:?}", req.rights)` in the canonical string instead.
- [ ] **Step 4: Run all four tests** → PASS; `cargo check -p oxios-kernel --all-features` clean.
- [ ] **Step 5: Commit** `feat(kernel): execution profile resolver with authority attenuation`

---

### Task 5: Built-in profile presets (incl. legacy template equivalents)

**Files:**
- Create: `crates/oxios-kernel/src/capability/builtin_profiles.rs`
- Modify: `capability/mod.rs` (`pub mod builtin_profiles;`)

**Interfaces:**
- Consumes: Tasks 2-4 types.
- Produces:
  - `pub fn builtin_profile(name: &str) -> Option<&'static ToolProfileSpec>` for: `"coding"`, `"code-review"`, `"research"`, `"writing"`, `"advisory"`, `"security-audit"`, `"operations"`, plus legacy `"worker"`, `"standard"`, `"operator"`, `"supervisor"`.
  - `pub fn builtin_profile_names() -> Vec<&'static str>`

Preset compositions (from design §6; `include` uses `ProviderTag` selectors against the catalog where the tag covers exactly the primitive — tags: `fs`, `web`, `exec`, `browser`, `memory`, `knowledge`, `user-interaction`, `delegation`, `kernel-observe`, `kernel-mutate`):

| preset | include selectors | ceiling |
|---|---|---|
| worker (legacy ≈ current worker template) | tags: fs, web, exec, browser | Fs R\|W, WebSearch X, Exec shell X, Browser X |
| standard (legacy) | worker + tags: memory, knowledge, user-interaction, delegation | worker + Memory R\|W, Knowledge R\|W, KernelDomain ask_user X, A2a X |
| operator (legacy) | standard + tags: kernel-observe, kernel-mutate | standard + KernelDomain {persona,space,agent,cron,security,budget,resource} R\|W |
| supervisor (legacy) | operator + KernelDomain{a2a} X (already), i.e. operator set | operator ceiling |
| coding | tags: fs, web, exec, browser, memory, knowledge, user-interaction, delegation | as standard |
| code-review | tags: fs, web, browser, memory, knowledge, user-interaction | Fs R only, WebSearch X, Browser X, Memory R, Knowledge R, ask_user X |
| research | tags: fs, web, browser, memory, knowledge, user-interaction | Fs R, WebSearch X, Browser X, Memory R, Knowledge R, ask_user X |
| writing | tags: fs, web, memory, knowledge, user-interaction | Fs R\|W, WebSearch X, Memory R\|W, Knowledge R\|W, ask_user X |
| advisory | tags: fs, memory, knowledge, user-interaction | Fs R, Memory R, Knowledge R, ask_user X |
| security-audit | tags: fs, web, memory, knowledge, user-interaction, kernel-observe + Tool(kernel.exec.run) excluded via `exclude` NOT set — instead include `kernel.security.query` explicitly; system-observe covered by kernel-observe tag | Fs R, WebSearch X, Memory R, Knowledge R, ask_user X, KernelDomain{security,budget,resource} R |
| operations | tags: fs, web, exec, memory, knowledge, user-interaction, kernel-observe + explicit Tools: kernel.cron.update, kernel.project.update, kernel.agent.update | Fs R\|W, WebSearch X, Exec shell X, Memory R\|W, Knowledge R, ask_user X, KernelDomain{cron,space,agent} W, KernelDomain{security,budget,resource} R |

Every preset: `activation: ToolActivationPolicy { max_active_tools: 16, required_core: vec![ToolId::new("kernel.fs.read"), ToolId::new("kernel.fs.grep")], sticky_turns: 2 }`, `revision: ProfileRevision(1)`, `extends: vec![]`, `dynamic_providers: vec![]`.

- [ ] **Step 1: Failing tests**:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::kernel_catalog;
    use crate::capability::resolver::resolve;
    use crate::capability::{CSpace, Capability, ResourceRef, Rights};
    use crate::types::AgentId;

    #[test]
    fn all_presets_pass_publish_validation() {
        for name in builtin_profile_names() {
            let p = builtin_profile(name).unwrap();
            p.validate_publish(kernel_catalog())
                .unwrap_or_else(|e| panic!("preset {name} invalid: {e}"));
        }
    }

    #[test]
    fn code_review_has_no_write_and_no_exec() {
        let agent = AgentId::new_v4();
        let mut bounding = CSpace::new(agent);
        for (r, rights) in [
            (ResourceRef::Fs, Rights::READ | Rights::WRITE),
            (ResourceRef::WebSearch, Rights::EXECUTE),
            (ResourceRef::Browser, Rights::EXECUTE),
            (ResourceRef::Exec { mode: "shell".into() }, Rights::EXECUTE),
            (ResourceRef::Memory, Rights::READ | Rights::WRITE),
            (ResourceRef::Knowledge, Rights::READ | Rights::WRITE),
            (ResourceRef::KernelDomain { domain: "ask_user".into() }, Rights::EXECUTE),
        ] { bounding.insert(Capability::kernel(r, rights)); }

        let resolved = resolve(builtin_profile("code-review").unwrap(), &bounding, agent, None);
        let names = resolved.active_registered_names();
        assert!(names.contains(&"read".to_string()));
        assert!(!names.contains(&"write".to_string()), "code-review must not write: {names:?}");
        assert!(!names.contains(&"edit".to_string()));
        assert!(!names.contains(&"exec".to_string()));
        assert!(names.contains(&"memory_read".to_string()));
        assert!(!names.contains(&"memory_write".to_string()));
    }

    #[test]
    fn worker_profile_replaces_legacy_template_surface() {
        let agent = AgentId::new_v4();
        let mut bounding = CSpace::new(agent);
        bounding.insert(Capability::kernel(ResourceRef::Fs, Rights::READ | Rights::WRITE));
        bounding.insert(Capability::kernel(ResourceRef::WebSearch, Rights::EXECUTE));
        bounding.insert(Capability::kernel(ResourceRef::Exec { mode: "shell".into() }, Rights::EXECUTE));
        bounding.insert(Capability::kernel(ResourceRef::Browser, Rights::EXECUTE));

        let resolved = resolve(builtin_profile("worker").unwrap(), &bounding, agent, None);
        let names = resolved.active_registered_names();
        for expected in ["read", "write", "edit", "grep", "find", "ls", "web_search", "get_search_results", "exec"] {
            assert!(names.contains(&expected.to_string()), "worker missing {expected}: {names:?}");
        }
    }

    #[test]
    fn operations_gets_scoped_kernel_mutation_not_persona() {
        let agent = AgentId::new_v4();
        let p = builtin_profile("operations").unwrap();
        let mut bounding = CSpace::new(agent);
        for (r, rights) in [
            (ResourceRef::Fs, Rights::READ | Rights::WRITE),
            (ResourceRef::WebSearch, Rights::EXECUTE),
            (ResourceRef::Exec { mode: "shell".into() }, Rights::EXECUTE),
            (ResourceRef::Memory, Rights::READ | Rights::WRITE),
            (ResourceRef::Knowledge, Rights::READ),
            (ResourceRef::KernelDomain { domain: "ask_user".into() }, Rights::EXECUTE),
            (ResourceRef::KernelDomain { domain: "cron".into() }, Rights::WRITE),
            (ResourceRef::KernelDomain { domain: "space".into() }, Rights::WRITE),
            (ResourceRef::KernelDomain { domain: "agent".into() }, Rights::WRITE),
            (ResourceRef::KernelDomain { domain: "persona".into() }, Rights::WRITE),
        ] { bounding.insert(Capability::kernel(r, rights)); }

        let resolved = resolve(p, &bounding, agent, None);
        let status_of = |name: &str| resolved.tools.iter().find(|t| t.descriptor.registered_name == name).map(|t| &t.status);
        // cron/project/agent mutate: ApprovalOnly class → RequiresApproval (authorized, gated).
        assert!(matches!(status_of("cron"), Some(ResolutionStatus::RequiresApproval)));
        // persona is NOT included by operations → not even present.
        assert!(status_of("persona").is_none());
    }
}
```

(`ResolutionStatus` import comes from resolver module.)

- [ ] **Step 2: Run** `cargo nextest run -p oxios-kernel all_presets_pass` → FAIL.
- [ ] **Step 3: Implement** statics via a `fn make(...) -> ToolProfileSpec` helper + `static CODEING: OnceLock<ToolProfileSpec>` or plain `lazy` consts — simplest: `pub fn builtin_profile` matches on name and returns `&'static` from `Box::leak`-free statics built with `const fn`-unfriendly types is hard; use `std::sync::OnceLock<[ToolProfileSpec; 11]>` and return references into it.
- [ ] **Step 4: Run tests + check** → PASS/clean.
- [ ] **Step 5: Commit** `feat(kernel): builtin tool profile presets incl. legacy template equivalents`

---

### Task 6: Registration from resolved profile

**Files:**
- Modify: `crates/oxios-kernel/src/tools/registration.rs` (append; keep existing fns)

**Interfaces:**
- Consumes: `ResolvedExecutionProfile` (Task 4); existing constructors visible in registration.rs imports.
- Produces:

```rust
#[allow(clippy::too_many_arguments)]
pub fn register_from_resolved_profile(
    registry: &ToolRegistry,
    kernel: &KernelHandle,
    resolved: &crate::capability::resolver::ResolvedExecutionProfile,
    search_cache: Arc<SearchCache>,
    agent_id: AgentId,
    gate: Arc<AccessGate>,
    context: AgentContext,
    approval_gate: Option<Arc<crate::approval::ApprovalGate>>,
    event_bus: Option<crate::event_bus::EventBus>,
    pending_approvals: Option<Arc<crate::tools::PendingToolApprovals>>,
    pending_path_access: Option<Arc<crate::tools::PendingPathAccess>>,
);
```

Body: for each `ResolvedTool` with status `Active` or `AvailableOnDemand` (v1 registers both; progressive activation lands in a later plan), match `descriptor.id.to_string().as_str()`:

- `kernel.fs.read|grep|find|ls` → `registry.register(GatedTool::with_approval(ReadTool::new() /* GrepTool/FindTool/LsTool */, ...))` — same wrapping as `register_always_on_gated` bodies (copy the exact `GatedTool::with_approval(...)` call shape from lines 103-111).
- `kernel.fs.write|edit` → WriteTool/EditTool gated likewise.
- `kernel.web.search` → WebSearchTool gated; `kernel.web.results` → GetSearchResultsTool gated.
- `kernel.exec.run` → GatedTool(ExecTool::from_kernel_with_context(kernel, context.clone())) as lines 322-331.
- `kernel.browse.*` → `register_browser_tools(kernel, registry)`.
- `kernel.memory.read|search|write` → `MemoryReadTool::from_kernel(kernel)` / `MemorySearchTool::from_kernel(kernel)` / `MemoryWriteTool::from_kernel(kernel)` (verify constructor names in `tools/builtin/` — they exist per bridge path; if a constructor differs, follow the actual one).
- `kernel.knowledge.read|write` → `KnowledgeTool::from_kernel(kernel)` / knowledge write tool if present else skip with a `tracing::warn!`.
- `kernel.ask_user.ask` → AskUserTool per builtin constructor (verify in builtin/mod.rs registration block).
- `kernel.persona.update` → `PersonaTool::from_kernel(kernel)`; `kernel.project.update` → `ProjectTool::from_kernel(kernel)`; `kernel.agent.update` → `KernelAgentTool::from_kernel(kernel)`; `kernel.cron.update` → `CronTool::from_kernel(kernel)`; `kernel.security.query|budget.query|resource.query` → SecurityTool/BudgetTool/ResourceTool.
- `kernel.a2a.delegate|send|query` → the three A2A tools with `agent_id` (lines 344-348 shape).
- `_ => tracing::warn!(id, "no registration arm for catalog tool")`.

- [ ] **Step 1: Failing test** (registration.rs tests): construct a resolved profile via `resolve(builtin_profile("worker")...)` with a bounding CSpace as in Task 5's worker test, call `register_from_resolved_profile` with `gate = Arc::new(AccessGate::new(...))` — check how existing tests construct AccessGate/AgentContext (search `AgentContext` in kernel tests); if heavyweight, build via `AccessGate::default()`-style path used elsewhere. Assert `registry.names()` contains the 9 worker names and NOT `memory_read`.

```rust
#[test]
fn register_from_resolved_worker_profile_registers_nine_tools() {
    // Build bounding CSpace + resolved profile exactly as Task 5's worker test.
    // Then:
    register_from_resolved_profile(&registry, &kernel, &resolved, cache, agent, gate, context, None, None, None, None);
    let names = registry.names();
    for expected in ["read","write","edit","grep","find","ls","web_search","get_search_results","exec"] {
        assert!(names.contains(&expected.to_string()));
    }
    assert!(!names.contains(&"memory_read".to_string()));
}
```

Kernel construction: find the lightest existing pattern (`KernelHandle` in unit tests — check `tools/kernel_bridge.rs` test or `builtin/mod.rs` tests for how a KernelHandle is built for registration tests; reuse it).

- [ ] **Step 2: Run** → FAIL (fn missing).
- [ ] **Step 3: Implement**; iterate on real constructor signatures empirically (`cargo check -p oxios-kernel` errors are the source of truth).
- [ ] **Step 4: Run** → PASS; full `cargo nextest run -p oxios-kernel` (no regressions in existing tests).
- [ ] **Step 5: Commit** `feat(kernel): registration from resolved execution profile`

---

### Task 7: Access gate + RBAC alignment

**Files:**
- Modify: `crates/oxios-kernel/src/access_manager/gate.rs` (`check_tool`, lines 308-352)
- Modify: `crates/oxios-kernel/src/access_manager/rbac.rs` (`Role::User` default policy, lines 58-74)
- Test: both files' test modules

**Changes:**
1. `check_tool` maps the tool name to the proper ResourceRef before the Layer-0 check (design: skip list dies, CSpace is the single grant source):

```rust
let resource = match tool {
    "read" | "write" | "edit" | "grep" | "find" | "ls" => ResourceRef::Fs,
    "web_search" | "get_search_results" => ResourceRef::WebSearch,
    "memory_read" | "memory_search" | "memory_write" => ResourceRef::Memory,
    "knowledge" | "knowledge_write" => ResourceRef::Knowledge,
    other => ResourceRef::KernelDomain { domain: other.to_string() },
};
```

Then require: Fs → `Rights::READ` (write/edit additionally need WRITE — split: `"write" | "edit" => ResourceRef::Fs` with required `Rights::WRITE`; others READ); WebSearch/Memory/Knowledge/KernelDomain → the right the tool needs (read-class → READ, write-class → WRITE, others EXECUTE). Delete the `always_on` array and its catch-22 comment block entirely.
2. rbac.rs `Role::User` `allowed_actions` += `Action::UseTool("ls".into())`, `Action::UseTool("web_search".into())`, `Action::UseTool("get_search_results".into())`.

- [ ] **Step 1: Failing tests:**

gate.rs tests:
```rust
#[test]
fn layer0_denies_web_search_without_cspace_grant() {
    // Build AgentContext with empty CSpace (follow existing gate tests' construction).
    // check_tool must now DENY "web_search" (skip list removed).
}
#[test]
fn layer0_allows_web_search_with_websearch_grant() {
    // CSpace with Capability::kernel(ResourceRef::WebSearch, Rights::EXECUTE) → allow.
}
```

rbac.rs test:
```rust
#[test]
fn user_default_policy_allows_web_tools() {
    let policy = Role::User.default_policy();
    for t in ["ls", "web_search", "get_search_results"] {
        assert!(policy.allowed_actions.contains(&Action::UseTool(t.into())), "{t}");
    }
}
```

- [ ] **Step 2: Run** → FAIL. **Step 3: Implement.** **Step 4: Run gate+rbac tests and FULL `cargo nextest run -p oxios-kernel`** — existing tests relying on the skip list must be updated to grant the capability explicitly (that is the point: no silent baseline). **Step 5: Commit** `feat(kernel): layer-0 CSpace mapping replaces always-on skip list`

---

### Task 8: agent_runtime cutover + prompt-from-registry

**Files:**
- Modify: `crates/oxios-kernel/src/agent_runtime.rs` (resolution site ~376-388; registration call ~949; prompt section ~1788-1791)
- Modify: `crates/oxios-kernel/src/persona/mod.rs` (Persona struct: `#[serde(default)] pub tool_profile: Option<crate::capability::profile::ToolProfileRef>`; update `Default` impl + all struct literals — `default_personas()` assigns: dev→coding, review→code-review, research→research, writer→writing, architect/mentor/planner→advisory, security→security-audit, ops→operations; custom create paths default `None`)
- Test: agent_runtime.rs tests + persona tests

**Changes:**
1. At the resolution site, new precedence:

```rust
let active = self.persona_manager.as_ref().and_then(|pm| pm.get_active_persona());
let profile_override = cspace_hint.and_then(|h| crate::capability::builtin_profiles::builtin_profile(h.trim()));
let persona_profile = active.as_ref()
    .and_then(|p| p.tool_profile.as_ref())
    .and_then(|r| crate::capability::builtin_profiles::builtin_profile(&r.id));
let legacy_role_profile = active.as_ref()
    .map(|p| p.role.trim().to_lowercase())
    .and_then(|r| crate::capability::builtin_profiles::builtin_profile(&r));
let profile = profile_override.or(persona_profile).or(legacy_role_profile)
    .unwrap_or_else(|| crate::capability::builtin_profiles::builtin_profile("worker").unwrap());
let resolved = crate::capability::resolver::resolve(profile, &bounding, agent_id, None);
```

`bounding` = the existing `resolve_cspace(...)` result repurposed as the bounding authority (compat adapter per design §17 step 1 — templates still seed the bounding CSpace; profiles attenuate). Keep the existing `resolve_cspace` call for the bounding, then resolve the profile against it.
2. Replace the `register_tools_from_cspace_gated(...)` call (~949) with `register_from_resolved_profile(...)` passing the resolved profile.
3. Delete the hardcoded "Your tools" advertisement (~1788-1791); generate from the registry after registration:

```rust
fn generate_tool_section(names: &[String]) -> String {
    use std::fmt::Write;
    let mut s = String::from("\n\n## Available tools\n");
    for n in names { let _ = write!(s, "- `{n}`\n"); }
    s
}
```

Append to the system prompt at the point the old section lived (consume `registry.names()` post-registration; thread the string into the prompt build or append before AgentConfig assembly).

- [ ] **Step 1: Failing tests** (agent_runtime tests are integration-heavy; add a focused unit test for `generate_tool_section` and a persona test):

```rust
#[test]
fn generate_tool_section_lists_exactly_registered_names() {
    let s = generate_tool_section(&["read".into(), "exec".into()]);
    assert!(s.contains("`read`"));
    assert!(s.contains("`exec`"));
    assert!(!s.contains("`write`"));
}

// persona/mod.rs tests
#[test]
fn default_personas_reference_builtin_profiles() {
    let personas = default_personas();
    let dev = personas.iter().find(|p| p.id == "dev").unwrap();
    assert_eq!(dev.tool_profile.as_ref().unwrap().id.as_ref(), "coding");
    let sec = personas.iter().find(|p| p.id == "security").unwrap();
    assert_eq!(sec.tool_profile.as_ref().unwrap().id.as_ref(), "security-audit");
}
```

- [ ] **Step 2: FAIL → Step 3: implement → Step 4:** `cargo nextest run -p oxios-kernel` full suite + `cargo check -p oxios-kernel --all-features`. **Step 5: Commit** `feat(kernel): agent runtime resolves via execution profiles; prompt from registry`

---

### Task 9: Bridge deletion + long-tail registration arms

**Files:**
- Delete: `crates/oxios-kernel/src/tools/kernel_bridge.rs`
- Modify: `crates/oxios-kernel/src/tools/mod.rs` (remove `pub mod kernel_bridge;` / re-exports)
- Modify: `crates/oxios-kernel/src/tools/builtin/mod.rs` (delete `register_all_kernel_tools` + its test; unique tool registrations move to registration arms)
- Modify: `crates/oxios-kernel/src/capability/descriptor.rs` (add descriptors for the long-tail tools)
- Modify: `crates/oxios-kernel/src/tools/registration.rs` (add arms)

**Long-tail descriptors + arms** (constructor per builtin/mod.rs's `register_all_kernel_tools` body — read it first, copy exact constructor calls, preserve `#[cfg(feature = ...)]` gating):

| id | registered_name | requirement | cfg |
|---|---|---|---|
| kernel.mount.manage | mount | KernelDomain{"mount"} WRITE | — |
| kernel.task.manage | task | KernelDomain{"task"} WRITE | conditional TaskStore |
| kernel.marketplace.manage | marketplace | KernelDomain{"marketplace"} WRITE | — |
| kernel.program.forge | skill_forge | KernelDomain{"program"} WRITE | — |
| kernel.calendar.manage | calendar | KernelDomain{"calendar"} WRITE | — |
| kernel.memo.manage | memo | KernelDomain{"memo"} WRITE | memo feature |
| kernel.timeline.manage | timeline | KernelDomain{"timeline"} WRITE | timeline feature |
| kernel.email.send | email | KernelDomain{"email"} WRITE | — |
| kernel.image.gen | image_gen | KernelDomain{"image_gen"} WRITE | — |
| kernel.screenshot.capture | screenshot | Browser EXECUTE | browser feature |

Add the corresponding `"kernel.mount.manage" => ...` arms to `register_from_resolved_profile` mirroring existing domain arms. Grep for `OxiosKernelBridge` and `register_all_kernel_tools` across the workspace (`src/`, `crates/`) and remove/update every reference (docs comments referencing the bridge in registration.rs header, token_maxing comment at maxer.rs:16 — update comment text to reference the profile ceiling instead).

- [ ] **Step 1: Failing test** (registration.rs): catalog completeness — every descriptor id has an arm. Simplest verifiable form:

```rust
#[test]
fn catalog_long_tail_tools_are_addressable() {
    for id in ["kernel.mount.manage", kernel_calendar etc.] {
        assert!(find_descriptor(&ToolId::new(id)).is_some(), "{id} missing");
    }
}
```

- [ ] **Step 2: FAIL → Step 3: implement deletions + arms → Step 4:** `cargo check -p oxios-kernel --all-features` + full nextest + `cargo check --workspace --all-features` (bridge removal must not break the binary crate). **Step 5: Commit** `refactor(kernel): delete kernel bridge; long-tail tools join unified registration`

---

### Task 10: Persistence v3 + persona API + catalog endpoints

**Files:**
- Modify: `crates/oxios-kernel/src/persona/persistence.rs` (`SCHEMA_VERSION = 3`; tests: v2 JSON with no tool_profile loads → `None`; v3 roundtrip preserves it)
- Modify: `src/api/persona_routes.rs` (`PersonaUpdateRequest` += `tool_profile: Option<ToolProfileRef>`; update handler merges; `PersonaSummary` includes it; validation: unknown profile id in `tool_profile.id` → 400 with `builtin_profile_names()`)
- Modify: `src/api/routes/mod.rs` (add `GET /api/tool-catalog` → `kernel_catalog()` serialized; `GET /api/tool-profiles` → names + compositions)
- Test: persistence tests in-file; route validation logic as a unit fn test if the router harness is heavy (extract `validate_tool_profile_ref` and test it directly)

- [ ] **Step 1: Failing tests** (persistence v2→v3 load, roundtrip; `validate_tool_profile_ref` rejects `"nope"` accepts `"coding"`).
- [ ] **Step 2: FAIL → Step 3: implement → Step 4:** `cargo nextest run -p oxios-kernel` + `cargo check --workspace --all-features`. **Step 5: Commit** `feat(kernel): persona tool_profile persistence v3 and catalog APIs`

---

### Task 11: Pool reuse guard (session + fingerprint)

**Files:**
- Modify: `crates/oxios-kernel/src/supervisor.rs` (`AgentPool`)

**Changes:** add an internal `fingerprints: RwLock<HashMap<AgentId, (String /*session*/, u64 /*fingerprint*/)>>`; new methods:

```rust
pub fn insert_profiled(&self, id: AgentId, agent: Arc<Agent>, session: &str, fingerprint: u64);
pub fn get_reusable(&self, id: &AgentId, session: &str, fingerprint: u64) -> Option<Arc<Agent>>;
```

`get_reusable` returns the agent only when session AND fingerprint match; mismatch → `None` (caller rebuilds). Existing `insert`/`get`/`export_state`/`import_state` untouched (state export keeps working). `remove` also drops the profile entry.

- [ ] **Step 1: Failing test:** insert_profiled + get_reusable same key → Some; different fingerprint → None; different session → None; remove → get_reusable None.
- [ ] **Step 2: FAIL → Step 3 → Step 4** full kernel suite. **Step 5: Commit** `feat(kernel): session+fingerprint agent pool reuse guard`

---

### Task 12: Full CI gates

- [ ] `cargo fmt --all -- --check` (if pre-existing drift fails, format ONLY files this plan touched, leave the rest)
- [ ] `cargo clippy --workspace --all-features -- -D warnings`
- [ ] `cargo nextest run --workspace --no-fail-fast` (fallback `cargo test --workspace`)
- [ ] `cargo test --workspace --doc`
- [ ] Fix real warnings; rerun; final commit `chore(kernel): persona execution profiles gate cleanup` if needed.

---

## Explicitly deferred to subsequent plans

- **Plan 2 (web):** `/personas` tool-profile editor, chat header tool chip, catalog/profile endpoints consumption.
- **Plan 3 (delegation):** `ExecEnv.persona` → `ExecutionProfileSnapshot` threading, `KernelDelegationEnvelope`, A2A caller-side scope.
- **Plan 4 (progressive activation):** on-demand activation tool, sticky-turn policy, `AvailableOnDemand` non-registration.
- **Plan 5 (principal bounding authority):** replace template-seeded bounding with principal/session policy derivation.

These are design §17 migration steps 5-7; each gets its own brainstormed plan when Plan 1 lands.
