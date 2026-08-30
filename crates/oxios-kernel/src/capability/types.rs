//! Core capability types for the Oxios capability system.
//!
//! Capabilities are unforgeable tokens that encode authority over specific
//! resources. An agent's capability space (CSpace) is the complete set of
//! capabilities it holds.
//!
//! # Design
//!
//! Inspired by capability-based security (seL4, Capsicum), each capability
//! binds a set of rights to a specific resource. Capabilities cannot be
//! forged — they are issued by the kernel or by agents with DELEGATE rights.
//!
//! ```
//! use oxios_kernel::capability::types::*;
//! use oxios_kernel::capability::template::CapabilityTemplate;
//!
//! let cspace = CapabilityTemplate::worker().build();
//! assert!(!cspace.is_empty());
//! ```

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::AgentId;

/// Unique identifier for a capability.
///
/// Each capability receives a random UUID at creation time, making
/// capabilities unforgeable — an agent cannot guess a valid CapabilityId.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityId(pub uuid::Uuid);

impl CapabilityId {
    /// Generate a new random capability ID.
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cap:{}", self.0)
    }
}

impl Default for CapabilityId {
    fn default() -> Self {
        Self::new()
    }
}

/// Who issued a capability.
///
/// The kernel is the root authority. Agents with DELEGATE rights can
/// issue derived capabilities scoped to their own authority.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "issuer", content = "id")]
pub enum Issuer {
    /// The Oxios kernel — root authority.
    Kernel,
    /// An agent that delegated a subset of its own authority.
    Agent(AgentId),
}

impl fmt::Display for Issuer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Issuer::Kernel => write!(f, "kernel"),
            Issuer::Agent(id) => write!(f, "agent:{id}"),
        }
    }
}

/// Bit-flag rights encoded in a capability.
///
/// Rights are represented as a `u8` bitmask. Standard combinations are
/// provided as associated constants.
///
/// | Flag     | Value |
/// |----------|-------|
/// | NONE     | 0x00  |
/// | READ     | 0x01  |
/// | WRITE    | 0x02  |
/// | EXECUTE  | 0x04  |
/// | DELEGATE | 0x08  |
/// | ALL      | 0x0F  |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rights(pub u8);

impl Rights {
    /// No rights at all.
    pub const NONE: Rights = Rights(0x00);
    /// Read access to a resource.
    pub const READ: Rights = Rights(0x01);
    /// Write / mutate access to a resource.
    pub const WRITE: Rights = Rights(0x02);
    /// Execute a resource (run a program, invoke a tool).
    pub const EXECUTE: Rights = Rights(0x04);
    /// Right to delegate a subset of this capability to another agent.
    pub const DELEGATE: Rights = Rights(0x08);
    /// All rights combined.
    pub const ALL: Rights = Rights(0x0F);

    /// Returns `true` if this set contains all the given rights.
    pub fn contains(self, other: Rights) -> bool {
        (self.0 & other.0) == other.0
    }

    /// Returns a new Rights set with the given flags added.
    pub fn union(self, other: Rights) -> Rights {
        Rights(self.0 | other.0)
    }

    /// Returns a new Rights set with only the flags present in both.
    pub fn intersect(self, other: Rights) -> Rights {
        Rights(self.0 & other.0)
    }

    /// Returns true if no rights are set.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl fmt::Display for Rights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 == Rights::ALL.0 {
            return write!(f, "ALL");
        }
        if self.0 == Rights::NONE.0 {
            return write!(f, "NONE");
        }
        let mut parts = Vec::new();
        if self.contains(Rights::READ) {
            parts.push("R");
        }
        if self.contains(Rights::WRITE) {
            parts.push("W");
        }
        if self.contains(Rights::EXECUTE) {
            parts.push("X");
        }
        if self.contains(Rights::DELEGATE) {
            parts.push("D");
        }
        write!(f, "{}", parts.join("|"))
    }
}

impl std::ops::BitOr for Rights {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Rights(self.0 | rhs.0)
    }
}

