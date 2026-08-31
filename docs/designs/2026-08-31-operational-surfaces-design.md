# Operational Surfaces Design

> **Status:** Proposed — implementation deferred to a separate session  
> **Date:** 2026-08-31  
> **Scope:** Non-chat, non-dashboard operational UX for the shared React UI  
> **Language:** English (product tool output remains bilingual through existing i18n)

## 1. Decision

Oxios should make a user's unit of work — a project, its running agents, its
permissions, and its deliverables — inspectable without requiring the user to
manually assemble that context across separate system pages.

This design introduces four complementary **operational surfaces**:

1. **Project Control Room (P0):** the decision-oriented home of one project.
2. **Capability Hub (P1):** the owner-facing view of what Oxios may do, where,
   and with what impact.
3. **Agent Run Center (P1):** an exception-oriented entry point for agent work
   that needs intervention.
4. **Project Readiness (P2):** an explicit preflight before meaningful work
   starts in a project.

The first two surfaces are illustrated by the associated exploratory mockups.
They are not visual reskins of Settings, the dashboard, or chat. They compose
existing authoritative state around the user's next decision. New runtime
authority, lifecycle semantics, and tool permissions are expressly out of
scope.

## 2. Scope and non-goals

### In scope

- React routes, page composition, and read-only projection APIs needed to
  present an existing project's operational context truthfully.
- Clear ownership and impact presentation for connections, tools, permissions,
  skills, and filesystem roots.
- Navigation that leads from an exceptional state to its canonical deep-detail
  route without duplicating the complete detail view.
- Desktop-first three-zone layouts with an accessible small-screen sheet
  equivalent.

### Out of scope

- The chat UI, transcript, composer, or persona-adaptive chat shell.
- The root command-center dashboard and its information architecture.
- New agent lifecycle states, scheduling behavior, security policy, or
  permission semantics.
- A new event transport, an alternate tool-registration path, or an additional
  state store.
- Replacing the canonical detail routes for agents, Git, Security, MCP, Skills,
  Settings, Issues, or Milestones.
- A native-shell-specific UI. `web/` remains the shared source for browser and
  macOS clients per RFC-042.

## 3. Existing context and problem

Oxios already has the important primitives:

- projects with roots, instructions, issue and milestone tracking;
- agent list, canvas monitor, detail, trace, logs, budgets, and A2A context;
- project-aware coding workbench affordances in the chat shell;
- approval queues, audit history, AccessManager policy, and path sandboxing;
- MCP server management, host-tool/integration settings, skills, and a
  marketplace;
- workspace/Git, assets, brain spaces, and session context.

Those primitives are organized mainly by subsystem. The console navigation
therefore correctly answers questions such as “show MCP servers” or “show
agent traces,” but makes the following routine questions unnecessarily
expensive:

- What is stopping this project from being complete?
- Which agent owns the next step, and is it actually able to proceed?
- What needs my approval, why, and what changes if I grant or revoke it?
- Which project relies on a connection, tool, skill, or filesystem root?

The current project detail route is deliberately light: it shows roots,
instructions, metadata, issues, and milestones. The agent monitor is strong at
topology, while Security is strong at policy and audit. Neither is currently a
project-scoped decision surface. The result is context switching, not a lack
of capabilities.

## 4. Product principles

1. **Decision before telemetry.** Show the next action, its owner, and its
   consequence before aggregate metrics.
2. **Project is a context boundary, not a folder alias.** A project may have
   zero or many roots. The UI must never infer a root, brain binding, or
   capability from the ambient workspace.
3. **Truthful state only.** Do not invent progress percentages, task phases,
   approvals, relationships, or impact. Missing data is rendered as unavailable
   or omitted.
4. **Canonical detail stays canonical.** Operational surfaces summarize and
   link out. Full traces, raw audit records, diffs, and settings remain in the
   routes that already own them.
5. **Authority is visible before it is exercised.** A user should see the
   resource, scope, reason, affected projects, and reversible choices before
   approving or changing access.
6. **Reuse the existing event and state buses.** SSE and existing query hooks
   remain the live source. Structured tool results continue to use the
   established result bus; this UI creates no parallel channel.
