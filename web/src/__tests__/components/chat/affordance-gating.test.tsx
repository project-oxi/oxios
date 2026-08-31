// Task 6 (project-roots persona workbench): profile-variant gating of every
// conditional Studio control.
//
// The Studio shell renders for real (StudioShell + ChatInput + BlockStream)
// and the ONLY lever is the server-derived EffectiveProfile in the chat
// store — no persona name/category/capability-string inference anywhere:
//   - code + roots  → terminal toggle + fan-out button visible
//   - code, folderless → terminal/fan-out hidden, diff still renders
//   - minimal / no profile → all conditional controls hidden
//   - a persona switch (profile change) must never unmount the transcript

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import type { ReactNode } from 'react'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { BlockStream } from '@/components/chat/messages/components/BlockStream'
import { StudioShell } from '@/components/studio/studio-shell'
import { useChatStore } from '@/stores/chat'
import type { ChatBlock, EffectiveProfile } from '@/types'
import { server } from '../../msw/server'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

// jsdom reports `ontouchstart`, which useIsTouch would read as a touch
// device and change composer keyboard behavior.
vi.mock('@/hooks/use-is-touch', () => ({ useIsTouch: () => false }))

const CODE_ROOTS: EffectiveProfile = {
  persona_id: 'dev',
  tool_profile: 'code',
  affordances: ['diff', 'files', 'terminal', 'worktree-fanout'],
  presentation_lens: 'code',
}

const CODE_FOLDERLESS: EffectiveProfile = {
  persona_id: 'dev',
  tool_profile: 'code',
  affordances: ['diff'],
  presentation_lens: 'code',
}

const MINIMAL: EffectiveProfile = {
  persona_id: 'writer',
  tool_profile: 'minimal',
  affordances: [],
  presentation_lens: 'writing',
}

// Inert WebSocket: connect() attaches handlers and waits — nothing opens,
// nothing fires timers, and the shell renders in its "reconnecting" state.
class FakeWebSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readyState = FakeWebSocket.CONNECTING
  onopen: unknown = null
  onmessage: unknown = null
  onclose: unknown = null
  onerror: unknown = null
  send() {}
  close() {}
}

beforeAll(() => {
  vi.stubGlobal('WebSocket', FakeWebSocket)
  if (!window.matchMedia) {
    window.matchMedia = (query: string) =>
      ({
        matches: false,
        media: query,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList
  }
  // jsdom has no layout: ProseMirror's scroll math needs 1×1 rects.
  for (const proto of [Text.prototype, Range.prototype] as unknown as Array<{
    getClientRects?: unknown
    getBoundingClientRect?: unknown
  }>) {
    proto.getClientRects ??= () => [new DOMRect(0, 0, 1, 1)] as unknown as DOMRectList
    proto.getBoundingClientRect ??= () => new DOMRect(0, 0, 1, 1)
  }
})

afterAll(() => {
  vi.unstubAllGlobals()
})

beforeEach(() => {
  // Boot-endpoint stubs so the shell mounts against deterministic data.
  server.use(
    http.get('/api/status', () => HttpResponse.json({ auth_enabled: false })),
    http.get('/api/engine/roles', () => HttpResponse.json({ roles: {} })),
    http.get('/api/engine/config', () => HttpResponse.json({ routingEnabled: false })),
    http.get('/api/engine/models', () => HttpResponse.json({ models: [], providers: [] })),
    http.get('/api/personas', () => HttpResponse.json([])),
    http.get('/api/personas/active', () => HttpResponse.json(null)),
    http.get('/api/sessions', () => HttpResponse.json({ items: [], total: 0 })),
  )
  useChatStore.getState().disconnect()
  useChatStore.setState({
    activeSessionId: 'sess-gating',
    effectiveProfile: null,
    messages: [],
    isLoadingSession: false,
    sessionLoadError: null,
  })
})

afterEach(() => {
  cleanup()
})

function renderShell() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<StudioShell />, {
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    ),
  })
}

function setProfile(profile: EffectiveProfile) {
  act(() => {
    useChatStore.setState({ effectiveProfile: profile })
  })
}

const TERMINAL_TOGGLE = 'chat.terminal.title'
const FANOUT_BUTTON = 'chat.fanout.title'