impl std::ops::BitAnd for Rights {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Rights(self.0 & rhs.0)
    }
}

/// A reference to a protected resource.
///
/// Resources are the objects that capabilities govern. Each variant
/// identifies a distinct resource class in the Oxios kernel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "ref")]
pub enum ResourceRef {
    /// A kernel domain (e.g., "memory", "scheduler").
    KernelDomain {
        /// Domain name within the kernel.
        domain: String,
    },
    /// An installed skill (unified programs + skills per RFC-009).
    Skill {
        /// Skill name as registered in the skill manager.
        name: String,
    },
    /// Another agent (for inter-agent communication).
    Agent {
        /// Agent identifier.
        id: AgentId,
    },
    /// Command execution (shell or structured).
    Exec {
        /// Execution mode: "shell" or "structured".
        mode: String,
    },
    /// Headless browser access.
    Browser,
    /// Agent-to-agent communication channel.
    A2a,
    /// MCP (Model Context Protocol) server bridge.
    Mcp {
        /// MCP server name.
        server: String,
    },
    /// Filesystem primitive tools (read/write/edit/grep/find/ls).
    Fs,
    /// Web search tools (web_search/get_search_results).
    WebSearch,
    /// Agent memory (oxibrain-backed recall/retain).
    Memory,
    /// User markdown knowledge base.
    Knowledge,
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceRef::KernelDomain { domain } => write!(f, "kernel:{domain}"),
            ResourceRef::Skill { name } => write!(f, "skill:{name}"),
            ResourceRef::Agent { id } => write!(f, "agent:{id}"),
            ResourceRef::Exec { mode } => write!(f, "exec:{mode}"),
            ResourceRef::Browser => write!(f, "browser"),
            ResourceRef::A2a => write!(f, "a2a"),
            ResourceRef::Mcp { server } => write!(f, "mcp:{server}"),
            ResourceRef::Fs => write!(f, "fs"),
            ResourceRef::WebSearch => write!(f, "web_search"),
            ResourceRef::Memory => write!(f, "memory"),
            ResourceRef::Knowledge => write!(f, "knowledge"),
        }
    }
}

/// An unforgeable token encoding authority over a specific resource.
///
/// A capability binds a set of `Rights` to a `ResourceRef`, and records
/// who issued it. Agents present capabilities to the `AccessManager` to
/// prove they are authorised to perform an action.
///
/// # Immutability
///
/// Capabilities are immutable once created. To change rights, create a
/// new capability and replace the old one in the CSpace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capability {
    /// Unique identifier for this capability.
    pub id: CapabilityId,
    /// The resource this capability governs.
    pub resource: ResourceRef,
    /// The rights granted over the resource.
    pub rights: Rights,
    /// Who issued this capability.
    pub issuer: Issuer,
}

impl Capability {
    /// Creates a new kernel-issued capability with the given resource and rights.
    pub fn kernel(resource: ResourceRef, rights: Rights) -> Self {
        Self {
            id: CapabilityId::new(),
            resource,
            rights,
            issuer: Issuer::Kernel,
        }
    }

    /// Creates a new agent-issued capability (delegation).
    pub fn delegated(resource: ResourceRef, rights: Rights, issuer: AgentId) -> Self {
        Self {
            id: CapabilityId::new(),
            resource,
            rights,
            issuer: Issuer::Agent(issuer),
        }
    }

    /// Returns true if this capability grants the requested rights.
    pub fn grants(&self, required: Rights) -> bool {
        self.rights.contains(required)
    }
}

/// An agent's **capability space**: the complete set of capabilities it holds.
///
/// The CSpace is a `HashMap<CapabilityId, Capability>`. When the
/// `AccessManager` checks whether an agent may perform an action, it
/// scans the agent's CSpace for a capability matching the target
/// resource with sufficient rights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CSpace {
    /// The agent that owns this capability space.
    pub agent_id: AgentId,
    /// Map from capability ID to capability.
    caps: HashMap<CapabilityId, Capability>,
}

