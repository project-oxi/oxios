//! Issue API — per-project issue tracking and milestones.
//!
//! Backed by `oxicode_sdk::FileIssueStore` opened against
//! [`crate::project::project_issues_dir`]. The SDK supplies the durable parts
//! (markdown documents, atomic writes, content-hash CAS, `flock`-based
//! assignment liveness); this facade supplies the project scoping, the
//! ownership identity, and the milestone layer.
//!
//! # Ownership identity
//!
//! The SDK's assignment lock only protects anything when the caller identity
//! is non-empty **and** matches a `flock` this process actually holds — the
//! engine documents this at length after shipping the bug where it did not
//! (`oxicode` AGENTS.md, defect #13). Oxios identity is
//! `oxios-<pid>-<chat-session-id>`; [`IssueApi`] holds one
//! `liveness::AliveGuard` per `(project, session)` for the session's lifetime,
//! and `AgentConfig.session_id` carries the same string into
//! `ToolContext.session_id` so the store sees the identity whose lock we hold.
//!
//! When the daemon exits the OS drops every guard, so assignments become
//! reclaimable without a timeout, a heartbeat, or a PID heuristic.
//!
//! # CAS policy
//!
//! Mirrors the engine's split. The store is strict and never retries itself.
//! The agent tool wraps mutations in `oxicode_sdk::cas_retry`, so a stale hash
//! from an earlier `read` is advisory. Human-facing callers (HTTP) pass the
//! hash the client last saw and get a real conflict back — a person editing an
//! issue should be told someone else changed it, not silently overwritten.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use oxicode_sdk::{
    FileIssueStore, Issue, IssueError, IssueFilter, IssuePatch, Priority, Status, liveness,
};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::project::milestone::{self, Milestone, MilestonePatch, MilestoneProgress};
use crate::project::{ProjectId, project_issues_dir, project_milestones_path};

/// Serialized assignment info for API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct AssignmentInfo {
    /// Ownership identity holding the claim.
    pub session: String,
    /// When the claim was taken (informational — expiry is by liveness).
    pub acquired_at: String,
    /// Whether the owning process is still alive. A `false` here means the
    /// claim is stale and the next `start` will reclaim it.
    pub alive: bool,
}

/// Serialized issue for API responses and tool output.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct IssueView {
    /// Project-scoped issue number.
    pub number: u32,
    pub title: String,
    pub status: String,
    pub priority: String,
    /// Labels with the reserved `milestone:` entry removed — it is surfaced
    /// as [`Self::milestone`] instead so clients never render it twice.
    pub labels: Vec<String>,
    /// Milestone slug, derived from the reserved label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub milestone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_to: Option<AssignmentInfo>,
    /// Markdown body. Present on single reads, omitted from list responses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Hash to send back on the next mutation for conflict detection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

/// Milestone plus its derived progress.
#[derive(Debug, Clone, Serialize)]
#[allow(missing_docs)]
pub struct MilestoneView {
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<NaiveDate>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    /// Issues carrying this milestone's label.
    pub total: usize,
    /// How many of them are closed.
    pub closed: usize,
    /// Completion percentage; an empty milestone is 0, not 100.
    pub percent: u8,
}

/// Arguments for [`IssueApi::create`].
///
/// A struct rather than a parameter list: the two callers (the `issue` tool
/// and the HTTP route) both build it field-by-field from optional input, and
/// four of the six fields are defaultable.
#[derive(Debug, Clone, Default)]
pub struct NewIssue {
    /// One-line summary. Required.
    pub title: String,
    /// Markdown body. Empty is fine.
    pub body: String,
    /// Urgency.
    pub priority: Priority,
    /// Free-form labels; the reserved `milestone:` entry is added separately
    /// from [`Self::milestone`] and must not be passed here.
    pub labels: Vec<String>,
    /// Milestone slug to file the issue under.
    pub milestone: Option<String>,
    /// Chat session to link; becomes the ownership identity in `sessions`.
    pub session: Option<String>,
}

impl NewIssue {
    /// A titled issue with every other field defaulted.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Default::default()
        }
    }

    /// Attach the creating session.
    pub fn session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }
}

