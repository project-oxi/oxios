# LobeHub Chat Borrow (Virtualization + Shiny Thinking) Implementation Plan
> **Status**: Shipped — v1.31.x–1.33.0 (ChatMiniMap, chat UX polish)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Borrow two patterns from LobeHub into the Oxios web chat: (A) virtualize the message list with `virtua` for long-conversation performance, and (B) add a shiny animated "Thinking…" title to the reasoning block.

**Architecture:** (A) Replace the `ScrollArea` + `CompressedGroup`-wrapped message list in `routes/chat.tsx` with a `virtua` `<VList>` driven by a pure `buildChatRows()` row model. The row model flattens messages + collapse-bar + interview/approval/path-access cards into a heterogeneous row array; CompressedGroup becomes a controlled collapse toggle (option B — keep the collapse affordance, virtualization handles the perf). Auto-scroll, at-bottom detection, minimap jump, and the text-selection bar are rewired to the VList handle. (B) A CSS keyframe shine animation applied to the Thinking title while streaming.

**Tech Stack:** React 18, TypeScript, virtua (new dep), Tailwind, vitest + @testing-library/react, biome.

## Global Constraints

- Web app root is `web/`. Run commands from `web/` (`cd web && …`).
- Package manager is `bun` (`bun add`, `bun run vitest run`, `bun run build`, `bunx tsc --noEmit`, `bunx biome check`).
- Bilingual UI: any new user-facing string needs keys in BOTH `web/src/i18n/locales/en.json` and `ko.json`. (This plan adds no new visible strings — it reuses existing `chat.compressedCollapsed`/`chat.compressedExpanded`/`chat.thinking`/`chat.thought`.)
- Naming/structure follows existing conventions: components under `web/src/components/chat/`, pure helpers under `web/src/lib/`, tests co-located (`*.test.ts(x)`).
- CI gate equivalent: `bunx tsc --noEmit` (exit 0), `bunx biome check <files>` (0 errors), `bun run vitest run` (all pass), `bun run build` (exit 0).
- Do NOT change the block-stream model, StreamProcessor, or BlockStream — out of scope.
- Do NOT remove CompressedGroup (option B: keep the collapse affordance).

---

## File Structure

| File | Responsibility | Action |
|---|---|---|
| `web/src/lib/chat-rows.ts` | Pure `ChatRow` type + `buildChatRows()` row-model builder | Create |
| `web/src/lib/chat-rows.test.ts` | Unit tests for `buildChatRows` | Create |
| `web/src/components/chat/compressed-group.tsx` | Collapse toggle bar — refactor to controlled (`expanded`/`onToggle`), drop `children` | Modify |
| `web/src/components/chat/compressed-group.test.tsx` | Controlled expand/collapse test | Create |
| `web/src/routes/chat.tsx` | Replace ScrollArea list with VList; wire rows, auto-scroll, minimap, selection bar | Modify |
| `web/src/index.css` | `@keyframes thinking-shine` + `.thinking-shiny` utility | Modify |
| `web/src/components/chat/thinking/index.tsx` | Apply `thinking-shiny` to the title while streaming | Modify |
| `web/src/components/chat/thinking/index.test.tsx` | Shiny-class-applied test | Create |
| `web/package.json` | Add `virtua` dependency | Modify (via `bun add`) |

---

## Task 1: Shiny "Thinking…" title (B)

**Files:**
- Modify: `web/src/index.css` (append keyframes + utility)
- Modify: `web/src/components/chat/thinking/index.tsx` (`ThinkingTitle`)
- Test: `web/src/components/chat/thinking/index.test.tsx`

**Interfaces:**
- Consumes: existing `Thinking` component (`ThinkingProps { content?, thinking?, duration?, messageId?, className? }`).
- Produces: when `thinking === true`, the title label `<span>` carries class `thinking-shiny`; when `false`, it does not.

- [ ] **Step 1: Write the failing test**

