//! Capability templates — preset CSpace configurations for common agent roles.
//!
//! Templates provide a declarative way to define an agent's initial
//! capability set. They encode the "principle of least privilege" by
//! starting from minimal access and layering on rights as the role
//! demands.
//!
//! # Hierarchy
//!
//! ```text
//! worker()     → Exec + Browser + Knowledge(RW)
//!   standard() → worker + Memory(READ)
//!   operator() → standard + Agent + A2a + Program + MCP + Memory(WRITE)
//!   supervisor() → operator + Security + Budget + Resource + Cron
//! ```
//!
//! # Example
//!
//! ```
//! use oxios_kernel::capability::template::CapabilityTemplate;
//! use oxios_kernel::types::AgentId;
//!
//! let cspace = CapabilityTemplate::standard().build_for(AgentId::new_v4());
//! assert!(cspace.len() > 0);
//! ```

use crate::types::AgentId;

use super::types::{CSpace, Capability, CapabilityId, Issuer, ResourceRef, Rights};

/// Builder for constructing preset capability spaces.
///
/// Use the associated constructors (`worker`, `standard`, `operator`,
/// `supervisor`, `with_skills`) to start from a template, then call
/// [`build`] or [`build_for`] to produce a [`CSpace`].
#[derive(Debug, Clone)]
pub struct CapabilityTemplate {
    caps: Vec<(ResourceRef, Rights)>,
}

impl CapabilityTemplate {
    // ── Preset constructors ─────────────────────────────────────────

    /// **Worker** — minimal execution capability.
    ///
    /// Rights: shell exec + headless browser.
    pub fn worker() -> Self {
        let mut t = Self { caps: Vec::new() };
        t.caps.push((
            ResourceRef::Exec {
                mode: "shell".into(),
            },
            Rights::EXECUTE | Rights::READ,
        ));
        t.caps
            .push((ResourceRef::Browser, Rights::READ | Rights::EXECUTE));
        t
    }

    /// **Standard** — worker + memory read access.
    ///
    /// Suitable for most agents that need to recall but not modify
    /// persistent state.
    pub fn standard() -> Self {
        let mut t = Self::worker();
        t.caps.push((
            ResourceRef::KernelDomain {
                domain: "memory".into(),
            },
            Rights::READ,
        ));
        t
    }

    /// **Operator** — standard + agent, A2A, persona, program,
    /// MCP, and memory write.
    ///
    /// Intended for agents that coordinate work across multiple
    /// subsystems (e.g., a project lead agent).
    pub fn operator() -> Self {
        let mut t = Self::standard();
        let extra = vec![
            (
                ResourceRef::Agent { id: AgentId::nil() },
                Rights::READ | Rights::WRITE,
            ),
            (
                ResourceRef::A2a,
                Rights::READ | Rights::WRITE | Rights::EXECUTE,
            ),
            (
                ResourceRef::KernelDomain {
                    domain: "persona".into(),
                },
                Rights::READ | Rights::WRITE,
            ),
            (
                ResourceRef::KernelDomain {
                    domain: "program".into(),
                },
                Rights::READ | Rights::WRITE | Rights::EXECUTE,
            ),
            (
                ResourceRef::Mcp { server: "*".into() },
                Rights::READ | Rights::EXECUTE,
            ),
            // Upgrade memory to RW
            (
                ResourceRef::KernelDomain {
                    domain: "memory".into(),
                },
                Rights::READ | Rights::WRITE,
            ),
        ];
        t.caps.extend(extra);
        t
    }

    /// **Supervisor** — operator + security, budget, and resource
    /// kernel domains.
    ///
    /// The most privileged built-in template. Use sparingly.
    pub fn supervisor() -> Self {
        let mut t = Self::operator();
        let admin = vec![
            (
                ResourceRef::KernelDomain {
                    domain: "security".into(),
                },
                Rights::ALL,
            ),
            (
                ResourceRef::KernelDomain {
                    domain: "budget".into(),
                },
                Rights::READ | Rights::WRITE,
            ),
            (
                ResourceRef::KernelDomain {
                    domain: "resource".into(),
                },
                Rights::READ | Rights::WRITE,
            ),
        ];
        t.caps.extend(admin);
        t
    }

