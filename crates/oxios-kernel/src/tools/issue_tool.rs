//! The `issue` agent tool — project-scoped issue and milestone management.
//!
//! Oxios-owned rather than the SDK's `IssueTool` for three reasons:
//!
//! 1. The SDK tool's description hardcodes "stored as markdown files in
//!    `.oxicode/issues/`", which is false here and would mislead the model.
//! 2. It binds one store at construction; ours resolves the store from the
//!    turn's project, which the runtime supplies per run.
//! 3. Milestones are an Oxios concept and need actions.
//!
//! Everything durable still comes from the SDK: the store, `cas_retry`, and
//! the liveness locking all live in `oxicode-sdk`. See
//! [`crate::kernel_handle::IssueApi`] for the CAS and ownership contracts.

use std::sync::Arc;

use async_trait::async_trait;
use oxicode_sdk::{
    AgentTool, AgentToolResult, IssueError, IssuePatch, Priority, Status, ToolContext, cas_retry,
};
use serde_json::{Value, json};

use super::structured_results::StructuredResultBus;
use crate::kernel_handle::issue_api::{IssueApi, IssueQuery, IssueView, NewIssue};
use crate::project::ProjectId;

/// Maximum title length, in characters (mirrors the SDK tool's guard).
const MAX_TITLE_LEN: usize = 512;
/// Maximum body length, in bytes.
const MAX_BODY_BYTES: usize = 256 * 1024;
/// Maximum number of labels on one issue.
const MAX_LABELS: usize = 32;
/// Maximum length of a single label, in characters.
const MAX_LABEL_LEN: usize = 64;

/// Project-scoped `issue` tool.
#[derive(Debug)]
pub struct IssueTool {
    issues: Arc<IssueApi>,
    bus: Arc<StructuredResultBus>,
    /// The turn's active project. `None` means the session has no project
    /// selected, and every action reports that rather than guessing one.
    project: Option<ProjectId>,
}

impl IssueTool {
    /// Bind the tool to a project for one agent run.
    pub fn new(
        issues: Arc<IssueApi>,
        bus: Arc<StructuredResultBus>,
        project: Option<ProjectId>,
    ) -> Self {
        Self {
            issues,
            bus,
            project,
        }
    }

    fn project(&self) -> Result<ProjectId, String> {
        self.project.ok_or_else(|| {
            "No project is selected for this session. Issues are per-project — \
             ask the user to select a project, then retry."
                .to_string()
        })
    }
}

fn priority_of(v: Option<&str>) -> Result<Priority, String> {
    match v {
        None | Some("medium") => Ok(Priority::Medium),
        Some("low") => Ok(Priority::Low),
        Some("high") => Ok(Priority::High),
        Some("critical") => Ok(Priority::Critical),
        Some(other) => Err(format!("invalid priority: {other}")),
    }
}

fn status_of(v: Option<&str>) -> Result<Option<Status>, String> {
    match v {
        None => Ok(None),
        Some("open") => Ok(Some(Status::Open)),
        Some("closed") => Ok(Some(Status::Closed)),
        Some(other) => Err(format!("invalid status: {other}")),
    }
}

fn labels_of(v: Option<&Value>) -> Result<Option<Vec<String>>, String> {
    let Some(arr) = v.and_then(|v| v.as_array()) else {
        return Ok(None);
    };
    if arr.len() > MAX_LABELS {
        return Err(format!("too many labels: {} (max {MAX_LABELS})", arr.len()));
    }
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let s = item
            .as_str()
            .ok_or_else(|| "labels must be strings".to_string())?;
        if s.chars().count() > MAX_LABEL_LEN {
            return Err(format!("label too long (max {MAX_LABEL_LEN} chars): {s}"));
        }
        out.push(s.to_string());
    }
    Ok(Some(out))
}

fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(|v| v.as_str())
}

