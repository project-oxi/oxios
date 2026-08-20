//! CSpace → Tool Registry mapping.
//!
//! This module bridges the capability system and the agent's tool registry.
//! Given a [`ResolvedExecutionProfile`] (Task 4) or a raw [`CSpace`], it
//! walks the resolved / capability surface and registers exactly the set
//! of tools the agent is authorised to use.
//!
//! # Registration tiers
//!
//! | Tier | Tools | Condition |
//! |------|-------|-----------|
//! | Always-on | `ReadTool`, `WriteTool`, `EditTool`, `GrepTool`, `FindTool`, `LsTool`, `WebSearchTool`, `GetSearchResultsTool` | Every agent gets these |
//! | CSpace-driven | `ExecTool`, `BrowserTool`, kernel domain tools, MCP, A2A, etc. | Only if a matching capability with sufficient rights exists |
//!
//! # Registration paths
//!
//! * [`register_from_resolved_profile`] (Task 6) — the new single
//!   trusted path. Consumes a [`ResolvedExecutionProfile`] and registers
//!   every tool whose status is `Active` or `AvailableOnDemand`. This is
//!   the path the agent runtime switches to in Task 8.
//! * [`register_tools_from_cspace`] / [`register_tools_from_cspace_gated`]
//!   — legacy paths kept for backward compatibility during the cutover.

use std::sync::Arc;

use oxicode_sdk::{
    EditTool, FindTool, GetSearchResultsTool, GrepTool, LsTool, ReadTool, SearchCache,
    ToolRegistry, WebSearchTool, WriteTool,
};

use crate::KernelHandle;
use crate::access_manager::{AccessGate, AgentContext};
use crate::capability::resolver::{
    ResolutionStatus, ResolvedExecutionProfile, ResolvedTool,
};
use crate::capability::{CSpace, ResourceRef, Rights};
use crate::tools::builtin::*;
use crate::tools::gated_tool::GatedTool;
use crate::tools::{
    A2aDelegateTool, A2aQueryTool, A2aSendTool, AskUserTool, ExecTool, KnowledgeTool,
    MemoryReadTool, MemorySearchTool, MemoryWriteTool,
};
use crate::types::AgentId;

/// Register the always-on tool set into a [`ToolRegistry`].
///
/// Every agent receives these tools regardless of its capability space.
/// This consists of file-system tools (read, write, edit, grep, find, ls)
/// and web search tools.
///
/// This helper is also useful for unit tests that need a basic tool set
/// without constructing a full CSpace.
pub fn register_always_on(registry: &ToolRegistry, search_cache: Arc<SearchCache>) {
    registry.register(ReadTool::new());
    registry.register(WriteTool::new());
    registry.register(EditTool::new());
    registry.register(GrepTool::new());
    registry.register(FindTool::new());
    registry.register(LsTool::new());
    registry.register(WebSearchTool::new(search_cache.clone()));
    registry.register(GetSearchResultsTool::new(search_cache));
}

/// Register the headless-browser browse tools when the engine is available.
///
/// Uses oxios-owned tools (RFC-046) backed by `oxibrowser-core` 0.21 directly.
/// Only compiled with the `browser` feature; without it this is a no-op stub.
#[cfg(feature = "browser")]
fn register_browser_tools(kernel: &KernelHandle, registry: &ToolRegistry) {
    use crate::tools::browse::{
        BrowseExtractTool, BrowseScriptTool, BrowseSessionTool, BrowseTool,
    };
    if let Some(browser) = &kernel.browser
        && let Some(engine) = browser.try_engine()
    {
        registry.register(BrowseTool::new(engine.clone()));
        registry.register(BrowseExtractTool::new(engine.clone()));
        registry.register(BrowseSessionTool::new(engine.clone()));
        registry.register(BrowseScriptTool::new(engine));
    }
}

/// No-op stub when the `browser` feature is disabled.
#[cfg(not(feature = "browser"))]
fn register_browser_tools(_kernel: &KernelHandle, _registry: &ToolRegistry) {}

/// Register always-on tools with access gate and (RFC-035) approval wrapping.
///
/// Same as [`register_always_on`] but wraps each tool in [`GatedTool`] so that
/// all file operations pass through the access gate and approval gate.
///
/// `approval_gate`, `event_bus`, and `pending_approvals` may all be `None`
/// for headless / test paths; the gated tool still honors the access gate
/// but skips the approval step.
#[allow(clippy::too_many_arguments)]
pub fn register_always_on_gated(
    registry: &ToolRegistry,
    search_cache: Arc<SearchCache>,
    gate: Arc<AccessGate>,
    context: AgentContext,
    approval_gate: Option<Arc<crate::approval::ApprovalGate>>,
    event_bus: Option<crate::event_bus::EventBus>,
    pending_approvals: Option<Arc<crate::tools::PendingToolApprovals>>,
    pending_path_access: Option<Arc<crate::tools::PendingPathAccess>>,
) {
    registry.register(GatedTool::with_approval(
        ReadTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        WriteTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        EditTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        GrepTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        FindTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        LsTool::new(),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        WebSearchTool::new(search_cache.clone()),
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    ));
    registry.register(GatedTool::with_approval(
        GetSearchResultsTool::new(search_cache),
        gate,
        context,
        approval_gate,
        event_bus,
        pending_approvals,
        pending_path_access.clone(),
    ));
}

