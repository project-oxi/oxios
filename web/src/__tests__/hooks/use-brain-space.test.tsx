import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, renderHook, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it } from 'vitest'
import {
  useBrainContradictions,
  useBrainEntity,
  useBrainSearch,
  useBrainSpaceOverview,
  useBrainStats,
} from '@/hooks/use-brain'
import { server } from '../msw/server'

// Brain-chat binding (Task 8): scoped brain hooks must carry the selected
// space in BOTH the queryKey and the request params, and skip the query
// entirely while no space is selected (the backend 400s unscoped calls).

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

function setup() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false, refetchInterval: false },
      mutations: { retry: false },
    },
  })
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
  return { queryClient, wrapper }
}

function requestedSpaces(urls: string[]): Array<string | null> {
  return urls.map((u) => new URL(u).searchParams.get('space'))
}

describe('scoped brain hooks carry space', () => {
  afterEach(() => {
    server.resetHandlers()
  })

  it('stats + space overview: space in queryKey and request params', async () => {
    const urls: string[] = []
    server.use(
      http.get('/api/brain/stats', ({ request }) => {
        urls.push(request.url)
        return HttpResponse.json({ episodes: 1, entities: 2, statements: 3, contradictions: 4 })
      }),
      http.get('/api/brain/space', ({ request }) => {
        urls.push(request.url)
        return HttpResponse.json(null)
      }),
    )
    const { queryClient, wrapper } = setup()
    renderHook(() => useBrainStats('personal'), { wrapper })
    renderHook(() => useBrainSpaceOverview('personal'), { wrapper })

    await waitFor(() => {
      expect(queryClient.getQueryData(['brain', 'stats', 'personal'])).toBeDefined()
      expect(queryClient.getQueryData(['brain', 'space', 'personal'])).toBeDefined()
    })
    expect(requestedSpaces(urls)).toEqual(['personal', 'personal'])
  })

  it('search: space in queryKey and request params', async () => {
    const urls: string[] = []
    server.use(
      http.get('/api/brain/search', ({ request }) => {
        urls.push(request.url)
        return HttpResponse.json({ memory: [], documents: [], freshness: null })
      }),
    )
    const { queryClient, wrapper } = setup()
    renderHook(() => useBrainSearch('rust', 'hybrid', 20, true, 'both', 'work'), { wrapper })

    await waitFor(() =>
      expect(
        queryClient.getQueryData(['brain', 'search', 'rust', 'hybrid', 20, 'both', 'work']),
      ).toBeDefined(),
    )
    expect(requestedSpaces(urls)).toEqual(['work'])
  })

  it('entity: space in queryKey and request params', async () => {
    const urls: string[] = []
    server.use(
      http.get('/api/brain/entity/ent-1', ({ request }) => {
        urls.push(request.url)
        return HttpResponse.json([])
      }),
    )
    const { queryClient, wrapper } = setup()
    renderHook(() => useBrainEntity('ent-1', 'work'), { wrapper })

    await waitFor(() =>
      expect(queryClient.getQueryData(['brain', 'entity', 'work', 'ent-1'])).toBeDefined(),
    )
    expect(requestedSpaces(urls)).toEqual(['work'])
  })

  it('skips the query entirely while space is empty', async () => {
    let calls = 0
    server.use(
      http.get('/api/brain/stats', () => {
        calls += 1
        return HttpResponse.json(null)
      }),
      http.get('/api/brain/contradictions', () => {
        calls += 1
        return HttpResponse.json([])
      }),
      http.get('/api/brain/entity/ent-1', () => {
        calls += 1
        return HttpResponse.json([])
      }),
    )
    const { queryClient, wrapper } = setup()
    renderHook(() => useBrainStats(''), { wrapper })
    renderHook(() => useBrainContradictions(''), { wrapper })
    renderHook(() => useBrainEntity('ent-1', ''), { wrapper })

    await act(async () => {})
    expect(calls).toBe(0)
    expect(queryClient.getQueryData(['brain', 'stats', ''])).toBeUndefined()
    expect(queryClient.getQueryData(['brain', 'contradictions', ''])).toBeUndefined()
  })
})