7. **Status is redundant.** Every state uses an icon, text label, and semantic
   status color, following `DESIGN.md`; color alone is never meaning.

## 5. Information architecture

The top-level mode set remains **Console / Brain / Chat**. This proposal does
not create a fourth global mode. It adds clearer operational destinations in
Console and preserves existing destinations for deep work.

```text
Console
├── Projects
│   └── Project detail
│       ├── Control Room        ← new primary project tab
│       ├── Work / issues
│       ├── Milestones
│       ├── Readiness           ← new, only when setup needs attention
│       └── Project settings
├── Agents
│   ├── Run Center              ← new default operational view
│   ├── Topology                ← existing canvas
│   └── All runs                ← existing table/detail routes
└── Infrastructure
    ├── Capability Hub          ← new composition route
    ├── MCP                     ← canonical server/tool configuration
    ├── Security                ← canonical policy, approval, and audit route
    └── Settings                ← canonical provider/host-tool configuration
```

The labels can be localized, but the information architecture should retain
these distinctions. “Control Room” describes a project's operational state;
“Run Center” describes cross-project execution intervention; “Capability Hub”
describes authority and dependencies. None is a catch-all dashboard.

## 6. Project Control Room (P0)

### 6.1 Purpose

The Control Room answers one question: **what must happen for this project to
move forward safely?** It is the default tab for an existing project once
implemented. Issues and milestones remain available as planning records, not
as a substitute for live execution context.

### 6.2 Desktop layout

```text
Application sidebar | Project Control Room                         | Inspector
                    | breadcrumb · project · roots · Git status    |
                    | Outcome and checkpoints                      | Needs your
                    | Work graph / active runs                     | decision
                    | Changeset and deliverables                   | Change review
```

The header identifies the project, active root summary (or the explicit
folderless state), current branch when a project root can supply it, and a
compact repository-health label. It must not claim a branch or clean tree when
the project has no usable Git root.

The main lane contains the following sections in order:

| Section | Content | Interaction |
|---|---|---|
| Outcome | A user-authored project goal when available, followed by real blockers/checkpoints | Open the relevant issue, task, or readiness item. Do not synthesize a goal from agent prompts. |
| Work graph | Active/recent project agents, their real lifecycle state, worktree/root identity, current trace step when available, and dependencies/A2A relationships | Select a run; open the existing agent detail or trace. |
| Changeset | Project-root Git summary and changed files only when the Git query can identify the root | Open the canonical Git/diff route. |
| Deliverables | Explicit artifacts linked to the project/session/run, such as a design note, report, or draft | Open the canonical artifact, workspace, or session location. |

The inspector exists only when there is a selected blocking item, approval, or
change review. On desktop it is a stable right column; on smaller screens it
becomes a modal sheet. It presents an action in this order:

1. action and owner;
2. resource and requested scope;
3. rationale and impact;
4. one-time and durable/reversible choices; and
5. a deep link to the canonical Security, Git, or agent route.

### 6.3 Checkpoint rules

Checkpoint rows use only source-backed states:

- **Done:** a completed issue, task, run, or verified result explicitly linked
  to the project.
- **Needs decision:** a project-linked pending approval or a documented
  operator-blocking state.
- **In progress:** a running/starting project-linked agent or task.
- **Queued:** a scheduled/queued task or dependent run that actually exists.
- **Unavailable:** required linking or source data is absent.

The UI must not turn a generic issue title into an execution checkpoint merely
because it is open. A project may legitimately render only its active-runs
section on day one.

### 6.4 Empty and failure states

- **No project roots:** explain the folderless state and offer the existing
  project-root editor; do not expose filesystem or Git controls.
- **No active work:** present a quiet start-work affordance linking to the
  existing project-aware task/chat flow, plus issues and milestones.
- **No project-linked approvals:** omit the inspector and approval section.
- **Git unavailable or root stale:** show the existing stale-root reasoning and
  a non-alarming unavailable changeset state.
- **Partial API failure:** fail the affected panel only. Roots, issues, and
  project metadata remain usable if the agent projection is unavailable.

## 7. Capability Hub (P1)

### 7.1 Purpose

