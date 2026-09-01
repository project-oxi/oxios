// Automation module — public API for automation lifecycle management.
//
// Web IA major-version cutover: replaces the retired RFC-043 task module.
// Definitions live in `{workspace}/automations.db` (fresh `automations` /
// `automation_runs` schema); every run carries an immutable
// [`AutomationContextSnapshot`] captured when the run opens.

pub mod model;
pub mod runner;
pub mod store;

pub use model::*;
pub use runner::{execute_automation_run, parse_verdict, repair_prompt, verifier_prompt};
pub use store::AutomationStore;