describe('chat shell affordance gating (per EffectiveProfile variant)', () => {
  it('code + roots shows the terminal toggle and the fan-out button', async () => {
    setProfile(CODE_ROOTS)
    renderShell()
    await waitFor(() => expect(screen.queryByTitle(TERMINAL_TOGGLE)).not.toBeNull())
    expect(screen.queryByTitle(FANOUT_BUTTON)).not.toBeNull()
  })

  it('folderless code hides terminal and fan-out (diff still renders below)', async () => {
    setProfile(CODE_FOLDERLESS)
    renderShell()
    await waitFor(() => expect(screen.queryByTitle(TERMINAL_TOGGLE)).toBeNull())
    expect(screen.queryByTitle(FANOUT_BUTTON)).toBeNull()
  })

  it('minimal hides all conditional controls', async () => {
    setProfile(MINIMAL)
    renderShell()
    await waitFor(() => expect(screen.queryByTitle(TERMINAL_TOGGLE)).toBeNull())
    expect(screen.queryByTitle(FANOUT_BUTTON)).toBeNull()
  })

  it('profile absent (no persona binds) hides all conditional controls', async () => {
    renderShell()
    await waitFor(() => expect(useChatStore.getState().activeSessionId).toBe('sess-gating'))
    expect(screen.queryByTitle(TERMINAL_TOGGLE)).toBeNull()
    expect(screen.queryByTitle(FANOUT_BUTTON)).toBeNull()
  })

  it('persona switch swaps affordances but never unmounts the transcript', async () => {
    // Seeded transcript row: its content must survive the in-route persona
    // swap — the shell keeps the conversation, not just an empty log node.
    useChatStore.setState({
      messages: [
        {
          id: 'u-seed',
          role: 'user' as const,
          content: 'TRANSCRIPT_ROW_KEEP',
          timestamp: new Date().toISOString(),
        },
      ],
    })
    setProfile(CODE_ROOTS)
    const { container } = renderShell()
    await waitFor(() => expect(screen.queryByTitle(FANOUT_BUTTON)).not.toBeNull())
    const transcript = container.querySelector('[role="log"]')
    expect(transcript).not.toBeNull()
    expect(transcript!.textContent).toContain('TRANSCRIPT_ROW_KEEP')
    act(() => {
      useChatStore.setState({ effectiveProfile: MINIMAL })
    })
    expect(screen.queryByTitle(FANOUT_BUTTON)).toBeNull()
    expect(screen.queryByTitle(TERMINAL_TOGGLE)).toBeNull()
    // The exact same transcript DOM node persists across the profile change —
    // the conversation is never a conditional child of the profile.
    expect(container.querySelector('[role="log"]')).toBe(transcript)
    // P-d tightening: the inner row content survives too, so the swap did
    // not merely keep a shell while discarding history.
    expect(container.querySelector('[role="log"]')!.textContent).toContain('TRANSCRIPT_ROW_KEEP')
  })
})

// ── InlineDiffViewer gate (BlockStream) ─────────────────────────────────────

const DIFF_NEW_LINE = 'TASK6_UNIQUE_NEW_LINE'
const editBlock = {
  type: 'tool',
  id: 't1',
  identifier: 'kernel',
  apiName: 'str_replace_edit',
  arguments: { path: '/src/a.ts', old_text: 'const a = 1', new_text: DIFF_NEW_LINE },
  status: 'success',
} as unknown as ChatBlock

function renderBlocks(blocks: ChatBlock[]) {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(<BlockStream blocks={blocks} messageId="m1" />, {
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    ),
  })
}

describe('BlockStream inline diff gating', () => {
  afterEach(() => {
    useChatStore.setState({ effectiveProfile: null })
  })

  it('code + roots renders the inline diff for a file edit', () => {
    setProfile(CODE_ROOTS)
    const { container } = renderBlocks([editBlock])
    expect(container.textContent).toContain(DIFF_NEW_LINE)
  })

  it('folderless code keeps the diff rendering', () => {
    setProfile(CODE_FOLDERLESS)
    const { container } = renderBlocks([editBlock])
    expect(container.textContent).toContain(DIFF_NEW_LINE)
  })

  it('minimal hides the diff (tool card still renders)', () => {
    setProfile(MINIMAL)
    const { container } = renderBlocks([editBlock])
    expect(container.textContent).not.toContain(DIFF_NEW_LINE)
  })
})