The Capability Hub is the owner-facing answer to: **what can Oxios access or
execute, why is it allowed, and what depends on it?** It composes existing MCP
servers/tools, configured host-tool connections, project roots, skills, and
approval policy. It does not replace their configuration or security
enforcement.

### 7.2 Capability model

A displayed capability is a presentation projection, not a new kernel
capability type. It has a stable visual identity and references the source
subsystem:

| Capability family | Source of truth | Examples |
|---|---|---|
| Connected service | host-tool/integration config and OAuth state | calendar, GitHub, email |
| MCP server/tool | MCP registry and tool discovery | filesystem server, database tool |
| Local resource | project roots plus AccessManager sandbox/policy | project filesystem, Git root |
| Skill | SkillManager metadata and dependency state | release skill, research skill |
| Execution policy | approval policy and allow-list | shell, browser, network access |

Every card shows its source, health/readiness, scopes or restrictions, last use
when the source exposes it, and explicit project/run references only when a
real relation is known. “Not connected” is not an error state; it is a setup
state with a precise action.

### 7.3 Layout and interactions

The center lane is a searchable, filterable list grouped by family and status:
Connected, Needs review, Restricted, Not configured, and Unavailable. Selecting
a card opens a right inspector with **Access**, **Usage**, and **Activity**
tabs.

- **Access:** scoped permissions, policy state, rationale, and per-project
  applicability. Editing delegates to the current configuration/security route
  or an embedded reuse of its existing form; the Hub never introduces an
  alternate save model.
- **Usage:** projects, agents, skills, and scheduled tasks with verified links.
  A missing link reads “No usage relationship recorded,” not “Unused.”
- **Activity:** bounded, redacted recent activity drawn from the existing audit
  and event sources. Raw argument values and secrets are never shown.

Before a destructive/restrictive action, the Hub shows an **impact preview**.
It may state a count or name only if the dependency projection can prove it;
otherwise it states the conservative effect (“future calls may be denied; open
runs retain their existing worktree state”) and links to details.

### 7.4 Security contract

The Hub is an explanatory surface. AccessManager, approval policy, MCP
registration, host-tool OAuth handling, and path sandboxing remain authoritative.
No Hub-only “allowed” state may bypass the current gate. Capability changes use
the same mutation endpoint, confirmation, audit record, and error handling as
their canonical management route.

## 8. Agent Run Center (P1)

The existing `/agents` canvas and table remain valuable views. The new default
Run Center adds an **attention queue** ahead of them:

1. waiting for user decision;
2. failed or stalled;
3. budget/quota constrained;
4. running unusually long according to an explicit, documented threshold;
5. healthy running work; then
6. recently completed work.

Each compact row explains the reason it appears, owning project/session where
known, latest source-backed step, elapsed time, and the next safe action.
Selecting a row opens a bounded inspector; trace and logs retain their current
full-detail routes. The existing topology canvas becomes a named view switch,
not the only default representation of multi-agent work.

The first implementation must avoid an invented “stalled” state. Until the
backend provides a defined heartbeat/update threshold, the Run Center can show
only server-reported statuses and explicit pending approvals/failures.

## 9. Project Readiness (P2)

Readiness is a guided review before an agent begins meaningful project work.
It is not a mandatory wizard and must not silently grant access.

It checks, in a fixed and understandable order:

1. project identity and zero-or-more root paths;
2. stale/missing roots and Git availability;
3. optional default brain space and persona/profile binding;
4. selected model/provider readiness and budget policy;
5. requested skills and their declared integration requirements; and
6. effective approval mode and any known capability prerequisites.

Each row is **Ready**, **Needs setup**, **Restricted**, **Optional**, or
**Unavailable**, with one action leading to the existing owner route. Starting
work remains possible when the user deliberately accepts an optional or
restricted condition; the agent is still stopped by normal gates if it later
requests unavailable authority.

## 10. Data and API design

### 10.1 Reuse first

The UI should first compose existing read endpoints and client hooks:

- project detail and root status;
- agents list, detail, trace, logs, and A2A monitor;
- approvals, audit, and security policy;
- Git, workspace, assets, task, issue, milestone, and session routes;
- MCP registry/tools, skills, providers, integrations, and system status;
- existing SSE event store.

