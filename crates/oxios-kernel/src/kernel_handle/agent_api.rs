//! Agent API — agent lifecycle, budget, history log.

use crate::agent_log_db::{AgentListFilter, AgentStats, QueryResult};
use crate::budget::{BudgetExceeded, BudgetInfo, BudgetLimit, BudgetManager};
use crate::state_store::StateStore;
use crate::supervisor::Supervisor;
use crate::types::{AgentId, AgentInfo};
use std::sync::Arc;

/// Agent management system calls.
pub struct AgentApi {
    pub(crate) supervisor: Arc<dyn Supervisor>,
    pub(crate) budget_manager: Arc<BudgetManager>,
    /// State store for filesystem agent persistence.
    pub(crate) state_store: Option<Arc<StateStore>>,
    /// SQLite-backed agent history query index.
    pub(crate) agent_log_db: Option<Arc<crate::agent_log_db::AgentLogDb>>,
}

impl AgentApi {
    /// Create a new AgentApi.
    pub fn new(
        supervisor: Arc<dyn Supervisor>,
        budget_manager: Arc<BudgetManager>,
    ) -> Self {
        Self {
            supervisor,
            budget_manager,
            state_store: None,
            agent_log_db: None,
        }
    }

    /// Attach a state store for agent history persistence.
    pub fn set_state_store(&mut self, store: Arc<StateStore>) {
        self.state_store = Some(store);
    }

    /// Attach an SQLite-backed agent log database.
    pub fn set_agent_log_db(&mut self, db: Arc<crate::agent_log_db::AgentLogDb>) {
        self.agent_log_db = Some(db);
    }

