//! Static tool descriptors and the kernel catalog.
//!
//! Every tool that an agent can invoke is described by a [`ToolDescriptor`]
//! entry in the kernel catalog. Task 2 of the persona execution-profile
//! design introduces this vocabulary so the selector (Task 3) and
//! resolver (Task 4) can reason about which tools are available, which
//! rights they require, and when they should be activated.
//!
//! # Catalog lifetime
//!
//! [`kernel_catalog`] returns a `&'static [ToolDescriptor]` built once
//! on first access and cached in a [`std::sync::LazyLock`]. Building the
//! catalog requires heap allocations (every [`CapabilityRequirement`]
//! owns a [`ResourceRef`] that may contain a `String` — e.g. the
//! `Exec { mode: "shell" }` variant). Because the public API surface
//! is fixed to `&'static [ToolDescriptor]`, those per-entry string and
//! tag slices are leaked once via [`Box::leak`] during initialization.
//! The leak is bounded (one descriptor slice + tag/requirement slices)
//! and intentional: re-validating this on every call would defeat the
//! purpose of the static lifetime.
use std::fmt;
use std::sync::{Arc, LazyLock};

use serde::{Deserialize, Serialize};

use super::types::{ResourceRef, Rights};

// ─── Identifier newtypes ─────────────────────────────────────────────

/// Stable opaque identifier for a tool (e.g. `kernel.fs.read`).
///
/// `ToolId` is the primary key used by the kernel catalog and the
/// execution-profile selector. The wrapped `Arc<str>` keeps the type
/// cheap to clone while still cheap to hash and compare by content.
///
/// `Serialize + Deserialize` are implemented by hand because the inner
/// `Arc<str>` does not satisfy serde's auto-derived `Deserialize`. The
/// wire format is a plain JSON string (no newtype envelope), which keeps
/// the format symmetric with `Display` output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolId(pub Arc<str>);

impl ToolId {
    /// Construct a `ToolId` from anything that can become an `Arc<str>`
    /// (`&str`, `String`, `Arc<str>`).
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ToolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ToolId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ToolId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Self(Arc::from(s)))
    }
}

/// Identifies which subsystem provides / implements the tool
/// (e.g. `kernel.fs`, `kernel.web`, `kernel.exec`).
///
/// `Serialize + Deserialize` — see [`ToolId`] for rationale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolProviderId(pub Arc<str>);

impl ToolProviderId {
    /// Construct a `ToolProviderId` from anything that can become an
    /// `Arc<str>`.
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ToolProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ToolProviderId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ToolProviderId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Self(Arc::from(s)))
    }
}

/// Free-form tag attached to a descriptor (e.g. `fs`, `browser`,
/// `kernel-mutate`). Tag strings are opaque to the kernel; persona
/// profile authors decide the taxonomy.
///
/// `Serialize + Deserialize` — see [`ToolId`] for rationale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolTag(pub Arc<str>);

impl ToolTag {
    /// Construct a `ToolTag` from anything that can become an `Arc<str>`.
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ToolTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ToolTag {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ToolTag {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Self(Arc::from(s)))
    }
}

// ─── Contract & requirement types ─────────────────────────────────────

/// Semantic-version contract for a tool's input/output schema.
///
/// The catalog pins a `contract_version` per descriptor so that persona
/// profiles can require a minimum compatibility floor before binding a
/// tool to a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolContractVersion {
    /// Breaking-change counter. Bumped on incompatible schema changes.
    pub major: u32,
    /// Backwards-compatible feature addition counter.
    pub minor: u32,
}

/// Pairing of a [`ResourceRef`] with the [`Rights`] an agent must hold
/// over that resource in order to invoke the parent tool.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityRequirement {
    /// Resource the tool needs authority over.
    pub resource: ResourceRef,
    /// Minimum rights on `resource` required to invoke the tool.
    pub rights: Rights,
}

// ─── Activation class ─────────────────────────────────────────────────

/// Coarse classifier controlling when a tool should be offered to the
/// agent by the selector and/or surfaced in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationClass {
    /// Always available — never gated by intent or approval.
    Always,
    /// Surfaced only when the active profile or selector flags an
    /// intent match.
    IntentMatched,
    /// Surfaced on demand but still auto-approved.
    OnDemand,
    /// Requires explicit operator approval before each invocation.
    ApprovalOnly,
}

// ─── Descriptor ───────────────────────────────────────────────────────

