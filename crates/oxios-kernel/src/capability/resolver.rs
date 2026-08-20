//! Execution-profile resolution — the single trusted composition point
//! that turns a [`ToolProfileSpec`] plus a bounding [`CSpace`] into a
//! [`ResolvedExecutionProfile`].
//!
//! Tasks 5 (presets) and 6 (registration) consume this module; this is
//! the only place in the kernel where a profile is turned into the
//! concrete set of activated / on-demand / approval-pending /
//! unavailable tools together with the effective CSpace the kernel
//! should hand to the agent.
//!
//! # Resolution algorithm (design §5.1)
//!
//! 1. `selected = profile.select_tools(kernel_catalog())` —
//!    selector-major post-exclusion set borrowed from the `'static`
//!    catalog.
//! 2. For each selected descriptor and each `CapabilityRequirement`:
//!    * **Ceiling check** — some ceiling entry covers the resource
//!      and grants rights ⊇ the requirement. If not, the tool is
//!      `Unavailable(CeilingExceeded)` *before* any authority check
//!      and is **never** approval-eligible (ceiling violations are
//!      hard policy, not operator discretion).
//!    * **Authority check** — does the bounding CSpace already hold
//!      the required capability? If yes, that requirement is
//!      satisfied. If no, the requirement is approval-eligible iff
//!      the `approval_grantable` predicate returns `true` for the
//!      (`ResourceRef`, `Rights`) pair; the tool then surfaces as
//!      `RequiresApproval`. Otherwise the tool is
//!      `Unavailable(MissingCapability { resource, rights })`.
//! 3. After every requirement is processed:
//!    * satisfied → status by `activation_class`:
//!      `Always`/`IntentMatched` → `Active`,
//!      `OnDemand` → `AvailableOnDemand`,
//!      `ApprovalOnly` → `RequiresApproval`.
//! 4. The effective CSpace collects `Capability::kernel(req.resource,
//!    req.rights)` for every *satisfied* requirement of every tool
//!    that reached `Active` or `AvailableOnDemand`. Tools that
//!    reached `RequiresApproval` are NOT inserted into the CSpace —
//!    their capabilities only land after a later re-resolve observes
//!    approval.
//! 5. The fingerprint is a 64-bit FNV-1a hash over a canonical
//!    string assembled from the profile identity, the sorted
//!    `(id, contract_major.minor, status_discriminant)` rows of the
//!    resolved set, and the sorted `(resource, rights)` capability
//!    strings of the effective CSpace. Stability across calls with
//!    identical inputs is guaranteed by the deterministic sort; any
//!    change in profile, bounding, or predicate flips at least one
//!    row and produces a different fingerprint.
//!
//! # Fingerprint discriminants
//!
//! Each [`ResolutionStatus`] maps to a single-byte ASCII code so the
//! fingerprint string stays human-readable in logs:
//!
//! | Status                       | Char |
//! |------------------------------|------|
//! | `Active`                     | `A`  |
//! | `AvailableOnDemand`          | `O`  |
//! | `RequiresApproval`           | `P`  |
//! | `Unavailable(CeilingExceeded)`| `C` |
//! | `Unavailable(MissingCapability)`| `M` |
//! | `Unavailable(ProviderUnavailable)`| `U` |

use std::sync::Arc;

use super::descriptor::{ActivationClass, ToolDescriptor, kernel_catalog};
use super::profile::{ProfileRevision, ToolProfileSpec};
use super::types::{CSpace, Capability, ResourceRef, Rights};
use crate::types::AgentId;

// ─── Unavailability reason ────────────────────────────────────────────

