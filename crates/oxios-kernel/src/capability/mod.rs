//! Capability-based access control for the Oxios kernel.
//!
//! This module implements a capability system inspired by seL4 and
//! capability-based security research. Capabilities are unforgeable
//! tokens that encode authority over specific resources.
//!
//! # Architecture
//!
//! - **Capability**: An unforgeable token binding rights to a resource.
//! - **CSpace**: An agent's complete set of capabilities (capability space).
//! - **Rights**: Bit-flag permissions (READ, WRITE, EXECUTE, DELEGATE).
//! - **ResourceRef**: Identifies a protected resource in the system.
//!
//! # Module layout
//!
//! | Module | Purpose |
//! |--------|---------|
//! | `types` | Core types: Capability, CSpace, Rights, ResourceRef, etc. |
//! | `template` | Preset CSpace configurations for common agent roles. |
//! | `resolve` | Resolves an agent's CSpace from Seed + Config. |
//! | `descriptor` | Tool descriptors and the static kernel catalog. |

pub mod resolve;
pub mod template;
pub mod types;

// Re-export core types at module root for convenience.
pub use types::{CSpace, Capability, CapabilityId, Issuer, ResourceRef, Rights};
