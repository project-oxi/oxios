//! Cron → task migration (RFC-043 Phase 5).
//!
//! Copies each `CronScheduler` job into a Task with
//! `automation_mode = schedule`. Copy-based by design: the original cron
//! job is left in place — retiring it stays an explicit user decision.

use anyhow::Result;

use crate::cron::{CronJob, CronScheduler};
use crate::task::model::{CreateTaskParams, SetScheduleParams, Task, TaskAutomationMode};
use crate::task::store::TaskStore;

/// Result of a cron → task migration run.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CronMigrationReport {
    /// Tasks created (empty when `dry_run`).
    pub created: Vec<Task>,
    /// Jobs skipped: (job name, reason).
    pub skipped: Vec<(String, String)>,
    /// True when this was a preview run.
    pub dry_run: bool,
}

/// Migrate cron jobs to tasks. Jobs whose slug identifier already exists
/// as a task are skipped. `dry_run` reports the plan without inserting.
pub async fn migrate_cron_to_tasks(
    cron: &CronScheduler,
    store: &TaskStore,
    dry_run: bool,
) -> Result<CronMigrationReport> {
    let mut report = CronMigrationReport {
        created: Vec::new(),
        skipped: Vec::new(),
        dry_run,
    };

    // Sort by name so reports are deterministic.
    let mut jobs: Vec<CronJob> = cron.list_jobs();
    jobs.sort_by(|a, b| a.name.cmp(&b.name));

    for job in jobs {
        let identifier = Task::slug_base(&job.name);
        if store.get_task_by_identifier(&identifier).await?.is_some() {
            report.skipped.push((
                job.name.clone(),
                "a task with this identifier already exists".into(),
            ));
            continue;
        }
        if dry_run {
            report
                .skipped
                .push((job.name.clone(), "would migrate".into()));
            continue;
        }
        let task = store
            .create_task(CreateTaskParams {
                name: job.name.clone(),
                instruction: job.goal.clone(),
                identifier: Some(identifier),
                description: Some(format!("Migrated from cron job {}", job.id)),
                priority: None,
                parent_task_id: None,
                assignee_agent_id: None,
                created_by_agent_id: None,
                created_by_session_id: None,
                sort_order: None,
            })
            .await?;
        store
            .set_automation(
                &task.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Schedule),
                    schedule_pattern: Some(job.schedule.clone()),
                    schedule_timezone: None,
                    heartbeat_interval_secs: None,
                    max_executions: None,
                },
            )
            .await?;
        report.created.push(store.get_task_by_id(&task.id).await?);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_store::StateStore;

    async fn scheduler_with(job_name: &str) -> CronScheduler {
        let tmp = tempfile::tempdir().unwrap();
        let store = StateStore::new(tmp.path().to_path_buf()).unwrap();
        let cron = CronScheduler::new(std::sync::Arc::new(store), 60);
        cron.add_job(CronJob::new(
            job_name.into(),
            "0 */6 * * *".into(),
            "do the thing".into(),
        ))
        .await
        .unwrap();
        cron
    }

    #[tokio::test]
    async fn migrates_jobs_as_scheduled_tasks() {
        let cron = scheduler_with("nightly-digest").await;
        let store = TaskStore::in_memory().unwrap();
        let report = migrate_cron_to_tasks(&cron, &store, false).await.unwrap();
        assert_eq!(report.created.len(), 1);
        assert!(report.skipped.is_empty());
        let t = &report.created[0];
        assert_eq!(t.instruction, "do the thing");
        assert_eq!(t.status, crate::task::model::TaskStatus::Scheduled);
        assert_eq!(t.schedule_pattern.as_deref(), Some("0 */6 * * *"));
        assert!(t.next_run_at.is_some(), "schedule set next_run_at");
    }

    #[tokio::test]
    async fn skips_existing_identifier_and_dry_run_creates_nothing() {
        let cron = scheduler_with("nightly-digest").await;
        let store = TaskStore::in_memory().unwrap();
        // Pre-create a task with the same slug identifier.
        store
            .create_task(CreateTaskParams {
                name: "nightly digest".into(),
                instruction: "existing".into(),
                identifier: Some("nightly-digest".into()),
                description: None,
                priority: None,
                parent_task_id: None,
                assignee_agent_id: None,
                created_by_agent_id: None,
                created_by_session_id: None,
                sort_order: None,
            })
            .await
            .unwrap();

        let report = migrate_cron_to_tasks(&cron, &store, false).await.unwrap();
        assert!(report.created.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].1.contains("already exists"));

        // Dry run with no collision reports the would-migrate job, creates nothing.
        let store2 = TaskStore::in_memory().unwrap();
        let dry = migrate_cron_to_tasks(&cron, &store2, true).await.unwrap();
        assert!(dry.dry_run);
        assert!(dry.created.is_empty());
        assert_eq!(dry.skipped.len(), 1);
        assert_eq!(dry.skipped[0].1, "would migrate");
        assert!(
            store2
                .list_tasks(Default::default())
                .await
                .unwrap()
                .is_empty()
        );
    }
}