### 10.2 Small read-only projections when composition is insufficient

Avoid N+1 route composition and undocumented client-side joins by adding only
the following read models if existing endpoints cannot provide the associated
truthful view:

| Endpoint | Purpose | Required safeguards |
|---|---|---|
| `GET /api/projects/:id/operating-context` | Project-linked active/recent runs, real blockers, root/Git summary, and explicit deliverable references | Omit unknown relationships; redact tool arguments and out-of-sandbox paths; do not create lifecycle state. |
| `GET /api/capabilities` | Unified presentation rows with source family, readiness, policy/scopes, and verified usage/dependency references | No secrets, raw OAuth values, or hidden policy data; source identifier and deep route are always included. |
| `GET /api/agents/attention` | Explicitly reasoned queue of exceptional runs | Every row includes machine-readable reason kind and source timestamp; no heuristic “stalled” result before a backend contract exists. |
| `GET /api/projects/:id/readiness` | Read-only preflight checks and canonical remediation links | Never evaluates a condition by acquiring access or invoking tools with side effects. |

These belong to `src/api/routes/` as shared daemon-control-plane projections.
They draw on existing KernelHandle APIs and domain stores; they do not belong in
Orchestrator, do not duplicate Supervisor lifecycle logic, and do not add a
second structured-result channel.

### 10.3 Link semantics

All projection links need an explicit origin:

- `project_id` from the agent/task/session record;
- a project root path matched through the registered project roots;
- a skill/integration dependency declared in structured metadata; or
- an approval/audit record that includes the corresponding project/run context.

If an existing record lacks that origin, the API returns no relationship. The
frontend may offer a generic deep link but must not display it as project usage.

## 11. Frontend boundaries

The route components should remain composition points. Keep data fetching and
state inside focused hooks/components rather than creating a global operational
store.

| Boundary | Responsibility | Reuses |
|---|---|---|
| `ProjectControlRoom` | Project header, local selected-item state, panel composition | project hooks, agent monitor, Git and approval hooks |
| `ProjectOutcome` | Source-backed outcome and checkpoint rendering | issues/tasks/readiness projection |
| `ProjectWorkGraph` | Project-filtered agent presentation | monitor nodes/edges, agent detail links |
| `DecisionInspector` | Approval/change action context, mobile sheet behavior | approval, Git, and agent mutations |
| `CapabilityHub` | List filtering, selected capability, query state | MCP, integration, skills, security hooks/projection |
| `CapabilityInspector` | Access/usage/activity tabs and impact preview | canonical settings/security mutations |
| `RunCenter` | Attention ordering and view switches | agent hooks/projection |
| `ReadinessPanel` | Source-backed preflight list and canonical remediation links | root, brain, profile, skill, engine, policy hooks |

Selection state is route-local and keyed by a stable project, agent, or
capability source identifier. It is not persisted globally unless a user-facing
requirement later justifies it.

## 12. Responsive, visual, and accessibility requirements

- At `lg` and above, use the existing sidebar + main + optional inspector
  grammar. The inspector does not consume width when nothing is selected.
- Below `lg`, selected inspector content opens as an accessible sheet; lists
  and tables reflow without hiding status labels or destructive actions.
- Follow `DESIGN.md`: semantic warm-paper surfaces, SUIT/SUITE typography,
  Geist Mono for IDs and paths, density of `gap-2` within components and
  `gap-4` between sections, and no raw color values or component-level
  `dark:` classes.
- Status always includes textual state and an icon. Destructive or durable
  access changes require a visible confirmation and announce their result.
- Keyboard order follows visual decision order. Inspector open/close is fully
  keyboard accessible, focus is restored to the trigger, and live updates use
  concise `aria-live` announcements without repeatedly reading the whole page.
- Capability and audit panels redact secrets, opaque credentials, raw token
  values, and sensitive command arguments. Error messages distinguish
  unavailable, unauthorized, and disconnected states where the API can do so.

## 13. Delivery sequence

### Phase A — Project Control Room foundation (P0)

1. Validate which existing endpoints can compose root, issue, agent, approval,
   Git, and artifact context.
