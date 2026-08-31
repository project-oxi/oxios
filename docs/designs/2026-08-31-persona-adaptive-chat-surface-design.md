# Persona-Adaptive Chat Surface Redesign

> **Date:** 2026-08-31  
> **Status:** Proposed — implementation deferred to a separate session  
> **Scope:** Web `/chat` surface only; Rust/tool authority semantics are unchanged

## 1. Decision

Oxios has one chat application, not a generic chat screen plus separate persona
apps. Every session uses the same durable transcript and composer. The selected
persona changes the **presentation lens**: the information that is foregrounded,
the result format that is easiest to inspect, and the context control offered at
the edge of the chat.

The result is deliberately not a collection of color themes. A coding persona
still chats; it simply gives code, agent progress, diffs, and review states room
to breathe. A research persona still chats; it makes evidence and uncertainty
inspectable. A writer still chats; it presents drafting controls and document
structure without turning the whole product into an editor.

The default/no-persona experience is a first-class **General chat lens**, not a
reduced version of the coding workbench. It is calm, readable, and useful before
the user has configured a project, brain, or custom persona.

## 2. Relationship to existing decisions

This document refines the visual and interaction composition of the web chat
surface. It does not replace runtime contracts already established elsewhere.

| Existing design | Authority retained | This document adds |
|---|---|---|
| `2026-08-22-persona-chat-picker-design.md` | Session-scoped persona selection and categories | How the selected profile changes the chat presentation |
| `2026-08-29-project-roots-persona-workbench-design.md` | Project roots, profile-derived affordances, coding workbench, emergent stages | The baseline chat shell, result density, and persona lens slots |
| `2026-08-29-brain-chat-binding-design.md` | Per-session brain binding and gating | A visible, non-intrusive brain context indicator |
| `2026-08-31-chat-slash-commands-design.md` | Slash-command execution semantics | Composer placement and discoverability only |

When this document conflicts with the project-root design on capability,
filesystem, or authority behavior, the project-root design wins. A presentation
lens never enables a tool or widens access.

## 3. Current-state findings

The present route already has strong ingredients:

- one shared `ChatPage`, session store, stream model, cancellation, and
  persona/project/brain bindings;
- `EffectiveProfile` as the server-derived source of conditional affordances;
- a `WorkbenchShell` that preserves the transcript node while coding chrome
  appears or disappears;
- structured assistant blocks for reasoning, tools, search, artifacts, errors,
  and follow-ups;
- a rich composer with model, persona, brain, approval, attachment, and
  fan-out controls.

The visible composition does not yet express that capability well:

1. Every transcript row and the composer are constrained to `max-w-3xl`.
   This is comfortable for prose but leaves code, tables, tool output, and
   artifacts needlessly narrow on desktop.
2. The fixed top-right search/terminal controls are detached from the session
   context. Project, persona, brain, model, and connection state are instead
   scattered across the composer or hidden in other surfaces.
3. The shared `ChatItem` hides the model/time/duration title by default. This
   makes a completed answer read cleanly, but significant agent work lacks a
   stable at-a-glance provenance line.
4. `PortalPanel`, the coding rail/stage, composer controls, and turn-local
   result cards are useful independently, but have no single hierarchy for
   “where should I look next?”.
5. A persona currently changes prompt/tool affordances but has little
   perceptible effect until a coding-only control happens to appear.

The redesign addresses these as a layout and information-hierarchy problem;
it does not add fake agent phases, fake health metrics, or persona-specific
routes.

## 4. Design principles

1. **Conversation is permanent.** The transcript is the durable record in
   every lens. Switching persona must not remount, rewrite, or reinterpret old
   messages.
2. **Context is visible before action.** Show the active project, persona,
   model, and brain binding in one predictable place. A user should not need
   to inspect the composer to discover where a turn will run.
3. **Results choose their own width.** Prose has a readable measure; code,
   tables, diffs, tool output, and rendered artifacts may use the full content
   lane. Do not put rich assistant output in a narrow bubble.
4. **Persona changes hierarchy, not brand.** All lenses use the Oxi token
   system, typography, sidebar grammar, status semantics, and component
   language. Persona is communicated with a named mode chip, an icon, and
   relevant layout—not a new palette.