/// Static description of a single tool that an agent can invoke.
///
/// `ToolDescriptor` instances live for the entire process — the kernel
/// catalog returns them as a `&'static [ToolDescriptor]` slice.
///
/// `Serialize` only: `&'static [..]` slice fields don't contribute
/// borrowed data during deserialization, so deriving `Deserialize` here
/// would require owned alternatives. The catalog is built once and
/// serialized out for diagnostics; deserialization is performed by
/// upstream consumers (the persona profile loader) via bespoke DTOs.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDescriptor {
    /// Stable tool identifier (e.g. `kernel.fs.read`).
    pub id: ToolId,
    /// Implementing subsystem (e.g. `kernel.fs`, `kernel.web`).
    pub provider: ToolProviderId,
    /// Schema contract version this descriptor pins.
    pub contract_version: ToolContractVersion,
    /// Human-readable description shown in diagnostics / docs.
    pub description: &'static str,
    /// Tags used by persona profile selectors to match this tool.
    pub tags: &'static [ToolTag],
    /// Capabilities an agent must hold to invoke this tool.
    pub required_capabilities: &'static [CapabilityRequirement],
    /// UI surfaces that can render affordances for this tool (optional
    /// taxonomy; empty when the tool has no UI binding).
    pub ui_affordances: &'static [ToolTag],
    /// When the tool should be activated / offered.
    pub activation_class: ActivationClass,
    /// Name the tool registers under today (matches the existing
    /// registration arm in the tool registry).
    pub registered_name: &'static str,
}

// ─── Catalog & lookup ─────────────────────────────────────────────────

static CATALOG: LazyLock<&'static [ToolDescriptor]> = LazyLock::new(|| {
    // Construct the catalog once, then leak the descriptor slice so
    // the returned `&'static [ToolDescriptor]` reference remains valid
    // for the rest of the process. See module-level docs for rationale.
    let entries: Vec<ToolDescriptor> = build_catalog();
    Box::leak(entries.into_boxed_slice())
});

/// Returns the process-lifetime kernel tool catalog.
///
/// The catalog is built lazily on the first call. Subsequent calls
/// return the same slice. Each descriptor references leaked tag and
/// `CapabilityRequirement` slices (see module docs).
pub fn kernel_catalog() -> &'static [ToolDescriptor] {
    *CATALOG
}

/// Returns the catalog entry for `id`, or `None` if no descriptor
/// matches.
pub fn find_descriptor(id: &ToolId) -> Option<&'static ToolDescriptor> {
    kernel_catalog().iter().find(|d| &d.id == id)
}

// ─── Builder helpers ──────────────────────────────────────────────────

/// Build a `&'static [T]` from an owned slice. Used during the one-time
/// catalog build to convert Vec-backed descriptors into long-lived
/// references. Acceptable here because the catalog itself leaks once
/// via [`Box::leak`].
fn leak_slice<T: Clone>(items: &[T]) -> &'static [T] {
    let boxed: Box<[T]> = items.to_vec().into_boxed_slice();
    Box::leak(boxed)
}

/// Construct a fresh `ToolTag` from a static literal. Each call allocates
/// a new `Arc<str>`, but those allocations are immediately consumed by
/// `leak_slice` and live for the rest of the process.
fn tag(value: &'static str) -> ToolTag {
    ToolTag::new(value)
}

/// Construct a fresh `ToolProviderId` from a static literal.
fn provider(value: &'static str) -> ToolProviderId {
    ToolProviderId::new(value)
}

// ─── Builder ──────────────────────────────────────────────────────────

