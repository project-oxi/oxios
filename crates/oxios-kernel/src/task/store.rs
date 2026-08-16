// Task store — SQLite-backed CRUD for tasks (RFC-043)
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::model::*;

/// SQLite-backed task store.
pub struct TaskStore {
    conn: Arc<Mutex<Connection>>,
}

impl TaskStore {
    /// Create a TaskStore from a raw connection. Schema is initialized
    /// on the connection *before* it is wrapped in the async mutex, so
    /// this constructor is safe to call from inside a Tokio runtime —
    /// no `blocking_lock` is involved.
    pub fn new(conn: Connection) -> Result<Self> {
        init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create a TaskStore from a database file path.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open task database: {path}"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        Self::new(conn)
    }

    /// Create an in-memory TaskStore (for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::new(conn)
    }

    pub async fn create_task(&self, params: CreateTaskParams) -> Result<Task> {
        let id = uuid::Uuid::new_v4().to_string();
        {
            let conn = self.conn.lock().await;
            let now = Utc::now().to_rfc3339();
            let identifier = params
                .identifier
                .unwrap_or_else(|| Task::slug_from_name(&params.name));

            conn.execute(
                r#"INSERT INTO tasks
                   (id, identifier, name, description, instruction, status, priority,
                    sort_order, parent_task_id, assignee_agent_id,
                    created_by_agent_id, created_by_session_id,
                    created_at, updated_at,
                    verify_enabled, execution_count, consecutive_failures)
                   VALUES (?1, ?2, ?3, ?4, ?5, 'backlog', ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0, 0, 0)"#,
                params![
                    id,
                    identifier,
                    params.name,
                    params.description,
                    params.instruction,
                    params.priority.unwrap_or(0),
                    params.sort_order,
                    params.parent_task_id,
                    params.assignee_agent_id,
                    params.created_by_agent_id,
                    params.created_by_session_id,
                    now,
                    now,
                ],
            )
            .context("insert task")?;
        }
        // Lock released — safe to call another `&self` method.
        self.get_task_by_id(&id).await
    }

    pub async fn get_task_by_id(&self, id: &str) -> Result<Task> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            r#"SELECT id, identifier, name, description, instruction, status, priority,
                      sort_order, parent_task_id, assignee_agent_id, created_by_agent_id,
                      created_by_session_id, automation_mode, schedule_pattern,
                      schedule_timezone, heartbeat_interval_secs, max_executions,
                      execution_count, verify_enabled, verify_requirement,
                      verify_max_iterations, verify_verifier_agent_id,
                      created_at, updated_at, started_at, completed_at,
                      last_run_at, next_run_at, last_error, consecutive_failures,
                      context_json
               FROM tasks WHERE id = ?1"#,
        )?;

        let mut task = stmt.query_row(params![id], map_task_row)?;
        task.dependencies = dependency_ids_locked(&conn, id)?;
        Ok(task)
    }

    /// Look up a task by its unique identifier. Used by cron migration
    /// dedup and identifier-conflict checks.
    pub async fn get_task_by_identifier(&self, identifier: &str) -> Result<Option<Task>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            r#"SELECT id, identifier, name, description, instruction, status, priority,
                      sort_order, parent_task_id, assignee_agent_id, created_by_agent_id,
                      created_by_session_id, automation_mode, schedule_pattern,
                      schedule_timezone, heartbeat_interval_secs, max_executions,
                      execution_count, verify_enabled, verify_requirement,
                      verify_max_iterations, verify_verifier_agent_id,
                      created_at, updated_at, started_at, completed_at,
                      last_run_at, next_run_at, last_error, consecutive_failures,
                      context_json
               FROM tasks WHERE identifier = ?1"#,
        )?;
        let task = stmt
            .query_row(params![identifier], map_task_row)
            .optional()?;
        Ok(task)
    }

    pub async fn list_tasks(&self, list_params: ListTasksParams) -> Result<Vec<Task>> {
        let conn = self.conn.lock().await;
        let limit = list_params.limit.unwrap_or(100).min(500);
        let offset = list_params.offset.unwrap_or(0);

        let mut sql = String::from(
            r#"SELECT id, identifier, name, description, instruction, status, priority,
                      sort_order, parent_task_id, assignee_agent_id, created_by_agent_id,
                      created_by_session_id, automation_mode, schedule_pattern,
                      schedule_timezone, heartbeat_interval_secs, max_executions,
                      execution_count, verify_enabled, verify_requirement,
                      verify_max_iterations, verify_verifier_agent_id,
                      created_at, updated_at, started_at, completed_at,
                      last_run_at, next_run_at, last_error, consecutive_failures,
                      context_json
               FROM tasks WHERE 1=1"#,
        );

        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(limit), Box::new(offset)];

        if let Some(statuses) = &list_params.statuses {
            let placeholders: Vec<String> = statuses
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", param_values.len() + i + 1))
                .collect();
            sql.push_str(&format!(" AND status IN ({})", placeholders.join(",")));
            for s in statuses {
                param_values.push(Box::new(s.clone()));
            }
        }
        if let Some(ref assignee) = list_params.assignee_agent_id {
            sql.push_str(&format!(
                " AND assignee_agent_id = ?{}",
                param_values.len() + 1
            ));
            param_values.push(Box::new(assignee.clone()));
        }
        if let Some(ref parent) = list_params.parent_task_id {
            sql.push_str(&format!(
                " AND parent_task_id = ?{}",
                param_values.len() + 1
            ));
            param_values.push(Box::new(parent.clone()));
        }

        sql.push_str(" ORDER BY sort_order, created_at DESC LIMIT ?1 OFFSET ?2");

        let param_refs: Vec<&dyn rusqlite::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let mut tasks: Vec<Task> = stmt
            .query_map(param_refs.as_slice(), map_task_row)?
            .filter_map(|r| r.ok())
            .collect();
        for task in &mut tasks {
            task.dependencies = dependency_ids_locked(&conn, &task.id)?;
        }

        Ok(tasks)
    }

    pub async fn delete_task(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        // `task_dependencies.depends_on` has no FK (shipped schema), so
        // edges referencing this task are removed explicitly.
        conn.execute(
            "DELETE FROM task_dependencies WHERE task_id = ?1 OR depends_on = ?1",
            params![id],
        )?;
        conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])
            .context("delete task")?;
        Ok(())
    }

    pub async fn update_status(&self, id: &str, status: &TaskStatus) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        let completed = if *status == TaskStatus::Completed {
            Some(now.clone())
        } else {
            None
        };
        conn.execute(
            r#"UPDATE tasks SET status = ?1, updated_at = ?2, completed_at = COALESCE(?3, completed_at)
               WHERE id = ?4"#,
            params![status.to_string(), now, completed, id],
        )?;
        Ok(())
    }

    /// Persist the verify-gate configuration (RFC-043 §Verify Gate).
    ///
    /// `None` fields keep their current value; unknown ids error.
    pub async fn set_verify(&self, id: &str, params: SetVerifyParams) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        let (enabled, requirement, max_iter, verifier): (
            i64,
            Option<String>,
            Option<i64>,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT verify_enabled, verify_requirement, verify_max_iterations, \
                 verify_verifier_agent_id FROM tasks WHERE id = ?1",
                params![id],
                |r| Ok((r.get::<_, i64>(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("task {id} not found"))?;

        conn.execute(
            "UPDATE tasks SET verify_enabled = ?1, verify_requirement = ?2, \
             verify_max_iterations = ?3, verify_verifier_agent_id = ?4, updated_at = ?5 \
             WHERE id = ?6",
            params![
                params.enabled.map(|b| b as i64).unwrap_or(enabled),
                params.requirement.or(requirement),
                params.max_iterations.map(|v| v as i64).or(max_iter),
                params.verifier_agent_id.or(verifier),
                now,
                id
            ],
        )?;
        Ok(())
    }

    /// Partially update editable task fields — `None` leaves a field
    /// unchanged (RFC-043 `PUT /api/tasks/:id`).
    pub async fn update_task(&self, id: &str, params: UpdateTaskParams) -> Result<()> {
        // Lock first: the boxed ToSql values are !Send, so they must not
        // live across an await point (async_trait futures must be Send).
        let conn = self.conn.lock().await;
        // (column, value) pairs built only from Some fields.
        let mut sets: Vec<(&str, Box<dyn rusqlite::ToSql>)> = Vec::new();
        if let Some(v) = params.name {
            sets.push(("name", Box::new(v)));
        }
        if let Some(v) = params.description {
            sets.push(("description", Box::new(v)));
        }
        if let Some(v) = params.instruction {
            sets.push(("instruction", Box::new(v)));
        }
        if let Some(v) = params.priority {
            sets.push(("priority", Box::new(v)));
        }
        if let Some(v) = params.sort_order {
            sets.push(("sort_order", Box::new(v)));
        }
        if let Some(v) = params.parent_task_id {
            sets.push(("parent_task_id", Box::new(v)));
        }
        if let Some(v) = params.assignee_agent_id {
            sets.push(("assignee_agent_id", Box::new(v)));
        }
        if sets.is_empty() {
            anyhow::bail!("update_task: no fields provided");
        }

        let now = Utc::now().to_rfc3339();
        sets.push(("updated_at", Box::new(now)));

        let mut sql = String::from("UPDATE tasks SET ");
        sql.push_str(
            &sets
                .iter()
                .enumerate()
                .map(|(i, (col, _))| format!("{col} = ?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", "),
        );
        sql.push_str(&format!(" WHERE id = ?{}", sets.len() + 1));
        let refs: Vec<&dyn rusqlite::ToSql> = sets.iter().map(|(_, v)| v.as_ref()).collect();
        let mut bind = Vec::with_capacity(refs.len() + 1);
        bind.extend(refs);
        bind.push(&id);
        let changed = conn.execute(&sql, bind.as_slice())?;
        if changed == 0 {
            anyhow::bail!("task {id} not found");
        }
        Ok(())
    }

    // ── Comments (RFC-043) ────────────────────────────────────────────

    /// Add a comment to a task. `author_agent_id = None` means a human
    /// comment. Errors when the parent task doesn't exist (FK).
    pub async fn add_comment(
        &self,
        task_id: &str,
        content: &str,
        author_agent_id: Option<&str>,
        author_session_id: Option<&str>,
    ) -> Result<TaskComment> {
        let conn = self.conn.lock().await;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO task_comments (id, task_id, content, author_agent_id, \
             author_session_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                task_id,
                content,
                author_agent_id,
                author_session_id,
                now
            ],
        )
        .context("insert comment")?;
        Ok(TaskComment {
            id,
            task_id: task_id.to_string(),
            content: content.to_string(),
            author_agent_id: author_agent_id.map(str::to_string),
            author_session_id: author_session_id.map(str::to_string),
            created_at: now,
            updated_at: None,
        })
    }

    /// Edit a comment's content (stamps `updated_at`). Unknown id errors.
    pub async fn update_comment(&self, comment_id: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        let changed = conn.execute(
            "UPDATE task_comments SET content = ?1, updated_at = ?2 WHERE id = ?3",
            params![content, now, comment_id],
        )?;
        if changed == 0 {
            anyhow::bail!("comment {comment_id} not found");
        }
        Ok(())
    }

    /// Delete a comment. Unknown id errors.
    pub async fn delete_comment(&self, comment_id: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        let changed = conn.execute(
            "DELETE FROM task_comments WHERE id = ?1",
            params![comment_id],
        )?;
        if changed == 0 {
            anyhow::bail!("comment {comment_id} not found");
        }
        Ok(())
    }

    /// List a task's comments, oldest first.
    pub async fn list_comments(&self, task_id: &str) -> Result<Vec<TaskComment>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, content, author_agent_id, author_session_id, \
             created_at, updated_at FROM task_comments WHERE task_id = ?1 \
             ORDER BY created_at, id",
        )?;
        let comments = stmt
            .query_map(params![task_id], |r| {
                Ok(TaskComment {
                    id: r.get(0)?,
                    task_id: r.get(1)?,
                    content: r.get(2)?,
                    author_agent_id: r.get(3)?,
                    author_session_id: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(comments)
    }

    // ── Dependencies (RFC-043) ────────────────────────────────────────

    /// Make `task_id` depend on `depends_on`. Rejects self-edges, unknown
    /// ids, duplicates, and edges that would create a cycle.
    pub async fn add_dependency(&self, task_id: &str, depends_on: &str) -> Result<()> {
        if task_id == depends_on {
            anyhow::bail!("a task cannot depend on itself");
        }
        let conn = self.conn.lock().await;
        for id in [task_id, depends_on] {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM tasks WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )?;
            if n == 0 {
                anyhow::bail!("task {id} not found");
            }
        }
        let dup: i64 = conn.query_row(
            "SELECT COUNT(*) FROM task_dependencies \
             WHERE task_id = ?1 AND depends_on = ?2",
            params![task_id, depends_on],
            |r| r.get(0),
        )?;
        if dup > 0 {
            anyhow::bail!("dependency already exists");
        }
        if creates_cycle(&conn, task_id, depends_on) {
            anyhow::bail!("adding this dependency would create a cycle");
        }
        conn.execute(
            "INSERT INTO task_dependencies (task_id, depends_on) VALUES (?1, ?2)",
            params![task_id, depends_on],
        )
        .context("insert dependency")?;
        Ok(())
    }

    /// Remove a dependency edge. Removing a missing edge errors.
    pub async fn remove_dependency(&self, task_id: &str, depends_on: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        let changed = conn.execute(
            "DELETE FROM task_dependencies WHERE task_id = ?1 AND depends_on = ?2",
            params![task_id, depends_on],
        )?;
        if changed == 0 {
            anyhow::bail!("dependency not found");
        }
        Ok(())
    }

    /// Dependency target ids for a task, insertion-ordered.
    pub async fn dependency_ids(&self, task_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().await;
        dependency_ids_locked(&conn, task_id)
    }

    /// Dependencies not yet satisfied (target missing or not `completed`).
    /// Used by the auto-run tick to gate execution.
    pub async fn unsatisfied_dependencies(&self, task_id: &str) -> Result<Vec<String>> {
        let conn = self.conn.lock().await;
        let ids = dependency_ids_locked(&conn, task_id)?;
        let mut unsatisfied = Vec::new();
        for dep in ids {
            let status: Option<String> = conn
                .query_row(
                    "SELECT status FROM tasks WHERE id = ?1",
                    params![dep],
                    |r| r.get(0),
                )
                .optional()?;
            if status.as_deref() != Some("completed") {
                unsatisfied.push(dep);
            }
        }
        Ok(unsatisfied)
    }

    pub async fn list_due_tasks(&self) -> Result<Vec<Task>> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        let mut stmt = conn.prepare(
            r#"SELECT id, identifier, name, description, instruction, status, priority,
                      sort_order, parent_task_id, assignee_agent_id, created_by_agent_id,
                      created_by_session_id, automation_mode, schedule_pattern,
                      schedule_timezone, heartbeat_interval_secs, max_executions,
                      execution_count, verify_enabled, verify_requirement,
                      verify_max_iterations, verify_verifier_agent_id,
                      created_at, updated_at, started_at, completed_at,
                      last_run_at, next_run_at, last_error, consecutive_failures,
                      context_json
               FROM tasks
               WHERE automation_mode IS NOT NULL
                 AND status = 'scheduled'
                 AND next_run_at IS NOT NULL
                 AND next_run_at <= ?1
               ORDER BY next_run_at"#,
        )?;
        let tasks = stmt
            .query_map(params![now], map_task_row)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(tasks)
    }

    pub async fn set_next_run(&self, id: &str, next_run: Option<&str>) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE tasks SET next_run_at = ?1, updated_at = ?2 WHERE id = ?3",
            params![next_run, now, id],
        )?;
        Ok(())
    }
    // ── Automation / scheduling ───────────────────────────────────────

    /// Persist automation/schedule fields and set status + `next_run_at`.
    ///
    /// - Schedule mode → `next_run_at` = next cron fire after now.
    /// - Heartbeat mode → `next_run_at` = now + interval.
    /// - No automation (mode None) → clears scheduling: status `backlog`,
    ///   `next_run_at` NULL.
    pub async fn set_automation(&self, id: &str, params: SetScheduleParams) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now();
        let now_rfc = now.to_rfc3339();

        let next_run = match &params.automation_mode {
            Some(TaskAutomationMode::Schedule) => params
                .schedule_pattern
                .as_deref()
                .and_then(|p| cron_next(p, &now).ok()),
            Some(TaskAutomationMode::Heartbeat) => params
                .heartbeat_interval_secs
                .map(|secs| (now + chrono::Duration::seconds(secs as i64)).to_rfc3339()),
            None => None,
        };

        let mode_str = params.automation_mode.as_ref().map(|m| m.to_string());
        let status = if params.automation_mode.is_some() {
            "scheduled"
        } else {
            "backlog"
        };

        conn.execute(
            r#"UPDATE tasks SET
                 automation_mode = ?1, schedule_pattern = ?2, schedule_timezone = ?3,
                 heartbeat_interval_secs = ?4, max_executions = ?5,
                 status = ?6, next_run_at = ?7, updated_at = ?8
               WHERE id = ?9"#,
            params![
                mode_str,
                params.schedule_pattern,
                params.schedule_timezone,
                params.heartbeat_interval_secs.map(|v| v as i64),
                params.max_executions,
                status,
                next_run,
                now_rfc,
                id,
            ],
        )?;
        Ok(())
    }

    // ── Execution lifecycle (backed by task_runs) ─────────────────────

    /// Mark a task as Running: set `started_at`/`last_run_at` and insert a
    /// `task_runs` row tagged with `trigger`. Returns the new run id so the
    /// caller can finalize it via [`Self::mark_finished`].
    pub async fn mark_running(&self, id: &str, trigger: TaskRunTrigger) -> Result<String> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE tasks SET status = 'running', started_at = ?1, last_run_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        let run_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            r#"INSERT INTO task_runs (id, task_id, trigger, status, started_at)
               VALUES (?1, ?2, ?3, 'running', ?4)"#,
            params![run_id, id, trigger.to_string(), now],
        )?;
        Ok(run_id)
    }

    /// Finalize a run: update the `task_runs` row and the task's terminal
    /// state (status, execution_count, consecutive_failures, timestamps).
    /// If the task still has active automation and hasn't hit `max_executions`,
    /// recompute `next_run_at` and flip status back to `scheduled`; otherwise
    /// leave it terminal (`completed`/`failed`).
    ///
    /// `verified` marks the run `verified` (verify gate passed) — the task's
    /// own status is still `completed`.
    ///
    /// Failure fuse (RFC-043): only scheduled/heartbeat failures increment
    /// `consecutive_failures` (read from the run's `trigger`); manual-run
    /// failures never touch it. At ≥ 3 consecutive auto failures the task is
    /// PAUSED (no reschedule) with a `last_error` fuse note.
    pub async fn mark_finished(
        &self,
        id: &str,
        run_id: &str,
        success: bool,
        verified: bool,
        summary: String,
        error: Option<String>,
    ) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now();
        let now_rfc = now.to_rfc3339();
        let run_status = if verified && success {
            "verified"
        } else if success {
            "completed"
        } else {
            "failed"
        };

        // 1. task_runs row: terminal status + payload + completed_at.
        conn.execute(
            r#"UPDATE task_runs SET status = ?1, summary = ?2, result_content = ?3,
               error = ?4, completed_at = ?5 WHERE id = ?6"#,
            params![run_status, &summary, &summary, &error, &now_rfc, run_id],
        )?;

        // 2. Read automation config + counts to decide terminal vs. reschedule.
        #[derive(Debug)]
        struct FinishCtx {
            mode: Option<String>,
            pattern: Option<String>,
            hb_secs: Option<i64>,
            exec_count: i64,
            max_exec: Option<i64>,
            consec_failures: i64,
            trigger: String,
        }
        let ctx: FinishCtx = conn.query_row(
            "SELECT t.automation_mode, t.schedule_pattern, t.heartbeat_interval_secs, \
             t.execution_count, t.max_executions, t.consecutive_failures, r.trigger \
             FROM tasks t JOIN task_runs r ON r.id = ?2 WHERE t.id = ?1",
            params![id, run_id],
            |r| {
                Ok(FinishCtx {
                    mode: r.get(0)?,
                    pattern: r.get(1)?,
                    hb_secs: r.get(2)?,
                    exec_count: r.get(3)?,
                    max_exec: r.get(4)?,
                    consec_failures: r.get(5)?,
                    trigger: r.get(6)?,
                })
            },
        )?;
        let FinishCtx {
            mode,
            pattern,
            hb_secs,
            exec_count,
            max_exec,
            consec_failures,
            trigger,
        } = ctx;

        let new_count = exec_count + 1;
        let exhausted = max_exec.is_some_and(|m| new_count >= m);
        // Fuse: only automation-triggered failures count; manual runs never do.
        let auto_failure = !success && trigger != "manual";
        let new_consec = if success {
            0
        } else if auto_failure {
            consec_failures + 1
        } else {
            consec_failures
        };
        let fused = new_consec >= 3 && !success;
        if fused {
            tracing::warn!(task = %id, consecutive = new_consec, "Task fuse tripped — paused");
        }

        // Recompute next_run only when automation is active, not exhausted,
        // and the fuse hasn't tripped. On ordinary failure we still
        // reschedule (transient errors shouldn't permanently disable a
        // scheduled task) — the fuse caps that.
        let reschedule = mode.is_some() && !exhausted && !fused;
        let next_run = if reschedule {
            match mode.as_deref() {
                Some("schedule") => pattern.as_deref().and_then(|p| cron_next(p, &now).ok()),
                Some("heartbeat") => {
                    hb_secs.map(|s| (now + chrono::Duration::seconds(s)).to_rfc3339())
                }
                _ => None,
            }
        } else {
            None
        };

        let terminal_status = if fused {
            "paused"
        } else if reschedule {
            "scheduled"
        } else if success {
            "completed"
        } else {
            "failed"
        };
        let last_error = if fused {
            Some(format!(
                "Paused after 3 consecutive failures: {}",
                error.as_deref().unwrap_or("unknown error")
            ))
        } else {
            error
        };
        conn.execute(
            r#"UPDATE tasks SET
                 status = ?1, execution_count = ?2,
                 completed_at = COALESCE(?3, completed_at),
                 consecutive_failures = ?4,
                 last_error = ?5, next_run_at = ?6, updated_at = ?7
               WHERE id = ?8"#,
            params![
                terminal_status,
                new_count,
                if !reschedule && !fused {
                    Some(&now_rfc)
                } else {
                    None
                },
                new_consec,
                last_error,
                next_run,
                now_rfc,
                id,
            ],
        )?;
        Ok(())
    }

    /// Latest run for a task (for the UI's "last result" display).
    pub async fn latest_run(&self, task_id: &str) -> Result<Option<TaskRun>> {
        let conn = self.conn.lock().await;
        Ok(conn
            .query_row(
                "SELECT id, task_id, session_id, trigger, status, summary, result_content, \
                 started_at, completed_at, error, cost_usd, tokens_used FROM task_runs \
                 WHERE task_id = ?1 ORDER BY started_at DESC LIMIT 1",
                params![task_id],
                map_run_row,
            )
            .optional()?)
    }

    /// Run history for a task (newest first).
    pub async fn list_runs(&self, task_id: &str) -> Result<Vec<TaskRun>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            "SELECT id, task_id, session_id, trigger, status, summary, result_content, \
             started_at, completed_at, error, cost_usd, tokens_used FROM task_runs \
             WHERE task_id = ?1 ORDER BY started_at DESC LIMIT 50",
        )?;
        let runs = stmt
            .query_map(params![task_id], map_run_row)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(runs)
    }

    /// Boot-time recovery: reset tasks stranded at `running` by a prior
    /// process crash (the Task model persists status to SQLite, unlike the
    /// CronScheduler's in-memory `running_jobs` set). Since `list_due_tasks`
    /// excludes `running`, stranded tasks would otherwise never be retried.
    ///
    /// - Orphaned `task_runs` rows still `running` → marked `failed`.
    /// - Stranded tasks → `scheduled` (if automation is set) or `backlog`.
    pub async fn recover_stranded(&self) -> Result<()> {
        let conn = self.conn.lock().await;
        let now = Utc::now().to_rfc3339();
        let runs = conn.execute(
            "UPDATE task_runs SET status = 'failed', \
             error = 'Interrupted by process restart', completed_at = ?1 \
             WHERE status = 'running'",
            params![now],
        )?;
        let tasks = conn.execute(
            "UPDATE tasks SET status = CASE WHEN automation_mode IS NOT NULL \
             THEN 'scheduled' ELSE 'backlog' END, updated_at = ?1 \
             WHERE status = 'running'",
            params![now],
        )?;
        if runs > 0 || tasks > 0 {
            tracing::info!(runs, tasks, "Recovered stranded tasks/runs after restart");
        }
        Ok(())
    }
}
fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            identifier TEXT UNIQUE NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            instruction TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'backlog',
            priority INTEGER DEFAULT 0,
            sort_order REAL,
            parent_task_id TEXT REFERENCES tasks(id) ON DELETE CASCADE,
            assignee_agent_id TEXT,
            created_by_agent_id TEXT,
            created_by_session_id TEXT,
            automation_mode TEXT,
            schedule_pattern TEXT,
            schedule_timezone TEXT,
            heartbeat_interval_secs INTEGER,
            max_executions INTEGER,
            execution_count INTEGER DEFAULT 0,
            verify_enabled INTEGER DEFAULT 0,
            verify_requirement TEXT,
            verify_max_iterations INTEGER DEFAULT 3,
            verify_verifier_agent_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            started_at TEXT,
            completed_at TEXT,
            last_run_at TEXT,
            next_run_at TEXT,
            last_error TEXT,
            consecutive_failures INTEGER DEFAULT 0,
            context_json TEXT
        );

        CREATE TABLE IF NOT EXISTS task_dependencies (
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            depends_on TEXT NOT NULL,
            PRIMARY KEY (task_id, depends_on)
        );

        CREATE TABLE IF NOT EXISTS task_comments (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            content TEXT NOT NULL,
            author_agent_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT
        );

        CREATE TABLE IF NOT EXISTS task_runs (
            id TEXT PRIMARY KEY,
            task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
            session_id TEXT,
            trigger TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'running',
            summary TEXT,
            result_content TEXT,
            started_at TEXT NOT NULL,
            completed_at TEXT,
            error TEXT,
            cost_usd REAL,
            tokens_used INTEGER
        );

        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
        CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id);
        CREATE INDEX IF NOT EXISTS idx_tasks_next_run ON tasks(next_run_at);
        CREATE INDEX IF NOT EXISTS idx_runs_task ON task_runs(task_id);
        CREATE INDEX IF NOT EXISTS idx_comments_task ON task_comments(task_id);
        CREATE INDEX IF NOT EXISTS idx_task_deps ON task_dependencies(depends_on);
        "#,
    )?;

    // Guarded column migration: `task_comments.author_session_id` was added
    // after the table first shipped (RFC-043 completion). SQLite has no
    // `ADD COLUMN IF NOT EXISTS`, so probe pragma first.
    let mut stmt = conn.prepare("PRAGMA table_info(task_comments)")?;
    let has_session_col = stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .any(|name| name == "author_session_id");
    drop(stmt);
    if !has_session_col {
        conn.execute_batch("ALTER TABLE task_comments ADD COLUMN author_session_id TEXT;")?;
    }

    Ok(())
}