/// Why a selected tool could not be activated by this resolve.
///
/// Every variant is a fail-closed classification: the resolver
/// reports a concrete, actionable reason so the operator (or the
/// session driver) can decide what to do next.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnavailabilityReason {
    /// A requirement exceeded the profile's capability ceiling.
    ///
    /// The ceiling is a *policy* boundary, not an authority
    /// boundary — even a bounding CSpace that holds the required
    /// right cannot rescue the tool. The resolver reports this
    /// reason before the authority check so the operator sees the
    /// more actionable error.
    CeilingExceeded,
    /// The bounding CSpace does not hold the required capability
    /// and the `approval_grantable` predicate (if any) refused it.
    ///
    /// Carries the offending `ResourceRef` and `Rights` so the
    /// caller can surface a precise prompt.
    MissingCapability {
        /// The resource the tool required.
        resource: ResourceRef,
        /// The rights the tool required over `resource`.
        rights: Rights,
    },
    /// No provider is currently registered for the descriptor.
    ///
    /// Reserved for dynamic-provider resolution (Task 6). The static
    /// kernel catalog always has a provider, so this variant does
    /// not fire from [`resolve`] today — it exists so callers can
    /// carry the classification through their own resolution
    /// paths.
    ProviderUnavailable,
}

// ─── Resolution status ─────────────────────────────────────────────────

/// Lifecycle status of a single resolved tool.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResolutionStatus {
    /// Live and immediately invocable.
    Active,
    /// Available on demand; auto-approved, surfaces when the agent
    /// signals intent.
    AvailableOnDemand,
    /// Surfaced but gated on an explicit approval flow.
    RequiresApproval,
    /// Not reachable in this session — see [`UnavailabilityReason`].
    Unavailable(UnavailabilityReason),
}

// ─── Per-tool resolution record ───────────────────────────────────────

/// One resolved tool: the static descriptor plus its resolution
/// status.
///
/// The descriptor borrows from the kernel's `'static` catalog, so
/// [`ResolvedExecutionProfile`] owns no per-descriptor allocation —
/// the catalog lifetime flows through unchanged.
#[derive(Debug, Clone)]
pub struct ResolvedTool {
    /// The kernel-catalog entry for this tool.
    pub descriptor: &'static ToolDescriptor,
    /// The classification the resolver assigned.
    pub status: ResolutionStatus,
}

// ─── Fingerprint ───────────────────────────────────────────────────────

/// Content-addressed fingerprint of a [`ResolvedExecutionProfile`].
///
/// Stable across calls with identical inputs and sensitive to every
/// observable input (profile id/revision/selection, bounding
/// CSpace, approval predicate). Suitable as the cache key for
/// per-session resolution caching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExecutionProfileFingerprint(pub u64);

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a 64-bit hash over an arbitrary byte slice.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(FNV_OFFSET, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(FNV_PRIME)
    })
}

/// Single-byte ASCII code for a [`ResolutionStatus`] in the
/// fingerprint string. See module-level table for the full mapping.
fn status_discriminant(status: &ResolutionStatus) -> &'static str {
    match status {
        ResolutionStatus::Active => "A",
        ResolutionStatus::AvailableOnDemand => "O",
        ResolutionStatus::RequiresApproval => "P",
        ResolutionStatus::Unavailable(UnavailabilityReason::CeilingExceeded) => "C",
        ResolutionStatus::Unavailable(UnavailabilityReason::MissingCapability { .. }) => "M",
        ResolutionStatus::Unavailable(UnavailabilityReason::ProviderUnavailable) => "U",
    }
}

/// Rights rendered into the canonical fingerprint string.
///
/// `Rights` is a tuple struct with a `pub u8` bit-pattern field
/// (`types.rs`), so we format the raw bits as a fixed-width two-digit
/// hex value. This is **bit-injective**: every distinct `u8` produces a
/// distinct token, including rights values that share the named-flag
/// rendering (`Display` collapses e.g. `Rights(0x01)` and `Rights(0x11)`
/// to `"R"`, which would let two semantically different capabilities
/// collide in the fingerprint). The token is stable for a given bit
/// pattern and zero-padded so leading-zero patterns sort correctly.
fn rights_token(r: Rights) -> String {
    format!("{:02x}", r.0)
}

// ─── Resolved profile ──────────────────────────────────────────────────

