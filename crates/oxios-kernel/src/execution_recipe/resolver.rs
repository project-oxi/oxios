//! Recipe resolution — the fixed-precedence selection of a turn's execution
//! recipe (design "Recipe selection").
//!
//! Precedence: explicit authorized request > project binding > persona
//! default > system default (`general-v1`). The resolver never widens
//! authority: a binding only selects a pack when `[execution_recipe]` is
//! enabled AND `coding_pilot` is on, and an unknown binding id degrades to
//! `general-v1` (a config typo must not fail a turn).

use super::types::{CODING_OMP_V1, CodingServiceFlags, RecipeId, RecipeSource, ResolvedExecution};

/// Inputs to one resolution. Channels never carry recipe ids; `explicit` is
/// reserved for config-originated selection (no producer yet).
#[derive(Debug, Clone, Default)]
pub struct RecipeRequest<'a> {
    /// Active project for the turn (from `ExecEnv.project_id`).
    pub project_id: Option<uuid::Uuid>,
    /// Explicit, authorized selection (reserved).
    pub explicit: Option<RecipeId>,
    /// Turn persona id (reserved — personas carry no recipe yet).
    pub persona_id: Option<&'a str>,
}

/// Deterministic recipe resolver, held on the kernel and resolved once per
/// turn alongside persona/CSpace resolution.
#[derive(Debug, Clone)]
pub struct ExecutionRecipeResolver {
    cfg: crate::config::ExecutionRecipeConfig,
}

/// What a recognized recipe id selects.
#[derive(Debug, Clone, Copy)]
enum Selection {
    /// No pack (explicit `general-v1`).
    General,
    /// The released `coding-omp-v1` behavior pack.
    Coding,
}

impl ExecutionRecipeResolver {
    /// Resolver bound to a config snapshot (re-resolved on config reload the
    /// same way other per-turn config snapshots are).
    pub fn new(cfg: crate::config::ExecutionRecipeConfig) -> Self {
        ExecutionRecipeResolver { cfg }
    }

    /// Resolve the turn's recipe.
    pub fn resolve(&self, req: RecipeRequest<'_>) -> ResolvedExecution {
        // System default when the resolver is disabled outright.
        if !self.cfg.enabled {
            return ResolvedExecution::general_v1();
        }
        // 1. Explicit authorized selection.
        if let Some(explicit) = &req.explicit
            && let Some(sel) = Self::select(explicit)
        {
            return self.build(sel, RecipeSource::Explicit, explicit);
        }
        // 2. Project binding (only honored while the pilot flag is on).
        if let Some(project) = req.project_id
            && self.cfg.coding_pilot
            && let Some(bound) = self.cfg.project_bindings.get(&project.to_string())
            && let Some(sel) = Self::select(&RecipeId(bound.clone()))
        {
            return self.build(sel, RecipeSource::ProjectBinding, &RecipeId(bound.clone()));
        }
        // 3. Persona default — no persona carries a recipe yet (reserved).
        // 4. System default.
        ResolvedExecution::general_v1()
    }

    /// What a recognized recipe id selects. Unknown ids are rejected (`None`)
    /// rather than guessed at.
    fn select(id: &RecipeId) -> Option<Selection> {
        match id.0.as_str() {
            CODING_OMP_V1 => Some(Selection::Coding),
            super::types::GENERAL_V1 => Some(Selection::General),
            _ => None,
        }
    }

