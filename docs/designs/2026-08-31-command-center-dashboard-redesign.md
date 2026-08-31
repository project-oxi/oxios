# Command Center Dashboard Redesign (2026-08-31)

## Status

Approved design direction for a future implementation session. This document changes no
runtime behavior by itself.

## Problem

The current root dashboard (`web/src/routes/index.tsx`) is a healthy, data-rich
overview, but it is organized as a sequence of independent cards:

1. six KPI cards;
2. agents/activity plus system health;
3. MCP, budget, brain, and skills/cron cards;
4. a pending-approvals queue only when there is work to approve.

That makes routine system telemetry easy to scan, but requires the operator to assemble
an answer to the operational questions that matter most:

- What requires my decision now?
- Which agent run needs attention, and what is it doing?
- What happened just before this state?
- Is the system healthy enough to continue?

The redesigned dashboard is therefore an **operational command center**, not a general
analytics page. It prioritizes decision and execution context ahead of aggregate metrics.
Detailed cost analytics remain in Settings and Budget; MCP, Skills, Brain, Resources, and
Security retain their dedicated routes.

## Goals

- Put actionable approvals ahead of passive monitoring.
- Make active agent work and the live event stream visible together on desktop.
- Let an operator inspect one selected run without leaving the dashboard.
- Preserve the existing status semantics: icon + text label + color; never color alone.
- Reuse the existing API and SSE contracts wherever they already provide the required
  data.
- Keep the dashboard usable with no active agents, no approvals, or partial API failure.

## Non-goals

- Replacing `/agents`, `/agents/$agentId`, or `/agents/$agentId/trace` as the complete
  agent-management and investigation surfaces.
- Moving Settings usage analytics onto the root route.
- Redesigning the global sidebar, header, chat, knowledge UI, or mobile navigation.
- Adding a second event transport; the existing SSE event store remains the live source.

## Information architecture

Desktop uses the existing application shell, with the command-center content divided into
an operational main area and a contextual inspector:

```
existing sidebar | main command center                                  | run inspector
                 | greeting + global health + New task                 |
                 | Needs your attention (only when actionable)         |
                 | Active execution             | Live timeline        |
                 | Today's operating picture                              |
```

### 1. Context header

The existing `PageHeader` becomes a compact command-center header:

- contextual greeting or `Dashboard` title;
- a single system-health summary (`System healthy`, `Degraded`, or `Attention needed`),
  with status icon and text;
- `New task` as the primary action, opening the existing task/chat creation flow rather
  than inventing a parallel composer;
- secondary version strings move out of the visual headline. They remain available in an
  overflow/details affordance or the system-health route.

### 2. Needs your attention

Render this section only when there are pending approvals or an explicitly defined
operator-blocking condition. Pending approvals are first-class rows, not a low-priority
bottom card. Each row shows the action/resource, risk/reason, age, and existing
`Approve` / `Deny` actions.

The current empty-state success card is deliberately not retained in the primary flow:
absence of this section means there is nothing awaiting intervention. The header health
summary still makes a degraded system visible without manufacturing an empty card.

### 3. Active execution

This is the dashboard's primary work surface. It lists running, starting, waiting-for-
input, and recently failed agents in priority order. A row contains:

- agent name and stable short ID;
- readable lifecycle label and matching status icon;
- the best available task/phase summary;
- elapsed time;
- optional progress only when it is a real backend value; never fabricate a percentage;
- a stop action for a running agent, reusing the existing kill endpoint and confirmation
  behavior;
- selection state. Selecting a row updates the desktop inspector and does not navigate.

`Running` agents sort first, then waiting-for-input, starting, failed, and all other
states. Within a state, most recently updated work sorts first. The existing agent list
only exposes `id`, `name`, `status`, and optional creation time, so task summary, phase,
updated timestamp, and genuine progress require a small dashboard projection endpoint
(defined below). Until that endpoint exists, the component must render the available
fields and omit unavailable detail rather than showing mock data.

### 4. Live timeline

The existing `LiveActivityFeed` becomes a sibling surface to Active execution, retaining
its current SSE subscription, event filtering, pause/resume, capped list, and auto-scroll
behavior. The command-center variant adds:

