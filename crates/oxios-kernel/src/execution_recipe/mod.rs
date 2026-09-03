//! Execution recipes — Oxios-owned declarative selection of portable
//! oxicode-sdk behavior packs.
//!
//! A recipe decides *what behavior is requested* for a turn; Oxios policy
//! (AccessManager, CSpace, approval, audit) decides *whether and how it may
//! execute*. The default recipe, `general-v1`, selects no pack and leaves the
//! existing CSpace tool composition untouched — ordinary turns pay nothing.
//! `coding-omp-v1` (pilot, behind `[execution_recipe].coding_pilot`) selects
//! the released oxicode-sdk coding behavior pack, installed through the same
//! gated registration path as native tools.
//!
//! Design: `docs/designs/2026-08-31-execution-recipe-coding-host-design.md`
//! (Oxios) and the paired
//! `oxicode/docs/designs/2026-08-31-omp-compatible-behavior-pack-design.md`
//! (SDK contract).

pub mod extensions;
pub mod installer;
pub mod resolver;
pub mod types;

pub use installer::{
    grant_pack_tool_permissions, install_coding_recipe, install_coding_turn, pack_installable_names,
};
pub use resolver::{ExecutionRecipeResolver, RecipeRequest};
pub use types::{
    CODING_OMP_V1, CodingServiceFlags, GENERAL_V1, RecipeId, RecipeSource, ResolvedExecution,
};