fn hash_param(params: &Value) -> Option<String> {
    str_param(params, "content_hash")
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn id_param(params: &Value) -> Result<u32, String> {
    params
        .get("id")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .ok_or_else(|| "missing required field: id".to_string())
}

/// Render one issue as a single line, GitHub-ish.
fn line(i: &IssueView) -> String {
    let lock = if i.assigned_to.as_ref().is_some_and(|a| a.alive) {
        "▣ "
    } else {
        ""
    };
    let milestone = i
        .milestone
        .as_deref()
        .map(|m| format!(" [{m}]"))
        .unwrap_or_default();
    let labels = if i.labels.is_empty() {
        String::new()
    } else {
        format!(" ({})", i.labels.join(", "))
    };
    format!(
        "#{} {}{} · {} · {}{}{}",
        i.number, lock, i.title, i.status, i.priority, milestone, labels
    )
}

/// Render one issue in full, including the body and the hash to reuse.
fn full(i: &IssueView) -> String {
    let mut out = line(i);
    if let Some(a) = &i.assigned_to {
        out.push_str(&format!(
            "\nassigned to: {} (since {}, {})",
            a.session,
            a.acquired_at,
            if a.alive { "live" } else { "stale" }
        ));
    }
    if let Some(h) = &i.content_hash {
        out.push_str(&format!("\ncontent_hash: {h}"));
    }
    if let Some(body) = &i.body
        && !body.trim().is_empty()
    {
        out.push_str("\n\n");
        out.push_str(body);
    }
    out
}

#[async_trait]
impl AgentTool for IssueTool {
    fn name(&self) -> &str {
        "issue"
    }

    fn label(&self) -> &str {
        "Issue"
    }

    fn essential(&self) -> bool {
        false
    }

    fn description(&self) -> &str {
        "Manage this project's issues and milestones. Always `list` first to \
         see what exists and avoid duplicates. Before editing an issue, call \
         `start` to claim it — that stops another session working the same \
         issue concurrently. Use `release` to give up a claim or `close` to \
         finish. For `update`, every field is optional: omit to keep, provide \
         to replace; `labels: []` clears all labels. Prefer the dedicated \
         `close`/`reopen`/`start`/`release` actions over `update {status}`. \
         Milestones group issues: `milestone_create` defines one, \
         `milestone_set` files an issue under it (or clears it with a null \
         slug). Concurrent edits are auto-reconciled, so a stale \
         `content_hash` from an earlier `read` still succeeds. Issues belong \
         to the selected project; there are none without one."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "list", "read", "create", "update", "reopen", "start", "release", "close",
                        "milestone_list", "milestone_create", "milestone_set"
                    ],
                    "description": "Issue or milestone operation."
                },
                "id": {"type": "integer", "description": "Issue number (read/update/reopen/start/release/close/milestone_set)."},
                "title": {"type": "string", "description": "create: required. update: replaces the title. milestone_create: required. Max 512 chars."},
                "body": {"type": "string", "description": "create: optional. update: replaces the body. Max 256 KiB."},
                "priority": {"type": "string", "enum": ["low", "medium", "high", "critical"], "description": "create/update: new priority. list: filter to this priority."},
                "labels": {"type": "array", "items": {"type": "string"}, "description": "create/update: REPLACES labels entirely. Omit to keep; pass [] to clear. Max 32 labels, 64 chars each."},
                "status": {"type": "string", "enum": ["open", "closed"], "description": "list: filter by status. update: new status (prefer close/reopen)."},
                "label": {"type": "string", "description": "list: filter to issues carrying this label."},
                "milestone": {"type": "string", "description": "list: filter to this milestone. create: file the new issue under it. milestone_set: target slug; null or omitted clears membership."},
                "description": {"type": "string", "description": "milestone_create: optional longer description."},
                "due": {"type": "string", "description": "milestone_create: optional target date, YYYY-MM-DD."},
                "text": {"type": "string", "description": "list: case-insensitive substring filter on the title."},
                "content_hash": {"type": "string", "description": "Hash from the last `read`. ADVISORY: the tool re-reads and retries on conflict, so a stale hash still succeeds."}
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        params: Value,
        _signal: Option<tokio::sync::oneshot::Receiver<()>>,
        ctx: &ToolContext,
    ) -> Result<AgentToolResult, String> {
        let Some(action) = str_param(&params, "action").map(str::to_string) else {
            return Ok(AgentToolResult::error("missing required field: action"));
        };
        let project = match self.project() {
            Ok(p) => p,
            Err(e) => return Ok(AgentToolResult::error(e)),
        };

        // `ToolContext.session_id` is the ownership identity's basis; the
        // runtime sets it from the chat session. An empty value would make
        // every claim instantly reclaimable, so refuse rather than write one.
        let session = ctx.session_id.clone().unwrap_or_default();
        if session.is_empty() && is_owning_action(&action) {
            return Ok(AgentToolResult::error(
                "no session identity for this run; cannot take or release an issue claim",
            ));
        }

        match self.dispatch(&action, &params, &session, project).await {
            Ok((text, payload)) => {
                if let Some(payload) = payload {
                    self.bus.insert(tool_call_id, payload);
                }
                Ok(AgentToolResult::success(text))
            }
            Err(e) => Ok(AgentToolResult::error(e)),
        }
    }
}

