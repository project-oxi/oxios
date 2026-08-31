// Task 6 (project-roots persona workbench): useEffectiveProfile resolution.
//
// Resolution order (design §6.1, Task 6 decisions):
//   1. chat-store `effectiveProfile` (SessionDetail on loadSession + done
//      chunks) — the fallback fetch must NOT fire while the store has one.
//   2. GET /api/sessions/:id (TanStack Query, staleTime 30s) when a session
//      is active but the store has no profile yet.
//   3. null — no session or no profile: every conditional control hides.
// Also pins: `effectiveProfile` is runtime-only store state and never lands
// in the persisted partial.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import type { ReactNode } from 'react'
import { beforeEach, describe, expect, it } from 'vitest'
import { useEffectiveProfile } from '@/hooks/use-effective-profile'
import { useChatStore } from '@/stores/chat'
import type { EffectiveProfile, SessionDetail } from '@/types'
import { server } from '../msw/server'

const CODE_ROOTS: EffectiveProfile = {
  persona_id: 'dev',
  tool_profile: 'code',
  affordances: ['diff', 'files', 'terminal', 'worktree-fanout'],
  presentation_lens: 'code',
}

const MINIMAL: EffectiveProfile = {
  persona_id: 'writer',
  tool_profile: 'minimal',
  affordances: [],
  presentation_lens: 'writing',
}

const sessionDetail = (effective_profile: EffectiveProfile | null): SessionDetail => ({
  id: 'sess-1',
  user_id: 'u',
  user_messages: [],
  agent_responses: [],
  active_persona_id: effective_profile?.persona_id,
  effective_profile,
  created_at: '2026-08-29T00:00:00Z',
  updated_at: '2026-08-29T00:00:00Z',
})

function renderResolved() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={qc}>{children}</QueryClientProvider>
  )
  return renderHook(() => useEffectiveProfile(), { wrapper })
}

// Any /api/sessions/:id request while this handler is installed fails the
// test — the fallback fetch must not fire when the store already resolves.
const forbidSessionFetch = () =>
  server.use(
    http.get('/api/sessions/*', () => {
      throw new Error('fallback fetch must not run while the store holds a profile')
    }),
  )

beforeEach(() => {
  useChatStore.setState({ activeSessionId: null, effectiveProfile: null })
})

describe('useEffectiveProfile', () => {
  it('returns the chat-store profile without any fallback fetch', async () => {
    forbidSessionFetch()
    const { result } = renderResolved()
    act(() => {
      useChatStore.setState({ activeSessionId: 'sess-1', effectiveProfile: CODE_ROOTS })
    })
    await waitFor(() => expect(result.current).toBe(CODE_ROOTS))
  })

  it('falls back to GET /api/sessions/:id when the store has none', async () => {
    server.use(http.get('/api/sessions/sess-2', () => HttpResponse.json(sessionDetail(MINIMAL))))
    const { result } = renderResolved()
    act(() => {
      useChatStore.setState({ activeSessionId: 'sess-2', effectiveProfile: null })
    })
    await waitFor(() => expect(result.current).toEqual(MINIMAL))
  })

  it('prefers the live store profile over a stale cached fetch', async () => {
    server.use(http.get('/api/sessions/sess-3', () => HttpResponse.json(sessionDetail(MINIMAL))))
    const { result } = renderResolved()
    act(() => {
      useChatStore.setState({ activeSessionId: 'sess-3', effectiveProfile: null })
    })
    await waitFor(() => expect(result.current).toEqual(MINIMAL))
    // A done chunk landed mid-session with a fresher profile — the store wins.
    const refreshed: EffectiveProfile = { ...MINIMAL, affordances: ['diff'] }
    act(() => {
      useChatStore.setState({ effectiveProfile: refreshed })
    })
    await waitFor(() => expect(result.current).toBe(refreshed))
  })

  it('no session + no profile ⇒ null (controls hidden), no fetch', async () => {
    forbidSessionFetch()
    const { result } = renderResolved()
    expect(result.current).toBeNull()
    // Flush microtasks: no query may have fired in the background.
    await act(async () => {})
    expect(result.current).toBeNull()
  })

  it('effectiveProfile stays runtime-only (never persisted)', () => {
    act(() => {
      useChatStore.setState({ activeSessionId: 'sess-1', effectiveProfile: CODE_ROOTS })
    })
    const persistApi = useChatStore.persist as
      | { getOptions: () => { partialize: (s: unknown) => unknown } }
      | undefined
    const persisted = persistApi?.getOptions().partialize(useChatStore.getState()) as
      | Record<string, unknown>
      | undefined
    expect('effectiveProfile' in (persisted ?? {})).toBe(false)
  })
})
