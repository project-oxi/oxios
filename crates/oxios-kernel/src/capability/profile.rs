//! Tool profile specifications — selectors, capability ceilings, and
//! publish-time validation.
//!
//! A [`ToolProfileSpec`] declares the tools an agent bound to that
//! profile may invoke, the upper bound on the rights the profile can
//! hand out, and the dynamic provider contracts that may bring
//! out-of-tree tools into scope at runtime.
//!
//! Two operations matter at the kernel boundary:
//!
//! - [`ToolProfileSpec::select_tools`] — resolves the static
//!   `include − exclude` set (dynamic providers add tools at
//!   resolution time, not here) against a tool catalog. Returns
//!   references that borrow from the catalog slice (not `self`) so
//!   the kernel's `'static` catalog propagates through unchanged.
//! - [`ToolProfileSpec::validate_publish`] — full pre-publish
//!   validation per design §4.2/§4.3/§5.1 step 4: every selected
//!   tool's required capabilities must fit some ceiling entry,
//!   `required_core` must lie inside the selected set, and the
//!   core set must not exceed `max_active_tools`.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::descriptor::{ToolContractVersion, ToolDescriptor, ToolId, ToolProviderId, ToolTag};
use super::types::{ResourceRef, Rights};
use crate::types::AgentId;

// ─── Profile identity ──────────────────────────────────────────────────

/// Monotonically-increasing revision number for a profile.
///
/// A [`ToolProfileRef`] is content-addressed by `(id, revision)` so
/// that resolution caches can store resolved tools against a specific
/// published version and invalidate cleanly when the revision changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileRevision(pub u64);

/// Content-addressed handle to a published profile.
///
/// The id is the profile's stable slug (e.g. `"coder"`, `"researcher"`)
/// and the revision disambiguates successive publishes of the same id.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolProfileRef {
    /// Stable profile slug.
    pub id: String,
    /// Published revision of `id`.
    pub revision: ProfileRevision,
}

// ─── Selectors ─────────────────────────────────────────────────────────

/// Predicate that matches one or more catalog entries by structural
/// property.
///
/// Two shapes are supported:
///
/// - [`ToolSelector::Tool`] — an exact [`ToolId`] match.
/// - [`ToolSelector::ProviderTag`] — every tool whose provider and
///   tag both match.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ToolSelector {
    /// Match exactly one catalog entry by stable id.
    Tool(ToolId),
    /// Match every tool where `provider` equals the descriptor's
    /// provider AND the descriptor's tag list contains `tag`.
    ProviderTag {
        /// Provider the matched tool must be served by.
        provider: ToolProviderId,
        /// Tag the matched tool must carry.
        tag: ToolTag,
    },
}

// ─── Resource selector + capability request ────────────────────────────

/// A pattern that matches a [`ResourceRef`] required by a tool.
///
/// `ResourceSelector` is the profile-author-facing vocabulary: it
/// mirrors [`ResourceRef`] variants so a profile can name a class of
/// resources (e.g. "all FS tools" via [`ResourceSelector::Fs`]) or a
/// specific resource (e.g. `Mcp { server: "github" }`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResourceSelector {
    /// A kernel domain of the given name (e.g. `"persona"`,
    /// `"security"`).
    KernelDomain(Arc<str>),
    /// An installed skill by registered name.
    Skill(Arc<str>),
    /// Another agent by its [`AgentId`].
    Agent(AgentId),
    /// Execution mode — `"shell"` or `"structured"`.
    Exec {
        /// Mode selector — equal to the descriptor's `Exec.mode`.
        mode: Arc<str>,
    },
    /// Any headless browser access.
    Browser,
    /// Any agent-to-agent channel.
    A2a,
    /// A specific MCP server (or `"*"` to cover every server).
    Mcp {
        /// Server name, or `"*"` to match any MCP server.
        server: Arc<str>,
    },
    /// Filesystem primitive tools.
    Fs,
    /// Web search tools.
    WebSearch,
    /// Agent memory tools.
    Memory,
    /// User markdown knowledge base tools.
    Knowledge,
}

