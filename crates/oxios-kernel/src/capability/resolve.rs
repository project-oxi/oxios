//! CSpace resolution — determines an agent's initial capability space from
//! Directive + Persona inputs.
//!
//! The resolution follows a priority chain:
//!
//! 1. **Explicit cspace hint** on the directive → template/profile by name.
//! 2. **Persona tool profile** (`base` | `code` | `minimal` | `control`) → the
//!    matching persona profile template.
//! 3. **Default** → fall back to the `code` profile.

//! # Example
//!
//! ```no_run
//! use oxios_kernel::capability::resolve::resolve_cspace;
//! use oxios_kernel::persona::ToolProfile;
//! use oxios_kernel::types::AgentId;
//!
//! let cspace = resolve_cspace(None, ToolProfile::Base, AgentId::new_v4());
//! assert!(cspace.len() > 2);
//! ```

use crate::persona::ToolProfile;
use crate::types::AgentId;

use super::template::CapabilityTemplate;
use super::types::CSpace;

/// Known template names selectable via [`ExecEnv::cspace_hint`].
const PROFILE_BASE: &str = "base";
const PROFILE_CODE: &str = "code";
const PROFILE_MINIMAL: &str = "minimal";
/// Legacy token-maxing hint — behaves like `code` (the old `standard`
/// template only differed by a memory-READ grant that never produced a
/// registered tool on this path).
const LEGACY_STANDARD: &str = "standard";

/// Resolve an agent's initial CSpace from the available context.
///
/// # Arguments
///
/// * `cspace_hint` — Optional hint string from the directive. Accepts a
///   profile name ("base", "code", "minimal") or the legacy "standard"
///   name (token-maxing; equivalent to `code`).
/// * `tool_profile` — The assigned persona's tool profile.
/// * `agent_id` — The agent that will own the resolved CSpace.
///
/// # Priority
///
/// 1. `cspace_hint` (if present and non-empty)
/// 2. `tool_profile`
/// 3. `code` fallback
pub fn resolve_cspace(
    cspace_hint: Option<&str>,
    tool_profile: ToolProfile,
    agent_id: AgentId,
) -> CSpace {
    // 1. Explicit hint from directive takes highest priority.
    if let Some(hint) = cspace_hint {
        let trimmed = hint.trim().to_lowercase();
        if !trimmed.is_empty() {
            return match trimmed.as_str() {
                PROFILE_BASE => CapabilityTemplate::base_profile().build_for(agent_id),
                PROFILE_CODE | LEGACY_STANDARD => {
                    CapabilityTemplate::code_profile().build_for(agent_id)
                }
                PROFILE_MINIMAL => CapabilityTemplate::minimal_profile().build_for(agent_id),
                other => {
                    tracing::warn!(
                        "Unknown cspace hint '{}', falling back to the persona tool profile",
                        other
                    );
                    profile_template(tool_profile).build_for(agent_id)
                }
            };
        }
    }

    // 2. Persona tool profile.
    profile_template(tool_profile).build_for(agent_id)
}

/// Map a [`ToolProfile`] to its capability template.
fn profile_template(profile: ToolProfile) -> CapabilityTemplate {
    match profile {
        ToolProfile::Base => CapabilityTemplate::base_profile(),
        ToolProfile::Code => CapabilityTemplate::code_profile(),
        ToolProfile::Minimal => CapabilityTemplate::minimal_profile(),
        ToolProfile::Control => CapabilityTemplate::control_profile(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::super::types::{ResourceRef, Rights};

    fn exec_shell() -> ResourceRef {
        ResourceRef::Exec {
            mode: "shell".into(),
        }
    }

    #[test]
    fn hint_takes_priority_over_profile() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(Some("minimal"), ToolProfile::Base, id);
        // minimal grants nothing, even though the profile is base
        assert!(!cs.can(&exec_shell(), Rights::EXECUTE));
        assert_eq!(cs.len(), 0);
    }

    #[test]
    fn base_profile_grants_control_domains() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(None, ToolProfile::Base, id);
        for domain in [
            "persona", "security", "budget", "resource", "agent", "project",
        ] {
            assert!(
                cs.can(
                    &ResourceRef::KernelDomain {
                        domain: domain.into()
                    },
                    Rights::EXECUTE
                ),
                "base profile missing {domain}"
            );
        }
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "a2a".into()
            },
            Rights::EXECUTE
        ));
        assert!(cs.can(&exec_shell(), Rights::EXECUTE));
    }

    #[test]
    fn code_profile_has_no_control_tools() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(None, ToolProfile::Code, id);
        assert!(cs.can(&exec_shell(), Rights::EXECUTE));
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "memory".into()
            },
            Rights::EXECUTE
        ));
        for domain in [
            "persona", "security", "budget", "resource", "agent", "project",
        ] {
            assert!(
                !cs.can(
                    &ResourceRef::KernelDomain {
                        domain: domain.into()
                    },
                    Rights::READ
                ),
                "code profile must not grant {domain}"
            );
        }
        assert!(!cs.can(
            &ResourceRef::KernelDomain {
                domain: "a2a".into()
            },
            Rights::READ
        ));
    }

    #[test]
    fn minimal_profile_is_empty() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(None, ToolProfile::Minimal, id);
        assert_eq!(cs.len(), 0);
    }

    #[test]
    fn legacy_standard_hint_maps_to_code() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(Some("standard"), ToolProfile::Minimal, id);
        assert!(cs.can(&exec_shell(), Rights::EXECUTE));
    }

    #[test]
    fn empty_hint_falls_through_to_profile() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(Some(""), ToolProfile::Base, id);
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "a2a".into()
            },
            Rights::EXECUTE
        ));
    }

    #[test]
    fn unknown_hint_falls_back_to_profile() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(Some("nonexistent"), ToolProfile::Code, id);
        assert!(cs.can(&exec_shell(), Rights::EXECUTE));
        assert!(!cs.can(&ResourceRef::A2a, Rights::READ));
    }

    #[test]
    fn control_profile_resolves_control_domains_only() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(None, ToolProfile::Control, id);
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "mcp_manage".into()
            },
            Rights::EXECUTE
        ));
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "engine".into()
            },
            Rights::WRITE
        ));
        // No exec shell, no browser.
        assert!(!cs.can(&exec_shell(), Rights::EXECUTE));
        assert!(!cs.can(&ResourceRef::Browser, Rights::EXECUTE));
    }
}