/// A profile that has been resolved against a concrete bounding
/// CSpace.
///
/// This is the value the session driver hands to downstream
/// consumers (preset engine, registration handler, manifest
/// builder). It carries:
/// * the resolved tool set (with per-tool status),
/// * the effective CSpace the kernel should hand to the agent, and
/// * a fingerprint suitable for downstream caching.
#[derive(Debug, Clone)]
pub struct ResolvedExecutionProfile {
    /// Stable profile slug (matches `ToolProfileSpec::id`).
    pub profile_id: Arc<str>,
    /// Published profile revision.
    pub profile_revision: ProfileRevision,
    /// Agent the profile was resolved for.
    pub agent_id: AgentId,
    /// Per-tool resolution records, in selector-major order.
    pub tools: Vec<ResolvedTool>,
    /// Effective CSpace: `kernel(req.resource, req.rights)` for
    /// every satisfied requirement of every tool that reached
    /// `Active` or `AvailableOnDemand`. Tools pending approval
    /// contribute nothing here.
    pub effective_cspace: CSpace,
    /// Content-addressed fingerprint of this resolution.
    pub fingerprint: ExecutionProfileFingerprint,
}

impl ResolvedExecutionProfile {
    /// Static descriptors of every tool that reached
    /// [`ResolutionStatus::Active`].
    ///
    /// Order matches the order they appear in [`Self::tools`].
    pub fn active_tool_descriptors(&self) -> Vec<&'static ToolDescriptor> {
        self.tools
            .iter()
            .filter_map(|t| match t.status {
                ResolutionStatus::Active => Some(t.descriptor),
                _ => None,
            })
            .collect()
    }

    /// Registered names of every tool that reached
    /// [`ResolutionStatus::Active`].
    ///
    /// Order matches the order they appear in [`Self::tools`].
    pub fn active_registered_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .filter_map(|t| match t.status {
                ResolutionStatus::Active => Some(t.descriptor.registered_name.to_string()),
                _ => None,
            })
            .collect()
    }
}

// ─── Resolution entry point ────────────────────────────────────────────

/// Predicate consulted by [`resolve`] when a tool requirement falls
/// outside the bounding CSpace: returning `true` surfaces the tool as
/// `RequiresApproval` instead of `Unavailable(MissingCapability)`.
pub type ApprovalPredicate<'a> = &'a dyn Fn(&ResourceRef, Rights) -> bool;