/// Actions that write an ownership record and therefore need a real identity.
fn is_owning_action(action: &str) -> bool {
    matches!(action, "start" | "release" | "close" | "update")
}

impl IssueTool {
    async fn dispatch(
        &self,
        action: &str,
        params: &Value,
        session: &str,
        project: ProjectId,
    ) -> Result<(String, Option<Value>), String> {
        match action {
            "list" => {
                let query = IssueQuery {
                    status: status_of(str_param(params, "status"))?,
                    priority: match str_param(params, "priority") {
                        Some(p) => Some(priority_of(Some(p))?),
                        None => None,
                    },
                    label: str_param(params, "label").map(String::from),
                    milestone: str_param(params, "milestone").map(String::from),
                    text: str_param(params, "text").map(String::from),
                };
                let issues = self.issues.list(project, &query).map_err(err)?;
                let text = if issues.is_empty() {
                    "(no issues)".to_string()
                } else {
                    issues.iter().map(line).collect::<Vec<_>>().join("\n")
                };
                Ok((text, Some(json!({ "kind": "list", "issues": issues }))))
            }
            "read" => {
                let issue = self.issues.read(project, id_param(params)?).map_err(err)?;
                Ok((
                    full(&issue),
                    Some(json!({ "kind": "issue", "issue": issue })),
                ))
            }
            "create" => {
                let title = str_param(params, "title")
                    .ok_or_else(|| "missing required field: title".to_string())?;
                if title.chars().count() > MAX_TITLE_LEN {
                    return Err(format!("title too long (max {MAX_TITLE_LEN} chars)"));
                }
                let body = str_param(params, "body").unwrap_or_default();
                if body.len() > MAX_BODY_BYTES {
                    return Err(format!("body too large (max {MAX_BODY_BYTES} bytes)"));
                }
                let new = NewIssue {
                    title: title.to_string(),
                    body: body.to_string(),
                    priority: priority_of(str_param(params, "priority"))?,
                    labels: labels_of(params.get("labels"))?.unwrap_or_default(),
                    milestone: str_param(params, "milestone").map(String::from),
                    session: Some(session.to_string()).filter(|s| !s.is_empty()),
                };
                let issue = self.issues.create(project, new).map_err(err)?;
                Ok((
                    format!("created {}", line(&issue)),
                    Some(json!({ "kind": "issue", "issue": issue })),
                ))
            }
            "update" => {
                let id = id_param(params)?;
                let title = str_param(params, "title").map(String::from);
                if let Some(t) = &title
                    && t.chars().count() > MAX_TITLE_LEN
                {
                    return Err(format!("title too long (max {MAX_TITLE_LEN} chars)"));
                }
                let body = str_param(params, "body").map(String::from);
                if let Some(b) = &body
                    && b.len() > MAX_BODY_BYTES
                {
                    return Err(format!("body too large (max {MAX_BODY_BYTES} bytes)"));
                }
                let patch = IssuePatch {
                    title,
                    body,
                    status: status_of(str_param(params, "status"))?,
                    priority: match str_param(params, "priority") {
                        Some(p) => Some(priority_of(Some(p))?),
                        None => None,
                    },
                    labels: labels_of(params.get("labels"))?,
                };
                let issue = self
                    .retrying(project, id, params, |hash| {
                        let patch = patch.clone();
                        async move {
                            self.issues
                                .patch(project, id, patch, Some(session), hash)
                                .await
                        }
                    })
                    .await?;
                Ok((
                    format!("updated {}", line(&issue)),
                    Some(json!({ "kind": "issue", "issue": issue })),
                ))
            }
            "start" => {
                let id = id_param(params)?;
                let issue = self
                    .retrying(project, id, params, |hash| async move {
                        self.issues.start(project, id, session, hash).await
                    })
                    .await?;
                Ok((
                    format!("claimed {}", line(&issue)),
                    Some(json!({ "kind": "issue", "issue": issue })),
                ))
            }
            "release" => {
                let id = id_param(params)?;
                let issue = self
                    .retrying(project, id, params, |hash| async move {
                        self.issues.release(project, id, session, hash).await
                    })
                    .await?;
                Ok((
                    format!("released {}", line(&issue)),
                    Some(json!({ "kind": "issue", "issue": issue })),
                ))
            }
            "close" => {
                let id = id_param(params)?;
                let issue = self
                    .retrying(project, id, params, |hash| async move {
                        self.issues.close(project, id, session, hash).await
                    })
                    .await?;
                Ok((
                    format!("closed {}", line(&issue)),
                    Some(json!({ "kind": "issue", "issue": issue })),
                ))
            }
            "reopen" => {
                let id = id_param(params)?;
                let issue = self
                    .retrying(project, id, params, |hash| async move {
                        self.issues.reopen(project, id, hash).await
                    })
                    .await?;
                Ok((
                    format!("reopened {}", line(&issue)),
                    Some(json!({ "kind": "issue", "issue": issue })),
                ))
            }
            "milestone_list" => {
                let milestones = self.issues.list_milestones(project).map_err(err)?;
                let text = if milestones.is_empty() {
                    "(no milestones)".to_string()
                } else {
                    milestones
                        .iter()
                        .map(|m| {
                            format!(
                                "{} — {} · {} · {}/{} ({}%)",
                                m.slug, m.title, m.status, m.closed, m.total, m.percent
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                Ok((
                    text,
                    Some(json!({ "kind": "milestones", "milestones": milestones })),
                ))
            }
            "milestone_create" => {
                let title = str_param(params, "title")
                    .ok_or_else(|| "missing required field: title".to_string())?;
                let due = match str_param(params, "due") {
                    Some(d) => Some(
                        d.parse::<chrono::NaiveDate>()
                            .map_err(|_| format!("invalid due date (expected YYYY-MM-DD): {d}"))?,
                    ),
                    None => None,
                };
                let m = self
                    .issues
                    .create_milestone(
                        project,
                        title.to_string(),
                        str_param(params, "description").map(String::from),
                        due,
                        str_param(params, "milestone").map(String::from),
                    )
                    .map_err(err)?;
                Ok((
                    format!("created milestone {} — {}", m.slug, m.title),
                    Some(json!({ "kind": "milestone", "milestone": m })),
                ))
            }
            "milestone_set" => {
                let id = id_param(params)?;
                let slug = str_param(params, "milestone");
                let issue = self
                    .issues
                    .set_milestone(project, id, slug, Some(session))
                    .await
                    .map_err(err)?;
                let text = match slug {
                    Some(s) => format!("filed #{id} under {s}"),
                    None => format!("cleared the milestone on #{id}"),
                };
                Ok((text, Some(json!({ "kind": "issue", "issue": issue }))))
            }
            other => Err(format!("unknown action: {other}")),
        }
    }

    /// Run a mutation under the SDK's CAS retry policy.
    ///
    /// The agent's `content_hash` is a fast path; on conflict `cas_retry`
    /// re-reads a fresh hash and retries, so a stale hash from an earlier
    /// `read` is advisory rather than fatal. This mirrors the engine's split:
    /// strict store, recovery in the tool.
    async fn retrying<F, Fut>(
        &self,
        project: ProjectId,
        id: u32,
        params: &Value,
        op: F,
    ) -> Result<IssueView, String>
    where
        F: FnMut(Option<String>) -> Fut,
        Fut: std::future::Future<Output = Result<IssueView, IssueError>> + Send,
    {
        let store = self.issues.store(project).map_err(|e| e.to_string())?;
        cas_retry(&store, id, hash_param(params), op)
            .await
            .map_err(err)
    }
}

/// Render a store error for the model, keeping the actionable detail.
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owning_actions_require_an_identity() {
        for a in ["start", "release", "close", "update"] {
            assert!(is_owning_action(a), "{a} writes ownership");
        }
        for a in ["list", "read", "create", "reopen", "milestone_list"] {
            assert!(!is_owning_action(a), "{a} does not write ownership");
        }
    }

    #[test]
    fn priority_and_status_parsing_rejects_junk() {
        assert_eq!(priority_of(None).unwrap(), Priority::Medium);
        assert_eq!(priority_of(Some("critical")).unwrap(), Priority::Critical);
        assert!(priority_of(Some("urgent")).is_err());

        assert_eq!(status_of(None).unwrap(), None);
        assert_eq!(status_of(Some("closed")).unwrap(), Some(Status::Closed));
        assert!(status_of(Some("wontfix")).is_err());
    }

    #[test]
    fn labels_distinguish_absent_from_cleared() {
        // `IssuePatch` semantics: omitted keeps, `[]` clears. The two must not
        // collapse to the same value or labels could never be cleared.
        assert_eq!(labels_of(None).unwrap(), None);
        assert_eq!(labels_of(Some(&json!([]))).unwrap(), Some(vec![]));
        assert_eq!(
            labels_of(Some(&json!(["bug"]))).unwrap(),
            Some(vec!["bug".to_string()])
        );
    }

    #[test]
    fn label_limits_are_enforced() {
        let many: Vec<Value> = (0..MAX_LABELS + 1).map(|i| json!(i.to_string())).collect();
        assert!(labels_of(Some(&Value::Array(many))).is_err());

        let long = json!(["x".repeat(MAX_LABEL_LEN + 1)]);
        assert!(labels_of(Some(&long)).is_err());
    }

    #[test]
    fn hash_param_treats_empty_as_absent() {
        assert_eq!(hash_param(&json!({"content_hash": ""})), None);
        assert_eq!(hash_param(&json!({})), None);
        assert_eq!(
            hash_param(&json!({"content_hash": "abc"})),
            Some("abc".to_string())
        );
    }

    #[test]
    fn line_marks_only_live_claims() {
        let mut view = IssueView {
            number: 7,
            title: "Fix login".into(),
            status: "open".into(),
            priority: "high".into(),
            labels: vec!["auth".into()],
            milestone: Some("v1".into()),
            assignee: None,
            created_at: String::new(),
            updated_at: String::new(),
            closed_at: None,
            assigned_to: None,
            body: None,
            content_hash: None,
        };
        assert_eq!(line(&view), "#7 Fix login · open · high [v1] (auth)");

        view.assigned_to = Some(crate::kernel_handle::issue_api::AssignmentInfo {
            session: "oxios-1-s".into(),
            acquired_at: String::new(),
            alive: false,
        });
        assert!(
            !line(&view).contains('▣'),
            "a stale claim must not read as locked"
        );

        if let Some(a) = view.assigned_to.as_mut() {
            a.alive = true;
        }
        assert!(line(&view).contains('▣'), "a live claim reads as locked");
    }

    #[tokio::test]
    async fn every_action_reports_a_missing_project() {
        let tool = IssueTool::new(
            Arc::new(IssueApi::new()),
            Arc::new(StructuredResultBus::new()),
            None,
        );
        let out = tool
            .execute(
                "call-1",
                json!({"action": "list"}),
                None,
                &ToolContext::default(),
            )
            .await
            .unwrap();
        assert!(!out.success, "expected an error result");
        assert!(
            out.output.contains("project"),
            "the message must name the cause: {}",
            out.output
        );
    }
}