fn build_catalog() -> Vec<ToolDescriptor> {
    vec![
        // ── Filesystem ──────────────────────────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.fs.read"),
            provider: provider("kernel.fs"),
            contract_version: V1,
            description: "Read a file from the agent workspace.",
            tags: leak_slice(&[tag("fs")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Fs,
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.fs.read")]),
            activation_class: ActivationClass::Always,
            registered_name: "read",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.fs.grep"),
            provider: provider("kernel.fs"),
            contract_version: V1,
            description: "Search file contents with a regex.",
            tags: leak_slice(&[tag("fs")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Fs,
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.fs.read")]),
            activation_class: ActivationClass::Always,
            registered_name: "grep",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.fs.find"),
            provider: provider("kernel.fs"),
            contract_version: V1,
            description: "Find files by glob / name.",
            tags: leak_slice(&[tag("fs")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Fs,
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.fs.read")]),
            activation_class: ActivationClass::Always,
            registered_name: "find",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.fs.ls"),
            provider: provider("kernel.fs"),
            contract_version: V1,
            description: "List a directory.",
            tags: leak_slice(&[tag("fs")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Fs,
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.fs.read")]),
            activation_class: ActivationClass::Always,
            registered_name: "ls",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.fs.write"),
            provider: provider("kernel.fs"),
            contract_version: V1,
            description: "Write a new file to the agent workspace.",
            tags: leak_slice(&[tag("fs")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Fs,
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.fs.write")]),
            activation_class: ActivationClass::Always,
            registered_name: "write",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.fs.edit"),
            provider: provider("kernel.fs"),
            contract_version: V1,
            description: "Apply a targeted edit to an existing file.",
            tags: leak_slice(&[tag("fs")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Fs,
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.fs.write")]),
            activation_class: ActivationClass::Always,
            registered_name: "edit",
        },
        // ── Web ─────────────────────────────────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.web.search"),
            provider: provider("kernel.web"),
            contract_version: V1,
            description: "Run a web search query.",
            tags: leak_slice(&[tag("web")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::WebSearch,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.web")]),
            activation_class: ActivationClass::Always,
            registered_name: "web_search",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.web.results"),
            provider: provider("kernel.web"),
            contract_version: V1,
            description: "Fetch results from the search index.",
            tags: leak_slice(&[tag("web")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::WebSearch,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.web")]),
            activation_class: ActivationClass::Always,
            registered_name: "get_search_results",
        },
        // ── Exec ────────────────────────────────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.exec.run"),
            provider: provider("kernel.exec"),
            contract_version: V1,
            description: "Run a shell command in the sandbox.",
            tags: leak_slice(&[tag("exec")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Exec {
                    mode: "shell".into(),
                },
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.exec")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "exec",
        },
        // ── Browser ─────────────────────────────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.browse.open"),
            provider: provider("kernel.browser"),
            contract_version: V1,
            description: "Open a URL in a headless browser.",
            tags: leak_slice(&[tag("browser")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Browser,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.browser")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "browse",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.browse.extract"),
            provider: provider("kernel.browser"),
            contract_version: V1,
            description: "Extract structured content from an open browser page.",
            tags: leak_slice(&[tag("browser")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Browser,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.browser")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "browse_extract",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.browse.session"),
            provider: provider("kernel.browser"),
            contract_version: V1,
            description: "Manage a long-lived browser session.",
            tags: leak_slice(&[tag("browser")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Browser,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.browser")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "browse_session",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.browse.script"),
            provider: provider("kernel.browser"),
            contract_version: V1,
            description: "Run a JS snippet inside a browser session.",
            tags: leak_slice(&[tag("browser")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Browser,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.browser")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "browse_script",
        },
        // ── Memory ──────────────────────────────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.memory.read"),
            provider: provider("kernel.memory"),
            contract_version: V1,
            description: "Recall a memory entry by id / query.",
            tags: leak_slice(&[tag("memory")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Memory,
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.memory")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "memory_read",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.memory.search"),
            provider: provider("kernel.memory"),
            contract_version: V1,
            description: "Search across the agent's memory.",
            tags: leak_slice(&[tag("memory")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Memory,
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.memory")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "memory_search",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.memory.write"),
            provider: provider("kernel.memory"),
            contract_version: V1,
            description: "Persist a new memory entry.",
            tags: leak_slice(&[tag("memory")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Memory,
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.memory")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "memory_write",
        },
        // ── Knowledge ───────────────────────────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.knowledge.read"),
            provider: provider("kernel.knowledge"),
            contract_version: V1,
            description: "Query the user markdown knowledge base.",
            tags: leak_slice(&[tag("knowledge")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Knowledge,
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.knowledge")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "knowledge",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.knowledge.write"),
            provider: provider("kernel.knowledge"),
            contract_version: V1,
            description: "Write to the user markdown knowledge base.",
            tags: leak_slice(&[tag("knowledge")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Knowledge,
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.knowledge")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "knowledge_write",
        },
        // ── User interaction ────────────────────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.ask_user.ask"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Prompt the user for clarification / input.",
            tags: leak_slice(&[tag("user-interaction")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "ask_user".into(),
                },
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.user_interaction")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "ask_user",
        },
        // ── Kernel mutation (ApprovalOnly) ──────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.persona.update"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Mutate persona definitions.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "persona".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "persona",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.project.update"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Mutate project state.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "project".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "project",
        },
        // Issues ride the project domain but carry their own descriptor so
        // profile selectors can address them. Arming is turn-path-only:
        // `IssueTool` needs the per-turn project binding
        // (`ExecEnv.project_id`) that `register_tools_from_cspace_gated`
        // receives and the resolved-profile path does not, so a profile
        // selecting this id falls through to the warn+skip catch-all by
        // design.
        ToolDescriptor {
            id: ToolId::new("kernel.issue.manage"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Manage a project's issues and milestones.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "project".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "issue",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.agent.update"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Mutate kernel agent registry entries.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "agent".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "kernel_agent",
        },
        // ── Kernel observation (OnDemand) ───────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.security.query"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Read the security domain.",
            tags: leak_slice(&[tag("kernel-observe")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "security".into(),
                },
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "security",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.budget.query"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Read the budget domain.",
            tags: leak_slice(&[tag("kernel-observe")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "budget".into(),
                },
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "budget",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.resource.query"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Read the resource domain.",
            tags: leak_slice(&[tag("kernel-observe")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "resource".into(),
                },
                rights: Rights::READ,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "resource",
        },
        // ── A2a ─────────────────────────────────────────────────────
        ToolDescriptor {
            id: ToolId::new("kernel.a2a.delegate"),
            provider: provider("kernel.a2a"),
            contract_version: V1,
            description: "Delegate a sub-task to another agent.",
            tags: leak_slice(&[tag("delegation")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::A2a,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.a2a")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "a2a_delegate",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.a2a.send"),
            provider: provider("kernel.a2a"),
            contract_version: V1,
            description: "Send a message over the agent-to-agent channel.",
            tags: leak_slice(&[tag("delegation")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::A2a,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.a2a")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "a2a_send",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.a2a.query"),
            provider: provider("kernel.a2a"),
            contract_version: V1,
            description: "Query the agent registry.",
            tags: leak_slice(&[tag("delegation")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::A2a,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.a2a")]),
            activation_class: ActivationClass::IntentMatched,
            registered_name: "a2a_query",
        },
        // ── Long-tail kernel domain tools (Task 9) ─────────────────
        // Each entry was previously registered unconditionally by the
        // legacy bulk-registration path (dissolved in Task 9). They now live
        // in the kernel catalog so the persona execution-profile
        // selector (Task 3) can reason about which long-tail tools
        // an agent is entitled to. The matching registration arms
        // live in `tools::registration::register_from_resolved_profile`
        // and key off these exact ids.
        ToolDescriptor {
            id: ToolId::new("kernel.automation.manage"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Create, inspect, edit, pause/resume, run, and delete automations.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "automation".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "automation",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.marketplace.manage"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Search, install, and update skills via ClawHub / Skills.sh.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "marketplace".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "marketplace",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.program.forge"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Author, validate, package, and ship skills.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "program".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "skill_forge",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.calendar.manage"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Create, update, delete, and search calendar events.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "calendar".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "calendar",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.memo.manage"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Manage oximemo entries (first-party app module).",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "memo".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "memo",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.timeline.manage"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Observe oxiline timeline entries (first-party app module).",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "timeline".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "timeline",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.email.send"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Send outbound email through the configured provider.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "email".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "send_email",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.image.gen"),
            provider: provider("kernel.kernel"),
            contract_version: V1,
            description: "Generate images through the configured image-gen provider.",
            tags: leak_slice(&[tag("kernel-mutate")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::KernelDomain {
                    domain: "image_gen".into(),
                },
                rights: Rights::WRITE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.kernel")]),
            activation_class: ActivationClass::ApprovalOnly,
            registered_name: "image_generation",
        },
        ToolDescriptor {
            id: ToolId::new("kernel.screenshot.capture"),
            provider: provider("kernel.browser"),
            contract_version: V1,
            description: "Capture a CSS-rendered screenshot of a web page.",
            tags: leak_slice(&[tag("browser")]),
            required_capabilities: leak_slice(&[CapabilityRequirement {
                resource: ResourceRef::Browser,
                rights: Rights::EXECUTE,
            }]),
            ui_affordances: leak_slice(&[tag("ui.browser")]),
            activation_class: ActivationClass::OnDemand,
            registered_name: "browse_screenshot",
        },
    ]
}

// ─── Internal: constants ──────────────────────────────────────────────

const V1: ToolContractVersion = ToolContractVersion { major: 1, minor: 0 };

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_requirements_nonempty() {
        let catalog = kernel_catalog();
        // Task 9: 28 base descriptors + 8 long-tail (task,
        // marketplace, skill_forge, calendar, memo, timeline, email,
        // image_gen, screenshot). Mount and cron descriptors were
        // dropped: both tools were deleted on main (mounts replaced
        // by project root_paths; cron scheduling left the kernel tool
        // surface).
        assert!(catalog.len() >= 36);
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

    #[test]
    fn tool_id_serde_roundtrip() {
        // JSON shape: a plain string, no newtype envelope.
        let original = ToolId::new("kernel.fs.read");
        let json = serde_json::to_string(&original).expect("serialize");
        assert_eq!(json, "\"kernel.fs.read\"");
        let decoded: ToolId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, original);
        assert_eq!(decoded.to_string(), "kernel.fs.read");
    }
}