/// Resolve `profile` against `bounding` and the optional approval
/// predicate.
///
/// See module-level docs for the full algorithm. The returned
/// [`ResolvedExecutionProfile`] owns no per-descriptor allocation —
/// tool descriptors borrow from the kernel's `'static` catalog.
///
/// # Arguments
///
/// * `profile` — the published profile to resolve.
/// * `bounding` — the CSpace the kernel hands to the agent. All
///   satisfied requirements must be backed by a capability in this
///   space.
/// * `agent_id` — the agent the resolution is for; the returned
///   effective CSpace is owned by this agent.
/// * `approval_grantable` — optional predicate over
///   `(&ResourceRef, Rights)`. When `None`, no requirement is
///   approval-eligible and any authority shortfall becomes
///   `Unavailable(MissingCapability)`. When `Some`, a missing
///   requirement that the predicate accepts surfaces as
///   `RequiresApproval` instead.
pub fn resolve(
    profile: &ToolProfileSpec,
    bounding: &CSpace,
    agent_id: AgentId,
    approval_grantable: Option<ApprovalPredicate<'_>>,
) -> ResolvedExecutionProfile {
    // Step 1 — selector-major selection against the static catalog.
    let selected = profile.select_tools(kernel_catalog());

    // Effective CSpace: fresh, agent-owned. Populated only from
    // tools that reach Active or AvailableOnDemand (RequiresApproval
    // tools contribute nothing — they enter the CSpace after a
    // later re-resolve observes the granted approval).
    let mut effective_cspace = CSpace::new(agent_id);

    let mut tools: Vec<ResolvedTool> = Vec::with_capacity(selected.len());

    for descriptor in selected {
        let mut status: Option<ResolutionStatus> = None;

        for req in descriptor.required_capabilities {
            // Step 2 — ceiling check.
            let ceiling_fits = profile.capability_ceiling.iter().any(|ceiling| {
                ceiling.resource.covers(&req.resource) && ceiling.rights.contains(req.rights)
            });
            if !ceiling_fits {
                status = Some(ResolutionStatus::Unavailable(
                    UnavailabilityReason::CeilingExceeded,
                ));
                break;
            }

            // Step 3 — authority check.
            if bounding.can(&req.resource, req.rights) {
                // Requirement satisfied; nothing to record here —
                // the CSpace insert happens after the loop, only
                // for tools that fully reach Active/AvailableOnDemand.
                continue;
            }

            // Authority missing — check the approval predicate.
            let approval_eligible = approval_grantable
                .map(|pred| pred(&req.resource, req.rights))
                .unwrap_or(false);

            if approval_eligible {
                status = Some(ResolutionStatus::RequiresApproval);
                // Continue checking the remaining requirements: a
                // later ceiling/authority failure on the same tool
                // still surfaces the more specific reason
                // (MissingCapability takes precedence over
                // RequiresApproval so the operator sees the
                // harder problem).
                continue;
            }

            status = Some(ResolutionStatus::Unavailable(
                UnavailabilityReason::MissingCapability {
                    resource: req.resource.clone(),
                    rights: req.rights,
                },
            ));
            break;
        }

        // Step 4 — every requirement satisfied; map activation
        // class to status.
        let final_status = status.unwrap_or(match descriptor.activation_class {
            ActivationClass::Always | ActivationClass::IntentMatched => ResolutionStatus::Active,
            ActivationClass::OnDemand => ResolutionStatus::AvailableOnDemand,
            ActivationClass::ApprovalOnly => ResolutionStatus::RequiresApproval,
        });

        // Step 5 — populate the effective CSpace from the
        // *satisfied* requirements of every tool that reached
        // Active or AvailableOnDemand. RequiresApproval tools
        // are deliberately excluded.
        if matches!(
            final_status,
            ResolutionStatus::Active | ResolutionStatus::AvailableOnDemand
        ) {
            for req in descriptor.required_capabilities {
                effective_cspace.insert(Capability::kernel(req.resource.clone(), req.rights));
            }
        }

        tools.push(ResolvedTool {
            descriptor,
            status: final_status,
        });
    }

    // Step 6 — fingerprint over a canonical, deterministic string.
    let fingerprint = compute_fingerprint(profile, &tools, &effective_cspace);

    ResolvedExecutionProfile {
        profile_id: profile.id.clone(),
        profile_revision: profile.revision,
        agent_id,
        tools,
        effective_cspace,
        fingerprint,
    }
}

/// Build the fingerprint over a canonical string assembled from
/// profile identity, the sorted resolved tool rows, and the sorted
/// effective CSpace rows.
///
/// Sorting guarantees that two resolutions with identical inputs
/// produce the same byte stream and therefore the same FNV-1a
/// digest. Any difference in profile, bounding, or predicate flips
/// at least one row.
fn compute_fingerprint(
    profile: &ToolProfileSpec,
    tools: &[ResolvedTool],
    effective_cspace: &CSpace,
) -> ExecutionProfileFingerprint {
    // Sorted resolved tool rows: id:major.minor:discriminant.
    let mut tool_rows: Vec<String> = tools
        .iter()
        .map(|t| {
            format!(
                "{}:{}.{}:{}",
                t.descriptor.id,
                t.descriptor.contract_version.major,
                t.descriptor.contract_version.minor,
                status_discriminant(&t.status),
            )
        })
        .collect();
    tool_rows.sort();

    // Sorted effective capability rows: resource:rights.
    let mut cap_rows: Vec<String> = effective_cspace
        .iter()
        .map(|c| format!("{}:{}", c.resource, rights_token(c.rights)))
        .collect();
    cap_rows.sort();

    let mut canonical = String::with_capacity(
        profile.id.len()
            + 24
            + tool_rows.iter().map(String::len).sum::<usize>()
            + cap_rows.iter().map(String::len).sum::<usize>(),
    );
    canonical.push_str(profile.id.as_ref());
    canonical.push('|');
    canonical.push_str(&profile.revision.0.to_string());
    canonical.push('|');
    for row in &tool_rows {
        canonical.push_str(row);
        canonical.push('|');
    }
    for row in &cap_rows {
        canonical.push_str(row);
        canonical.push('|');
    }

    ExecutionProfileFingerprint(fnv1a(canonical.as_bytes()))
}