5. **Only truthful state earns persistent space.** Sources appear when a turn
   has sources, approvals when one is pending, and operational status when the
   runtime emits it. Empty rails and invented dashboards are forbidden.
6. **Progressive disclosure wins.** The main canvas stays focused. Deep
   context goes to an on-demand inspector; coding stages emerge only when
   relevant and remain pinned only when review requires it.
7. **Profiles gate controls; lenses do not.** All capability visibility
   continues to use the server-derived effective profile. The presentation
   lens is descriptive metadata only.

## 5. The shared chat shell

### 5.1 Layout

Every persona uses the same desktop shell. The existing left application
sidebar remains the session navigator. Within `/chat`, replace the current
free-floating controls and uniform transcript wrapper with this structure:

```text
┌ Application sidebar ┬───────────────────────────────────────────────────┐
│ sessions/projects   │ Session context bar                                │
│                     │ project · persona · model · brain · connection    │
│                     ├──────────────────────────────────────────┬────────┤
│                     │ Transcript canvas                        │ Inspector
│                     │ intent → activity → result → next action │ (on demand)
│                     │                                          │        │
│                     ├──────────────────────────────────────────┴────────┤
│                     │ contextual composer                                │
└─────────────────────┴───────────────────────────────────────────────────┘
```

- **Session context bar:** sticky inside the chat main area, 44–48px high.
  Its left side contains the editable session title and project selector; its
  right side contains persona, model, brain, connection state, search, and
  only profile-authorized quick actions. It replaces the current fixed
  top-right toolbar.
- **Transcript canvas:** scrolls independently. It is one DOM tree across all
  personas and retains the current session-switch/streaming anchor behavior.
- **Inspector:** a right-side, tabbed, on-demand panel. It replaces neither
  the coding stage nor the current portal semantics; it gives global context
  a single home. Closed by default in General chat. It may open automatically
  only in response to user intent or a focal, truth-backed event.
- **Composer:** remains bottom-anchored and uses the same send/cancel queue.
  It becomes a contextual command surface rather than the only location for
  session configuration.

`WorkbenchShell` continues to own the coding activity rail and emergent
diff/preview/terminal stage. The shared chat shell is rendered inside the same
stable child slot, so a profile change does not lose transcript scroll state.

### 5.2 Content lanes

The current single `max-w-3xl` wrapper is replaced by two nested lanes:

| Lane | Desktop width | Contents |
|---|---:|---|
| Reading measure | 680–760px | User intent, normal prose answers, questions, short summaries |
| Work measure | 1080–1240px; constrained by available stage/inspector width | Code, tables, tool cards, diffs, images, rendered artifacts, multi-column research results |

The transcript row decides which lane it needs; persona does not force width.
An answer with a code block expands its result section to the work measure,
while its prose introduction remains readable. On narrow windows and when a
stage is open, the work measure fills the remaining canvas rather than forcing
horizontal scrolling. A tool's inner result can still collapse intentionally,
but it must never be horizontally squeezed solely by message-bubble chrome.

User prompts stay right-aligned, but use a quiet intent block rather than a
large bubble. Assistant results are left-aligned, full-width documents with a
small provenance header. This retains conversational scanning without
pretending every agent result is a chat sentence.

### 5.3 Turn anatomy

Each non-system turn is composed from consistent blocks, in order:

1. **Intent block** — the user's prompt, attachments, and turn-command label.
   It is compact, right-aligned, and expands for long text or files.
2. **Live activity block** — only while the turn is active. It reuses the
   existing truthful `LiveActivityBar` state and moves the dominant status
   from inside the composer to the active turn as well. The composer retains a
   small status for accessibility and stop control.
3. **Result header** — assistant/model name, completion status, duration, and
   a lens-specific one-line summary. It is visible for completed substantive
   turns, not hover-only. Secondary actions remain hover/focus revealed.
4. **Structured result** — ordered reasoning, tool, search, display, and
   artifact blocks. Blocks stay in stream order and independently choose a
   reading or work measure.
5. **Outcome and next action** — review status, citations, error/retry,
   follow-up chips, or a terse “nothing further required” end marker. Only
   data that exists is rendered.

