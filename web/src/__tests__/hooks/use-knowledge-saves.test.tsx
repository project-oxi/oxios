import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it } from 'vitest'
import { useSaveToKnowledge } from '@/hooks/use-knowledge-saves'
import { server } from '../msw/server'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchInterval: false },
      mutations: { retry: false },
    },
  })
  return ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
}

describe('useSaveToKnowledge', () => {
  afterEach(() => {
    server.resetHandlers()
  })

  it('always sends a JSON object body — the backend Json<SaveToKnowledgeRequest> extractor rejects bodyless POSTs with 400', async () => {
    let capturedBody: string | null = null
    let contentType: string | null = null
    server.use(
      http.post('/api/chat/s1/messages/0/save-to-knowledge', async ({ request }) => {
        contentType = request.headers.get('content-type')
        capturedBody = await request.text()
        return HttpResponse.json({ path: 'chat/s1.md' })
      }),
    )

    const { result } = renderHook(() => useSaveToKnowledge('s1'), { wrapper: createWrapper() })

    // No path — the plain "Save to Knowledge" button call.
    await act(async () => {
      await result.current.mutateAsync({ messageIndex: 0 })
    })

    // Assert on the wire capture (house pattern — see use-budget.test.tsx):
    // the mutation observer snapshot is not reliably re-rendered in tests.
    expect(contentType).toContain('application/json')
    // Empty body ('') would 400 on the axum side; assert a parseable JSON object.
    expect(JSON.parse(capturedBody === null || capturedBody === '' ? 'INVALID' : capturedBody)).toEqual({})
  })

  it('forwards the path hint in the body when provided', async () => {
    let capturedBody: string | null = null
    server.use(
      http.post('/api/chat/s2/messages/3/save-to-knowledge', async ({ request }) => {
        capturedBody = await request.text()
        return HttpResponse.json({ path: 'chat/s2.md' })
      }),
    )

    const { result } = renderHook(() => useSaveToKnowledge('s2'), { wrapper: createWrapper() })

    await act(async () => {
      await result.current.mutateAsync({ messageIndex: 3, path: 'notes/hint.md' })
    })

    expect(JSON.parse(capturedBody ?? 'INVALID')).toEqual({ path: 'notes/hint.md' })
  })
})
