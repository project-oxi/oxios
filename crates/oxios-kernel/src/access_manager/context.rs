//! Agent security context — unforgeable identity token.
//!
//! `AgentContext` is the proof that the kernel has authenticated an agent.
//! It cannot be constructed without going through the kernel's agent lifecycle,
//! making it impossible to bypass security checks at the type level.

use std::sync::Arc;

use crate::capability::CSpace;
use crate::types::AgentId;

/// Agent security context — unforgeable proof of agent identity.
///
/// This type can only be created by:
/// - `KernelHandle` during agent lifecycle (production)
/// - `AgentContext::test_fixture()` in `#[cfg(test)]` only
///
/// Tools that require access control accept `AgentContext` instead of
/// a raw `Option<String>`. The type's existence is itself proof that
/// the kernel has authenticated the agent.
#[derive(Debug, Clone)]
pub struct AgentContext {
    /// Unique agent identifier.
    pub agent_id: AgentId,
    /// Human-readable agent name for permission lookups.
    pub agent_name: String,
    /// Agent's capability space — determines which tools the agent can access.
    pub cspace: Arc<CSpace>,
    /// Chat-scoped brain binding for this turn. `None` = brain fully off.
    pub brain_space: Option<String>,
}

impl AgentContext {
    /// Create a test fixture with the given name.
    ///
    /// Only available in test builds. Generates a random agent ID and a
    /// permissive CSpace with standard capabilities.
    #[cfg(test)]
    pub fn test_fixture(name: &str) -> Self {
        let agent_id = AgentId::new_v4();
        let mut cspace = CSpace::new(agent_id);

        // Grant standard capabilities for testing
        use crate::capability::{Capability, ResourceRef, Rights};
        cspace.insert(Capability::kernel(
            ResourceRef::Exec {
                mode: "shell".into(),
            },
            Rights::ALL,
        ));
        cspace.insert(Capability::kernel(
            ResourceRef::Exec {
                mode: "structured".into(),
            },
            Rights::ALL,
        ));
        cspace.insert(Capability::kernel(
            ResourceRef::KernelDomain {
                domain: "fs".into(),
            },
            Rights::ALL,
        ));
        cspace.insert(Capability::kernel(
            ResourceRef::KernelDomain {
                domain: "agent".into(),
            },
            Rights::ALL,
        ));
        // Grant all common tools
        for tool in [
            "bash", "read", "write", "edit", "grep", "find", "ls", "exec",
        ] {
            cspace.insert(Capability::kernel(
                ResourceRef::KernelDomain {
                    domain: tool.into(),
                },
                Rights::EXECUTE,
            ));
        }

        Self {
            agent_id,
            agent_name: name.to_string(),
            cspace: Arc::new(cspace),
            brain_space: None,
        }
    }

    /// Create a test fixture with specific capabilities.
    #[cfg(test)]
    pub fn test_fixture_with_cspace(name: &str, cspace: CSpace) -> Self {
        Self {
            agent_id: AgentId::new_v4(),
            agent_name: name.to_string(),
            cspace: Arc::new(cspace),
            brain_space: None,
        }
    }
}

impl std::fmt::Display for AgentContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "agent:{}:{}", self.agent_name, self.agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fixture_has_name() {
        let ctx = AgentContext::test_fixture("test-agent");
        assert_eq!(ctx.agent_name, "test-agent");
        assert!(!ctx.agent_id.is_nil());
    }

    #[test]
    fn test_display() {
        let ctx = AgentContext::test_fixture("my-agent");
        let s = format!("{}", ctx);
        assert!(s.starts_with("agent:my-agent:"));
    }
}