impl ResourceSelector {
    /// Compile this selector to the typed [`ResourceRef`] it governs.
    ///
    /// The conversion is total and lossless for the variants that have
    /// a single canonical target (the unit variants and `KernelDomain`
    /// / `Skill` / `Agent` / `Mcp` / `Exec` carry their
    /// identifying payload across verbatim).
    pub fn to_resource_ref(&self) -> ResourceRef {
        match self {
            ResourceSelector::KernelDomain(name) => ResourceRef::KernelDomain {
                domain: name.to_string(),
            },
            ResourceSelector::Skill(name) => ResourceRef::Skill {
                name: name.to_string(),
            },
            ResourceSelector::Agent(id) => ResourceRef::Agent { id: *id },
            ResourceSelector::Exec { mode } => ResourceRef::Exec {
                mode: mode.to_string(),
            },
            ResourceSelector::Browser => ResourceRef::Browser,
            ResourceSelector::A2a => ResourceRef::A2a,
            ResourceSelector::Mcp { server } => ResourceRef::Mcp {
                server: server.to_string(),
            },
            ResourceSelector::Fs => ResourceRef::Fs,
            ResourceSelector::WebSearch => ResourceRef::WebSearch,
            ResourceSelector::Memory => ResourceRef::Memory,
            ResourceSelector::Knowledge => ResourceRef::Knowledge,
        }
    }

    /// Return `true` when the requirements of a descriptor (the
    /// `resource` argument) fall within this selector's scope.
    ///
    /// Semantics:
    /// - `Fs` covers `ResourceRef::Fs`; likewise for `WebSearch`,
    ///   `Memory`, `Knowledge`, `Browser`, `A2a`.
    /// - `KernelDomain(name)` covers `ResourceRef::KernelDomain`
    ///   with equal name.
    /// - `Skill(name)`, `Agent(id)`, `Exec { mode }`
    ///   cover their unit counterparts with equal payload.
    /// - `Mcp { server }` covers equal `server`, or any MCP resource
    ///   when `server == "*"`.
    pub fn covers(&self, resource: &ResourceRef) -> bool {
        match (self, resource) {
            (ResourceSelector::Fs, ResourceRef::Fs) => true,
            (ResourceSelector::WebSearch, ResourceRef::WebSearch) => true,
            (ResourceSelector::Memory, ResourceRef::Memory) => true,
            (ResourceSelector::Knowledge, ResourceRef::Knowledge) => true,
            (ResourceSelector::Browser, ResourceRef::Browser) => true,
            (ResourceSelector::A2a, ResourceRef::A2a) => true,
            (ResourceSelector::KernelDomain(a), ResourceRef::KernelDomain { domain: b }) => {
                a.as_ref() == b
            }
            (ResourceSelector::Skill(a), ResourceRef::Skill { name: b }) => a.as_ref() == b,
            (ResourceSelector::Agent(a), ResourceRef::Agent { id: b }) => a == b,
            (ResourceSelector::Exec { mode: a }, ResourceRef::Exec { mode: b }) => a.as_ref() == b,
            (ResourceSelector::Mcp { server: a }, ResourceRef::Mcp { server: b }) => {
                a.as_ref() == "*" || a.as_ref() == b
            }
            _ => false,
        }
    }
}

/// Pairing of a [`ResourceSelector`] with the rights the selector
/// grants. A profile's [`ToolProfileSpec::capability_ceiling`] is a
/// list of these — the upper bound on rights the profile can hand out
/// for each resource it names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityRequest {
    /// Resource scope the ceiling entry covers.
    pub resource: ResourceSelector,
    /// Rights granted by this ceiling entry.
    pub rights: Rights,
}

// ─── Dynamic provider contracts ────────────────────────────────────────