- a compact, timestamped row treatment;
- a selected-run filter when an inspector run is selected, if the event has an agent/run
  identity;
- a visible `View full timeline` link to the existing events surface;
- a connection state that names a disconnected/reconnecting stream, not merely a colored
  dot.

No new live channel is introduced. Events that cannot be related to the selected run
remain visible in the unfiltered All view.

### 5. Run inspector

On desktop, the right inspector is present when a run is selected and absent otherwise;
its absence lets the main content use the available width. It contains:

- agent/run identity, lifecycle state, owner/session when available, selected model, and
  start/elapsed timestamps;
- ordered execution steps with completed, active, pending, and failed states;
- a compact recent-tool-call list with outcome and time;
- `Open run`, linking to `/agents/$agentId` or its trace route; and
- a destructive `Stop` action only while the agent can be stopped.

The inspector is an operational summary, not a duplicate trace viewer. It must cap lists
and send deep investigation to the canonical agent detail/trace routes.

### 6. Today's operating picture

The final section gives operational context without competing with active work. It
contains a 24-hour activity series and four compact aggregates:

- active agents;
- completed runs today;
- tokens today;
- spend today.

Existing resource history, token-rate history, cost summaries, and agent status data
remain the source of truth. If a daily aggregate is not exposed today, the UI must show
an unavailable state or use an explicit documented period label; it must not relabel an
all-time value as today.

System health, provider routing, MCP connection state, Brain availability, and skills/
cron remain accessible through compact status links below the operating picture or an
overflow `System` panel. This avoids losing visibility while removing four equal-weight
cards from the primary scan path.

## Responsive behavior

| Breakpoint | Layout |
|---|---|
| `lg` and above | Main area uses a two-column Active execution / Live timeline row. The Run inspector is a fixed right column only when a row is selected. |
| Below `lg` | Inspector opens as a sheet/drawer from the selected row. Active execution and timeline become stacked, preserving the feed rather than remounting it. |
| Small screens | Approval rows wrap actions below the reason; the operating-picture metrics use a two-column grid. Stop and approval actions retain accessible names and minimum touch targets. |

## Component boundaries

The implementation should replace the page composition in `web/src/routes/index.tsx`,
not turn the route into a large view-model component. Proposed frontend boundaries:

| Component | Responsibility | Existing source to reuse |
|---|---|---|
| `CommandCenterHeader` | Title, health summary, new-task action | `PageHeader`, `SystemStatus` |
| `AttentionQueue` | Actionable approval rows and mutations | `ApprovalsQueue`, approval hooks |
| `ActiveExecutionList` | Ordering, selection, stop affordance, states | `AgentsActivityCard` agent list, agent hooks |
| `LiveTimelinePanel` | Command-center presentation of the existing feed | `LiveActivityFeed` |
| `RunInspector` | Selected-run detail and bounded tool/step summaries | agent detail, trace, and logs APIs |
| `OperatingPicture` | Daily series plus four metrics | resource history, token rate, cost hooks |
| `SystemLinks` | De-emphasized MCP/Brain/Skills/health entry points | existing dashboard cards/routes |

Selection is dashboard-local React state keyed by `agentId`. Query data, SSE data, and
mutations remain in their existing hooks/stores. Do not introduce a global selected-agent
store solely for this route.

## Data and API contract

### Reused without backend changes

| Need | Existing source |
|---|---|
| system status and component health | `GET /api/status` |
| agent list and lifecycle | `GET /api/agents` |
| stop agent | `POST /api/agents/{id}/kill` |
| agent detail / trace / logs | `GET /api/agents/{id}`, `/trace`, `/logs` |
| approval list and decisions | `GET /api/approvals`, approval mutation endpoints |
| live events | existing `/api/events` SSE store |
| token and resource history | existing `useTokenRate`, `useResourceHistory` |
| current-period cost | existing `/api/costs/summary` and daily cost endpoint |

### New dashboard projection: required before claiming run-level progress

The existing agent list type is intentionally minimal and cannot truthfully populate the
mockup's task name, step progress, model, or recent tool calls. Add a read-only endpoint
such as `GET /api/dashboard/active-runs` only if the agent-detail/trace calls prove too
expensive or incomplete for the dashboard.

