import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it } from 'vitest'
import {
  useOperateAttention,
  useOperateCapabilities,
  useOperateProjectContext,
  useOperateRuns,
} from '@/hooks/use-operate'
import { server } from '../msw/server'

// Operate projections: query keys carry every scope input, request URLs
// match the frozen /api/operate/* contract, the runs limit is clamped to
// the backend page bounds, and the project-context query stays disabled
// without a project id.

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

function setup() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  })
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
  return { queryClient, wrapper }
}

const ATTENTION = { items: [], generatedAt: '2026-08-31T00:00:00Z' }

describe('useOperateAttention', () => {
  afterEach(() => server.resetHandlers())

  it('keys on [operate, attention] and GETs /api/operate/attention', async () => {
    const urls: string[] = []
    server.use(
      http.get('/api/operate/attention', ({ request }) => {
        urls.push(request.url)
        return HttpResponse.json(ATTENTION)
      }),
    )
    const { queryClient, wrapper } = setup()
    const { result } = renderHook(() => useOperateAttention(), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toEqual(ATTENTION)
    expect(queryClient.getQueryCache().find({ queryKey: ['operate', 'attention'] })).toBeDefined()
    expect(urls).toEqual(['http://localhost:3000/api/operate/attention'])
  })
})

describe('useOperateRuns', () => {
  afterEach(() => server.resetHandlers())

  it('defaults to limit=50 in key and request', async () => {
    const limits: string[] = []
    server.use(
      http.get('/api/operate/runs', ({ request }) => {
        limits.push(new URL(request.url).searchParams.get('limit') ?? '')
        return HttpResponse.json({ runs: [], total: 0 })
      }),
    )
    const { queryClient, wrapper } = setup()
    const { result } = renderHook(() => useOperateRuns(), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(queryClient.getQueryCache().find({ queryKey: ['operate', 'runs', 50] })).toBeDefined()
    expect(limits).toEqual(['50'])
  })

  it('clamps an over-limit request to the backend max of 200', async () => {
    const limits: string[] = []
    server.use(
      http.get('/api/operate/runs', ({ request }) => {
        limits.push(new URL(request.url).searchParams.get('limit') ?? '')
        return HttpResponse.json({ runs: [], total: 0 })
      }),
    )
    const { queryClient, wrapper } = setup()
    const { result } = renderHook(() => useOperateRuns(999), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(queryClient.getQueryCache().find({ queryKey: ['operate', 'runs', 200] })).toBeDefined()
    expect(queryClient.getQueryCache().find({ queryKey: ['operate', 'runs', 999] })).toBeUndefined()
    expect(limits).toEqual(['200'])
  })

  it('clamps a non-positive limit to 1', async () => {
    const limits: string[] = []
    server.use(
      http.get('/api/operate/runs', ({ request }) => {
        limits.push(new URL(request.url).searchParams.get('limit') ?? '')
        return HttpResponse.json({ runs: [], total: 0 })
      }),
    )
    const { queryClient, wrapper } = setup()
    const { result } = renderHook(() => useOperateRuns(0), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(queryClient.getQueryCache().find({ queryKey: ['operate', 'runs', 1] })).toBeDefined()
    expect(limits).toEqual(['1'])
  })
})

describe('useOperateProjectContext', () => {
  afterEach(() => server.resetHandlers())

  it('encodes the project id in key and path', async () => {
    const urls: string[] = []
    server.use(
      http.get('/api/operate/projects/:id/context', ({ request }) => {
        urls.push(request.url)
        return HttpResponse.json({
          project: { id: 'p1', name: 'Alpha', rootPaths: [] },
          issuesOpen: [],
          milestones: [],
          activeRuns: [],
        })
      }),
    )
    const { queryClient, wrapper } = setup()
    const { result } = renderHook(() => useOperateProjectContext('p1'), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(
      queryClient.getQueryCache().find({ queryKey: ['operate', 'project', 'p1', 'context'] }),
    ).toBeDefined()
    expect(urls).toEqual(['http://localhost:3000/api/operate/projects/p1/context'])
  })

  it('stays disabled without a project id (no request)', async () => {
    const urls: string[] = []
    server.use(
      http.get('/api/operate/projects/:id/context', ({ request }) => {
        urls.push(request.url)
        return HttpResponse.json({})
      }),
    )
    const { result } = renderHook(() => useOperateProjectContext(null), {
      wrapper: setup().wrapper,
    })
    expect(result.current.fetchStatus).toBe('idle')
    expect(urls).toEqual([])
  })
})

describe('useOperateCapabilities', () => {
  afterEach(() => server.resetHandlers())

  it('keys on [operate, capabilities], GETs the endpoint, and caches for 60s', async () => {
    const { queryClient, wrapper } = setup()
    server.use(
      http.get('/api/operate/capabilities', () =>
        HttpResponse.json({
          families: [
            {
              family: 'mcp',
              id: 'm1',
              name: 'fs',
              status: 'connected',
              deepRoute: '/operate/system/mcp',
            },
          ],
        }),
      ),
    )
    const { result } = renderHook(() => useOperateCapabilities(), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    const query = queryClient.getQueryCache().find({ queryKey: ['operate', 'capabilities'] })
    // staleTime is read off the observer options in v5, not the bare
    // QueryOptions — assert via the observer, which is what actually
    // governs client caching.
    expect(query?.observers[0]?.options.staleTime).toBe(60_000)
  })
})