// ─── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::descriptor::ToolId;
    use crate::capability::profile::{
        CapabilityRequest, ResourceSelector, ToolActivationPolicy, ToolSelector,
    };

    /// Bounding CSpace covering every resource the `coding_like_profile`
    /// declares in its ceiling — read/write FS, web search exec, shell
    /// exec, browser exec, read/write memory, knowledge read, and the
    /// `ask_user` kernel domain.
    fn worker_bounding(agent_id: AgentId) -> CSpace {
        let mut c = CSpace::new(agent_id);
        c.insert(Capability::kernel(
            ResourceRef::Fs,
            Rights::READ | Rights::WRITE,
        ));
        c.insert(Capability::kernel(ResourceRef::WebSearch, Rights::EXECUTE));
        c.insert(Capability::kernel(
            ResourceRef::Exec {
                mode: "shell".into(),
            },
            Rights::EXECUTE,
        ));
        c.insert(Capability::kernel(ResourceRef::Browser, Rights::EXECUTE));
        c.insert(Capability::kernel(
            ResourceRef::Memory,
            Rights::READ | Rights::WRITE,
        ));
        c.insert(Capability::kernel(ResourceRef::Knowledge, Rights::READ));
        c.insert(Capability::kernel(
            ResourceRef::KernelDomain {
                domain: "ask_user".into(),
            },
            Rights::EXECUTE,
        ));
        c
    }

    /// Build a profile whose include list and ceiling together cover
    /// the seven tools exercised by the brief tests, each with a
    /// ceiling entry that fits its sole requirement.
    fn coding_like_profile() -> ToolProfileSpec {
        ToolProfileSpec {
            id: "test-coding".into(),
            revision: ProfileRevision(1),
            extends: vec![],
            capability_ceiling: vec![
                CapabilityRequest {
                    resource: ResourceSelector::Fs,
                    rights: Rights::READ | Rights::WRITE,
                },
                CapabilityRequest {
                    resource: ResourceSelector::WebSearch,
                    rights: Rights::EXECUTE,
                },
                CapabilityRequest {
                    resource: ResourceSelector::Exec {
                        mode: "shell".into(),
                    },
                    rights: Rights::EXECUTE,
                },
                CapabilityRequest {
                    resource: ResourceSelector::Browser,
                    rights: Rights::EXECUTE,
                },
                CapabilityRequest {
                    resource: ResourceSelector::Memory,
                    rights: Rights::READ | Rights::WRITE,
                },
                CapabilityRequest {
                    resource: ResourceSelector::Knowledge,
                    rights: Rights::READ,
                },
                CapabilityRequest {
                    resource: ResourceSelector::KernelDomain("ask_user".into()),
                    rights: Rights::EXECUTE,
                },
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
            activation: ToolActivationPolicy {
                max_active_tools: 16,
                required_core: vec![],
                sticky_turns: 2,
            },
        }
    }

    #[test]
    fn active_tools_land_in_cspace_and_names() {
        let agent = AgentId::new_v4();
        let resolved = resolve(&coding_like_profile(), &worker_bounding(agent), agent, None);
        let names = resolved.active_registered_names();
        for expected in [
            "read",
            "write",
            "web_search",
            "exec",
            "memory_read",
            "memory_write",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing {expected} in {names:?}"
            );
        }
        assert!(names.contains(&"ask_user".to_string())); // IntentMatched → Active
        assert!(
            resolved
                .effective_cspace
                .can(&ResourceRef::Fs, Rights::WRITE)
        );
    }

    #[test]
    fn ceiling_exceeded_is_unavailable_even_with_authority() {
        let agent = AgentId::new_v4();
        let mut p = coding_like_profile();
        p.capability_ceiling
            .retain(|c| !matches!(c.resource, ResourceSelector::Exec { .. }));
        let resolved = resolve(&p, &worker_bounding(agent), agent, None);
        let exec = resolved
            .tools
            .iter()
            .find(|t| t.descriptor.registered_name == "exec")
            .unwrap();
        assert!(matches!(
            exec.status,
            ResolutionStatus::Unavailable(UnavailabilityReason::CeilingExceeded)
        ));
    }

    #[test]
    fn missing_authority_is_unavailable_not_escalated() {
        let agent = AgentId::new_v4();
        let mut bounding = worker_bounding(agent);
        bounding.retain(|c| c.resource != ResourceRef::Memory);
        let resolved = resolve(&coding_like_profile(), &bounding, agent, None);
        for t in &resolved.tools {
            if t.descriptor.registered_name.starts_with("memory") {
                assert!(matches!(
                    t.status,
                    ResolutionStatus::Unavailable(UnavailabilityReason::MissingCapability { .. })
                ));
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
        p2.exclude
            .push(ToolSelector::Tool(ToolId::new("kernel.exec.run")));
        let c = resolve(&p2, &worker_bounding(agent), agent, None);
        assert_ne!(a.fingerprint, c.fingerprint);
    }

    #[test]
    fn rights_token_is_bit_injective_and_fingerprint_sensitive() {
        // 1. `rights_token` must be bit-injective on the raw `u8` field.
        // Two distinct bit patterns that `Display` collapses to the
        // same string ("R") must produce distinct tokens — this is
        // the exact bug the hex encoding fixes.
        assert_ne!(rights_token(Rights(0x01)), rights_token(Rights(0x11)));
        assert_eq!(rights_token(Rights(0x01)), "01");
        assert_eq!(rights_token(Rights(0x11)), "11");
        // Named combos that the old `Display` form differentiated
        // also round-trip exactly through the hex form.
        assert_eq!(rights_token(Rights::READ | Rights::WRITE), "03");
        assert_eq!(rights_token(Rights::READ | Rights::EXECUTE), "05");

        // 2. End-to-end fingerprint sensitivity: two profiles that
        // resolve to the same resource but different required-rights
        // bits must produce distinct fingerprints. We exercise this
        // by selecting a tool whose requirement is `Fs, READ` (0x01)
        // vs one whose requirement is `Fs, WRITE` (0x02). Both are
        // ⊇ the named-flag case, so this also covers the R-only vs
        // W-only named-rights pair the brief calls out as the
        // raw-bit-case fallback.
        let agent = AgentId::new_v4();
        let bounding = worker_bounding(agent);

        let read_only_profile = ToolProfileSpec {
            id: "fp-read".into(),
            revision: ProfileRevision(1),
            extends: vec![],
            capability_ceiling: vec![CapabilityRequest {
                resource: ResourceSelector::Fs,
                rights: Rights::READ | Rights::WRITE,
            }],
            include: vec![ToolSelector::Tool(ToolId::new("kernel.fs.read"))],
            exclude: vec![],
            dynamic_providers: vec![],
            activation: ToolActivationPolicy {
                max_active_tools: 1,
                required_core: vec![],
                sticky_turns: 1,
            },
        };
        let write_only_profile = ToolProfileSpec {
            id: "fp-write".into(),
            revision: ProfileRevision(1),
            extends: vec![],
            capability_ceiling: vec![CapabilityRequest {
                resource: ResourceSelector::Fs,
                rights: Rights::READ | Rights::WRITE,
            }],
            include: vec![ToolSelector::Tool(ToolId::new("kernel.fs.write"))],
            exclude: vec![],
            dynamic_providers: vec![],
            activation: ToolActivationPolicy {
                max_active_tools: 1,
                required_core: vec![],
                sticky_turns: 1,
            },
        };
        let r_read = resolve(&read_only_profile, &bounding, agent, None);
        let r_write = resolve(&write_only_profile, &bounding, agent, None);
        assert_ne!(
            r_read.fingerprint, r_write.fingerprint,
            "fingerprints over the same resource must differ when required-rights bits differ"
        );
    }
}