// ── Row mapper ──

fn map_task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let automation_mode_str: Option<String> = row.get(12)?;
    let automation_mode = automation_mode_str.as_deref().and_then(|s| s.parse().ok());

    let status_str: String = row.get(5)?;
    let status = status_str.parse().unwrap_or(TaskStatus::Backlog);

    let context_json: Option<String> = row.get(30)?;
    let context: HashMap<String, serde_json::Value> = context_json
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    Ok(Task {
        id: row.get(0)?,
        identifier: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        instruction: row.get(4)?,
        status,
        priority: row.get(6)?,
        sort_order: row.get(7)?,
        parent_task_id: row.get(8)?,
        assignee_agent_id: row.get(9)?,
        created_by_agent_id: row.get(10)?,
        created_by_session_id: row.get(11)?,
        automation_mode,
        schedule_pattern: row.get(13)?,
        schedule_timezone: row.get(14)?,
        heartbeat_interval_secs: row.get::<_, Option<i64>>(15)?.map(|v| v as u64),
        max_executions: row.get(16)?,
        execution_count: row.get(17)?,
        verify_enabled: row.get::<_, i64>(18)? != 0,
        verify_requirement: row.get(19)?,
        verify_max_iterations: row.get::<_, i64>(20)? as u32,
        verify_verifier_agent_id: row.get(21)?,
        created_at: row.get(22)?,
        updated_at: row.get(23)?,
        started_at: row.get(24)?,
        completed_at: row.get(25)?,
        last_run_at: row.get(26)?,
        next_run_at: row.get(27)?,
        last_error: row.get(28)?,
        consecutive_failures: row.get::<_, i64>(29)? as u32,
        context,
        dependencies: Vec::new(),
    })
}

