import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { BrainSearch } from './search'

// Mock i18next — verbatim convention from existing component tests
// (see web/src/components/brain/status-banner.test.tsx).
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

function withQueryClient(ui: React.ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  })
  return <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
}

// Daemon search contract: a bare array of entity hits.
const HITS = [
  {
    entity_id: 'ent-user-1',
    entity_surface: 'The user',
    entity_type: 'Person',
    score: 0.812,
    snippet: 'has_skill Rust; works_on oxibrain migration',
  },
  {
    entity_id: 'ent-rust-1',
    entity_surface: 'Rust',
    entity_type: 'Concept',
    score: 0.5,
    snippet: 'a skill of The user',
  },
]

describe('BrainSearch', () => {
  afterEach(() => {
    server.resetHandlers()
  })

  it('renders daemon hit rows (surface, snippet, score, type) and selects the entity on click', async () => {
    server.use(http.get('/api/brain/search', () => HttpResponse.json(HITS)))
    const onSelectEntity = vi.fn()
    render(withQueryClient(<BrainSearch onSelectEntity={onSelectEntity} />))

    const user = userEvent.setup()
    await user.type(screen.getByLabelText('brain.searchPlaceholder'), 'user')
    await user.click(screen.getByRole('button', { name: 'brain.search' }))

    expect(await screen.findByText('The user')).toBeInTheDocument()
    expect(screen.getByText('has_skill Rust; works_on oxibrain migration')).toBeInTheDocument()
    expect(screen.getByText('0.812')).toBeInTheDocument()
    expect(screen.getByText('Rust')).toBeInTheDocument()

    await user.click(screen.getByText('The user'))
    expect(onSelectEntity).toHaveBeenCalledWith('ent-user-1')
  })

  it('shows the empty state when the daemon returns no hits', async () => {
    server.use(http.get('/api/brain/search', () => HttpResponse.json([])))
    render(withQueryClient(<BrainSearch />))

    const user = userEvent.setup()
    await user.type(screen.getByLabelText('brain.searchPlaceholder'), 'zzz')
    await user.click(screen.getByRole('button', { name: 'brain.search' }))

    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent('brain.noSearchResults'),
    )
  })
})