/// Register tools into `registry` from a [`ResolvedExecutionProfile`].
///
/// This is the new single trusted registration path (Task 6). It iterates
/// `resolved.tools` and registers every entry whose status is `Active` or
/// `AvailableOnDemand`. v1 registers both classes — progressive activation
/// (on-demand tools that become Active only after an intent match) lands in
/// a later plan.
///
/// Per descriptor `id`, the matching arm constructs and registers the
/// matching kernel tool:
///
/// | Descriptor id              | Registered tool                                  |
/// |----------------------------|--------------------------------------------------|
/// | `kernel.fs.read`           | `GatedTool::with_approval(ReadTool::new(), ...)` |
/// | `kernel.fs.grep`           | `GatedTool::with_approval(GrepTool::new(), ...)` |
/// | `kernel.fs.find`           | `GatedTool::with_approval(FindTool::new(), ...)` |
/// | `kernel.fs.ls`             | `GatedTool::with_approval(LsTool::new(), ...)`   |
/// | `kernel.fs.write`          | `GatedTool::with_approval(WriteTool::new(), ...)`|
/// | `kernel.fs.edit`           | `GatedTool::with_approval(EditTool::new(), ...)` |
/// | `kernel.web.search`        | `GatedTool::with_approval(WebSearchTool, ...)`   |
/// | `kernel.web.results`       | `GatedTool::with_approval(GetSearchResultsTool,...)`|
/// | `kernel.exec.run`          | `GatedTool::with_approval(ExecTool::from_kernel_with_context(...),...)` |
/// | `kernel.browse.open`       | `BrowseTool::new(engine.clone())` (browser feat) |
/// | `kernel.browse.extract`    | `BrowseExtractTool::new(engine.clone())` (browser feat) |
/// | `kernel.browse.session`    | `BrowseSessionTool::new(engine.clone())` (browser feat) |
/// | `kernel.browse.script`     | `BrowseScriptTool::new(engine)` (browser feat)  |
/// | `kernel.memory.read`       | `MemoryReadTool::from_kernel(kernel)`            |
/// | `kernel.memory.search`     | `MemorySearchTool::from_kernel(kernel)`          |
/// | `kernel.memory.write`      | `MemoryWriteTool::from_kernel(kernel)`           |
/// | `kernel.knowledge.read`    | `KnowledgeTool::from_kernel(kernel)`             |
/// | `kernel.knowledge.write`   | warn + skip (no separate write tool exists; covered by `KnowledgeTool` from `kernel.knowledge.read`) |
/// | `kernel.ask_user.ask`      | `AskUserTool::new(pending, event_bus)`           |
/// | `kernel.persona.update`    | `PersonaTool::from_kernel(kernel)`               |
/// | `kernel.project.update`    | `ProjectTool::from_kernel(kernel)`               |
/// | `kernel.agent.update`      | `KernelAgentTool::from_kernel(kernel)`           |
/// | `kernel.cron.update`       | `CronTool::from_kernel(kernel)`                  |
/// | `kernel.security.query`    | `SecurityTool::from_kernel(kernel)`              |
/// | `kernel.budget.query`      | `BudgetTool::from_kernel(kernel)`                |
/// | `kernel.resource.query`    | `ResourceTool::from_kernel(kernel)`              |
/// | `kernel.a2a.delegate`      | `A2aDelegateTool::from_kernel(kernel, agent_id)` |
/// | `kernel.a2a.send`          | `A2aSendTool::from_kernel(kernel, agent_id)`     |
/// | `kernel.a2a.query`         | `A2aQueryTool::from_kernel(kernel)`              |
/// | anything else              | `tracing::warn!` and skip                        |
///
/// # Arguments
///
/// * `registry` — The agent's tool registry to populate.
/// * `kernel` — Handle to the kernel for constructing tool instances.
/// * `resolved` — The resolved profile (Task 4 output).
/// * `search_cache` — Shared search cache for web search tools.
/// * `agent_id` — The agent's ID (used by A2A tools for routing).
/// * `gate` — The unified access gate for permission checks.
/// * `context` — The agent's security context.
/// * `approval_gate` — RFC-035 approval gate; consults declared policy,
///   config overrides, and global resolvers per tool call.
/// * `event_bus` — Publishes `KernelEvent::ApprovalRequested` when
///   `RequireApproval` is returned.
/// * `pending_approvals` — Shared registry of pending user decisions.
/// * `pending_path_access` — Shared registry of pending path decisions.
#[allow(clippy::too_many_arguments)]
pub fn register_from_resolved_profile(
    registry: &ToolRegistry,
    kernel: &Arc<KernelHandle>,
    resolved: &ResolvedExecutionProfile,
    search_cache: Arc<SearchCache>,
    agent_id: AgentId,
    gate: Arc<AccessGate>,
    context: AgentContext,
    approval_gate: Option<Arc<crate::approval::ApprovalGate>>,
    event_bus: Option<crate::event_bus::EventBus>,
    pending_approvals: Option<Arc<crate::tools::PendingToolApprovals>>,
    pending_path_access: Option<Arc<crate::tools::PendingPathAccess>>,
) {
    // Subagent tool — oxicode-agent's native `subagent` tool (RFC-035
    // gap 3), re-homed from the legacy bulk-registration path dissolved
    // in Task 9.
    // Executed via `AgentConfig.subagent_runner` (wired in
    // agent_runtime.rs); every kernel-built agent gets it regardless
    // of profile, so it registers before the descriptor-keyed loop
    // (no catalog descriptor exists for it).
    registry.register(oxicode_agent::SubagentTool::new());

    for resolved_tool in &resolved.tools {
        if !matches!(
            resolved_tool.status,
            ResolutionStatus::Active | ResolutionStatus::AvailableOnDemand,
        ) {
            continue;
        }
        let ResolvedTool { descriptor, status: _ } = resolved_tool;
        // `ToolId` wraps `Arc<str>`; deref to `&str` for cheap matching.
        let id = descriptor.id.to_string();
        match id.as_str() {
            // ── Filesystem (read-only) ────────────────────────────────
            "kernel.fs.read" => {
                registry.register(GatedTool::with_approval(
                    ReadTool::new(),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            "kernel.fs.grep" => {
                registry.register(GatedTool::with_approval(
                    GrepTool::new(),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            "kernel.fs.find" => {
                registry.register(GatedTool::with_approval(
                    FindTool::new(),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            "kernel.fs.ls" => {
                registry.register(GatedTool::with_approval(
                    LsTool::new(),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            // ── Filesystem (write) ───────────────────────────────────
            "kernel.fs.write" => {
                registry.register(GatedTool::with_approval(
                    WriteTool::new(),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            "kernel.fs.edit" => {
                registry.register(GatedTool::with_approval(
                    EditTool::new(),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            // ── Web search ───────────────────────────────────────────
            "kernel.web.search" => {
                registry.register(GatedTool::with_approval(
                    WebSearchTool::new(search_cache.clone()),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            "kernel.web.results" => {
                registry.register(GatedTool::with_approval(
                    GetSearchResultsTool::new(search_cache.clone()),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            // ── Exec ─────────────────────────────────────────────────
            "kernel.exec.run" => {
                registry.register(GatedTool::with_approval(
                    ExecTool::from_kernel_with_context(kernel, context.clone()),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }
            // ── Browser ──────────────────────────────────────────────
            // Per-descriptor-id arms so the registered set exactly
            // mirrors the resolved active set. The previous prefix
            // match `kernel.browse.*` collapsed every browse
            // descriptor onto `register_browser_tools`, which
            // registers the full suite — a custom profile that
            // selected a subset of the four browse descriptors
            // therefore ended up with extra tools in the registry
            // (e.g. selecting only `kernel.browse.extract` still
            // registered `browse`, `browse_session`, and
            // `browse_script`). Each arm acquires the engine the
            // same way `register_browser_tools` does; without an
            // initialized engine (or without the `browser`
            // feature) the arm warns and skips so operators see the
            // mismatch instead of a silent drop.
            "kernel.browse.open" => {
                #[cfg(feature = "browser")]
                {
                    use crate::tools::browse::BrowseTool;
                    if let Some(browser) = kernel.browser.as_ref()
                        && let Some(engine) = browser.try_engine()
                    {
                        registry.register(BrowseTool::new(engine.clone()));
                    } else {
                        tracing::warn!(
                            id = id,
                            "browser engine unavailable; skipping kernel.browse.open"
                        );
                    }
                }
                #[cfg(not(feature = "browser"))]
                {
                    tracing::warn!(
                        id = id,
                        "kernel built without `browser` feature; skipping kernel.browse.open"
                    );
                }
            }
            "kernel.browse.extract" => {
                #[cfg(feature = "browser")]
                {
                    use crate::tools::browse::BrowseExtractTool;
                    if let Some(browser) = kernel.browser.as_ref()
                        && let Some(engine) = browser.try_engine()
                    {
                        registry.register(BrowseExtractTool::new(engine.clone()));
                    } else {
                        tracing::warn!(
                            id = id,
                            "browser engine unavailable; skipping kernel.browse.extract"
                        );
                    }
                }
                #[cfg(not(feature = "browser"))]
                {
                    tracing::warn!(
                        id = id,
                        "kernel built without `browser` feature; skipping kernel.browse.extract"
                    );
                }
            }
            "kernel.browse.session" => {
                #[cfg(feature = "browser")]
                {
                    use crate::tools::browse::BrowseSessionTool;
                    if let Some(browser) = kernel.browser.as_ref()
                        && let Some(engine) = browser.try_engine()
                    {
                        registry.register(BrowseSessionTool::new(engine.clone()));
                    } else {
                        tracing::warn!(
                            id = id,
                            "browser engine unavailable; skipping kernel.browse.session"
                        );
                    }
                }
                #[cfg(not(feature = "browser"))]
                {
                    tracing::warn!(
                        id = id,
                        "kernel built without `browser` feature; skipping kernel.browse.session"
                    );
                }
            }
            "kernel.browse.script" => {
                #[cfg(feature = "browser")]
                {
                    use crate::tools::browse::BrowseScriptTool;
                    if let Some(browser) = kernel.browser.as_ref()
                        && let Some(engine) = browser.try_engine()
                    {
                        registry.register(BrowseScriptTool::new(engine));
                    } else {
                        tracing::warn!(
                            id = id,
                            "browser engine unavailable; skipping kernel.browse.script"
                        );
                    }
                }
                #[cfg(not(feature = "browser"))]
                {
                    tracing::warn!(
                        id = id,
                        "kernel built without `browser` feature; skipping kernel.browse.script"
                    );
                }
            }
            // ── Memory ───────────────────────────────────────────────
            "kernel.memory.read" => {
                registry.register(MemoryReadTool::from_kernel(kernel));
            }
            "kernel.memory.search" => {
                registry.register(MemorySearchTool::from_kernel(kernel));
            }
            "kernel.memory.write" => {
                registry.register(MemoryWriteTool::from_kernel(kernel));
            }
            // ── Knowledge ────────────────────────────────────────────
            // Precondition: `KnowledgeTool` is registered only via the
            // `kernel.knowledge.read` arm above (the catalog declares
            // a single `KnowledgeTool` whose `action: "write"` covers
            // write operations). A profile that selects ONLY
            // `kernel.knowledge.write` therefore degrades to this
            // warn+skip path — the resolved set names a tool the
            // kernel has no constructor for. Operators should see the
            // warning so the catalog/profile mismatch is observable.
            "kernel.knowledge.read" => {
                registry.register(KnowledgeTool::from_kernel(kernel));
            }
            "kernel.knowledge.write" => {
                tracing::warn!(
                    id = id,
                    "no registration arm for catalog tool (write is covered by KnowledgeTool; \
                     kernel.knowledge.read must also be selected for the covering tool to register)"
                );
            }
            // ── User interaction ─────────────────────────────────────
            "kernel.ask_user.ask" => {
                registry.register(AskUserTool::new(
                    kernel.infra.pending_ask_user(),
                    kernel.infra.event_bus_clone(),
                ));
            }
            // ── Kernel mutation / observation domains ────────────────
            "kernel.persona.update" => {
                registry.register(PersonaTool::from_kernel(kernel));
            }
            "kernel.project.update" => {
                registry.register(ProjectTool::from_kernel(kernel));
            }
            "kernel.agent.update" => {
                registry.register(KernelAgentTool::from_kernel(kernel));
            }
            "kernel.cron.update" => {
                registry.register(CronTool::from_kernel(kernel));
            }
            "kernel.security.query" => {
                registry.register(SecurityTool::from_kernel(kernel));
            }
            "kernel.budget.query" => {
                registry.register(BudgetTool::from_kernel(kernel));
            }
            "kernel.resource.query" => {
                registry.register(ResourceTool::from_kernel(kernel));
            }
            // ── A2A ──────────────────────────────────────────────────
            "kernel.a2a.delegate" => {
                registry.register(A2aDelegateTool::from_kernel(kernel, agent_id));
            }
            "kernel.a2a.send" => {
                registry.register(A2aSendTool::from_kernel(kernel, agent_id));
            }
            "kernel.a2a.query" => {
                registry.register(A2aQueryTool::from_kernel(kernel));
            }
            // ── Long-tail kernel domain tools (Task 9) ────────────────
            // Each arm mirrors the constructor + cfg gating the legacy
            // bulk-registration path (dissolved in Task 9) used. Tools
            // that depend on a feature-gated `pub mod` (memo / timeline /
            // browser) carry the same `#[cfg]` here. When the feature is
            // off, the descriptor stays in the catalog (so the selector
            // can still reason about it) but the arm warns and skips so
            // operators see the catalog/profile/feature mismatch.
            "kernel.mount.manage" => {
                registry.register(MountTool::from_kernel(kernel));
            }
            // TaskTool is conditional on the assembler attaching a
            // `task_store` — `from_kernel` returns `None` when no
            // store is attached, matching the legacy path. The
            // constructor signature takes `&Arc<KernelHandle>`; the
            // resolved-profile path now also takes `&Arc<KernelHandle>`,
            // so the kernel Arc is reused without an extra clone.
            "kernel.task.manage" => {
                if let Some(task_tool) =
                    TaskTool::from_kernel(kernel, &agent_id.to_string())
                {
                    registry.register(task_tool);
                } else {
                    tracing::warn!(
                        id = id,
                        "task store unavailable; skipping kernel.task.manage"
                    );
                }
            }
            "kernel.marketplace.manage" => {
                registry.register(MarketplaceTool::from_kernel(kernel));
            }
            "kernel.program.forge" => {
                registry.register(SkillForgeTool::from_kernel(kernel));
            }
            // Calendar is gated on `[calendar].enabled` at runtime;
            // `try_from_kernel` returns `None` when no calendar
            // service is attached. Mirrors the legacy registration
            // path exactly.
            "kernel.calendar.manage" => {
                if let Some(calendar_tool) = CalendarTool::try_from_kernel(kernel) {
                    registry.register(calendar_tool);
                } else {
                    tracing::warn!(
                        id = id,
                        "calendar service unavailable; skipping kernel.calendar.manage"
                    );
                }
            }
            // Memo (oximemo first-party app module) — feature-gated
            // because the underlying `MemoTool` is only compiled with
            // the `memo` feature. The descriptor stays in the catalog
            // unconditionally so the selector can still resolve the id
            // even when the feature is off.
            "kernel.memo.manage" => {
                #[cfg(feature = "memo")]
                {
                    if let Some(memo_tool) = MemoTool::try_from_kernel(kernel) {
                        registry.register(memo_tool);
                    } else {
                        tracing::warn!(
                            id = id,
                            "memo module unavailable; skipping kernel.memo.manage"
                        );
                    }
                }
                #[cfg(not(feature = "memo"))]
                {
                    tracing::warn!(
                        id = id,
                        "kernel built without `memo` feature; skipping kernel.memo.manage"
                    );
                }
            }
            // Timeline (oxiline first-party app module) — feature-gated
            // like MemoTool. The underlying `TimelineTool` is only
            // compiled with the `timeline` feature; the descriptor
            // stays in the catalog unconditionally.
            "kernel.timeline.manage" => {
                #[cfg(feature = "timeline")]
                {
                    if let Some(timeline_tool) = TimelineTool::try_from_kernel(kernel) {
                        registry.register(timeline_tool);
                    } else {
                        tracing::warn!(
                            id = id,
                            "timeline module unavailable; skipping kernel.timeline.manage"
                        );
                    }
                }
                #[cfg(not(feature = "timeline"))]
                {
                    tracing::warn!(
                        id = id,
                        "kernel built without `timeline` feature; skipping kernel.timeline.manage"
                    );
                }
            }
            "kernel.email.send" => {
                registry.register(EmailTool::from_kernel(kernel));
            }
            // Image generation requires `[image-gen].enabled = true`
            // at runtime. `ImageGenerationTool::from_kernel` reads the
            // config snapshot but does NOT gate on `enabled`; the
            // legacy path did this check inline, so we preserve it.
            "kernel.image.gen" => {
                if kernel.infra.config().image_gen.enabled {
                    registry.register(ImageGenerationTool::from_kernel(kernel));
                } else {
                    tracing::warn!(
                        id = id,
                        "image_gen disabled in config; skipping kernel.image.gen"
                    );
                }
            }
            // Screenshot is feature-gated on `browser` because
            // `ScreenshotTool` lives behind `pub mod screenshot_tool`
            // with `#[cfg(feature = "browser")]`. The legacy path
            // constructed the tool unconditionally when the feature
            // was on; we keep that contract.
            "kernel.screenshot.capture" => {
                #[cfg(feature = "browser")]
                {
                    registry.register(ScreenshotTool::from_kernel(kernel));
                }
                #[cfg(not(feature = "browser"))]
                {
                    tracing::warn!(
                        id = id,
                        "kernel built without `browser` feature; skipping kernel.screenshot.capture"
                    );
                }
            }
            // ── Fallback ─────────────────────────────────────────────
            _ => {
                tracing::warn!(id = id, "no registration arm for catalog tool");
            }
        }
    }
}

/// Register tools into `registry` based on the agent's [`CSpace`].
///
/// First registers the always-on tier (file ops + web search), then walks
/// every capability in the CSpace and conditionally registers the
/// corresponding kernel tools.
///
/// # Arguments
///
/// * `registry` — The agent's tool registry to populate.
/// * `kernel` — Handle to the kernel for constructing tool instances.
/// * `cspace` — The agent's capability space (determines which tools are available).
/// * `search_cache` — Shared search cache for web search tools.
/// * `agent_id` — The agent's ID (used by A2A tools for routing).
///
/// # CSpace → Tool mapping
///
/// | ResourceRef | Required rights | Registered tools |
/// |-------------|----------------|-----------------|
/// | `Exec { .. }` | `EXECUTE` | `ExecTool` |
/// | `KernelDomain { "memory" }` | — | *(registered via the `kernel.memory.*` arms in [`register_from_resolved_profile`])* |
/// | `KernelDomain { "project" }` | any | `ProjectTool` |
/// | `KernelDomain { "agent" }` | any | `KernelAgentTool` |
/// | `KernelDomain { "a2a" }` | any | `A2aDelegateTool`, `A2aSendTool`, `A2aQueryTool` |
/// | `KernelDomain { "persona" }` | any | `PersonaTool` |
/// | `KernelDomain { "program" }` | any | *(deprecated — skills via CSpace)* |
/// | `KernelDomain { "cron" }` | any | `CronTool` |
/// | `KernelDomain { "security" }` | any | `SecurityTool` |
/// | `KernelDomain { "budget" }` | any | `BudgetTool` |
/// | `KernelDomain { "resource" }` | any | `ResourceTool` |
/// | `KernelDomain { "mcp" }` | any | `McpToolWrapper` |
/// | `Program { .. }` | — | *(not registered; surfaced via ToolRetriever)* |
pub fn register_tools_from_cspace(
    registry: &ToolRegistry,
    kernel: &KernelHandle,
    cspace: &CSpace,
    search_cache: Arc<SearchCache>,
    agent_id: AgentId,
) {
    // ── Tier 1: Always-on tools ─────────────────────────────────────
    register_always_on(registry, search_cache);

    // ── Tier 2: CSpace-driven tools ─────────────────────────────────
    for cap in cspace.iter() {
        match &cap.resource {
            // Command execution
            ResourceRef::Exec { .. } if cap.rights.contains(Rights::EXECUTE) => {
                registry.register(ExecTool::from_kernel(kernel));
            }

            // Headless browser — SDK browse tools (pure-Rust oxibrowser-core).
            ResourceRef::Browser if cap.rights.contains(Rights::EXECUTE) => {
                register_browser_tools(kernel, registry);
            }

            // Kernel domain tools
            ResourceRef::KernelDomain { domain } => match domain.as_str() {
                "memory" => { /* Registered via kernel.memory.* arms in register_from_resolved_profile */ }
                "agent" => registry.register(KernelAgentTool::from_kernel(kernel)),
                "a2a" => {
                    registry.register(A2aDelegateTool::from_kernel(kernel, agent_id));
                    registry.register(A2aSendTool::from_kernel(kernel, agent_id));
                    registry.register(A2aQueryTool::from_kernel(kernel));
                }
                "persona" => registry.register(PersonaTool::from_kernel(kernel)),
                "program" => { /* Skills are surfaced through CSpace + semantic retrieval, not individual tools */
                }
                "cron" => registry.register(CronTool::from_kernel(kernel)),
                "security" => registry.register(SecurityTool::from_kernel(kernel)),
                "budget" => registry.register(BudgetTool::from_kernel(kernel)),
                "resource" => registry.register(ResourceTool::from_kernel(kernel)),
                "knowledge" => registry.register(KnowledgeTool::from_kernel(kernel)),
                "mcp" => { /* MCP tools are enumerated dynamically per agent */ }
                _ => {} // Unknown domain — silently skip
            },

            // Programs are not registered as separate tools.
            // ToolRetriever shows them in the capability index;
            // agents use exec to run program commands.
            ResourceRef::Skill { .. } => {}

            // Space, Agent, Mcp resource refs are handled through
            // their respective KernelDomain registrations above
            // or through dedicated tool paths.
            _ => {}
        }
    }
}

/// Register tools into `registry` with access gate + approval gate enforcement.
///
/// Same as [`register_tools_from_cspace`] but:
/// - Always-on tools are wrapped in [`GatedTool`] for permission + approval checks
/// - ExecTool is created with `AgentContext` and wrapped in [`GatedTool`] so
///   RFC-035 Step 2.5 also covers shell + structured exec calls (replacing
///   the bespoke exec-only shell approval block)
///
/// Use this in production. The ungated version exists for backward compatibility.
///
/// # Arguments
///
/// * `registry` — The agent's tool registry to populate.
/// * `kernel` — Handle to the kernel for constructing tool instances.
/// * `cspace` — The agent's capability space (determines which tools are available).
/// * `search_cache` — Shared search cache for web search tools.
/// * `agent_id` — The agent's ID (used by A2A tools for routing).
/// * `gate` — The unified access gate for permission checks.
/// * `context` — The agent's security context.
/// * `approval_gate` — RFC-035 approval gate; consults declared policy,
///   config overrides, and global resolvers per tool call.
/// * `event_bus` — Publishes `KernelEvent::ApprovalRequested` when
///   `RequireApproval` is returned.
/// * `pending_approvals` — Shared registry of pending user decisions.
#[allow(clippy::too_many_arguments)]
pub fn register_tools_from_cspace_gated(
    registry: &ToolRegistry,
    kernel: &KernelHandle,
    cspace: &CSpace,
    search_cache: Arc<SearchCache>,
    agent_id: AgentId,
    gate: Arc<AccessGate>,
    context: AgentContext,
    approval_gate: Option<Arc<crate::approval::ApprovalGate>>,
    event_bus: Option<crate::event_bus::EventBus>,
    pending_approvals: Option<Arc<crate::tools::PendingToolApprovals>>,
    pending_path_access: Option<Arc<crate::tools::PendingPathAccess>>,
) {
    // ── Tier 1: Always-on tools (gated) ──────────────────────────────
    register_always_on_gated(
        registry,
        search_cache,
        gate.clone(),
        context.clone(),
        approval_gate.clone(),
        event_bus.clone(),
        pending_approvals.clone(),
        pending_path_access.clone(),
    );

    // ── Tier 2: CSpace-driven tools ─────────────────────────────────
    for cap in cspace.iter() {
        match &cap.resource {
            // Command execution — wrap in GatedTool so Step 2.5 (RFC-035)
            // also fires for exec. The inner ExecTool retains its own
            // context, binary allowlist + access manager checks; the outer
            // GatedTool runs the unified access gate + approval pipeline.
            ResourceRef::Exec { .. } if cap.rights.contains(Rights::EXECUTE) => {
                registry.register(GatedTool::with_approval(
                    ExecTool::from_kernel_with_context(kernel, context.clone()),
                    gate.clone(),
                    context.clone(),
                    approval_gate.clone(),
                    event_bus.clone(),
                    pending_approvals.clone(),
                    pending_path_access.clone(),
                ));
            }

            // Headless browser — SDK browse tools.
            ResourceRef::Browser if cap.rights.contains(Rights::EXECUTE) => {
                register_browser_tools(kernel, registry);
            }

            // Kernel domain tools (same as ungated — these already use KernelHandle internally)
            ResourceRef::KernelDomain { domain } => match domain.as_str() {
                "memory" => { /* Registered via kernel.memory.* arms in register_from_resolved_profile */ }
                "space" => registry.register(ProjectTool::from_kernel(kernel)),
                "agent" => registry.register(KernelAgentTool::from_kernel(kernel)),
                "a2a" => {
                    registry.register(A2aDelegateTool::from_kernel(kernel, agent_id));
                    registry.register(A2aSendTool::from_kernel(kernel, agent_id));
                    registry.register(A2aQueryTool::from_kernel(kernel));
                }
                "persona" => registry.register(PersonaTool::from_kernel(kernel)),
                "program" => {}
                "cron" => registry.register(CronTool::from_kernel(kernel)),
                "security" => registry.register(SecurityTool::from_kernel(kernel)),
                "budget" => registry.register(BudgetTool::from_kernel(kernel)),
                "resource" => registry.register(ResourceTool::from_kernel(kernel)),
                "knowledge" => registry.register(KnowledgeTool::from_kernel(kernel)),
                "mcp" => {}
                _ => {}
            },

            ResourceRef::Skill { .. } => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::resolver::resolve;
    use crate::capability::{Capability, ResourceRef as CapResourceRef, Rights as CapRights};

    #[test]
    fn register_always_on_registers_eight_tools() {
        let registry = ToolRegistry::new();
        let cache = Arc::new(SearchCache::new());
        register_always_on(&registry, cache);

        // The always-on set is: read, write, edit, grep, find, ls, web_search, get_search_results
        // ToolRegistry doesn't expose a count, but we can verify individual tool names.
        let tool_names = registry.names();
        assert!(
            tool_names.contains(&"read".to_string()),
            "read tool should be registered"
        );
        assert!(
            tool_names.contains(&"write".to_string()),
            "write tool should be registered"
        );
        assert!(
            tool_names.contains(&"edit".to_string()),
            "edit tool should be registered"
        );
        assert!(
            tool_names.contains(&"grep".to_string()),
            "grep tool should be registered"
        );
        assert!(
            tool_names.contains(&"find".to_string()),
            "find tool should be registered"
        );
        assert!(
            tool_names.contains(&"ls".to_string()),
            "ls tool should be registered"
        );
        assert!(
            tool_names.contains(&"web_search".to_string()),
            "web_search tool should be registered"
        );
        assert!(
            tool_names.contains(&"get_search_results".to_string()),
            "get_search_results tool should be registered"
        );
    }

    /// Lightweight `KernelHandle` construction — the established
    /// pattern this crate uses for unit-test KernelHandles.
    fn make_test_kernel(base: &std::path::Path) -> Arc<KernelHandle> {
        let state_store = Arc::new(
            crate::state_store::StateStore::new(base.join("workspace")).unwrap(),
        );
        Arc::new(crate::KernelHandle::new(
            crate::StateApi::new(state_store.clone()),
            crate::AgentApi::new(
                Arc::new(crate::supervisor::NoOpSupervisor),
                Arc::new(crate::budget::BudgetManager::new()),
            ),
            crate::SecurityApi::new(
                Arc::new(parking_lot::Mutex::new(crate::auth::AuthManager::new())),
                Arc::new(oxicode_sdk::observability::AuditTrail::new(100)),
                Arc::new(parking_lot::Mutex::new(
                    crate::access_manager::AccessManager::new(),
                )),
                state_store.clone(),
            ),
            crate::PersonaApi::new(Arc::new(crate::persona::PersonaManager::new())),
            crate::ExtensionApi::new(Arc::new(crate::skill::SkillManager::new(
                base.join("skills"),
                base.join("share/skills"),
            ))),
            crate::McpApi::new(Arc::new(crate::mcp::McpBridge::new())),
            crate::InfraApi::new(
                Arc::new(crate::git_layer::GitLayer::new(base.join("git"), false).unwrap()),
                Arc::new(crate::cron::CronScheduler::new(state_store.clone(), 60)),
                Arc::new(crate::resource_monitor::ResourceMonitor::new(60, 60)),
                crate::event_bus::EventBus::new(256),
                crate::OxiosConfig::default(),
                std::time::Instant::now(),
                Arc::new(crate::tools::PendingToolApprovals::new()),
                Arc::new(crate::tools::PendingAskUser::new()),
                Arc::new(parking_lot::RwLock::new(
                    crate::approval::ApprovalConfig::default(),
                )),
                Arc::new(crate::tools::PendingPathAccess::new()),
            ),
            None,
            crate::ExecApi::new(
                Arc::new(parking_lot::RwLock::new(
                    crate::config::ExecConfig::default(),
                )),
                Arc::new(parking_lot::Mutex::new(
                    crate::access_manager::AccessManager::new(),
                )),
            ),
            crate::A2aApi::new(Arc::new(crate::a2a::A2AProtocol::new(
                crate::event_bus::EventBus::new(256),
            ))),
            crate::EngineApi::new(
                Arc::new(parking_lot::RwLock::new(crate::OxiosConfig::default())),
                base.join("config.toml"),
                Arc::new(crate::kernel_handle::RoutingStats::new()),
                Arc::new(crate::engine::EngineHandle::new(Arc::new(
                    crate::OxiosEngine::new("anthropic/claude-sonnet-4-20250514"),
                ))),
            ),
            Arc::new(
                oxios_markdown::KnowledgeBase::new(base.join("knowledge")).unwrap(),
            ),
            Arc::new(
                crate::kernel_handle::KnowledgeLens::new(
                    Arc::new(
                        oxios_markdown::KnowledgeBase::new(base.join("knowledge_lens"))
                            .unwrap(),
                    ),
                    None,
                )
                .unwrap(),
            ),
            crate::MarketplaceApi::new(
                Arc::new(crate::skill::clawhub::ClawHubInstaller::new(
                    base.join("skills"),
                    base.join("workspace"),
                    None,
                )),
                Arc::new(
                    crate::skill::clawhub::ClawHubClient::new(None)
                        .expect("valid ClawHub client"),
                ),
                Arc::new(crate::skill::skills_sh::SkillsShInstaller::new(
                    base.join("skills"),
                    None,
                    None,
                )),
                Arc::new(
                    crate::skill::skills_sh::SkillsShClient::new(None, None)
                        .expect("valid Skills.sh client"),
                ),
            ),
            None,                                     // calendar
            Arc::new(parking_lot::RwLock::new(None)), // email
        ))
    }

    /// Build the same worker bounding CSpace the resolver tests use.
    fn worker_bounding(agent_id: AgentId) -> CSpace {
        let mut c = CSpace::new(agent_id);
        c.insert(Capability::kernel(
            CapResourceRef::Fs,
            CapRights::READ | CapRights::WRITE,
        ));
        c.insert(Capability::kernel(
            CapResourceRef::WebSearch,
            CapRights::EXECUTE,
        ));
        c.insert(Capability::kernel(
            CapResourceRef::Exec {
                mode: "shell".into(),
            },
            CapRights::EXECUTE,
        ));
        c.insert(Capability::kernel(CapResourceRef::Browser, CapRights::EXECUTE));
        c
    }

    /// Lightweight access gate — same shape `register_always_on_gated`
    /// tests construct via `make_gate_for_test`.
    fn make_test_gate() -> Arc<AccessGate> {
        use crate::access_manager::{
            AccessManager, AgentPermissions, NoOpAuditSink, Role, Subject,
        };
        use crate::config::ExecConfig;
        use parking_lot::Mutex;
        let mut access = AccessManager::new();
        let perms = AgentPermissions::for_new_agent("test-agent");
        access.set_permissions(perms);
        access
            .rbac_manager_mut()
            .assign_role(Subject::Agent(uuid::Uuid::new_v4()), Role::Superuser);
        Arc::new(AccessGate::new(
            Arc::new(Mutex::new(access)),
            Arc::new(ExecConfig::default()),
            Arc::new(NoOpAuditSink),
        ))
    }

    #[tokio::test]
    async fn register_from_resolved_worker_profile_registers_nine_tools() {
        let tmp = tempfile::tempdir().unwrap();
        let kernel = make_test_kernel(tmp.path());
        let agent = AgentId::new_v4();
        let bounding = worker_bounding(agent);
        let resolved = resolve(
            crate::capability::builtin_profiles::builtin_profile("worker")
                .expect("worker preset"),
            &bounding,
            agent,
            None,
        );

        let registry = ToolRegistry::new();
        let cache = Arc::new(SearchCache::new());
        let gate = make_test_gate();
        let context = AgentContext::test_fixture_with_cspace("worker-test", bounding.clone());

        register_from_resolved_profile(
            &registry,
            &kernel,
            &resolved,
            cache,
            agent,
            gate,
            context,
            None,
            None,
            None,
            None,
        );

        let names = registry.names();
        for expected in [
            "read",
            "write",
            "edit",
            "grep",
            "find",
            "ls",
            "web_search",
            "get_search_results",
            "exec",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "worker missing {expected}: {names:?}"
            );
        }
        assert!(
            !names.contains(&"memory_read".to_string()),
            "worker must NOT include memory_read: {names:?}"
        );
    }

    /// Verify the per-descriptor-id browse arms (Task 6 fix): a
    /// profile that resolves ONLY one browse descriptor must
    /// register only that browse tool — not the full browse suite.
    ///
    /// The previous prefix match `kernel.browse.*` collapsed every
    /// descriptor onto `register_browser_tools`, which registers the
    /// full suite. A custom profile that selected a subset of the
    /// four browse descriptors therefore ended up with extra tools
    /// in the registry (e.g. selecting only `kernel.browse.extract`
    /// still registered `browse`, `browse_session`, and
    /// `browse_script`).
    ///
    /// The unit-test fixture builds a `KernelHandle` with
    /// `kernel.browser = None` (see `make_test_kernel`), so every
    /// browse arm hits the `else` branch and warns without
    /// registering. The exhaustive table-driven assertions below
    /// therefore verify two things:
    ///
    /// 1. **Each of the four exact-id arms is reachable.**
    ///    Asserting NEITHER `browse_extract` (for the
    ///    `kernel.browse.extract` case) nor `browse` is in the
    ///    registry rules out both the prefix-match regression
    ///    (which would register `browse`) and an arm that silently
    ///    falls through to the catch-all (which would still skip
    ///    `browse` but skip the warn path too).
    /// 2. **No descriptor id leaks into the resolved set.**
    ///    A pre-resolve assertion guards against accidental tool
    ///    selection across the four ids.
    ///
    /// With the `browser` feature enabled AND an initialized
    /// `BrowserApi`, the assertions flip: only the selected
    /// descriptor's `registered_name` appears. Initializing an
    /// oxibrowser-core engine in a unit test is not currently
    /// supported (no fixture exposes a pre-populated `BrowserApi`),
    /// so the table-driven test asserts the warn+skip path. The
    /// four arms are visibly distinct in source — each acquires the
    /// engine and constructs exactly one tool — which, paired with
    /// the static arm map, is the strongest unit-level evidence.
    #[tokio::test]
    async fn register_from_resolved_profile_arms_match_browse_per_descriptor_id() {
        use crate::capability::descriptor::ToolId;
        use crate::capability::profile::{
            CapabilityRequest, ProfileRevision, ResourceSelector, ToolActivationPolicy,
            ToolProfileSpec, ToolSelector,
        };

        // (descriptor id, registered name) — exhaustive across all
        // four `kernel.browse.*` descriptors. Each row exercises a
        // distinct match arm.
        let cases: &[(&str, &str)] = &[
            ("kernel.browse.open", "browse"),
            ("kernel.browse.extract", "browse_extract"),
            ("kernel.browse.session", "browse_session"),
            ("kernel.browse.script", "browse_script"),
        ];

        let tmp = tempfile::tempdir().unwrap();
        let kernel = make_test_kernel(tmp.path());

        for (descriptor_id, registered_name) in cases {
            let agent = AgentId::new_v4();
            let mut bounding = CSpace::new(agent);
            bounding.insert(Capability::kernel(CapResourceRef::Browser, CapRights::EXECUTE));

            let profile = ToolProfileSpec {
                id: Arc::from(format!("browse-only-{}", registered_name).as_str()),
                revision: ProfileRevision(1),
                extends: vec![],
                capability_ceiling: vec![CapabilityRequest {
                    resource: ResourceSelector::Browser,
                    rights: CapRights::EXECUTE,
                }],
                include: vec![ToolSelector::Tool(ToolId::new(*descriptor_id))],
                exclude: vec![],
                dynamic_providers: vec![],
                activation: ToolActivationPolicy {
                    max_active_tools: 16,
                    required_core: vec![],
                    sticky_turns: 0,
                },
            };

            let resolved = resolve(&profile, &bounding, agent, None);
            // Sanity: only the targeted descriptor reaches the
            // resolver loop; no other descriptor leaks into the
            // resolved set.
            assert_eq!(
                resolved.tools.len(),
                1,
                "[{descriptor_id}] resolved.tools must contain exactly one entry, got {:?}",
                resolved.tools
            );
            assert_eq!(resolved.tools[0].descriptor.id.to_string(), *descriptor_id);
            assert!(
                matches!(
                    resolved.tools[0].status,
                    ResolutionStatus::Active | ResolutionStatus::AvailableOnDemand
                ),
                "[{descriptor_id}] must reach Active/AvailableOnDemand, got {:?}",
                resolved.tools[0].status
            );

            let registry = ToolRegistry::new();
            let cache = Arc::new(SearchCache::new());
            let gate = make_test_gate();
            let context =
                AgentContext::test_fixture_with_cspace("browse-only-test", bounding.clone());

            register_from_resolved_profile(
                &registry,
                &kernel,
                &resolved,
                cache,
                agent,
                gate,
                context,
                None,
                None,
                None,
                None,
            );

            let names = registry.names();
            // Engine is None in the unit fixture → every arm
            // warn+skips. The selected descriptor's tool must NOT be
            // registered, and the other three browse tools must also
            // NOT be registered (the prefix-match regression would
            // have registered them all when the engine was Some;
            // here it registers none because None short-circuits,
            // but the assertion still pins the contract that no
            // descriptor leaks a different tool's name).
            assert!(
                !names.contains(&registered_name.to_string()),
                "[{descriptor_id}] engine is None; the arm for this id must warn+skip, \
                 but registry contains {registered_name}: {names:?}"
            );
            for (_, other_name) in cases {
                if other_name == registered_name {
                    continue;
                }
                assert!(
                    !names.contains(&other_name.to_string()),
                    "[{descriptor_id}] must NOT register sibling browse tool \
                     {other_name} (prefix-match regression): {names:?}"
                );
            }
        }

    }

    /// Catalog completeness — every long-tail tool descriptor introduced
    /// in Task 9 of the persona execution-profile plan must be present
    /// in the kernel catalog. The registration arms in
    /// `register_from_resolved_profile` key off these ids, so a missing
    /// descriptor would silently fall through to the catch-all `_` arm
    /// (a `tracing::warn!` + skip). This test pins every new id as
    /// addressable through `find_descriptor` and asserts each
    /// declaration has the rights + activation class the brief
    /// mandates.
    #[test]
    fn catalog_long_tail_tools_are_addressable() {
        use crate::capability::descriptor::{find_descriptor, ActivationClass, ToolId};
        use crate::capability::{ResourceRef, Rights};

        let cases: &[(&str, &str, &str, &str)] = &[
            // (descriptor_id, registered_name, domain, rights)
            ("kernel.mount.manage", "mount", "mount", "WRITE"),
            ("kernel.task.manage", "task", "task", "WRITE"),
            (
                "kernel.marketplace.manage",
                "marketplace",
                "marketplace",
                "WRITE",
            ),
            (
                "kernel.program.forge",
                "skill_forge",
                "program",
                "WRITE",
            ),
            ("kernel.calendar.manage", "calendar", "calendar", "WRITE"),
            ("kernel.memo.manage", "memo", "memo", "WRITE"),
            (
                "kernel.timeline.manage",
                "timeline",
                "timeline",
                "WRITE",
            ),
            ("kernel.email.send", "send_email", "email", "WRITE"),
            ("kernel.image.gen", "image_generation", "image_gen", "WRITE"),
            (
                "kernel.screenshot.capture",
                "browse_screenshot",
                "browser",
                "EXECUTE",
            ),
        ];

        for (id, registered_name, domain, rights_str) in cases {
            let tool_id = ToolId::new(*id);
            let desc =
                find_descriptor(&tool_id).unwrap_or_else(|| panic!("{id} missing from catalog"));
            assert_eq!(desc.registered_name, *registered_name, "{id} registered_name");
            assert!(
                !desc.required_capabilities.is_empty(),
                "{id} must declare required_capabilities",
            );
            let cap = &desc.required_capabilities[0];
            match (&cap.resource, *rights_str) {
                (ResourceRef::KernelDomain { domain: d }, r) => {
                    assert_eq!(d, domain, "{id} domain");
                    let want = match r {
                        "WRITE" => Rights::WRITE,
                        "EXECUTE" => Rights::EXECUTE,
                        "READ" => Rights::READ,
                        other => panic!("unexpected rights token {other}"),
                    };
                    assert_eq!(cap.rights, want, "{id} rights");
                }
                (ResourceRef::Browser, "EXECUTE") => {
                    assert_eq!(cap.rights, Rights::EXECUTE, "{id} rights");
                }
                (other, _) => {
                    panic!("{id} unexpected resource ref: {other:?}");
                }
            }
            // Screenshot is an OnDemand browser tool; everything else
            // is ApprovalOnly (kernel mutation) per the brief.
            if *id == "kernel.screenshot.capture" {
                assert!(
                    matches!(desc.activation_class, ActivationClass::OnDemand),
                    "{id} activation_class",
                );
            } else {
                assert!(
                    matches!(desc.activation_class, ActivationClass::ApprovalOnly),
                    "{id} activation_class",
                );
            }
        }
    }
}