fn map_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRun> {
    let trigger_str: String = row.get(3)?;
    let trigger = trigger_str.parse().unwrap_or(TaskRunTrigger::Manual);
    Ok(TaskRun {
        id: row.get(0)?,
        task_id: row.get(1)?,
        session_id: row.get(2)?,
        trigger,
        status: row.get(4)?,
        summary: row.get(5)?,
        result_content: row.get(6)?,
        started_at: row.get(7)?,
        completed_at: row.get(8)?,
        error: row.get(9)?,
        cost_usd: row.get(10)?,
        tokens_used: row.get::<_, Option<i64>>(11)?.map(|v| v as u64),
    })
}

/// Compute the next cron fire time after `after` as an RFC3339 string.
/// Normalizes 5-field (Linux cron) expressions by prepending a seconds field,
/// matching `CronScheduler::normalize_expr`.
fn cron_next(pattern: &str, after: &DateTime<Utc>) -> Result<String> {
    let normalized = {
        let fields: Vec<&str> = pattern.split_whitespace().collect();
        if fields.len() == 5 {
            format!("0 {pattern}")
        } else {
            pattern.to_string()
        }
    };
    let schedule = cron::Schedule::from_str(&normalized)
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{pattern}': {e}"))?;
    let next = schedule
        .after(after)
        .next()
        .ok_or_else(|| anyhow::anyhow!("No future fire time for cron '{pattern}'"))?;
    Ok(next.to_rfc3339())
}