/// Filter for [`IssueApi::list`]. All fields optional.
#[derive(Debug, Clone, Default)]
pub struct IssueQuery {
    /// Constrain by status.
    pub status: Option<Status>,
    /// Constrain by priority.
    pub priority: Option<Priority>,
    /// Constrain to issues carrying this label.
    pub label: Option<String>,
    /// Constrain to issues in this milestone.
    pub milestone: Option<String>,
    /// Case-insensitive substring match on the title.
    pub text: Option<String>,
}

/// Per-project issue tracking.
///
/// One instance is shared across every [`crate::kernel_handle::KernelHandle`]
/// so the store cache and — critically — the ownership guards are common. Two
/// instances would each hold their own `flock` map and would not see each
/// other's claims.
pub struct IssueApi {
    stores: Mutex<HashMap<ProjectId, Arc<FileIssueStore>>>,
    guards: Mutex<HashMap<(ProjectId, String), liveness::AliveGuard>>,
}

impl std::fmt::Debug for IssueApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssueApi")
            .field("stores", &self.stores.lock().len())
            .field("guards", &self.guards.lock().len())
            .finish()
    }
}

impl Default for IssueApi {
    fn default() -> Self {
        Self::new()
    }
}

impl IssueApi {
    /// Create an empty facade. Stores open lazily on first use.
    pub fn new() -> Self {
        Self {
            stores: Mutex::new(HashMap::new()),
            guards: Mutex::new(HashMap::new()),
        }
    }

    /// The ownership identity for a chat session.
    ///
    /// The session id is stable across a session's turns: the orchestrator
    /// computes `session_id.unwrap_or(request_id)` and the first turn's value
    /// is returned to the client and echoed back thereafter — the single turn
    /// key `AGENTS.md` mandates. `ownership_id_is_stable_across_turns` pins
    /// that; without it a claim taken on turn 1 could not be closed on turn 2.
    pub fn ownership_id(session: &str) -> String {
        format!("oxios-{}-{}", std::process::id(), session)
    }

    /// Open (or reuse) the issue store for a project.
    pub fn store(&self, project: ProjectId) -> Result<Arc<FileIssueStore>> {
        if let Some(store) = self.stores.lock().get(&project) {
            return Ok(store.clone());
        }
        let dir = project_issues_dir(project);
        std::fs::create_dir_all(&dir).context("failed to create project issues directory")?;
        let store = Arc::new(FileIssueStore::open(dir).context("failed to open issue store")?);
        // Re-check under the lock: a concurrent caller may have won the race,
        // and two `FileIssueStore` values over one directory would keep
        // independent mtime caches.
        let mut guard = self.stores.lock();
        Ok(guard.entry(project).or_insert(store).clone())
    }

    /// Acquire and hold this session's liveness lock for a project.
    ///
    /// Idempotent: a session that already holds the lock keeps the same guard.
    /// Must be called before any mutation that records ownership, otherwise
    /// the store writes an owner whose lock nobody holds.
    pub fn claim_identity(&self, project: ProjectId, session: &str) -> Result<String> {
        let id = Self::ownership_id(session);
        let key = (project, id.clone());
        let dir = project_issues_dir(project);
        std::fs::create_dir_all(&dir).context("failed to create project issues directory")?;
        // Held across the flock attempt: flock locks are per open file
        // description, so two concurrent claims for one identity racing
        // between the map check and `acquire` would have the loser's
        // non-blocking attempt fail outright. `acquire` uses LOCK_NB (it
        // errors rather than waits), so this critical section is bounded.
        let mut guards = self.guards.lock();
        if guards.contains_key(&key) {
            return Ok(id);
        }
        let guard = liveness::acquire(&dir, &id)
            .with_context(|| format!("failed to acquire liveness lock for {id}"))?;
        guards.entry(key).or_insert(guard);
        Ok(id)
    }

    /// Drop every liveness guard held for a chat session.
    ///
    /// Called when a session is deleted. Assignments the session held become
    /// reclaimable immediately rather than waiting for daemon exit.
    pub fn release_session(&self, session: &str) {
        let id = Self::ownership_id(session);
        self.guards.lock().retain(|(_, held), _| held != &id);
    }

    // ── Issues ──────────────────────────────────────────────────────────