impl CSpace {
    /// Creates an empty CSpace for the given agent.
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            caps: HashMap::new(),
        }
    }

    /// Inserts a capability into this space, returning the old one if replaced.
    pub fn insert(&mut self, cap: Capability) -> Option<Capability> {
        self.caps.insert(cap.id, cap)
    }

    /// Removes a capability by ID.
    pub fn remove(&mut self, id: &CapabilityId) -> Option<Capability> {
        self.caps.remove(id)
    }

    /// Looks up a capability by ID.
    pub fn get(&self, id: &CapabilityId) -> Option<&Capability> {
        self.caps.get(id)
    }

    /// Returns true if any capability in this space matches the resource
    /// and grants the requested rights.
    pub fn can(&self, resource: &ResourceRef, required: Rights) -> bool {
        self.caps
            .values()
            .any(|cap| &cap.resource == resource && cap.grants(required))
    }

    /// Returns an iterator over all capabilities in this space.
    pub fn iter(&self) -> impl Iterator<Item = &Capability> {
        self.caps.values()
    }

    /// Returns the number of capabilities.
    pub fn len(&self) -> usize {
        self.caps.len()
    }

    /// Returns true if there are no capabilities.
    pub fn is_empty(&self) -> bool {
        self.caps.is_empty()
    }

    /// Retains only capabilities for which `pred` returns true.
    pub fn retain(&mut self, pred: impl Fn(&Capability) -> bool) {
        self.caps.retain(|_, cap| pred(cap));
    }

    /// Returns capabilities matching a specific resource type filter.
    pub fn filter_resource(&self, f: impl Fn(&ResourceRef) -> bool) -> Vec<&Capability> {
        self.caps.values().filter(|c| f(&c.resource)).collect()
    }

    /// Returns the unique kernel domain names active in this CSpace.
    ///
    /// Useful for building a kernel manifest for the system prompt.
    pub fn active_domains(&self) -> Vec<&str> {
        let mut domains: Vec<&str> = self
            .caps
            .values()
            .filter_map(|cap| match &cap.resource {
                ResourceRef::KernelDomain { domain } => Some(domain.as_str()),
                ResourceRef::Exec { .. } => Some("exec"),
                ResourceRef::Browser => Some("browser"),
                _ => None,
            })
            .collect();
        domains.sort();
        domains.dedup();
        domains
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rights_bit_ops() {
        let rw = Rights::READ | Rights::WRITE;
        assert!(rw.contains(Rights::READ));
        assert!(rw.contains(Rights::WRITE));
        assert!(!rw.contains(Rights::EXECUTE));

        let r = rw.intersect(Rights::READ);
        assert_eq!(r, Rights::READ);
    }

    #[test]
    fn rights_display() {
        assert_eq!(format!("{}", Rights::ALL), "ALL");
        assert_eq!(format!("{}", Rights::NONE), "NONE");
        assert_eq!(format!("{}", Rights::READ | Rights::EXECUTE), "R|X");
    }

    #[test]
    fn cspace_can_check() {
        let agent = AgentId::new_v4();
        let mut cs = CSpace::new(agent);
        let cap = Capability::kernel(
            ResourceRef::Exec {
                mode: "shell".into(),
            },
            Rights::READ | Rights::EXECUTE,
        );
        cs.insert(cap);

        assert!(cs.can(
            &ResourceRef::Exec {
                mode: "shell".into()
            },
            Rights::EXECUTE
        ));
        assert!(!cs.can(
            &ResourceRef::Exec {
                mode: "shell".into()
            },
            Rights::WRITE
        ));
        assert!(!cs.can(&ResourceRef::Browser, Rights::READ));
    }

    #[test]
    fn capability_delegation() {
        let issuer = AgentId::new_v4();
        let cap = Capability::delegated(
            ResourceRef::Agent { id: issuer },
            Rights::READ | Rights::DELEGATE,
            issuer,
        );
        assert!(matches!(cap.issuer, Issuer::Agent(_)));
        assert!(cap.grants(Rights::DELEGATE));
    }
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
            ResourceRef::KernelDomain {
                domain: "memory".into()
            }
        );
    }
}