/// How a profile tolerates contract drift on a dynamic provider.
///
/// `ExactContract` rejects any provider whose pinned version does
/// not match the listed value. `CompatibleMajor` accepts any minor
/// bump under a given (contract, major) tuple.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProviderCompatibilityPolicy {
    /// Provider must pin exactly this contract version.
    ExactContract(ToolContractVersion),
    /// Provider must pin the named contract family at the given major
    /// version; any minor is accepted.
    CompatibleMajor {
        /// Contract family identifier (e.g. `"mcp.tools.v1"`).
        contract: Arc<str>,
        /// Major version that must match.
        major: u32,
    },
}

/// A dynamic provider's contribution to the tool set.
///
/// Dynamic providers are not part of the static kernel catalog — they
/// are loaded at startup and contribute a discoverable tool set under
/// the ceiling the profile declares here.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DynamicProviderContract {
    /// Which provider this contract governs.
    pub provider: ToolProviderId,
    /// Which tools the provider contributes to the resolved set.
    pub selector: ProviderToolSelector,
    /// Upper bound on the rights the profile grants to the provider's
    /// selected tools.
    pub capability_ceiling: Vec<CapabilityRequest>,
    /// Contract-version policy the provider must satisfy.
    pub compatibility: ProviderCompatibilityPolicy,
}

/// Selector for the tools a dynamic provider contributes.
///
/// `AllTools` adopts every tool the provider exposes; `Tag` restricts
/// the adopted set to a single tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProviderToolSelector {
    /// Every tool the provider exposes.
    AllTools,
    /// Only tools that carry the listed tag.
    Tag(ToolTag),
}

// ─── Activation policy ─────────────────────────────────────────────────

/// Activation-policy knobs that gate the live agent session.
///
/// `required_core` declares the tools that MUST be present in the
/// resolved set; `max_active_tools` is the published upper bound on
/// simultaneous live tools; `sticky_turns` records how long a
/// resolved tool stays in the active set without re-selection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolActivationPolicy {
    /// Maximum number of tools that may be active in a single
    /// resolved session.
    pub max_active_tools: usize,
    /// Tools that MUST be present in the resolved set. Validation
    /// rejects profiles where any of these ids is not selected.
    pub required_core: Vec<ToolId>,
    /// Number of turns a tool remains active after last selection.
    pub sticky_turns: u32,
}

// ─── Tool profile spec ─────────────────────────────────────────────────

/// A published tool profile: a static declaration of the tools an
/// agent bound to this profile may invoke, plus the upper bound on the
/// rights the profile can hand out for each resource class.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolProfileSpec {
    /// Stable profile slug.
    pub id: Arc<str>,
    /// Published revision of this profile.
    pub revision: ProfileRevision,
    /// Parent profiles whose rights and selections are inherited
    /// before this profile's own selectors are applied. The kernel
    /// flattens `extends` before resolution (Task 4 / Task 5).
    pub extends: Vec<ToolProfileRef>,
    /// Upper bound on the rights this profile can hand out for each
    /// resource it names. Every selected tool's required capability
    /// must fit some entry here.
    pub capability_ceiling: Vec<CapabilityRequest>,
    /// Selectors whose matched tools are added to the resolved set.
    pub include: Vec<ToolSelector>,
    /// Selectors whose matched tools are removed from the resolved
    /// set. Exclusion is evaluated AFTER inclusion; exclude always
    /// wins.
    pub exclude: Vec<ToolSelector>,
    /// Dynamic provider contracts that may contribute out-of-tree
    /// tools to the resolved set.
    pub dynamic_providers: Vec<DynamicProviderContract>,
    /// Activation policy applied to the resolved set at session
    /// start.
    pub activation: ToolActivationPolicy,
}

// ─── Errors ────────────────────────────────────────────────────────────

