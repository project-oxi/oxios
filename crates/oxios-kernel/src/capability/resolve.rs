//! CSpace resolution — determines an agent's initial capability space from
//! Directive + Config inputs.
//!
//! The resolution follows a priority chain:
//!
//! 1. **Explicit cspace hint** on the directive → parse and use it.
//! 2. **Persona role** → map known roles to built-in templates.
//! 3. **Default** → fall back to the `worker` template.
//!
//! # Example
//!
//! ```no_run
//! use oxios_kernel::capability::resolve::resolve_cspace;
//! use oxios_kernel::types::AgentId;
//!
//! let cspace = resolve_cspace(None, Some("operator"), None, AgentId::new_v4());
//! assert!(cspace.len() > 2);
//! ```

use crate::types::AgentId;

use super::template::CapabilityTemplate;
use super::types::CSpace;

/// Known role names that map to built-in capability templates.
const ROLE_WORKER: &str = "worker";
const ROLE_STANDARD: &str = "standard";
const ROLE_OPERATOR: &str = "operator";
const ROLE_SUPERVISOR: &str = "supervisor";

/// Resolve an agent's initial CSpace from the available context.
///
/// # Arguments
///
/// * `cspace_hint` — Optional hint string from the Directive. Can be a known
///   template name ("worker", "standard", "operator", "supervisor") or a
///   JSON object describing custom capabilities.
/// * `persona_role` — The role field of the assigned persona, if any.
/// * `default_template` — Optional override for the fallback template name.
///   Defaults to "worker" if not specified.
/// * `agent_id` — The agent that will own the resolved CSpace.
///
/// # Priority
///
/// 1. `cspace_hint` (if present and non-empty)
/// 2. `persona_role` (if present and matches a known role)
/// 3. `default_template` or `"worker"` as fallback
pub fn resolve_cspace(
    cspace_hint: Option<&str>,
    persona_role: Option<&str>,
    default_template: Option<&str>,
    agent_id: AgentId,
) -> CSpace {
    // 1. Explicit hint from directive takes highest priority.
    if let Some(hint) = cspace_hint {
        let trimmed = hint.trim();
        if !trimmed.is_empty() {
            return resolve_from_template_name(trimmed, agent_id);
        }
    }

    // 2. Persona role maps to a template.
    if let Some(role) = persona_role {
        let trimmed = role.trim().to_lowercase();
        if !trimmed.is_empty() {
            return resolve_from_template_name(&trimmed, agent_id);
        }
    }

    // 3. Default fallback.
    let fallback = default_template.unwrap_or(ROLE_WORKER);
    resolve_from_template_name(fallback, agent_id)
}

/// Map a template name to a built-in CapabilityTemplate.
///
/// If the name is a JSON object, we try to parse it as a custom template
/// (future: parse JSON capabilities). For now, unknown names fall back to
/// worker.
fn resolve_from_template_name(name: &str, agent_id: AgentId) -> CSpace {
    match name {
        ROLE_WORKER => CapabilityTemplate::worker().build_for(agent_id),
        ROLE_STANDARD => CapabilityTemplate::standard().build_for(agent_id),
        ROLE_OPERATOR => CapabilityTemplate::operator().build_for(agent_id),
        ROLE_SUPERVISOR => CapabilityTemplate::supervisor().build_for(agent_id),
        _ => {
            // If it looks like JSON, log a warning and fall back.
            // Full JSON capability parsing is a future enhancement.
            if name.starts_with('{') {
                tracing::warn!(
                    "JSON cspace_hint not yet supported, falling back to worker: {}",
                    name
                );
            } else {
                tracing::warn!(
                    "Unknown capability template '{}', falling back to worker",
                    name
                );
            }
            CapabilityTemplate::worker().build_for(agent_id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_takes_priority_over_role() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(Some("supervisor"), Some("worker"), None, id);
        // supervisor has security domain, worker does not
        use super::super::types::{ResourceRef, Rights};
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "security".into()
            },
            Rights::ALL,
        ));
    }

    #[test]
    fn role_used_when_no_hint() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(None, Some("operator"), None, id);
        use super::super::types::{ResourceRef, Rights};
        assert!(cs.can(&ResourceRef::A2a, Rights::EXECUTE));
    }

    #[test]
    fn default_is_worker() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(None, None, None, id);
        use super::super::types::{ResourceRef, Rights};
        assert!(cs.can(
            &ResourceRef::Exec {
                mode: "shell".into()
            },
            Rights::EXECUTE
        ));
        // Worker should NOT have A2A
        assert!(!cs.can(&ResourceRef::A2a, Rights::READ));
    }

    #[test]
    fn custom_default_template() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(None, None, Some("standard"), id);
        use super::super::types::{ResourceRef, Rights};
        // Standard template now grants typed `ResourceRef::Memory`
        // (READ) — not the legacy `KernelDomain{"memory"}` string.
        assert!(cs.can(&ResourceRef::Memory, Rights::READ));
    }

    #[test]
    fn empty_hint_falls_through() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(Some(""), Some("operator"), None, id);
        use super::super::types::{ResourceRef, Rights};
        assert!(cs.can(&ResourceRef::A2a, Rights::EXECUTE));
    }

    #[test]
    fn unknown_name_falls_back_to_worker() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(Some("nonexistent"), None, None, id);
        use super::super::types::{ResourceRef, Rights};
        assert!(cs.can(
            &ResourceRef::Exec {
                mode: "shell".into()
            },
            Rights::EXECUTE
        ));
    }

    #[test]
    fn json_hint_falls_back_gracefully() {
        let id = AgentId::new_v4();
        let cs = resolve_cspace(Some(r#"{"custom": true}"#), None, None, id);
        use super::super::types::ResourceRef;
        // Falls back to worker
        assert!(cs.can(
            &ResourceRef::Exec {
                mode: "shell".into()
            },
            super::super::types::Rights::EXECUTE
        ));
    }
}