System notices remain compact centered pills, as they are today. They must not
be promoted to assistant output.

### 5.4 Inspector tabs

The inspector has a shared tab set, with unavailable tabs omitted rather than
disabled. It accepts a focus request from a turn, but a focus request never
opens a persistent panel against an explicit user close.

| Tab | Appears when | Content |
|---|---|---|
| Context | Always | project roots summary, persona description, active model, brain binding, approval mode |
| Sources | Current selected turn has search/RAG references | citations, source preview, retrieval chunks, confidence/coverage when supplied |
| Activity | Current/selected turn has tools or agent events | chronological tool and agent activity, durations, errors, approvals |
| Outline | Writing lens has a generated/attached structured outline | sections, selected section, tone/length settings |
| Review | Coding profile has changed files or a review outcome | compact change list and link to the existing diff stage |

`Context` is intentionally useful even when there are no special persona
capabilities. It makes the default UI explain itself.

## 6. Persona presentation lenses

### 6.1 Contract

Add a server-emitted presentation hint alongside `EffectiveProfile`:

```ts
type PresentationLens = 'general' | 'code' | 'research' | 'operations' | 'writing'

interface EffectiveProfile {
  persona_id: string
  tool_profile: 'base' | 'code' | 'minimal' | 'control'
  affordances: Affordance[]
  presentation_lens: PresentationLens
}
```

The runtime resolves this from the effective persona/profile. It is returned
where `effective_profile` is already returned (session load and WS completion),
and defaults to `general` when no persona binds. The web client never derives
the lens from a persona name or category. This preserves the existing rule that
UI capability gates must consume server truth.

`presentation_lens` grants no tool, project root, brain access, or approval
state. A custom persona may have `presentation_lens: 'writing'` and zero extra
affordances. Conversely, a custom persona with coding affordances must resolve
to the `code` lens even if its display name does not say “developer.”

### 6.2 Lens matrix

| Lens | Main emphasis | Inspector default | Composer additions | Never do |
|---|---|---|---|---|
| General | readable conversation and clear current context | closed | contextual mode summary | show empty operational/coding chrome |
| Code | task execution, agent activity, code/result artifacts, review | Activity when a turn is active; Review when changes await action | project context, profile-authorized fan-out/attachments | remove chat or create a permanent IDE |
| Research | synthesis, citations, source coverage, uncertainty | Sources when the selected turn has sources | web/search affordances only if profile-authorized | fabricate confidence or sources |
| Operations | incident/task state, approvals, scheduled work, ownership | Activity when actual runtime events exist | approval mode and safe action visibility | render a dashboard without live state |
| Writing | draft, outline, tone, revision intent | Outline when structured writing content exists | tone/length controls when supported by the turn | replace the transcript with a document editor |

All lenses use the same title location, session context bar, transcript order,
composer silhouette, keyboard shortcuts, and status vocabulary.

### 6.3 General lens — the improved default

The general lens is the reference experience. It should answer “what chat am I
in, what context will my next turn use, and what happened?” without visual
training.

- The context bar shows neutral labels such as `No project`, `General`,
  `Model`, and `Brain off` rather than hiding configuration entirely.
- The empty state offers three task-oriented starts: ask, plan, and explore.
  Recent sessions remain below these actions, not as the primary content.
- Completed assistant turns show the model/status header and a compact summary
  of actual work performed; simple answers remain almost as light as plain
  chat.
- The inspector stays closed until the user selects context/search or a turn
  supplies inspectable material.
- The composer surfaces the active persona and project as removable context
  chips above its input only when changing them is the likely next action.

### 6.4 Code lens — conversation-first workbench

The code lens fulfills the existing workbench decision; it does not eliminate
chat. The transcript remains the widest persistent surface.

- The context bar foregrounds project name, root summary, branch/working-tree
  state when available, persona, and model.
- A live turn elevates its task plan/agent activity immediately below the
  intent block. Short tools remain inline; a diff, preview, or long-running
  terminal operation triggers the existing emergent stage.
- The Activity tab collects tool chronology and parallel-agent state. The
  existing activity rail remains a fast navigation affordance for Files,
  Terminal, Changes, and Preview; it is not a replacement transcript.