/// Errors a profile can raise at publish time.
#[derive(Debug, Error)]
pub enum ProfileError {
    /// Raised by the resolver when a profile references a tool
    /// absent from the catalog.
    #[error("tool {0} is not in the catalog")]
    UnknownTool(ToolId),
    /// A selected tool requires a capability the profile's ceiling
    /// does not grant. The `req` payload is the human-readable
    /// representation of the offending `CapabilityRequirement`.
    #[error("capability requirement of {tool} exceeds the profile ceiling: {req}")]
    CeilingExceeded {
        /// Tool whose requirement violated the ceiling.
        tool: ToolId,
        /// Human-readable description of the requirement.
        req: String,
    },
    /// A `required_core` id was not selected by the profile.
    #[error("required core tool {0} is not selected")]
    CoreNotSelected(ToolId),
    /// The profile declares more required-core tools than the
    /// activation policy permits.
    #[error("required core ({0} tools) exceeds max_active_tools {1}")]
    CoreTooLarge(usize, usize),
}

// ─── Implementation ────────────────────────────────────────────────────

impl ToolProfileSpec {
    /// Resolve the post-exclusion selected set against `catalog`.
    ///
    /// Algorithm:
    /// 1. For each [`ToolSelector`] in `include`, collect every
    ///    catalog entry it matches (exact id, or provider+tag).
    /// 2. For each [`ToolSelector`] in `exclude`, drop every entry
    ///    already collected that the exclude matches.
    /// 3. Return the survivors, de-duplicated by `ToolId`. The
    ///    order is **selector-major**: entries are appended in the
    ///    order each `include` selector matches the catalog, not in
    ///    catalog source order.
    ///
    /// The returned references borrow from `catalog`, not `self` —
    /// callers passing the kernel's `'static` catalog get
    /// `Vec<&'static ToolDescriptor>` back, which `ResolvedTool` can
    /// store without re-borrowing the profile struct.
    pub fn select_tools<'c>(&self, catalog: &'c [ToolDescriptor]) -> Vec<&'c ToolDescriptor> {
        // Step 1: union of include matches. Iteration is
        // selector-major (include outer, catalog inner), so the
        // resulting order matches the include selectors, not the
        // catalog's source order. De-duplicate by `ToolId` (the
        // catalog's primary key) so an id matched by two selectors
        // appears exactly once.
        let mut selected: Vec<&ToolDescriptor> = Vec::new();
        for selector in &self.include {
            for descriptor in catalog {
                if matches_selector(selector, descriptor)
                    && !selected.iter().any(|d| d.id == descriptor.id)
                {
                    selected.push(descriptor);
                }
            }
        }

        // Step 2: drop everything matched by any exclude selector.
        if !self.exclude.is_empty() {
            selected.retain(|descriptor| {
                self.exclude
                    .iter()
                    .all(|selector| !matches_selector(selector, descriptor))
            });
        }

        selected
    }

    /// Publish-time validation per design §4.2/§4.3/§5.1 step 4.
    ///
    /// Checks (in order):
    /// 1. Every selected tool's every `CapabilityRequirement` fits
    ///    some ceiling entry: `ceiling.resource.covers(&req.resource)`
    ///    AND `ceiling.rights.contains(req.rights)`.
    /// 2. Every `required_core` tool is in the resolved set.
    /// 3. `required_core.len() <= max_active_tools`.
    ///
    /// The first violation is returned.
    pub fn validate_publish(&self, catalog: &[ToolDescriptor]) -> Result<(), ProfileError> {
        // 1. Ceiling check: every selected tool's required capabilities
        //    must fit some ceiling entry. Doing this first matches the
        //    brief test order (ceiling violation surfaces before core
        //    check), and it gives the operator the most actionable
        //    error: which tool needs which missing right.
        let selected = self.select_tools(catalog);
        for descriptor in &selected {
            for req in descriptor.required_capabilities {
                let fits = self.capability_ceiling.iter().any(|ceiling| {
                    ceiling.resource.covers(&req.resource) && ceiling.rights.contains(req.rights)
                });
                if !fits {
                    return Err(ProfileError::CeilingExceeded {
                        tool: descriptor.id.clone(),
                        req: format!("{} on {}", req.rights, req.resource),
                    });
                }
            }
        }

        // 2. required_core ⊆ selected ids.
        for core in &self.activation.required_core {
            let present = selected.iter().any(|d| &d.id == core);
            if !present {
                return Err(ProfileError::CoreNotSelected(core.clone()));
            }
        }

        // 3. Core size must fit the activation budget.
        let core_len = self.activation.required_core.len();
        if core_len > self.activation.max_active_tools {
            return Err(ProfileError::CoreTooLarge(
                core_len,
                self.activation.max_active_tools,
            ));
        }

        Ok(())
    }
}