    /// List issues, newest first (the store sorts by `updated_at` desc).
    pub fn list(&self, project: ProjectId, query: &IssueQuery) -> Result<Vec<IssueView>> {
        let store = self.store(project)?;
        let filter = IssueFilter {
            status: query.status,
            priority: query.priority,
            label: query.label.clone(),
            assigned_to_session: None,
            text: query.text.clone(),
        };
        let dir = store.issues_dir();
        let issues = store.list(&filter)?;
        Ok(issues
            .into_iter()
            .filter(|i| match &query.milestone {
                // Milestone is a derived view over labels, so it cannot ride
                // `IssueFilter`; filter it here instead.
                Some(slug) => milestone::milestone_of(&i.meta.labels).as_deref() == Some(slug),
                None => true,
            })
            .map(|i| view_of(&i, &dir, None, false))
            .collect())
    }

    /// Read one issue with its body and content hash.
    pub fn read(&self, project: ProjectId, number: u32) -> Result<IssueView, IssueError> {
        let store = self.store(project).map_err(IssueError::Other)?;
        let dir = store.issues_dir();
        let (issue, hash) = store.read(number).map_err(IssueError::Other)?;
        Ok(view_of(&issue, &dir, Some(hash), true))
    }

    /// Create an issue. `session` is linked into the issue's session list.
    pub fn create(&self, project: ProjectId, new: NewIssue) -> Result<IssueView> {
        let store = self.store(project)?;
        let dir = store.issues_dir();
        let labels = match new.milestone.as_deref() {
            Some(slug) => milestone::with_milestone(&new.labels, Some(slug)),
            None => new.labels,
        };
        let owner = new.session.as_deref().map(Self::ownership_id);
        let issue = store.create(new.title, new.body, new.priority, labels, owner.as_deref())?;
        Ok(view_of(&issue, &dir, None, true))
    }

    /// Apply a patch. Enforces ownership when the issue is claimed.
    pub async fn patch(
        &self,
        project: ProjectId,
        number: u32,
        patch: IssuePatch,
        session: Option<&str>,
        expected_hash: Option<String>,
    ) -> Result<IssueView, IssueError> {
        let store = self.store(project).map_err(IssueError::Other)?;
        let dir = store.issues_dir();
        let caller = session.map(Self::ownership_id);
        let issue = store
            .apply_patch(number, patch, caller, expected_hash)
            .await?;
        Ok(view_of(&issue, &dir, None, true))
    }

    /// Claim an issue for `session`.
    pub async fn start(
        &self,
        project: ProjectId,
        number: u32,
        session: &str,
        expected_hash: Option<String>,
    ) -> Result<IssueView, IssueError> {
        let id = self
            .claim_identity(project, session)
            .map_err(IssueError::Other)?;
        let store = self.store(project).map_err(IssueError::Other)?;
        let dir = store.issues_dir();
        let issue = store.start(number, &id, expected_hash).await?;
        Ok(view_of(&issue, &dir, None, true))
    }

    /// Give up a claim.
    pub async fn release(
        &self,
        project: ProjectId,
        number: u32,
        session: &str,
        expected_hash: Option<String>,
    ) -> Result<IssueView, IssueError> {
        let store = self.store(project).map_err(IssueError::Other)?;
        let dir = store.issues_dir();
        let id = Self::ownership_id(session);
        let issue = store.release(number, &id, expected_hash).await?;
        Ok(view_of(&issue, &dir, None, true))
    }

    /// Close an issue. The caller must hold the claim; `start` first.
    pub async fn close(
        &self,
        project: ProjectId,
        number: u32,
        session: &str,
        expected_hash: Option<String>,
    ) -> Result<IssueView, IssueError> {
        let store = self.store(project).map_err(IssueError::Other)?;
        let dir = store.issues_dir();
        let id = Self::ownership_id(session);
        let issue = store.close(number, &id, expected_hash).await?;
        Ok(view_of(&issue, &dir, None, true))
    }

    /// Reopen a closed issue. No claim required.
    pub async fn reopen(
        &self,
        project: ProjectId,
        number: u32,
        expected_hash: Option<String>,
    ) -> Result<IssueView, IssueError> {
        let store = self.store(project).map_err(IssueError::Other)?;
        let dir = store.issues_dir();
        let issue = store.reopen(number, expected_hash).await?;
        Ok(view_of(&issue, &dir, None, true))
    }

    // ── Milestones ──────────────────────────────────────────────────────