```json
{
  "items": [{
    "agent_id": "agent_…",
    "name": "Release QA",
    "status": "running",
    "summary": "End-to-end regression suite",
    "phase": "testing",
    "started_at": "2026-08-31T05:28:31Z",
    "updated_at": "2026-08-31T05:37:12Z",
    "progress": { "completed": 3, "total": 5 },
    "model": "provider/model",
    "steps": [{ "label": "Run test suite", "state": "running", "duration_ms": 362000 }],
    "recent_tool_calls": [{ "tool": "bash", "state": "succeeded", "at": "…" }]
  }]
}
```

`progress` is optional. It represents completed/total steps, not a guessed percentage.
Sensitive command arguments, paths outside the sandbox, and raw tool output must not be
included in this compact endpoint; the full trace route retains its existing access and
redaction rules. The frontend owns formatting elapsed times and must tolerate every
optional field being absent.

The endpoint belongs in the binary API layer and draws from existing KernelHandle agent
and trace/read models. It must not add lifecycle logic to Orchestrator or duplicate the
Supervisor's state.

## States and failure handling

- **No active work:** show a quiet empty state in Active execution with a route/action to
  start a task; timeline and operating picture remain visible.
- **No approvals:** omit the attention section.
- **Selected run ends or disappears:** keep the inspector briefly as a terminal state if
  detail is cached; otherwise close it and announce that the run is no longer active.
- **SSE disconnect:** keep the most recent timeline rows, show a textual reconnection
  state, and preserve pause/filter state.
- **One query fails:** localize the error to its panel with retry. Do not replace the
  whole command center if the status API, for example, remains available.
- **Stop/approval mutation fails:** retain the item and report the failure through the
  existing toast/error pattern. Disable duplicate submissions while the mutation is in
  flight.
- **Missing projection fields:** render a neutral `Unavailable`/omitted detail; never
  display invented progress, owners, or tool results.

## Visual and accessibility rules

- Follow `DESIGN.md` and `web/src/index.css`: warm-paper semantic surfaces, SUIT/SUITE
  typography, Geist Mono for IDs/timestamps, and the `.dark` token model. No raw hex or
  `dark:` classes in component markup.
- Preserve Oxios density: `gap-2` within components and `gap-4` between sections.
- Use status icon + label + semantic color (`success`, `warning`, `error`, `info`) for all
  agent and system state. On neutral surfaces, use the documented `-on-surface` status
  tokens for text contrast.
- Keep the existing `aria-live`/log semantics for live events. Selection must be keyboard
  reachable, visibly focused, and expose its selected state. All approval and stop controls
  need explicit labels.
- Respect reduced-motion preferences. A live indicator may pulse only when motion is
  allowed; progress changes must be understandable without animation.

## Migration and verification

1. Add the new layout beside the current component composition, reusing existing hooks.
2. Implement and test the run projection only after confirming the existing per-agent
   endpoints cannot supply the needed concise data within dashboard performance bounds.
3. Remove the root-route placement of the old equal-weight cards once their route links or
   compact system entry points are present. Keep the underlying components for their
   dedicated routes where applicable.

Required checks for the implementing session:

- Web unit tests for run ordering, inspector selection/closure, empty states, and absent
  projection fields.
- Mutation tests for optimistic approval/stop behavior and failure recovery.
- Accessibility tests for keyboard selection, focus return from the mobile inspector, and
  status text not depending on color.
- Responsive visual checks at mobile, tablet, and desktop widths in both themes.
- SSE smoke test: live event arrival, pause/resume, filter persistence, disconnect copy,
  and selected-run filtering when correlation exists.
- Full web CI: frozen Bun install, typecheck, tests, Biome lint, and production build.

## Acceptance criteria

- A user can identify every pending approval and act on it before scanning passive metrics.
- A user can identify active agents, their concrete lifecycle state, and elapsed time from
  the first viewport.
- Selecting an active run reveals its available steps and recent tool-call summary without
  route navigation on desktop.
- The dashboard remains useful and truthful with zero agents, zero approvals, or a missing
  optional run-detail field.
- Existing dedicated routes remain the canonical places for complete trace, provider, MCP,
  skill, budget, and security management.
