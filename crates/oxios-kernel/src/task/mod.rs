// Task module — public API for task lifecycle management (RFC-043)

pub mod migrate;
pub mod model;
pub mod runner;
pub mod store;

pub use migrate::{CronMigrationReport, migrate_cron_to_tasks};
pub use model::*;
pub use runner::{
    cron_next_after, execute_task_run, parse_verdict, repair_prompt, verifier_prompt,
};
pub use store::TaskStore;