    /// List milestones with derived progress.
    pub fn list_milestones(&self, project: ProjectId) -> Result<Vec<MilestoneView>> {
        let defs = milestone::load(&project_milestones_path(project))?;
        let store = self.store(project)?;
        let issues = store.list(&IssueFilter::default())?;
        let counts = milestone::tally(
            issues
                .iter()
                .map(|i| (i.meta.labels.as_slice(), i.meta.status == Status::Closed)),
        );
        Ok(defs
            .into_iter()
            .map(|m| {
                let p = counts.get(&m.slug).copied().unwrap_or_default();
                milestone_view(m, p)
            })
            .collect())
    }

    /// Create a milestone. The slug is derived from the title when absent.
    pub fn create_milestone(
        &self,
        project: ProjectId,
        title: String,
        description: Option<String>,
        due: Option<NaiveDate>,
        slug: Option<String>,
    ) -> Result<MilestoneView> {
        let path = project_milestones_path(project);
        let mut defs = milestone::load(&path)?;

        let slug = slug.unwrap_or_else(|| milestone::slugify(&title));
        if slug.is_empty() {
            anyhow::bail!(
                "milestone slug is empty; give the milestone a title with letters or digits"
            );
        }
        if defs.iter().any(|m| m.slug == slug) {
            anyhow::bail!("milestone `{slug}` already exists");
        }

        let mut m = Milestone::new(slug, title);
        m.description = description.unwrap_or_default();
        m.due = due;
        defs.push(m.clone());
        milestone::save(&path, &defs)?;
        Ok(milestone_view(m, MilestoneProgress::default()))
    }

    /// Update a milestone's own fields. Membership is changed on the issues.
    pub fn update_milestone(
        &self,
        project: ProjectId,
        slug: &str,
        patch: MilestonePatch,
    ) -> Result<MilestoneView> {
        let path = project_milestones_path(project);
        let mut defs = milestone::load(&path)?;
        let m = defs
            .iter_mut()
            .find(|m| m.slug == slug)
            .with_context(|| format!("milestone `{slug}` not found"))?;

        if let Some(title) = patch.title {
            m.title = title;
        }
        if let Some(description) = patch.description {
            m.description = description;
        }
        if let Some(due) = patch.due {
            m.due = due;
        }
        if let Some(status) = patch.status {
            m.status = status;
        }
        m.updated_at = chrono::Utc::now();
        let updated = m.clone();
        milestone::save(&path, &defs)?;

        let progress = self.progress_for(project, slug).unwrap_or_default();
        Ok(milestone_view(updated, progress))
    }

    /// Delete a milestone and strip its label from every member issue.
    ///
    /// Returns the numbers of issues that could not be updated. A partial
    /// failure keeps the definition in place so members are never orphaned
    /// against a milestone that no longer exists.
    pub async fn delete_milestone(&self, project: ProjectId, slug: &str) -> Result<Vec<u32>> {
        let store = self.store(project)?;
        let members: Vec<u32> = store
            .list(&IssueFilter::default())?
            .iter()
            .filter(|i| milestone::milestone_of(&i.meta.labels).as_deref() == Some(slug))
            .map(|i| i.meta.id)
            .collect();

        let mut failed = Vec::new();
        for number in members {
            if self
                .set_milestone(project, number, None, None)
                .await
                .is_err()
            {
                failed.push(number);
            }
        }
        if !failed.is_empty() {
            return Ok(failed);
        }

        let path = project_milestones_path(project);
        let mut defs = milestone::load(&path)?;
        defs.retain(|m| m.slug != slug);
        milestone::save(&path, &defs)?;
        Ok(Vec::new())
    }

    /// Set (or clear) an issue's milestone membership.
    pub async fn set_milestone(
        &self,
        project: ProjectId,
        number: u32,
        slug: Option<&str>,
        session: Option<&str>,
    ) -> Result<IssueView, IssueError> {
        let store = self.store(project).map_err(IssueError::Other)?;
        let (issue, hash) = store.read(number).map_err(IssueError::Other)?;
        let labels = milestone::with_milestone(&issue.meta.labels, slug);
        let patch = IssuePatch {
            labels: Some(labels),
            ..Default::default()
        };
        self.patch(project, number, patch, session, Some(hash))
            .await
    }

    fn progress_for(&self, project: ProjectId, slug: &str) -> Option<MilestoneProgress> {
        let store = self.store(project).ok()?;
        let issues = store.list(&IssueFilter::default()).ok()?;
        let counts = milestone::tally(
            issues
                .iter()
                .map(|i| (i.meta.labels.as_slice(), i.meta.status == Status::Closed)),
        );
        Some(counts.get(slug).copied().unwrap_or_default())
    }
}