    /// **With skills** — worker + specific named skills.
    ///
    /// Creates a worker-level agent with EXECUTE rights on the listed
    /// skills only. This is the recommended template for agents that
    /// should have access to a known set of tools.
    pub fn with_skills(names: &[&str]) -> Self {
        let mut t = Self::worker();
        for name in names {
            t.caps.push((
                ResourceRef::Skill {
                    name: (*name).into(),
                },
                Rights::EXECUTE | Rights::READ,
            ));
        }
        t
    }

    // ── Persona tool-profile templates ─────────────────────────────
    //
    // These back `Persona::tool_profile`. They grant exactly the
    // resources the CSpace→tool registration walk understands
    // (`tools::registration::register_tools_from_cspace_gated`):
    // `ResourceRef::Exec` / `Browser` for those tiers, and
    // `KernelDomain` names for kernel tools. Registration presence is
    // the restriction mechanism — kernel tools are not wrapped in the
    // access gate, so an ungranted domain means the tool is simply
    // absent from the agent's registry.

    /// **Code** — coding/working tools, no Oxios control.
    ///
    /// Always-on file/web-search tools (registered unconditionally) plus
    /// exec, browser, memory, and knowledge. No persona/cron/security/
    /// budget/resource/agent/a2a/project registration.
    pub fn code_profile() -> Self {
        let mut t = Self::worker();
        t.caps.extend([
            (
                ResourceRef::KernelDomain {
                    domain: "memory".into(),
                },
                Rights::EXECUTE,
            ),
            (
                ResourceRef::KernelDomain {
                    domain: "knowledge".into(),
                },
                Rights::EXECUTE,
            ),
        ]);
        t
    }

    /// **Base** — every tool including Oxios control.
    ///
    /// Superset of [`code_profile`](Self::code_profile) plus the full
    /// control plane: project, agent lifecycle, A2A, persona,
    /// security, budget, and resource domains.
    pub fn base_profile() -> Self {
        let mut t = Self::code_profile();
        let control: [&str; 7] = [
            "project", "agent", "a2a", "persona", "security", "budget", "resource",
        ];
        t.caps.extend(control.map(|domain| {
            (
                ResourceRef::KernelDomain {
                    domain: domain.into(),
                },
                Rights::READ | Rights::WRITE | Rights::EXECUTE,
            )
        }));
        t
    }

    /// **Minimal** — always-on tier only (file ops + web search).
    ///
    /// Grants nothing: the always-on tools are registered unconditionally
    /// and the gate skips its CSpace check for them. No exec, browser,
    /// memory, knowledge, or control tools.
    pub fn minimal_profile() -> Self {
        Self { caps: Vec::new() }
    }

    /// **Control** — Oxios self-management only. No exec/browser/file tools,
    /// no user-content tools (email send, calendar events).
    ///
    /// Grants exactly the domains needed to inspect and mutate Oxios's own
    /// configuration: projects, personas, tasks, security, budget,
    /// resource limits, running agents, marketplace skills, MCP server
    /// management, and model/engine settings.
    pub fn control_profile() -> Self {
        let mut t = Self { caps: Vec::new() };
        let domains = [
            "project",
            "persona",
            "task",
            "security",
            "budget",
            "resource",
            "agent",
            "marketplace",
            "mcp_manage",
            "engine",
        ];
        t.caps.extend(domains.map(|d| {
            (
                ResourceRef::KernelDomain { domain: d.into() },
                Rights::READ | Rights::WRITE | Rights::EXECUTE,
            )
        }));
        t
    }

    // ── Builder methods ─────────────────────────────────────────────

    /// Add an additional capability to the template.
    pub fn with(mut self, resource: ResourceRef, rights: Rights) -> Self {
        self.caps.push((resource, rights));
        self
    }

    /// Build a CSpace with kernel-issued capabilities for a fresh agent ID.
    pub fn build(&self) -> CSpace {
        self.build_for(AgentId::new_v4())
    }