- Changed files produce a compact turn-local change summary with an explicit
  `Review changes` entry point. Pending review pins the existing diff stage;
  all other stages may auto-dismiss as defined by the workbench design.
- Code blocks and command output take the work measure. Long output collapses
  with a line/count summary and an explicit expand/copy action.

### 6.5 Research lens — evidence desk

Research adds an evidence hierarchy, not a second note-taking application.

- The assistant result starts with the answer/synthesis, followed by claims
  with inline source markers. Source cards must remain linked to the exact
  producing turn.
- `Sources` opens only when a selected turn contains citations or retrieved
  chunks. It lists title, publisher/domain, date when supplied, and the
  excerpt used; it never invents a reliability score.
- If the runtime supplies coverage, confidence, disagreement, or a search
  failure, render it as labeled evidence metadata. If it does not, omit it.
- Follow-up chips favor evidence actions: compare sources, inspect a claim,
  widen the search, or save the result to knowledge. These are suggestions,
  not hidden commands.

### 6.6 Operations lens — decision-safe control room

Operations elevates state that needs a human decision, while preserving calm.

- A running task renders a chronological activity view with actor, action,
  target, state, and elapsed time. The transcript remains the audit-friendly
  explanation of why the action occurred.
- Pending approval or path access becomes the focal card directly after the
  related activity, with a clear consequence and primary approve/deny action.
  It also creates an Activity inspector entry.
- Completed automation/scheduled-work information is shown only when sent by
  the runtime; do not add static “healthy” tiles to a chat page.
- Errors retain retry/recovery guidance and link to the originating tool
  context. A failure must never look like a successful assistant result.

### 6.7 Writing lens — editorial conversation

Writing keeps a conversation loop but makes revision easier to understand.

- The assistant result is a clean editorial document block: title/intent,
  draft, then optional revision notes. It uses the reading measure by default
  and the work measure only for comparison or wide structured content.
- If the turn includes a real structured outline, the Outline inspector lets
  the user jump to sections. It is not synthesized from arbitrary prose.
- Tone, audience, and length controls live as small composer popovers or
  turn-scoped chips. They modify future instructions; they do not silently
  rewrite earlier text.
- Revision comparisons are displayed as inline diff artifacts or the shared
  review surface. A full document editor is out of scope for this chat route.

## 7. Composer redesign

The composer stays rich-text and bottom-anchored, but adopts three tiers:

1. **Context strip** above the input — attached files, knowledge/memory
   references, and changed session context. It collapses to a count when it
   wraps beyond two rows.
2. **Input plane** — the primary writing field and live state. During a turn,
   keep the status label, elapsed time, and Stop control close to the input so
   the user can interrupt without hunting.
3. **Action rail** below the input — model, persona, brain, approval, and
   profile-authorized tools on the left; queued count and send/stop on the
   right. The active selection is summarized in the context bar, so these are
   controls rather than the only labels explaining the state.

On compact width, the action rail keeps Persona, Model, attach, and Send
visible. Brain, approval, parameters, and fan-out move into an overflow menu
with accessible names and current-value summaries. Controls hidden by an
effective profile are omitted, not disabled.

Slash-command suggestions open above the composer as today. Their commands are
grouped visually by local, session, and turn action, matching the existing
execution classes without exposing transport details.

## 8. Component architecture

The redesign is a composition refactor, not a new chat data model. The target
component boundaries are:

| Component | Responsibility | Primary inputs |
|---|---|---|
| `ChatShell` | places context bar, transcript, inspector, composer; preserves route-level connection/error handling | session state, `EffectiveProfile` |
| `ChatContextBar` | session title and concise project/persona/model/brain/connection context | session bindings, profile, queries |
| `TranscriptViewport` | scroll anchoring, row rendering, reading/work lane selection | existing `buildChatRows`, stream state |
| `TurnFrame` | shared intent/activity/result/outcome anatomy | `ChatMessage`, selection/focus state, lens |
| `TurnResultHeader` | visible provenance and outcome summary | message metadata, model, lens |
| `ChatInspector` | tabbed global/turn context; honors user close/pin | selected turn, profile, facts from blocks |
| `useChatPresentation` | derives declarative display choices from `effective_profile.presentation_lens` and real turn data | `EffectiveProfile`, messages |
| `ChatComposer` | retains editor/send protocol; composes context strip/input/action rail | current props + profile |