/// Dependency target ids for a task, insertion-ordered (rowid order).
/// Caller must hold the connection lock.
fn dependency_ids_locked(conn: &Connection, task_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT depends_on FROM task_dependencies WHERE task_id = ?1")?;
    let ids = stmt
        .query_map(params![task_id], |r| r.get::<_, String>(0))?
        .filter_map(|r| r.ok())
        .collect();
    Ok(ids)
}

/// Whether adding `task_id → depends_on` would create a cycle: walk
/// forward from `depends_on` through existing edges; reaching `task_id`
/// means the new edge closes a loop.
fn creates_cycle(conn: &Connection, task_id: &str, depends_on: &str) -> bool {
    let mut stack = vec![depends_on.to_string()];
    let mut seen = std::collections::HashSet::new();
    while let Some(cur) = stack.pop() {
        if cur == task_id {
            return true;
        }
        if !seen.insert(cur.clone()) {
            continue;
        }
        let Ok(next) = dependency_ids_locked(conn, &cur) else {
            continue;
        };
        stack.extend(next);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_params(name: &str) -> CreateTaskParams {
        CreateTaskParams {
            name: name.to_string(),
            instruction: format!("do {name}"),
            identifier: None,
            description: None,
            priority: None,
            parent_task_id: None,
            assignee_agent_id: None,
            created_by_agent_id: None,
            created_by_session_id: None,
            sort_order: None,
        }
    }

    // Regression: `TaskStore::open` / `in_memory` must be safe to call from
    // inside a Tokio runtime. The production web surface constructs the
    // store on the runtime (`src/api/plugin.rs`); an earlier version used
    // `blocking_lock()` during schema init and panicked at startup with
    // "Cannot block the current thread from within a runtime".
    #[tokio::test]
    async fn in_memory_store_construction_does_not_panic_on_runtime() {
        let store = TaskStore::in_memory().expect("in-memory store builds");
        // Sanity: schema is usable.
        let task = store
            .create_task(sample_params("regression"))
            .await
            .expect("create works");
        assert_eq!(task.name, "regression");
    }

    #[tokio::test]
    async fn open_from_file_path_works_on_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tasks.db");
        let path_str = path.to_str().expect("utf8 path");
        let store = TaskStore::open(path_str).expect("open builds");
        let created = store
            .create_task(sample_params("from-disk"))
            .await
            .expect("create");
        // Re-open the same file — schema init must be idempotent and the
        // row must survive reopen.
        drop(store);
        let reopened = TaskStore::open(path_str).expect("reopen builds");
        let fetched = reopened
            .get_task_by_id(&created.id)
            .await
            .expect("get_by_id");
        assert_eq!(fetched.name, "from-disk");
    }

    #[tokio::test]
    async fn create_list_update_delete_roundtrip() {
        let store = TaskStore::in_memory().expect("in-memory store builds");
        let t1 = store
            .create_task(sample_params("alpha"))
            .await
            .expect("create alpha");
        let _t2 = store
            .create_task(sample_params("beta"))
            .await
            .expect("create beta");

        let listed = store
            .list_tasks(ListTasksParams::default())
            .await
            .expect("list");
        assert_eq!(listed.len(), 2);

        store
            .update_status(&t1.id, &TaskStatus::Completed)
            .await
            .expect("update");
        let fetched = store.get_task_by_id(&t1.id).await.expect("get_by_id");
        assert_eq!(fetched.status, TaskStatus::Completed);
        assert!(fetched.completed_at.is_some());

        store.delete_task(&t1.id).await.expect("delete");
        let after = store
            .list_tasks(ListTasksParams::default())
            .await
            .expect("list after delete");
        assert_eq!(after.len(), 1);
    }
    // ── Scheduling ──

    #[tokio::test]
    async fn set_automation_schedule_computes_next_run_and_status() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("sched")).await.unwrap();
        store
            .set_automation(
                &t.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Schedule),
                    schedule_pattern: Some("0 9 * * *".into()),
                    schedule_timezone: None,
                    heartbeat_interval_secs: None,
                    max_executions: None,
                },
            )
            .await
            .unwrap();
        let fetched = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(fetched.status, TaskStatus::Scheduled);
        assert_eq!(fetched.schedule_pattern.as_deref(), Some("0 9 * * *"));
        assert!(
            fetched.next_run_at.is_some(),
            "next_run_at must be computed"
        );
    }

    #[tokio::test]
    async fn set_automation_heartbeat_computes_next_run() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("hb")).await.unwrap();
        store
            .set_automation(
                &t.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Heartbeat),
                    schedule_pattern: None,
                    schedule_timezone: None,
                    heartbeat_interval_secs: Some(600),
                    max_executions: None,
                },
            )
            .await
            .unwrap();
        let fetched = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(fetched.status, TaskStatus::Scheduled);
        assert_eq!(fetched.heartbeat_interval_secs, Some(600));
        assert!(fetched.next_run_at.is_some());
    }

    #[tokio::test]
    async fn set_automation_none_clears_scheduling() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("clear")).await.unwrap();
        // First schedule, then clear.
        store
            .set_automation(
                &t.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Heartbeat),
                    schedule_pattern: None,
                    schedule_timezone: None,
                    heartbeat_interval_secs: Some(60),
                    max_executions: None,
                },
            )
            .await
            .unwrap();
        store
            .set_automation(
                &t.id,
                SetScheduleParams {
                    automation_mode: None,
                    schedule_pattern: None,
                    schedule_timezone: None,
                    heartbeat_interval_secs: None,
                    max_executions: None,
                },
            )
            .await
            .unwrap();
        let fetched = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(fetched.status, TaskStatus::Backlog);
        assert!(fetched.next_run_at.is_none());
        assert!(fetched.automation_mode.is_none());
    }

    // ── Run lifecycle ──

    #[tokio::test]
    async fn mark_running_then_finished_success_reschedules_and_counts() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("lifecycle")).await.unwrap();
        // Give it a heartbeat schedule so success reschedules.
        store
            .set_automation(
                &t.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Heartbeat),
                    schedule_pattern: None,
                    schedule_timezone: None,
                    heartbeat_interval_secs: Some(300),
                    max_executions: None,
                },
            )
            .await
            .unwrap();

        let run_id = store
            .mark_running(&t.id, TaskRunTrigger::Manual)
            .await
            .unwrap();
        let mid = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(mid.status, TaskStatus::Running);
        assert!(mid.started_at.is_some());

        store
            .mark_finished(&t.id, &run_id, true, false, "ok".into(), None)
            .await
            .unwrap();
        let done = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(done.status, TaskStatus::Scheduled, "success reschedules");
        assert_eq!(done.execution_count, 1);
        assert_eq!(done.consecutive_failures, 0, "success resets failures");
        assert!(done.next_run_at.is_some(), "next_run recomputed");

        // Run history recorded.
        let runs = store.list_runs(&t.id).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "completed");
        assert_eq!(runs[0].summary.as_deref(), Some("ok"));
    }

    #[tokio::test]
    async fn consecutive_failures_reset_on_success() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("flap")).await.unwrap();
        store
            .set_automation(
                &t.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Heartbeat),
                    schedule_pattern: None,
                    schedule_timezone: None,
                    heartbeat_interval_secs: Some(60),
                    max_executions: None,
                },
            )
            .await
            .unwrap();

        // Two failures.
        for _ in 0..2 {
            let rid = store
                .mark_running(&t.id, TaskRunTrigger::Heartbeat)
                .await
                .unwrap();
            store
                .mark_finished(
                    &t.id,
                    &rid,
                    false,
                    false,
                    String::new(),
                    Some("boom".into()),
                )
                .await
                .unwrap();
        }
        let after_fails = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(after_fails.consecutive_failures, 2);

        // Then a success — consecutive_failures must reset to 0.
        let rid = store
            .mark_running(&t.id, TaskRunTrigger::Heartbeat)
            .await
            .unwrap();
        store
            .mark_finished(&t.id, &rid, true, false, "recovered".into(), None)
            .await
            .unwrap();
        let after_ok = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(after_ok.consecutive_failures, 0, "reset on success");
        assert_eq!(after_ok.execution_count, 3);
    }

    #[tokio::test]
    async fn max_executions_exhausts_to_completed() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("max")).await.unwrap();
        store
            .set_automation(
                &t.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Heartbeat),
                    schedule_pattern: None,
                    schedule_timezone: None,
                    heartbeat_interval_secs: Some(60),
                    max_executions: Some(2),
                },
            )
            .await
            .unwrap();
        for _ in 0..2 {
            let rid = store
                .mark_running(&t.id, TaskRunTrigger::Heartbeat)
                .await
                .unwrap();
            store
                .mark_finished(&t.id, &rid, true, false, "ok".into(), None)
                .await
                .unwrap();
        }
        let done = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(done.status, TaskStatus::Completed, "exhausted → completed");
        assert_eq!(done.execution_count, 2);
        assert!(done.next_run_at.is_none(), "no further reschedule");
    }

    // ── Stranded recovery ──

    #[tokio::test]
    async fn recover_stranded_resets_running_tasks() {
        let store = TaskStore::in_memory().unwrap();
        // Task WITH automation → should recover to 'scheduled'.
        let t_auto = store.create_task(sample_params("auto")).await.unwrap();
        store
            .set_automation(
                &t_auto.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Heartbeat),
                    schedule_pattern: None,
                    schedule_timezone: None,
                    heartbeat_interval_secs: Some(60),
                    max_executions: None,
                },
            )
            .await
            .unwrap();
        // Task WITHOUT automation → should recover to 'backlog'.
        let t_plain = store.create_task(sample_params("plain")).await.unwrap();

        // Simulate a crash mid-run: mark both running.
        store
            .mark_running(&t_auto.id, TaskRunTrigger::Manual)
            .await
            .unwrap();
        store
            .mark_running(&t_plain.id, TaskRunTrigger::Manual)
            .await
            .unwrap();

        store.recover_stranded().await.unwrap();

        let auto = store.get_task_by_id(&t_auto.id).await.unwrap();
        assert_eq!(
            auto.status,
            TaskStatus::Scheduled,
            "automated task rescheduled"
        );
        let plain = store.get_task_by_id(&t_plain.id).await.unwrap();
        assert_eq!(
            plain.status,
            TaskStatus::Backlog,
            "plain task back to backlog"
        );

        // Orphaned task_runs rows closed as failed.
        let runs = store.list_runs(&t_auto.id).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "failed");
        assert!(runs[0].error.is_some());
    }

    // ── Verify config + partial update (RFC-043 completion) ──

    #[tokio::test]
    async fn set_verify_persists_and_errors_on_unknown() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("fonts")).await.unwrap();
        store
            .set_verify(
                &t.id,
                SetVerifyParams {
                    enabled: Some(true),
                    requirement: Some("Must include 3 pairings".into()),
                    max_iterations: Some(5),
                    verifier_agent_id: None,
                },
            )
            .await
            .unwrap();
        let got = store.get_task_by_id(&t.id).await.unwrap();
        assert!(got.verify_enabled);
        assert_eq!(
            got.verify_requirement.as_deref(),
            Some("Must include 3 pairings")
        );
        assert_eq!(got.verify_max_iterations, 5);

        // Unknown id errors instead of silently succeeding.
        assert!(
            store
                .set_verify("nope", SetVerifyParams::default())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn update_task_partial_updates_only_provided() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("partial")).await.unwrap();
        store
            .update_task(
                &t.id,
                UpdateTaskParams {
                    name: Some("Renamed".into()),
                    priority: Some(2),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let got = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(got.name, "Renamed");
        assert_eq!(got.instruction, t.instruction, "instruction untouched");
        assert_eq!(got.priority, 2);

        // Empty update is rejected (nothing to do).
        assert!(
            store
                .update_task(&t.id, UpdateTaskParams::default())
                .await
                .is_err()
        );
        // Unknown id errors.
        assert!(
            store
                .update_task(
                    "nope",
                    UpdateTaskParams {
                        name: Some("x".into()),
                        ..Default::default()
                    },
                )
                .await
                .is_err()
        );
    }

    // ── Verified runs + trigger-aware failure fuse ──

    #[tokio::test]
    async fn verified_run_marks_run_verified_and_task_completed() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("vrun")).await.unwrap();
        let rid = store
            .mark_running(&t.id, TaskRunTrigger::Manual)
            .await
            .unwrap();
        store
            .mark_finished(&t.id, &rid, true, true, "done".into(), None)
            .await
            .unwrap();
        let task = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        let runs = store.list_runs(&t.id).await.unwrap();
        assert_eq!(runs[0].status, "verified");
    }

    #[tokio::test]
    async fn manual_failure_does_not_touch_fuse() {
        let store = TaskStore::in_memory().unwrap();
        let t = store
            .create_task(sample_params("manualfuse"))
            .await
            .unwrap();
        for _ in 0..4 {
            let rid = store
                .mark_running(&t.id, TaskRunTrigger::Manual)
                .await
                .unwrap();
            store
                .mark_finished(
                    &t.id,
                    &rid,
                    false,
                    false,
                    String::new(),
                    Some("boom".into()),
                )
                .await
                .unwrap();
        }
        let task = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(task.consecutive_failures, 0, "manual failures never count");
        assert_eq!(
            task.status,
            TaskStatus::Failed,
            "terminal without automation"
        );
    }

    #[tokio::test]
    async fn fuse_pauses_after_three_auto_failures() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("fuse")).await.unwrap();
        store
            .set_automation(
                &t.id,
                SetScheduleParams {
                    automation_mode: Some(TaskAutomationMode::Heartbeat),
                    heartbeat_interval_secs: Some(60),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        for _ in 0..3 {
            let rid = store
                .mark_running(&t.id, TaskRunTrigger::Heartbeat)
                .await
                .unwrap();
            store
                .mark_finished(
                    &t.id,
                    &rid,
                    false,
                    false,
                    String::new(),
                    Some("boom".into()),
                )
                .await
                .unwrap();
        }
        let task = store.get_task_by_id(&t.id).await.unwrap();
        assert_eq!(task.status, TaskStatus::Paused, "fused → paused");
        assert!(task.next_run_at.is_none(), "paused → no reschedule");
        assert!(
            task.last_error
                .as_deref()
                .unwrap_or_default()
                .starts_with("Paused after 3 consecutive failures"),
            "error carries the fuse note: {:?}",
            task.last_error
        );
    }

    // ── Comments (RFC-043) ──

    #[tokio::test]
    async fn comments_roundtrip_update_delete_and_cascade() {
        let store = TaskStore::in_memory().unwrap();
        let t = store.create_task(sample_params("cmt")).await.unwrap();
        let c1 = store.add_comment(&t.id, "first", None, None).await.unwrap();
        let c2 = store
            .add_comment(&t.id, "second", Some("agent-7"), Some("sess-1"))
            .await
            .unwrap();
        assert_eq!(c1.content, "first");
        assert_eq!(c2.author_agent_id.as_deref(), Some("agent-7"));

        let listed = store.list_comments(&t.id).await.unwrap();
        assert_eq!(listed.len(), 2, "oldest first");
        assert_eq!(listed[0].id, c1.id);
        assert_eq!(listed[1].id, c2.id);

        store.update_comment(&c1.id, "edited").await.unwrap();
        let listed = store.list_comments(&t.id).await.unwrap();
        assert_eq!(listed[0].content, "edited");
        assert!(listed[0].updated_at.is_some());
        assert!(
            store.update_comment("nope", "x").await.is_err(),
            "unknown comment errors"
        );

        store.delete_comment(&c2.id).await.unwrap();
        assert_eq!(store.list_comments(&t.id).await.unwrap().len(), 1);

        // Comment on an unknown task errors (FK enforced).
        assert!(store.add_comment("ghost", "x", None, None).await.is_err());

        // Deleting the task cascades comments.
        store.delete_task(&t.id).await.unwrap();
        assert_eq!(store.list_comments(&t.id).await.unwrap().len(), 0);
    }

    // ── Dependencies (RFC-043) ──

    #[tokio::test]
    async fn dependencies_add_list_remove_and_populates_task_field() {
        let store = TaskStore::in_memory().unwrap();
        let a = store.create_task(sample_params("dep-a")).await.unwrap();
        let b = store.create_task(sample_params("dep-b")).await.unwrap();
        store.add_dependency(&a.id, &b.id).await.unwrap();

        let got = store.get_task_by_id(&a.id).await.unwrap();
        assert_eq!(got.dependencies, vec![b.id.clone()]);
        let listed = store.list_tasks(ListTasksParams::default()).await.unwrap();
        let a_row = listed.iter().find(|t| t.id == a.id).unwrap();
        assert_eq!(a_row.dependencies, vec![b.id.clone()]);

        assert_eq!(store.dependency_ids(&a.id).await.unwrap().len(), 1);
        // Unsatisfied while B is backlog.
        assert_eq!(
            store.unsatisfied_dependencies(&a.id).await.unwrap().len(),
            1
        );
        store
            .update_status(&b.id, &TaskStatus::Completed)
            .await
            .unwrap();
        assert!(
            store
                .unsatisfied_dependencies(&a.id)
                .await
                .unwrap()
                .is_empty()
        );

        store.remove_dependency(&a.id, &b.id).await.unwrap();
        assert_eq!(store.dependency_ids(&a.id).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn dependencies_reject_self_duplicate_unknown_and_cycles() {
        let store = TaskStore::in_memory().unwrap();
        let a = store.create_task(sample_params("cy-a")).await.unwrap();
        let b = store.create_task(sample_params("cy-b")).await.unwrap();
        let c = store.create_task(sample_params("cy-c")).await.unwrap();

        // Self-edge.
        let err = store.add_dependency(&a.id, &a.id).await.unwrap_err();
        assert!(err.to_string().contains("itself"), "{err}");
        // Unknown dependency target.
        let err = store.add_dependency(&a.id, "ghost").await.unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");

        // a → b → c, then c → a must be rejected as a cycle.
        store.add_dependency(&a.id, &b.id).await.unwrap();
        store.add_dependency(&b.id, &c.id).await.unwrap();
        let err = store.add_dependency(&c.id, &a.id).await.unwrap_err();
        assert!(err.to_string().contains("cycle"), "{err}");

        // Duplicate is a no-op error.
        let err = store.add_dependency(&a.id, &b.id).await.unwrap_err();
        assert!(err.to_string().contains("already"), "{err}");

        // Deleting a task cascades its dependency edges.
        store.delete_task(&b.id).await.unwrap();
        assert!(store.dependency_ids(&a.id).await.unwrap().is_empty());
    }
}