    /// Build a CSpace with kernel-issued capabilities for a specific agent.
    pub fn build_for(&self, agent_id: AgentId) -> CSpace {
        let mut cspace = CSpace::new(agent_id);
        for (resource, rights) in &self.caps {
            let cap = Capability {
                id: CapabilityId::new(),
                resource: resource.clone(),
                rights: *rights,
                issuer: Issuer::Kernel,
            };
            cspace.insert(cap);
        }
        cspace
    }

    /// Returns the number of capabilities in this template.
    pub fn len(&self) -> usize {
        self.caps.len()
    }

    /// Returns true if the template has no capabilities.
    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }
}

impl Default for CapabilityTemplate {
    fn default() -> Self {
        Self::worker()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_has_exec_and_browser() {
        let cs = CapabilityTemplate::worker().build();
        assert!(cs.can(
            &ResourceRef::Exec {
                mode: "shell".into()
            },
            Rights::EXECUTE
        ));
        assert!(cs.can(&ResourceRef::Browser, Rights::READ));
        assert_eq!(cs.len(), 2);
    }

    #[test]
    fn standard_grants_memory_domain_read_only() {
        let cs = CapabilityTemplate::standard().build();
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "memory".into()
            },
            Rights::READ
        ));
        assert!(!cs.can(
            &ResourceRef::KernelDomain {
                domain: "memory".into()
            },
            Rights::WRITE
        ));
    }

    #[test]
    fn operator_has_a2a_and_mcp() {
        let cs = CapabilityTemplate::operator().build();
        assert!(cs.can(&ResourceRef::A2a, Rights::EXECUTE));
        assert!(cs.can(&ResourceRef::Mcp { server: "*".into() }, Rights::EXECUTE));
    }

    #[test]
    fn supervisor_has_security_all() {
        let cs = CapabilityTemplate::supervisor().build();
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "security".into()
            },
            Rights::ALL
        ));
    }

    #[test]
    fn with_skills_scoped() {
        let cs = CapabilityTemplate::with_skills(&["git", "gh"]).build();
        assert!(cs.can(&ResourceRef::Skill { name: "git".into() }, Rights::EXECUTE));
        assert!(cs.can(&ResourceRef::Skill { name: "gh".into() }, Rights::EXECUTE));
        assert!(!cs.can(
            &ResourceRef::Skill {
                name: "curl".into()
            },
            Rights::EXECUTE
        ));
    }

    #[test]
    fn builder_chaining() {
        let cs = CapabilityTemplate::worker()
            .with(
                ResourceRef::KernelDomain {
                    domain: "custom".into(),
                },
                Rights::READ,
            )
            .build();
        assert!(cs.can(
            &ResourceRef::KernelDomain {
                domain: "custom".into()
            },
            Rights::READ
        ));
    }

    #[test]
    fn build_for_specific_agent() {
        let id = AgentId::new_v4();
        let cs = CapabilityTemplate::worker().build_for(id);
        assert_eq!(cs.agent_id, id);
    }

    #[test]
    fn control_profile_grants_only_control_domains() {
        let t = CapabilityTemplate::control_profile();
        let cs = t.build_for(AgentId::new_v4());
        let kd: Vec<String> = cs
            .iter()
            .filter_map(|c| match &c.resource {
                ResourceRef::KernelDomain { domain } => Some(domain.clone()),
                _ => None,
            })
            .collect();
        let mut expected = vec![
            "project",
            "persona",
            "task",
            "security",
            "budget",
            "resource",
            "agent",
            "marketplace",
            "mcp_manage",
            "engine",
        ];
        expected.sort();
        let mut got = kd.clone();
        got.sort();
        assert_eq!(got, expected);
        // No exec, browser, memory, knowledge, a2a, or skill resources.
        for cap in cs.iter() {
            assert!(
                !matches!(
                    &cap.resource,
                    ResourceRef::Exec { .. } | ResourceRef::Browser | ResourceRef::Skill { .. }
                ),
                "control profile must not grant {:?}",
                cap.resource
            );
        }
    }
}