Existing components remain the owners of their domains:

- `WorkbenchShell`, `ActivityRail`, and stage components own coding stage
  emergence and do not move into persona picker code.
- `BlockStream`, tool renderers, `SearchGrounding`, artifacts, approval/path
  cards, and error cards retain rendering responsibility. They receive lane
  class props or render variants; they are not duplicated per persona.
- `PortalPanel` remains the host for global search/knowledge browsing. The
  Chat Inspector may reuse its view/state primitives, but it must not conflate
  global navigation history with a turn-local evidence selection.
- `useEffectiveProfile` remains the one capability gate. New visual behavior
  consumes `presentation_lens` only after that server value is available.

## 9. State and interaction rules

### 9.1 Session and persona changes

- Selecting a persona updates the context bar immediately as a pending local
  selection and uses the server-confirmed effective profile after a turn or
  session reload.
- A lens transition inserts no fake chat message. The bar may show a quiet
  transient “Now using Research” confirmation. Existing transcript content and
  block order are never reinterpreted; the shared reading/work lane remains
  stable, while future turn-local emphasis and inspector focus use the newly
  confirmed lens. Do not persist a presentation lens per historical message
  merely to restyle old rows.
- Selecting a session restores project, persona, brain binding, and effective
  profile before the user sends the next message. The transcript must not
  remount while these values hydrate.

### 9.2 Inspector behavior

- A user opening a tab pins it for that browser session until closed or a
  different tab is explicitly chosen.
- A turn may request focus (for example, `Sources` after a research result),
  but auto-open occurs only if the inspector has not been explicitly closed in
  the current session and the requested tab has real content.
- On an unavailable tab, preserve the canvas width; never show an empty
  placeholder rail merely to sustain a persona look.

### 9.3 Streaming, approvals, and errors

- The trailing active turn gets the only persistent live indicator. Older
  completed turns never animate.
- Tool approval and path access remain hard interaction blocks in transcript
  order. They get a corresponding Activity entry but are never hidden behind
  the inspector.
- Stop retains partial output and displays its existing interrupted state.
  Reconnect/session-load failures remain explicit and do not erase rendered
  content.
- Retry uses the existing retry behavior and returns focus to the relevant
  turn; it must not create a visually duplicate user intent block.

### 9.4 Responsive behavior

| Viewport | Behavior |
|---|---|
| ≥1280px | sidebar, work lane, optional inspector; coding stage overlays the remaining canvas as today |
| 900–1279px | inspector becomes an overlay/drawer; work lane fills the canvas |
| <900px | app sidebar remains existing mobile sheet; inspector and coding stage are full-height sheets; context bar horizontally scrolls only compact chips, never the transcript |

No persona gets a mobile-specific information architecture. The same context
and turn anatomy reflow into a single column.

## 10. Accessibility and visual system

- Use the project’s Oxi semantic utilities (`bg-surface`, `text-text`,
  `border-line`, status utilities) and existing light/dark token handling;
  do not introduce persona-specific raw colors or `dark:` component classes.
- Persona mode uses icon + localized text. Color may reinforce the mode but
  cannot be its only identifier.
- The context bar, inspector tabs, stage launchers, and composer overflow are
  fully keyboard reachable with visible focus. `Escape` closes only the topmost
  inspector/stage/popover, never the whole chat.
- Preserve the `role=log` transcript semantics; live updates use concise
  `aria-live` status, not a re-announcement of streaming answer text.
- Respect reduced motion for live pulses, stage transitions, and inspector
  opening. Status remains visible without animation.
- Korean and English strings are designed together. Tool/API error text stays
  English at its source; the surrounding action guidance is localized.

## 11. Data, API, and persistence impact

Most work is web-only. The only proposed cross-layer addition is
`presentation_lens` on the existing effective-profile snapshot.