Create `web/src/components/chat/thinking/index.test.tsx`:

```tsx
import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { Thinking } from './index'

describe('Thinking shiny title', () => {
  it('applies the shiny animation class to the title while thinking', () => {
    render(<Thinking content="some reasoning" thinking />)
    // The streaming label is chat.thinking ("Thinking..."); i18n is stubbed to
    // return the key in tests, so match by the shiny class on a span.
    const shiny = document.querySelector('.thinking-shiny')
    expect(shiny).not.toBeNull()
  })

  it('does not apply the shiny class once thinking is done', () => {
    render(<Thinking content="some reasoning" thinking={false} duration={1200} />)
    expect(document.querySelector('.thinking-shiny')).toBeNull()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/components/chat/thinking/index.test.tsx`
Expected: FAIL — no element has class `thinking-shiny` (class not yet added).

- [ ] **Step 3: Add the CSS animation**

Append to `web/src/index.css`:

```css
/* Shiny sweep for the active "Thinking…" reasoning title (LobeHub borrow). */
@keyframes thinking-shine {
  to {
    background-position: 200% center;
  }
}
.thinking-shiny {
  background: linear-gradient(
    90deg,
    currentColor 40%,
    var(--foreground) 50%,
    currentColor 60%
  );
  background-size: 200% auto;
  -webkit-background-clip: text;
  background-clip: text;
  color: transparent;
  animation: thinking-shine 2s linear infinite;
}
@media (prefers-reduced-motion: reduce) {
  .thinking-shiny {
    animation: none;
  }
}
```

- [ ] **Step 4: Apply the class in ThinkingTitle**

In `web/src/components/chat/thinking/index.tsx`, in `ThinkingTitle`, change the label span to:

```tsx
<span className={cn('font-medium', thinking && 'thinking-shiny')}>
  {thinking ? t('chat.thinking') : t('chat.thought')}
</span>
```

(Ensure `cn` is imported — it already is in this file.)

- [ ] **Step 5: Run test to verify it passes**

Run: `cd web && bun run vitest run src/components/chat/thinking/index.test.tsx`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
cd web && git add src/index.css src/components/chat/thinking/index.tsx src/components/chat/thinking/index.test.tsx
git commit -m "feat(web): shiny animated Thinking title while reasoning streams"
```

---

## Task 2: Row model — `buildChatRows` (pure, A)

**Files:**
- Create: `web/src/lib/chat-rows.ts`
- Test: `web/src/lib/chat-rows.test.ts`

**Interfaces:**
- Consumes: `ChatMessage` from `@/types`.
- Produces:
  ```ts
  export type ChatRow =
    | { kind: 'empty' }
    | { kind: 'collapse-bar'; count: number }
    | { kind: 'message'; message: ChatMessage; index: number }
    | { kind: 'interview' }
    | { kind: 'tool-approval' }
    | { kind: 'path-access' }

  export interface BuildChatRowsOptions {
    messages: ChatMessage[]
    expanded: boolean
    collapseThreshold: number
    visibleTail: number
    hasInterview: boolean
    hasToolApproval: boolean
    hasPathAccess: boolean
  }
  export function buildChatRows(opts: BuildChatRowsOptions): ChatRow[]
  ```
  Semantics: `collapseCount = messages.length > collapseThreshold ? messages.length - visibleTail : 0`. If no messages and no cards → `[{kind:'empty'}]`. If `collapseCount > 0`: push `{kind:'collapse-bar', count}`, then messages from `expanded ? 0 : collapseCount` to end (each `{kind:'message', message, index}`). Else all messages. Then append interview/tool-approval/path-access rows in that order when their flag is set. `index` is the message's position in the FULL `messages` array (used for `assistantIndex` + minimap).

- [ ] **Step 1: Write the failing tests**

Create `web/src/lib/chat-rows.test.ts`:

```ts
import { describe, expect, it } from 'vitest'
import type { ChatMessage } from '@/types'
import { buildChatRows } from './chat-rows'