fn milestone_view(m: Milestone, p: MilestoneProgress) -> MilestoneView {
    MilestoneView {
        slug: m.slug,
        title: m.title,
        description: m.description,
        due: m.due,
        status: m.status.to_string(),
        created_at: m.created_at.to_rfc3339(),
        updated_at: m.updated_at.to_rfc3339(),
        total: p.total,
        closed: p.closed,
        percent: p.percent(),
    }
}

fn view_of(
    issue: &Issue,
    issues_dir: &std::path::Path,
    content_hash: Option<String>,
    with_body: bool,
) -> IssueView {
    let meta = &issue.meta;
    IssueView {
        number: meta.id,
        title: meta.title.clone(),
        status: meta.status.to_string(),
        priority: meta.priority.to_string(),
        labels: meta
            .labels
            .iter()
            .filter(|l| !l.starts_with(milestone::MILESTONE_LABEL_PREFIX))
            .cloned()
            .collect(),
        milestone: milestone::milestone_of(&meta.labels),
        assignee: meta.assignee.clone(),
        created_at: meta.created_at.to_rfc3339(),
        updated_at: meta.updated_at.to_rfc3339(),
        closed_at: meta.closed_at.map(|t| t.to_rfc3339()),
        assigned_to: meta.assigned_to.as_ref().map(|a| AssignmentInfo {
            session: a.session.clone(),
            acquired_at: a.acquired_at.to_rfc3339(),
            alive: liveness::is_session_alive(issues_dir, &a.session),
        }),
        body: with_body.then(|| issue.body.clone()),
        content_hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scoped `data_home` override. The lock lives in `project::paths` so
    /// every test touching that variable — here and there — shares one, which
    /// two separate mutexes would not accomplish.
    use crate::project::paths::test_support::temp_data_home;

    #[test]
    fn ownership_id_is_stable_across_turns() {
        // The orchestrator's turn key is the same string on every turn of a
        // session (turn 1 uses request_id, which then *becomes* the session
        // id). If that ever changes, a claim taken on turn 1 could not be
        // closed on turn 2 — and nothing else would fail visibly.
        let a = IssueApi::ownership_id("sess-1");
        let b = IssueApi::ownership_id("sess-1");
        assert_eq!(a, b);
        assert_ne!(a, IssueApi::ownership_id("sess-2"));
        assert!(a.starts_with("oxios-"), "unexpected identity shape: {a}");
        assert!(
            !a.ends_with('-'),
            "identity must not be empty-suffixed: {a}"
        );
    }

    #[test]
    fn release_session_is_idempotent_for_unknown_sessions() {
        let api = IssueApi::new();
        // Session-delete cleanup runs for every deleted session; most never
        // claimed a liveness lock. Releasing twice must stay a quiet no-op.
        api.release_session("never-seen");
        api.release_session("never-seen");
    }

    #[tokio::test]
    async fn issues_are_numbered_per_project() {
        let _home = temp_data_home();
        let api = IssueApi::new();
        let p1 = ProjectId::new_v4();
        let p2 = ProjectId::new_v4();

        let a = api.create(p1, NewIssue::new("first").session("s")).unwrap();
        let b = api
            .create(p1, NewIssue::new("second").session("s"))
            .unwrap();
        let c = api
            .create(
                p2,
                NewIssue {
                    priority: Priority::High,
                    ..NewIssue::new("other project").session("s")
                },
            )
            .unwrap();

        assert_eq!((a.number, b.number), (1, 2));
        assert_eq!(c.number, 1, "numbering restarts in a separate project");
    }

    #[tokio::test]
    async fn milestone_membership_survives_an_sdk_write() {
        let _home = temp_data_home();
        let api = IssueApi::new();
        let project = ProjectId::new_v4();

        api.create_milestone(project, "v0.4 release".into(), None, None, None)
            .unwrap();
        let issue = api
            .create(
                project,
                NewIssue {
                    labels: vec!["bug".into()],
                    milestone: Some("v0-4-release".into()),
                    ..NewIssue::new("task").session("s")
                },
            )
            .unwrap();
        assert_eq!(issue.milestone.as_deref(), Some("v0-4-release"));
        assert_eq!(
            issue.labels,
            vec!["bug".to_string()],
            "reserved label is not shown twice"
        );

        // A title-only patch round-trips through the SDK serializer; the
        // reserved label must still be there afterwards.
        let hash = api.read(project, issue.number).unwrap().content_hash;
        let patched = api
            .patch(
                project,
                issue.number,
                IssuePatch {
                    title: Some("renamed".into()),
                    ..Default::default()
                },
                Some("s"),
                hash,
            )
            .await
            .unwrap();
        assert_eq!(patched.milestone.as_deref(), Some("v0-4-release"));
    }

    #[tokio::test]
    async fn milestone_progress_is_derived_from_issues() {
        let _home = temp_data_home();
        let api = IssueApi::new();
        let project = ProjectId::new_v4();
        api.create_milestone(project, "v1".into(), None, None, None)
            .unwrap();

        for title in ["a", "b"] {
            api.create(
                project,
                NewIssue {
                    milestone: Some("v1".into()),
                    ..NewIssue::new(title).session("s")
                },
            )
            .unwrap();
        }
        api.start(project, 1, "s", None).await.unwrap();
        api.close(project, 1, "s", None).await.unwrap();

        let ms = api.list_milestones(project).unwrap();
        assert_eq!((ms[0].total, ms[0].closed, ms[0].percent), (2, 1, 50));
    }

    #[tokio::test]
    async fn a_live_claim_blocks_another_session() {
        let _home = temp_data_home();
        let api = IssueApi::new();
        let project = ProjectId::new_v4();
        api.create(project, NewIssue::new("contended").session("a"))
            .unwrap();

        api.start(project, 1, "session-a", None).await.unwrap();
        let err = api.start(project, 1, "session-b", None).await.unwrap_err();
        assert!(
            matches!(err, IssueError::Assigned { .. }),
            "expected Assigned, got {err:?}"
        );
    }

    #[tokio::test]
    async fn releasing_a_session_makes_its_claim_reclaimable() {
        let _home = temp_data_home();
        let api = IssueApi::new();
        let project = ProjectId::new_v4();
        api.create(project, NewIssue::new("handover").session("a"))
            .unwrap();
        api.start(project, 1, "session-a", None).await.unwrap();

        api.release_session("session-a");

        let taken = api.start(project, 1, "session-b", None).await.unwrap();
        let owner = taken.assigned_to.expect("issue should be claimed");
        assert_eq!(owner.session, IssueApi::ownership_id("session-b"));
    }

    #[tokio::test]
    async fn deleting_a_milestone_strips_it_from_members() {
        let _home = temp_data_home();
        let api = IssueApi::new();
        let project = ProjectId::new_v4();
        api.create_milestone(project, "doomed".into(), None, None, None)
            .unwrap();
        api.create(
            project,
            NewIssue {
                labels: vec!["keep".into()],
                milestone: Some("doomed".into()),
                ..NewIssue::new("member").session("s")
            },
        )
        .unwrap();

        let failed = api.delete_milestone(project, "doomed").await.unwrap();
        assert!(failed.is_empty(), "unexpected failures: {failed:?}");

        let issue = api.read(project, 1).unwrap();
        assert_eq!(issue.milestone, None);
        assert_eq!(issue.labels, vec!["keep".to_string()]);
        assert!(api.list_milestones(project).unwrap().is_empty());
    }

    #[tokio::test]
    async fn duplicate_milestone_slugs_are_rejected() {
        let _home = temp_data_home();
        let api = IssueApi::new();
        let project = ProjectId::new_v4();
        api.create_milestone(project, "v1".into(), None, None, None)
            .unwrap();
        assert!(
            api.create_milestone(project, "v1".into(), None, None, None)
                .is_err()
        );
    }

    #[tokio::test]
    async fn milestone_filter_selects_only_members() {
        let _home = temp_data_home();
        let api = IssueApi::new();
        let project = ProjectId::new_v4();
        api.create(
            project,
            NewIssue {
                milestone: Some("v1".into()),
                ..NewIssue::new("in").session("s")
            },
        )
        .unwrap();
        api.create(project, NewIssue::new("out").session("s"))
            .unwrap();

        let found = api
            .list(
                project,
                &IssueQuery {
                    milestone: Some("v1".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "in");
    }
}