2. Define the minimal `operating-context` projection only for missing
   project-linked data.
3. Add the Control Room project tab, quiet states, selected-item inspector,
   and deep links.
4. Test zero-root, no-agent, stale-root, no-approval, partial-failure, and
   active-approval cases.

### Phase B — Capability Hub (P1)

1. Inventory current MCP, host-tool, skill, policy, and project-root sources.
2. Define `CapabilityRow` and the provenance/usage rules before adding any UI.
3. Implement browsing and read-only inspection first.
4. Reuse canonical configuration/security mutations and audit behavior for
   edits; add impact preview only where dependency truth is available.

### Phase C — Run Center (P1)

1. Ship explicit server-reported waiting/failed/approval conditions.
2. Keep topology and table as alternate views.
3. Add heuristic exception reasons only after a documented backend state or
   threshold contract exists.

### Phase D — Project Readiness (P2)

1. Start with read-only checks and canonical setup links.
2. Add inline remediation only if it reuses an existing safe mutation path.
3. Measure completion and first-successful-run outcomes before expanding the
   checklist.

## 14. Acceptance criteria and evaluation

### Functional acceptance

- A project with active work can reveal its source-backed blockers, agents,
  Git/change state, and linked deliverables without leaving the project route.
- A project with no roots or no active work renders an honest, useful state and
  never grants new filesystem access.
- A pending, project-linked approval exposes action, resource, scope, reason,
  impact, and the current one-time/durable decision actions.
- A Capability Hub row identifies its authoritative source and opens the
  appropriate canonical configuration or security route.
- Revoking/changing access uses existing gates and emits the existing audit
  record; no UI path bypasses AccessManager.
- The Run Center clearly states why a run appears; unknown or heuristic reasons
  are never represented as facts.

### Quality gates

- Unit tests cover projection transformation, relationship omission, ordering,
  redaction, and all defined status mappings.
- Route/component tests cover keyboard navigation, inspector focus return,
  empty/partial-error states, and action mutation failures.
- API tests verify project scoping, no secret leakage, and stable deep-link
  source identifiers.
- Visual checks cover light/dark themes and desktop/mobile layouts against the
  Oxi token and status-contrast rules.

### Product signals

Measure, with privacy-preserving local telemetry only if/when telemetry is
approved, the time from opening a project to resolving an approval or opening
the relevant run/detail route; Control Room empty/partial-error rates; and the
number of capability changes that are abandoned after seeing impact. These are
decision-quality signals, not activity KPIs.

## 15. Risks and decisions to resolve before implementation

| Risk / question | Decision rule |
|---|---|
| Project linkage is incomplete across approvals, artifacts, and skills | Omit the relation; do not infer it from names or current browser route. Add explicit provenance only in a separately approved backend change. |
| One large projection becomes a new hidden domain model | Keep projections read-only, bounded to a route, and source-labeled. Promote a type into the kernel only when more than one domain must own/write it. |
| Capability Hub duplicates Settings or Security | Hub explains relationship and impact; canonical routes own detailed configuration and enforcement. |
| Control Room overlaps the command-center dashboard | Dashboard is cross-project operations; Control Room is a single project's work context. Shared components may be reused, but the routes answer different questions. |
| "Always allow" is too broad for a contextual approval | Preserve the existing approval-policy vocabulary and surface its exact scope. Do not create a new persistent grant level for this UI. |
| Readiness becomes a blocking setup ritual | Keep it optional, explain restrictions, and preserve the current gates as the real enforcement point. |

## 16. Related material

- `DESIGN.md` — Oxios visual system and accessibility rules.
- `docs/rfc-042-product-interface-layout.md` — shared React surface and thin
  desktop-client contract.
- `docs/designs/2026-08-31-command-center-dashboard-redesign.md` —
  cross-project operational dashboard; explicitly separate scope.
- `docs/designs/2026-08-31-persona-adaptive-chat-surface-design.md` — chat
  presentation lens; explicitly separate scope.
- `docs/designs/2026-08-29-issues-milestones-todo-design.md` — project work
  records and their ownership rules.
- `docs/rfc-043-task-management.md` — task scheduling, dependencies, and
  verification concepts.