// ─── Helpers ───────────────────────────────────────────────────────────

/// Return `true` when `descriptor` is matched by `selector`.
fn matches_selector(selector: &ToolSelector, descriptor: &ToolDescriptor) -> bool {
    match selector {
        ToolSelector::Tool(id) => &descriptor.id == id,
        ToolSelector::ProviderTag { provider, tag } => {
            &descriptor.provider == provider && descriptor.tags.iter().any(|t| t == tag)
        }
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::kernel_catalog;

    fn sel(id: &str) -> ToolSelector {
        ToolSelector::Tool(ToolId::new(id))
    }

    #[test]
    fn exclude_always_wins_over_include() {
        let spec = ToolProfileSpec {
            id: "test".into(),
            revision: ProfileRevision(1),
            extends: vec![],
            capability_ceiling: vec![CapabilityRequest {
                resource: ResourceSelector::Fs,
                rights: Rights::READ | Rights::WRITE,
            }],
            include: vec![sel("kernel.fs.read"), sel("kernel.fs.write")],
            exclude: vec![sel("kernel.fs.write")],
            dynamic_providers: vec![],
            activation: ToolActivationPolicy {
                max_active_tools: 16,
                required_core: vec![],
                sticky_turns: 2,
            },
        };
        let selected = spec.select_tools(kernel_catalog());
        let ids: Vec<_> = selected.iter().map(|d| d.id.to_string()).collect();
        assert!(ids.contains(&"kernel.fs.read".to_string()));
        assert!(
            !ids.contains(&"kernel.fs.write".to_string()),
            "exclude must win"
        );
    }

    #[test]
    fn validate_rejects_ceiling_violation_and_missing_core() {
        let mut spec = ToolProfileSpec {
            id: "t".into(),
            revision: ProfileRevision(1),
            extends: vec![],
            // Fs READ only — but exec is included → its Exec EXECUTE req exceeds ceiling.
            capability_ceiling: vec![CapabilityRequest {
                resource: ResourceSelector::Fs,
                rights: Rights::READ,
            }],
            include: vec![sel("kernel.fs.read"), sel("kernel.exec.run")],
            exclude: vec![],
            dynamic_providers: vec![],
            activation: ToolActivationPolicy {
                max_active_tools: 16,
                required_core: vec![ToolId::new("kernel.browse.open")],
                sticky_turns: 0,
            },
        };
        let err = spec.validate_publish(kernel_catalog()).unwrap_err();
        assert!(
            matches!(err, ProfileError::CeilingExceeded { .. }),
            "got: {err}"
        );

        spec.capability_ceiling.push(CapabilityRequest {
            resource: ResourceSelector::Exec {
                mode: "shell".into(),
            },
            rights: Rights::EXECUTE,
        });
        let err = spec.validate_publish(kernel_catalog()).unwrap_err();
        assert!(
            matches!(err, ProfileError::CoreNotSelected(_)),
            "got: {err}"
        );

        spec.include.push(sel("kernel.browse.open"));
        spec.capability_ceiling.push(CapabilityRequest {
            resource: ResourceSelector::Browser,
            rights: Rights::EXECUTE,
        });
        spec.validate_publish(kernel_catalog())
            .expect("valid after fixes");
    }
}