const msg = (id: string, role: 'user' | 'assistant' = 'user'): ChatMessage => ({
  id,
  role,
  content: id,
  timestamp: '',
})

const base = {
  expanded: false,
  collapseThreshold: 40,
  visibleTail: 20,
  hasInterview: false,
  hasToolApproval: false,
  hasPathAccess: false,
}

describe('buildChatRows', () => {
  it('returns a single empty row when there are no messages or cards', () => {
    expect(buildChatRows({ ...base, messages: [] })).toEqual([{ kind: 'empty' }])
  })

  it('lists all messages below the collapse threshold (no bar)', () => {
    const messages = [msg('u1'), msg('a1', 'assistant'), msg('u2')]
    const rows = buildChatRows({ ...base, messages })
    expect(rows.map((r) => r.kind)).toEqual(['message', 'message', 'message'])
    expect(rows[0]).toMatchObject({ kind: 'message', index: 0 })
    expect(rows[2]).toMatchObject({ kind: 'message', index: 2 })
  })

  it('collapses older messages behind a bar when over threshold', () => {
    const messages = Array.from({ length: 45 }, (_, i) => msg(`m${i}`))
    const rows = buildChatRows({ ...base, messages })
    // collapseCount = 45 - 20 = 25 → bar + last 20 messages.
    expect(rows[0]).toEqual({ kind: 'collapse-bar', count: 25 })
    expect(rows).toHaveLength(21)
    expect(rows[1]).toMatchObject({ kind: 'message', index: 25 })
    expect(rows[20]).toMatchObject({ kind: 'message', index: 44 })
  })

  it('expands to all messages (bar stays, full list follows) when expanded', () => {
    const messages = Array.from({ length: 45 }, (_, i) => msg(`m${i}`))
    const rows = buildChatRows({ ...base, messages, expanded: true })
    expect(rows[0]).toEqual({ kind: 'collapse-bar', count: 25 })
    expect(rows).toHaveLength(46) // bar + 45 messages
    expect(rows[1]).toMatchObject({ kind: 'message', index: 0 })
  })

  it('appends interview/approval/path-access rows after messages', () => {
    const rows = buildChatRows({
      ...base,
      messages: [msg('u1')],
      hasInterview: true,
      hasToolApproval: true,
      hasPathAccess: true,
    })
    expect(rows.map((r) => r.kind)).toEqual([
      'message',
      'interview',
      'tool-approval',
      'path-access',
    ])
  })

  it('shows cards even with no messages (no empty row)', () => {
    const rows = buildChatRows({ ...base, messages: [], hasToolApproval: true })
    expect(rows.map((r) => r.kind)).toEqual(['tool-approval'])
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/lib/chat-rows.test.ts`
Expected: FAIL — cannot resolve `./chat-rows`.

- [ ] **Step 3: Implement `buildChatRows`**

Create `web/src/lib/chat-rows.ts`:

```ts
// chat-rows — pure row model for the virtualized chat list (LobeHub borrow).
//
// Flattens the conversation into a heterogeneous row array consumed by virtua's
// <VList>: an optional collapse bar (older messages folded past a threshold),
// the message rows, and any active intervention cards (interview / tool
// approval / path access). `index` on a message row is its position in the
// FULL messages array — used to derive assistantIndex and minimap jumps.

import type { ChatMessage } from '@/types'

/** One renderable row in the virtualized chat list. */
export type ChatRow =
  | { kind: 'empty' }
  | { kind: 'collapse-bar'; count: number }
  | { kind: 'message'; message: ChatMessage; index: number }
  | { kind: 'interview' }
  | { kind: 'tool-approval' }
  | { kind: 'path-access' }

export interface BuildChatRowsOptions {
  messages: ChatMessage[]
  /** Whether the collapse group is expanded (show all messages). */
  expanded: boolean
  /** Message count above which older messages collapse. */
  collapseThreshold: number
  /** Number of recent messages kept visible when collapsed. */
  visibleTail: number
  hasInterview: boolean
  hasToolApproval: boolean
  hasPathAccess: boolean
}

export function buildChatRows(opts: BuildChatRowsOptions): ChatRow[] {
  const { messages, expanded, collapseThreshold, visibleTail } = opts
  const hasCard = opts.hasInterview || opts.hasToolApproval || opts.hasPathAccess

  if (messages.length === 0 && !hasCard) return [{ kind: 'empty' }]

  const rows: ChatRow[] = []
  const collapseCount =
    messages.length > collapseThreshold ? messages.length - visibleTail : 0

  if (collapseCount > 0) {
    rows.push({ kind: 'collapse-bar', count: collapseCount })
    const start = expanded ? 0 : collapseCount
    for (let i = start; i < messages.length; i++) {
      rows.push({ kind: 'message', message: messages[i]!, index: i })
    }
  } else {
    for (let i = 0; i < messages.length; i++) {
      rows.push({ kind: 'message', message: messages[i]!, index: i })
    }
  }

  if (opts.hasInterview) rows.push({ kind: 'interview' })
  if (opts.hasToolApproval) rows.push({ kind: 'tool-approval' })
  if (opts.hasPathAccess) rows.push({ kind: 'path-access' })
  return rows
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd web && bun run vitest run src/lib/chat-rows.test.ts`
Expected: PASS (6 tests).

- [ ] **Step 5: Commit**

```bash
cd web && git add src/lib/chat-rows.ts src/lib/chat-rows.test.ts
git commit -m "feat(web): pure chat row model for virtualized list"
```

---

## Task 3: CompressedGroup → controlled toggle (A)

**Files:**
- Modify: `web/src/components/chat/compressed-group.tsx`
- Test: `web/src/components/chat/compressed-group.test.tsx`

**Interfaces:**
- New props: `{ count: number; expanded: boolean; onToggle: () => void; className?: string }`. The component renders ONLY the toggle button (chevron + MessagesSquare + label); it no longer manages internal state or renders `children`. Label: `expanded ? t('chat.compressedExpanded') : t('chat.compressedCollapsed', { count })`.

- [ ] **Step 1: Write the failing test**

Create `web/src/components/chat/compressed-group.test.tsx`:

```tsx
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { CompressedGroup } from './compressed-group'

describe('CompressedGroup (controlled)', () => {
  it('calls onToggle when clicked and reflects the expanded prop', () => {
    const onToggle = vi.fn()
    const { rerender } = render(
      <CompressedGroup count={25} expanded={false} onToggle={onToggle} />,
    )
    fireEvent.click(screen.getByRole('button'))
    expect(onToggle).toHaveBeenCalledTimes(1)
    // Re-render expanded — still a single toggle button, no crash.
    rerender(<CompressedGroup count={25} expanded onToggle={onToggle} />)
    expect(screen.getByRole('button')).toBeTruthy()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd web && bun run vitest run src/components/chat/compressed-group.test.tsx`
Expected: FAIL — `CompressedGroup` currently requires `children` and has no `expanded`/`onToggle` props (type error / wrong behavior).

- [ ] **Step 3: Refactor CompressedGroup to controlled**

Replace `web/src/components/chat/compressed-group.tsx` with:

```tsx
// CompressedGroup — controlled collapse toggle for older messages in long
// conversations (LobeHub analogue: Messages/CompressedGroup).
//
// The virtualized chat list (routes/chat.tsx) owns the expanded state and the
// row model: when collapsed, older messages are omitted from the VList and this
// bar is the first row; when expanded, all messages render as rows. This
// component is purely the toggle affordance — no internal state, no children.

import { ChevronDown, ChevronRight, MessagesSquare } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'

interface CompressedGroupProps {
  /** Number of messages hidden while collapsed. */
  count: number
  expanded: boolean
  onToggle: () => void
  className?: string
}

export function CompressedGroup({ count, expanded, onToggle, className }: CompressedGroupProps) {
  const { t } = useTranslation()
  return (
    <button
      type="button"
      onClick={onToggle}
      className={cn(
        'flex w-full items-center gap-2 rounded-lg border border-dashed bg-muted/30 px-3 py-2 text-xs text-muted-foreground transition-colors hover:bg-muted/60',
        className,
      )}
    >
      {expanded ? (
        <ChevronDown className="size-3.5 shrink-0" />
      ) : (
        <ChevronRight className="size-3.5 shrink-0" />
      )}
      <MessagesSquare className="size-3.5 shrink-0" />
      <span>{expanded ? t('chat.compressedExpanded') : t('chat.compressedCollapsed', { count })}</span>
    </button>
  )
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd web && bun run vitest run src/components/chat/compressed-group.test.tsx`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd web && git add src/components/chat/compressed-group.tsx src/components/chat/compressed-group.test.tsx
git commit -m "refactor(web): CompressedGroup as controlled collapse toggle"
```

---

## Task 4: Virtualize the chat list with virtua (A — integration)

**Files:**
- Modify: `web/package.json` (`bun add virtua`)
- Modify: `web/src/routes/chat.tsx`

**Interfaces:**
- Consumes: `buildChatRows`/`ChatRow` (Task 2), controlled `CompressedGroup` (Task 3), `virtua`'s `VList` + `VListHandle`.
- virtua API used: `<VList ref onScroll className role aria-label>`; handle methods `scrollToIndex(index, { align?, smooth? })`; getters `scrollOffset`, `viewportSize`, `scrollSize`. (Verify exact names against the installed `virtua` types before relying on them — `cd web && bunx tsc --noEmit` will surface mismatches.)

This task is integration-heavy and not unit-testable (DOM + virtua). Verify via typecheck + build + the browser pass in Task 5.

- [ ] **Step 1: Add the dependency**

```bash
cd web && bun add virtua
```

- [ ] **Step 2: Rewire imports and state in chat.tsx**

In `web/src/routes/chat.tsx`:
- Add: `import { VList, type VListHandle } from 'virtua'` and `import { buildChatRows } from '@/lib/chat-rows'`.
- Remove the `ScrollArea` import if it becomes unused elsewhere in the file (check other usages first; keep if used).
- Replace `const scrollAreaRef = useRef<HTMLDivElement>(null)` and `const bottomRef = useRef<HTMLDivElement>(null)` with:
  ```ts
  const vListRef = useRef<VListHandle>(null)
  const messagesContainerRef = useRef<HTMLDivElement>(null)
  const atBottomRef = useRef(true)
  const [expanded, setExpanded] = useState(false)
  ```
- Keep `userScrolledUp` state.

- [ ] **Step 3: Build the row model**

Add (near the existing `collapseCount` logic — remove the old `collapseCount`/`VISIBLE_TAIL` inline slicing usage):

```ts
const COLLAPSE_THRESHOLD = 40
const VISIBLE_TAIL = 20

const rows = useMemo(
  () =>
    buildChatRows({
      messages,
      expanded,
      collapseThreshold: COLLAPSE_THRESHOLD,
      visibleTail: VISIBLE_TAIL,
      hasInterview: !!activeInterview && activeInterview.length > 0,
      hasToolApproval: !!activeToolApproval,
      hasPathAccess: !!activePathAccess,
    }),
  [messages, expanded, activeInterview, activeToolApproval, activePathAccess],
)
```

(Ensure `useMemo` is imported.)

- [ ] **Step 4: Replace the auto-scroll effect**

Replace the `useEffect(() => { bottomRef.current?.scrollIntoView(...) }, [messages, isStreaming, userScrolledUp])` with:

```ts
// Auto-scroll to the last row when the user is at (or near) the bottom.
// Re-anchors as the streaming message grows (dep on last message content length).
const lastContentLen = messages.at(-1)?.content?.length ?? 0
useEffect(() => {
  if (atBottomRef.current) {
    vListRef.current?.scrollToIndex(rows.length - 1, { align: 'end' })
  }
}, [rows.length, isStreaming, lastContentLen])
```

- [ ] **Step 5: Replace the scroll handlers**

Replace `handleScroll` with a virtua `onScroll` handler:

```ts
const handleVListScroll = (offset: number) => {
  const vl = vListRef.current
  if (!vl) return
  const atBottom = vl.scrollSize - offset - vl.viewportSize < 80
  atBottomRef.current = atBottom
  setUserScrolledUp(!atBottom)
}
```

Replace `handleMiniMapJump` (querySelector-based) with index-based:

```ts
const handleMiniMapJump = (index: number) => {
  const rowIndex = rows.findIndex((r) => r.kind === 'message' && r.index === index)
  if (rowIndex >= 0) {
    vListRef.current?.scrollToIndex(rowIndex, { align: 'center', smooth: true })
  }
}
```

- [ ] **Step 6: Replace the ScrollArea JSX with VList**

Replace the `<ScrollArea …>…</ScrollArea>` block (and its inner `max-w-3xl` wrapper, CompressedGroup, message maps, cards, and `bottomRef` div) with:

```tsx
<VList
  ref={vListRef}
  onScroll={handleVListScroll}
  className="h-full"
  role="log"
  aria-label={t('common.chatMessages')}
>
  {rows.map((row) => {
    if (row.kind === 'empty') {
      return (
        <div key="empty" className="mx-auto max-w-3xl px-4 py-6">
          <EmptyChatState />
        </div>
      )
    }
    if (row.kind === 'collapse-bar') {
      return (
        <div key="collapse-bar" className="mx-auto max-w-3xl px-4 pt-6">
          <CompressedGroup
            count={row.count}
            expanded={expanded}
            onToggle={() => setExpanded((v) => !v)}
          />
        </div>
      )
    }
    if (row.kind === 'message') {
      const m = row.message
      const assistantIndex =
        m.role === 'assistant'
          ? messages.slice(0, row.index).filter((x) => x.role === 'assistant').length
          : undefined
      return (
        <div key={m.id} data-msg-index={row.index} className="mx-auto max-w-3xl px-4 py-0.5">
          <MessageBubble
            message={m}
            sessionId={activeSessionId ?? undefined}
            assistantIndex={assistantIndex}
            onRetry={m.metadata?.isError ? () => handleRetry(m.id) : undefined}
          />
        </div>
      )
    }
    if (row.kind === 'interview') {
      return (
        <div key="interview" className="mx-auto max-w-3xl px-4 py-2">
          <InterviewWizard
            questions={activeInterview!}
            round={interviewRound}
            ambiguity={interviewAmbiguity}
            onSubmit={submitInterviewResponse}
            disabled={isStreaming}
          />
        </div>
      )
    }
    if (row.kind === 'tool-approval') {
      return (
        <div key="tool-approval" className="mx-auto max-w-3xl px-4 py-2">
          <ToolApprovalCard
            toolName={activeToolApproval!.toolName}
            reason={activeToolApproval!.reason}
            onApprove={(remember) => resolveToolApproval(activeToolApproval!.id, true, remember)}
            onDeny={() => resolveToolApproval(activeToolApproval!.id, false)}
            disabled={isStreaming}
          />
        </div>
      )
    }
    // path-access
    return (
      <div key="path-access" className="mx-auto max-w-3xl px-4 py-2">
        <PathAccessCard
          path={activePathAccess!.path}
          mode={activePathAccess!.mode}
          toolName={activePathAccess!.toolName}
          reason={activePathAccess!.reason}
          onMount={() => resolvePathAccess(activePathAccess!.id, 'mount')}
          onTempAllow={() => resolvePathAccess(activePathAccess!.id, 'temp')}
          onDeny={() => resolvePathAccess(activePathAccess!.id, 'deny')}
          disabled={isStreaming}
        />
      </div>
    )
  })}
</VList>
```

- [ ] **Step 7: Rewire the scroll-to-bottom button + selection bar container**

- The scroll-to-bottom button `onClick`: replace `bottomRef.current?.scrollIntoView(...)` with `vListRef.current?.scrollToIndex(rows.length - 1, { align: 'end', smooth: true })`.
- Wrap the VList (and the overlaid buttons) in `<div ref={messagesContainerRef} className="relative flex-1 min-h-0">` and pass `containerRef={messagesContainerRef}` to `<TextSelectionBar>` (replacing `scrollAreaRef`).
- Remove the now-unused `<div ref={bottomRef} />`.

- [ ] **Step 8: Typecheck + build**

Run: `cd web && bunx tsc --noEmit && bun run build`
Expected: both exit 0. Fix any virtua API mismatches surfaced by tsc (e.g. method/prop names) against the installed `virtua` types.

- [ ] **Step 9: Lint**

Run: `cd web && bunx biome check src/routes/chat.tsx`
Expected: 0 errors (run `--write` if only formatting).

- [ ] **Step 10: Commit**

```bash
cd web && git add package.json bun.lock src/routes/chat.tsx
git commit -m "feat(web): virtualize chat message list with virtua"
```

---

## Task 5: Browser verification (A — interactive proof)

**Files:** none (verification only).

This proves the integration behaves correctly — virtualization is not verifiable by unit tests.

- [ ] **Step 1: Ensure the dev server is running** (the `webdev` daemon), open the app in the browser, navigate to `/chat`.

- [ ] **Step 2: Long-conversation scroll** — load/generate a conversation with > 40 messages. Verify: the collapse bar appears as the first row; scrolling is smooth; only visible rows are in the DOM (inspect: far fewer message nodes than total messages).

- [ ] **Step 3: Collapse/expand** — click the collapse bar: collapsed shows only the recent tail; expanded shows all messages as rows and the bar remains as a re-collapse toggle.

- [ ] **Step 4: Streaming auto-scroll** — send a message that streams a long response. Verify the view stays pinned to the bottom while streaming (when the user hasn't scrolled up), and the "scroll to bottom" button appears when scrolled up and returns to the latest on click.

- [ ] **Step 5: Minimap jump** — with > 20 messages, click a minimap marker; the list scrolls to that message (works even for off-screen messages — proves `scrollToIndex` replaced the querySelector path).

- [ ] **Step 6: Text selection bar** — select text inside a message; the selection bar appears positioned above the selection (proves the container ref rewire works).

- [ ] **Step 7: Intervention cards** — trigger (or mock) an interview / tool-approval / path-access; verify the card renders as a row after the messages and is reachable by scroll.

- [ ] **Step 8: Shiny title** — during a reasoning stream, verify the "Thinking…/생각 중…" title shows the shiny sweep animation; once done it shows "Thought/생각한 내용 · Ns" without animation.

- [ ] **Step 9: Final gate** — `cd web && bunx tsc --noEmit && bunx biome check src && bun run vitest run && bun run build` all green.

---

## Self-Review Notes

- **Spec coverage:** Borrow list = virtualization (Tasks 2–5) + shiny title (Task 1). Workflow/answer folding explicitly excluded (conflicts with Oxios transparency). ✓
- **Type consistency:** `ChatRow`/`buildChatRows`/`BuildChatRowsOptions` names are identical across Task 2 (definition) and Task 4 (consumption). `CompressedGroup` props `{count, expanded, onToggle, className}` consistent across Task 3 and Task 4. ✓
- **virtua API risk:** Task 4 Step 8 explicitly gates on tsc to catch API-name drift; the plan names the expected API but defers to the installed types.
- **Option B honored:** CompressedGroup kept as a controlled affordance (Task 3), not removed.
