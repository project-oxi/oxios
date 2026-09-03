//! Execution-recipe core types.
//!
//! Ownership follows the design's layer table: the recipe resolver selects
//! packs/overlays; Oxios policy (AccessManager/CSpace/approval) stays the sole
//! authority over execution. `ResolvedExecution` is the materialized selection
//! handed to the runtime before tool registration — it never widens authority.

use serde::{Deserialize, Serialize};

/// System-default recipe: no behavior pack, current CSpace composition.
pub const GENERAL_V1: &str = "general-v1";

/// Reference coding pack (oxicode-sdk `coding-omp-v1`), pilot-gated.
pub const CODING_OMP_V1: &str = "coding-omp-v1";

/// Stable recipe identifier (`general-v1`, `coding-omp-v1`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecipeId(pub String);

impl RecipeId {
    /// The system default.
    pub fn general_v1() -> Self {
        RecipeId(GENERAL_V1.to_string())
    }
}

impl std::fmt::Display for RecipeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why the resolver selected a recipe — fixed precedence order (design
/// "Recipe selection"): explicit authorized request > project binding >
/// persona default > system default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecipeSource {
    /// Explicitly authorized request selection (config-originated; reserved —
    /// channels never send arbitrary recipe ids).
    Explicit,
    /// Project recipe/coding binding (`[execution_recipe].project_bindings`).
    ProjectBinding,
    /// Persona default (reserved — no persona carries a recipe yet).
    PersonaDefault,
    /// System default.
    SystemDefault,
}

/// Which host services a coding recipe turn may wire into the pack installer.
///
/// lsp / debug / ttsr / delegation have no host-side implementation yet —
/// they always resolve to structured degradations (design rollout step 4),
/// never to a silently substituted tool.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingServiceFlags {
    /// Provide a persistent shell session (`PersistentShellSession`) so the
    /// pack's `bash` tool routes through OMP-style session semantics.
    pub shell: bool,
    /// Provide persistent Python/JavaScript eval kernels.
    pub eval: bool,
}

/// Materialized recipe selection for one turn, resolved before tool
/// registration and recorded in turn metadata.
#[derive(Debug, Clone)]
pub struct ResolvedExecution {
    /// Selected recipe id.
    pub recipe: RecipeId,
    /// Behavior packs to install (empty for `general-v1`).
    pub pack_ids: Vec<oxicode_sdk::behavior::BehaviorPackId>,
    /// Precedence level that produced this selection.
    pub source: RecipeSource,
    /// Host services the coding pilot may wire.
    pub services: CodingServiceFlags,
}

impl ResolvedExecution {
    /// The system default: no pack, no change to the existing composition.
    pub fn general_v1() -> Self {
        ResolvedExecution {
            recipe: RecipeId::general_v1(),
            pack_ids: Vec::new(),
            source: RecipeSource::SystemDefault,
            services: CodingServiceFlags::default(),
        }
    }

    /// Whether this resolution selects a behavior pack for the turn.
    pub fn is_coding(&self) -> bool {
        !self.pack_ids.is_empty()
    }
}