    /// Materialize a selection under this resolver's service flags.
    fn build(&self, sel: Selection, source: RecipeSource, id: &RecipeId) -> ResolvedExecution {
        match sel {
            Selection::General => ResolvedExecution {
                recipe: RecipeId::general_v1(),
                pack_ids: Vec::new(),
                source,
                services: CodingServiceFlags::default(),
            },
            Selection::Coding => ResolvedExecution {
                recipe: id.clone(),
                pack_ids: vec![oxicode_sdk::behavior::BehaviorPackId(
                    CODING_OMP_V1.to_string(),
                )],
                source,
                services: CodingServiceFlags {
                    shell: self.cfg.coding_shell,
                    eval: self.cfg.coding_eval,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExecutionRecipeConfig;
    use std::collections::HashMap;

    fn cfg(mut c: ExecutionRecipeConfig) -> ExecutionRecipeConfig {
        c.enabled = true;
        c
    }

    fn project() -> uuid::Uuid {
        uuid::Uuid::nil()
    }

    #[test]
    fn no_inputs_resolves_system_default() {
        let r = ExecutionRecipeResolver::new(cfg(ExecutionRecipeConfig::default()));
        let resolved = r.resolve(RecipeRequest::default());
        assert_eq!(resolved.recipe, RecipeId::general_v1());
        assert_eq!(resolved.source, RecipeSource::SystemDefault);
        assert!(resolved.pack_ids.is_empty());
        assert!(!resolved.is_coding());
    }

    #[test]
    fn project_binding_ignored_without_pilot_flag() {
        let mut c = cfg(ExecutionRecipeConfig::default());
        c.project_bindings
            .insert(project().to_string(), CODING_OMP_V1.to_string());
        let r = ExecutionRecipeResolver::new(c);
        let resolved = r.resolve(RecipeRequest {
            project_id: Some(project()),
            ..Default::default()
        });
        assert_eq!(resolved.recipe, RecipeId::general_v1());
        assert_eq!(resolved.source, RecipeSource::SystemDefault);
    }

    #[test]
    fn project_binding_selects_coding_pack_when_pilot_on() {
        let mut c = cfg(ExecutionRecipeConfig::default());
        c.coding_pilot = true;
        c.coding_shell = true;
        c.coding_eval = false;
        c.project_bindings
            .insert(project().to_string(), CODING_OMP_V1.to_string());
        let r = ExecutionRecipeResolver::new(c);
        let resolved = r.resolve(RecipeRequest {
            project_id: Some(project()),
            ..Default::default()
        });
        assert_eq!(resolved.recipe.0, CODING_OMP_V1);
        assert_eq!(resolved.source, RecipeSource::ProjectBinding);
        assert_eq!(resolved.pack_ids.len(), 1);
        assert_eq!(resolved.pack_ids[0].0, CODING_OMP_V1);
        assert!(resolved.is_coding());
        // Service flags follow the config.
        assert!(resolved.services.shell);
        assert!(!resolved.services.eval);
    }

    #[test]
    fn unknown_binding_id_degrades_to_general_v1() {
        let mut c = cfg(ExecutionRecipeConfig::default());
        c.coding_pilot = true;
        c.project_bindings
            .insert(project().to_string(), "coding-omp-v9".to_string());
        let r = ExecutionRecipeResolver::new(c);
        let resolved = r.resolve(RecipeRequest {
            project_id: Some(project()),
            ..Default::default()
        });
        assert_eq!(resolved.recipe, RecipeId::general_v1());
        assert_eq!(resolved.source, RecipeSource::SystemDefault);
    }

    #[test]
    fn disabled_resolver_always_yields_general_v1() {
        let c = ExecutionRecipeConfig {
            enabled: false,
            coding_pilot: true,
            project_bindings: HashMap::from([(project().to_string(), CODING_OMP_V1.to_string())]),
            ..Default::default()
        };
        let r = ExecutionRecipeResolver::new(c);
        let resolved = r.resolve(RecipeRequest {
            project_id: Some(project()),
            explicit: Some(RecipeId(CODING_OMP_V1.to_string())),
            persona_id: None,
        });
        assert_eq!(resolved.recipe, RecipeId::general_v1());
    }

    #[test]
    fn explicit_selection_wins_when_authorized() {
        let mut c = cfg(ExecutionRecipeConfig::default());
        c.coding_pilot = true;
        let r = ExecutionRecipeResolver::new(c);
        let resolved = r.resolve(RecipeRequest {
            explicit: Some(RecipeId(CODING_OMP_V1.to_string())),
            ..Default::default()
        });
        assert_eq!(resolved.recipe.0, CODING_OMP_V1);
        assert_eq!(resolved.source, RecipeSource::Explicit);
    }
}
