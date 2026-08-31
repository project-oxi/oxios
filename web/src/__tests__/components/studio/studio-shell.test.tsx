// WT-3 Studio shell — projectless happy path, context bar neutrality,
// inspector default state, and content lanes (IA design §8).

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import type { ReactNode } from 'react'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { StudioShell } from '@/components/studio/studio-shell'
import { useChatStore } from '@/stores/chat'
import type { EffectiveProfile } from '@/types'
import { server } from '../../msw/server'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

vi.mock('@/hooks/use-is-touch', () => ({ useIsTouch: () => false }))

// Inert WebSocket: the shell renders in its reconnecting state, so no
// network turn can interfere with the assertions.
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
  vi.stubGlobal('WebSocket', FakeWebSocket as unknown as typeof WebSocket)
})

afterAll(() => vi.unstubAllGlobals())

const PROSE_MESSAGE = {
  id: 'm1',
  role: 'user' as const,
  content: 'What is a token bucket?',
  timestamp: '2026-08-31T00:00:00Z',
}

const CODE_MESSAGE = {
  id: 'm2',
  role: 'assistant' as const,
  content: 'Here is an example:\n```rust\nfn main() {}\n```\nDone.',
  timestamp: '2026-08-31T00:00:01Z',
}

beforeEach(() => {
  server.use(
    http.get('/api/status', () => HttpResponse.json({ auth_enabled: false })),
    http.get('/api/engine/roles', () => HttpResponse.json({ roles: {} })),
    http.get('/api/engine/config', () => HttpResponse.json({ routingEnabled: false })),
    http.get('/api/engine/models', () => HttpResponse.json({ models: [], providers: [] })),
    http.get('/api/personas', () => HttpResponse.json([])),
    http.get('/api/personas/active', () => HttpResponse.json(null)),
    http.get('/api/sessions', () => HttpResponse.json({ items: [], total: 0 })),
    http.get('/api/projects', () => HttpResponse.json({ projects: [] })),
    http.get('/api/brain/spaces', () => HttpResponse.json(null)),
    http.get('/api/sessions/:id', () => HttpResponse.json({ effective_profile: null })),
  )
  useChatStore.getState().disconnect()
  useChatStore.setState({
    activeSessionId: null,
    activeProjectId: null,
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

describe('StudioShell — projectless first-class visit (IA design §8.3)', () => {
  it('fresh visit renders the context bar with No project and a neutral Brain', async () => {
    renderShell()
    expect(screen.getByTestId('studio-shell')).toBeTruthy()
    const bar = await screen.findByTestId('studio-context-bar')
    expect(bar).toBeTruthy()
    // The project chip label comes from the shared popover copy.
    expect(screen.getByText('workbench.noProject')).toBeTruthy()
    expect(screen.getByTestId('studio-context-brain-unbound')).toBeTruthy()
    // Projectless is a happy path: no setup warning is rendered.
    expect(screen.queryByText('chat.connectionLost')).toBeNull()
  })

  it('inspector is closed by default and opens/closes explicitly', async () => {
    const user = userEvent.setup()
    renderShell()
    expect(screen.queryByTestId('studio-inspector')).toBeNull()
    await user.click(screen.getByTestId('studio-inspector-toggle'))
    expect(screen.getByTestId('studio-inspector')).toBeTruthy()
    await user.click(screen.getByTestId('studio-inspector-close'))
    await waitFor(() => expect(screen.queryByTestId('studio-inspector')).toBeNull())
  })

  it('renders prose rows narrow and code rows in the work lane', () => {
    act(() => {
      useChatStore.setState({
        activeSessionId: 'sess-1',
        messages: [PROSE_MESSAGE, CODE_MESSAGE],
      })
    })
    const { container } = renderShell()
    const proseRow = container.querySelector('[data-msg-index="0"]')
    expect(proseRow?.className).toContain('max-w-[760px]')
    const codeRow = container.querySelector('[data-msg-index="1"]')
    expect(codeRow?.className).toContain('max-w-[1240px]')
  })

  it('a Code persona with zero roots still shows no filesystem controls', async () => {
    const profile: EffectiveProfile = {
      persona_id: 'dev',
      tool_profile: 'code',
      affordances: ['diff'],
      presentation_lens: 'code',
    }
    act(() => {
      useChatStore.setState({ effectiveProfile: profile })
    })
    renderShell()
    await waitFor(() => expect(screen.queryByTitle('chat.terminal.title')).toBeNull())
    // The lens is presentation only: the context bar still renders.
    expect(await screen.findByTestId('studio-context-bar')).toBeTruthy()
  })
})