    /// List running agents (in-memory only).
    pub async fn list(&self) -> anyhow::Result<Vec<AgentInfo>> {
        self.supervisor
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("supervisor: {e}"))
    }

    /// Kill a running agent.
    pub async fn kill(&self, agent_id: &str) -> anyhow::Result<()> {
        let id = uuid::Uuid::parse_str(agent_id)
            .map_err(|e| anyhow::anyhow!("invalid agent id: {e}"))?;
        self.supervisor
            .kill(id)
            .await
            .map_err(|e| anyhow::anyhow!("supervisor: {e}"))
    }

    /// Check budget for an agent.
    pub fn check_budget(&self, agent_id: &AgentId) -> BudgetInfo {
        self.budget_manager.remaining(agent_id)
    }

    /// Set budget for an agent.
    pub fn set_budget(&self, limit: BudgetLimit) {
        self.budget_manager.set_budget(limit);
    }

    /// Remove budget for an agent.
    pub fn remove_budget(&self, agent_id: &AgentId) {
        self.budget_manager.remove_budget(agent_id);
    }

    /// Reserve tokens for an agent.
    pub fn reserve_budget(&self, agent_id: &AgentId, tokens: u64) -> Result<(), BudgetExceeded> {
        self.budget_manager.reserve(agent_id, tokens)
    }

    /// Reset budget window for an agent.
    pub fn reset_budget(&self, agent_id: &AgentId) {
        self.budget_manager.reset_window(agent_id);
    }

    /// Get full budget info (limits + usage) for an agent.
    pub fn full_budget_info(&self, agent_id: &AgentId) -> Option<crate::budget::FullBudgetInfo> {
        self.budget_manager.full_info(agent_id)
    }

    /// Get full budget info for all agents with configured budgets.
    pub fn all_budget_info(&self) -> Vec<crate::budget::FullBudgetInfo> {
        self.budget_manager.all_full_info()
    }

    // ── Agent History Log ─────────────────────────────────────────

    /// Query agent history (in-memory + SQLite) with filters.
    ///
    /// Merges running agents from supervisor with persisted agents
    /// from the SQLite query index. Running agents are prepended.
    pub async fn query(&self, filter: &AgentListFilter) -> anyhow::Result<QueryResult> {
        // Get running agents from supervisor
        let running = self.supervisor.list().await.unwrap_or_default();

        // Query SQLite for historical agents
        if let Some(ref db) = self.agent_log_db {
            let mut result = db.query(filter).map_err(|e| anyhow::anyhow!("{e}"))?;

            // Only prepend running agents on the first page — otherwise we'd
            // inject them into every requested page and break pagination.
            if filter.page == 1 {
                // Dedup against the SQLite items so a running agent that is
                // also already persisted isn't shown twice.
                let existing_ids: std::collections::HashSet<_> =
                    result.items.iter().map(|a| a.id).collect();
                let mut prepended = 0u64;
                for agent in &running {
                    if existing_ids.contains(&agent.id) {
                        continue;
                    }
                    if filter_matches(agent, filter) {
                        result.items.insert(0, agent.clone());
                        prepended += 1;
                    }
                }
                result.total = result.total.saturating_add(prepended);
                // Recompute total_pages so callers see a consistent view.
                result.total_pages = if result.total == 0 {
                    1
                } else {
                    ((result.total as f64) / filter.per_page.max(1) as f64).ceil() as u32
                };
            }

            return Ok(result);
        }

        // Fallback: filesystem-only scan
        #[allow(unused_mut)]
        let mut persisted: Vec<AgentInfo> = Vec::new();
        if let Some(ref store) = self.state_store {
            let names = store.list_category("agents").await.unwrap_or_default();
            for name in &names {
                if let Ok(Some(agent)) = store.load_json::<AgentInfo>("agents", name).await {
                    persisted.push(agent);
                }
            }
        }

        // Merge running + persisted, dedup by id (running wins)
        let running_ids: std::collections::HashSet<_> = running.iter().map(|a| a.id).collect();
        persisted.retain(|a| !running_ids.contains(&a.id));

        let mut all = running;
        all.extend(persisted);

        // In-memory filter/sort/paginate (basic fallback)
        let filtered = fallback_filter(all, filter);
        let total = filtered.len() as u64;
        let offset = ((filter.page.max(1) - 1) * filter.per_page) as usize;
        let limit = filter.per_page.min(200) as usize;
        let items: Vec<AgentInfo> = filtered.into_iter().skip(offset).take(limit).collect();
        let total_pages = if total == 0 {
            1
        } else {
            ((total as f64) / filter.per_page as f64).ceil() as u32
        };

        Ok(QueryResult {
            items,
            total,
            page: filter.page,
            per_page: filter.per_page,
            total_pages,
            stats: crate::agent_log_db::FilteredStats::default(),
        })
    }

    /// Get an agent by ID (from SQLite or filesystem fallback).
    pub async fn get(&self, id: &str) -> anyhow::Result<Option<AgentInfo>> {
        // Try SQLite first
        if let Some(ref db) = self.agent_log_db
            && let Ok(Some(agent)) = db.get(id)
        {
            return Ok(Some(agent));
        }

        // Fallback: filesystem JSON
        if let Some(ref store) = self.state_store
            && let Ok(Some(agent)) = store.load_json::<AgentInfo>("agents", id).await
        {
            return Ok(Some(agent));
        }

        // Fallback: in-memory
        if let Ok(agents) = self.supervisor.list().await
            && let Some(agent) = agents.into_iter().find(|a| a.id.to_string() == id)
        {
            return Ok(Some(agent));
        }

        Ok(None)
    }

    /// Global agent stats (unfiltered).
    pub async fn stats(&self) -> anyhow::Result<AgentStats> {
        // Try SQLite first
        if let Some(ref db) = self.agent_log_db {
            return db.stats().map_err(|e| anyhow::anyhow!("{e}"));
        }

        // Fallback: compute from in-memory + filesystem
        let mut s = AgentStats::default();
        let running = self.supervisor.list().await.unwrap_or_default();
        for a in &running {
            s.total_agents += 1;
            match a.status {
                crate::types::AgentStatus::Running | crate::types::AgentStatus::Starting => {
                    s.running += 1
                }
                crate::types::AgentStatus::Idle
                | crate::types::AgentStatus::Stopped
                | crate::types::AgentStatus::Completed => s.completed += 1,
                crate::types::AgentStatus::Failed => s.failed += 1,
            }
            s.total_tokens += a.tokens_input + a.tokens_output;
            s.total_cost_usd += a.cost_usd;
        }
        Ok(s)
    }
    // ── Cost aggregation (dollar-based spend views) ─────────────────

    /// Aggregate spend for a time window.
    pub fn cost_summary(
        &self,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<crate::agent_log_db::CostSummary> {
        if let Some(ref db) = self.agent_log_db {
            return db.cost_summary(since).map_err(|e| anyhow::anyhow!("{e}"));
        }
        Ok(crate::agent_log_db::CostSummary::default())
    }

    /// Per-model spend breakdown for a time window.
    pub fn cost_by_model(
        &self,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<crate::agent_log_db::ModelCostRow>> {
        if let Some(ref db) = self.agent_log_db {
            return db.cost_by_model(since).map_err(|e| anyhow::anyhow!("{e}"));
        }
        Ok(Vec::new())
    }

    /// Per-project spend breakdown for a time window.
    pub fn cost_by_project(
        &self,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> anyhow::Result<Vec<crate::agent_log_db::ProjectCostRow>> {
        if let Some(ref db) = self.agent_log_db {
            return db
                .cost_by_project(since)
                .map_err(|e| anyhow::anyhow!("{e}"));
        }
        Ok(Vec::new())
    }

    /// Daily spend time-series for the last `days` days.
    pub fn cost_daily(&self, days: u32) -> anyhow::Result<Vec<crate::agent_log_db::DailyCostRow>> {
        if let Some(ref db) = self.agent_log_db {
            return db.cost_daily(days).map_err(|e| anyhow::anyhow!("{e}"));
        }
        Ok(Vec::new())
    }

    /// Rebuild SQLite agent log index from filesystem JSON.
    pub async fn reindex(&self) -> anyhow::Result<crate::agent_log_db::RebuildReport> {
        match (self.agent_log_db.as_ref(), self.state_store.as_ref()) {
            (Some(db), Some(store)) => db
                .reindex_all(store)
                .await
                .map_err(|e| anyhow::anyhow!("{e}")),
            _ => anyhow::bail!("Agent log DB not initialized"),
        }
    }
}

/// Check if an agent matches the filter (used for prepending running agents).
fn filter_matches(agent: &AgentInfo, filter: &AgentListFilter) -> bool {
    // Status filter
    if let Some(status) = filter.status {
        let status_str = agent.status.to_string();
        if status_str != status.as_sql()
            && !(status_str == "idle" && status.as_sql() == "completed")
            && !(status_str == "idle" && status.as_sql() == "running")
        {
            return false;
        }
    }

    // Date range
    if let Some(from) = filter.date_from
        && agent.created_at < from
    {
        return false;
    }
    if let Some(to) = filter.date_to
        && agent.created_at > to
    {
        return false;
    }

    // Session / project
    if let Some(ref sid) = filter.session_id
        && agent.session_id.as_deref() != Some(sid.as_str())
    {
        return false;
    }
    if let Some(ref pid) = filter.project_id
        && agent.project_id.map(|p| p.to_string()).as_deref() != Some(pid.as_str())
    {
        return false;
    }

    // Model filter (substring)
    if let Some(ref model) = filter.model_id
        && !agent.model_id.contains(model)
    {
        return false;
    }

    // Text search (name + error only for in-memory agents — no tool_calls scan)
    if let Some(ref q) = filter.q {
        let q_lower = q.to_lowercase();
        let name_match = agent.name.to_lowercase().contains(&q_lower);
        let error_match = agent
            .error
            .as_deref()
            .is_some_and(|e| e.to_lowercase().contains(&q_lower));
        if !name_match && !error_match {
            return false;
        }
    }

    // Error filter
    if let Some(has_err) = filter.has_error {
        let agent_has_err = agent.error.as_deref().is_some_and(|e| !e.is_empty());
        if has_err != agent_has_err {
            return false;
        }
    }

    // Budget ranges
    if let Some(min) = filter.cost_min
        && agent.cost_usd < min
    {
        return false;
    }
    if let Some(max) = filter.cost_max
        && agent.cost_usd > max
    {
        return false;
    }

    true
}

/// Fallback in-memory filtering (used when SQLite is not available).
fn fallback_filter(mut agents: Vec<AgentInfo>, filter: &AgentListFilter) -> Vec<AgentInfo> {
    // Sort
    match filter.sort_by {
        crate::agent_log_db::SortBy::CreatedAt => {
            agents.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        }
        crate::agent_log_db::SortBy::Cost => {
            agents.sort_by(|a, b| {
                b.cost_usd
                    .partial_cmp(&a.cost_usd)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        crate::agent_log_db::SortBy::Duration => {
            let dur = |a: &AgentInfo| -> i64 {
                match (a.started_at, a.completed_at) {
                    (Some(s), Some(e)) => (e - s).num_seconds(),
                    _ => 0,
                }
            };
            agents.sort_by_key(|a| std::cmp::Reverse(dur(a)));
        }
        crate::agent_log_db::SortBy::Tokens => {
            agents.sort_by_key(|a| std::cmp::Reverse(a.tokens_input + a.tokens_output));
        }
        crate::agent_log_db::SortBy::Name => {
            agents.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }

    agents
}