| Concern | Decision |
|---|---|
| Persona selection | Existing session-scoped `active_persona_id`; no new route |
| Capability/authority | Existing effective profile, RBAC, approvals, project roots; unchanged |
| Lens | Server-derived `presentation_lens`; UI-only, no authority effect |
| Inspector selection/width | Browser-local UI state; not persisted to session or prompt |
| Selected turn in inspector | Browser-local; derived from clicked/focused transcript turn |
| Existing sessions/profile missing lens | Treat as `general` until the next server snapshot arrives |
| Message persistence | Existing message/block contracts; no migration solely for presentation |

This makes rollout safe: the revised client has a useful General lens against
an older server, and the server can add the field without changing tool
execution semantics.

## 12. Implementation sequence for the follow-up session

1. Add `presentation_lens` to the effective-profile contract and its default;
   test every API/WS/session restore path. Do not gate tools from this value.
2. Extract the route’s fixed toolbar into `ChatContextBar`, then introduce
   `ChatShell` without changing streaming, session load, or workbench-stage
   behavior.
3. Refactor transcript rows into `TurnFrame` and apply reading/work lane
   classes to structured blocks. Keep `MessageView` role dispatch intact.
4. Add `ChatInspector` with Context and Activity first. Add Sources, Outline,
   and Review only by reusing existing real data producers.
5. Refactor the composer visually into its three tiers while preserving its
   current protocol, slash commands, Tiptap behavior, and focus shortcuts.
6. Add the lens-specific presentation rules one at a time: General → Code →
   Research → Operations → Writing. Each lens begins with an empty-state,
   normal-completion, streaming, error, and narrow-viewport check.
7. Run accessibility and visual regression checks in light/dark themes, then
   remove only chrome made obsolete by the new context bar.

## 13. Verification criteria

### Unit and component coverage

- `useChatPresentation` maps every returned lens and safely defaults missing
  profiles/lenses to General.
- Profile changes alter presentation chrome without unmounting the transcript
  container or losing scroll state.
- A code block/table/tool result selects the work measure; plain prose remains
  on the reading measure.
- Context bar reflects session project/persona/model/brain state and has clear
  empty states.
- Inspector tab availability is driven by actual turn data; explicit close
  prevents unwanted auto-reopen.
- Each lens renders its special surface only from real data (citations,
  structured outline, tools/approvals, changes). No-data states remain
  General-like.
- Composer overflow preserves keyboard access and correct active-value labels.

### Browser smoke scenarios

1. New, unbound chat: understand `General`, `No project`, selected model, and
   how to begin without opening a settings page.
2. General chat with a long markdown table and code block: content is readable
   at desktop work width and remains usable with the inspector open.
3. Coding session: send a task, inspect live tool activity, open a changed-file
   review, close the stage, and continue chatting without lost context.
4. Research session: complete a cited answer, inspect Sources, close the
   inspector, and verify the answer remains self-contained.
5. Operations session: surface an approval, decide it in transcript order, and
   verify final activity/error states remain visible.
6. Writing session: generate an outline/draft, jump through a real outline,
   change tone for the next turn, and verify prior content is unchanged.
7. Switch persona and then reload the session: the server-confirmed lens and
   existing transcript restore without a blank/remounted canvas.
8. Repeat key scenarios in dark mode, at 1024px, and on a mobile-width layout.

## 14. Non-goals

- No separate `/code`, `/research`, `/writer`, or `/ops` chat routes.
- No persona-specific authority, filesystem scope, model routing, or tool
  grants.
- No fixed IDE file tree, permanent terminal, or permanently visible dashboard.
- No hidden automatic document rewrite, source scoring, or activity synthesis.
- No replacement of the global Portal search/knowledge experience.
- No backend transport rewrite or changes to cancellation/reconnect semantics.

## 15. Acceptance criteria

1. A user can identify the active session context before sending a message.
2. The default chat is materially clearer and more capable without selecting a
   persona or project.
3. Persona/profile changes are visibly useful but retain one transcript,
   composer, route, and state model.
4. Coding remains a conversation-first workbench; it never becomes chatless.
5. Rich assistant output is readable at an appropriate width, while prose
   remains comfortable to read.
6. Evidence, approvals, activity, reviews, and outlines appear only when
   backed by actual runtime/message data.
7. Lens presentation never changes permission, tool availability, or project
   scope.
