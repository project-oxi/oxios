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
    t: (key: string, params?: Record<string, unknown>) => {
      if (params && 'roots' in params) return `${key}:${String(params.roots)}`
      return key
    },
    i18n: { language: 'en' },
  }),
}))

function withQueryClient(ui: React.ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  })
  return <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
}

const MEMORY_HITS = [
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

const ENVELOPE = {
  memory: MEMORY_HITS,
  documents: [],
  freshness: {
    reconciled_roots: [],
    skipped_roots: [],
    skipped_files: 0,
    stale_after_retry: [],
    dense_coverage: null,
  },
}

const ENVELOPE_WITH_DOCS = {
  memory: MEMORY_HITS,
  documents: [
    {
      document_id: 'doc-1',
      root: 'vault',
      locator: 'notes/2026-08-27.md',
      revision: 'rev-abcdef0123456789',
      ordinal: 0,
      text: 'oxibrain 0.8 ships a daemonless session model.',
      modified_at: 1700000000000,
      score: 0.91,
    },
  ],
  freshness: {
    reconciled_roots: ['vault'],
    skipped_roots: [],
    skipped_files: 0,
    stale_after_retry: [],
    dense_coverage: 0.85,
  },
}

describe('BrainSearch', () => {
  afterEach(() => {
    server.resetHandlers()
  })

  it('renders memory hit rows and selects the entity on click', async () => {
    let requestedSpace: string | null = null
    server.use(
      http.get('/api/brain/search', ({ request }) => {
        requestedSpace = new URL(request.url).searchParams.get('space')
        return HttpResponse.json(ENVELOPE)
      }),
    )
    const onSelectEntity = vi.fn()
    render(withQueryClient(<BrainSearch space="personal" onSelectEntity={onSelectEntity} />))

    const user = userEvent.setup()
    await user.type(screen.getByLabelText('brain.searchPlaceholder'), 'user')
    await user.click(screen.getByRole('button', { name: 'brain.search' }))

    expect(await screen.findByText('The user')).toBeInTheDocument()
    expect(screen.getByText('has_skill Rust; works_on oxibrain migration')).toBeInTheDocument()
    expect(screen.getByText('0.812')).toBeInTheDocument()
    expect(screen.getByText('Rust')).toBeInTheDocument()

    await user.click(screen.getByText('The user'))
    expect(onSelectEntity).toHaveBeenCalledWith('ent-user-1')
    // Scoped contract: the search request carries the bound space.
    await waitFor(() => expect(requestedSpace).toBe('personal'))
  })

  it('shows the empty state when the envelope has no memory or document hits', async () => {
    server.use(
      http.get('/api/brain/search', () =>
        HttpResponse.json({
          memory: [],
          documents: [],
          freshness: {
            reconciled_roots: [],
            skipped_roots: [],
            skipped_files: 0,
            stale_after_retry: [],
            dense_coverage: null,
          },
        }),
      ),
    )
    render(withQueryClient(<BrainSearch space="personal" />))

    const user = userEvent.setup()
    await user.type(screen.getByLabelText('brain.searchPlaceholder'), 'zzz')
    await user.click(screen.getByRole('button', { name: 'brain.search' }))

    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent('brain.noSearchResults'),
    )
  })

  it('renders the documents card when the envelope carries document hits', async () => {
    server.use(http.get('/api/brain/search', () => HttpResponse.json(ENVELOPE_WITH_DOCS)))
    render(withQueryClient(<BrainSearch space="personal" />))

    const user = userEvent.setup()
    await user.type(screen.getByLabelText('brain.searchPlaceholder'), 'oxibrain')
    await user.click(screen.getByRole('button', { name: 'brain.search' }))

    expect(await screen.findByText(/^brain\.documents/)).toBeInTheDocument()
    expect(screen.getByText('oxibrain 0.8 ships a daemonless session model.')).toBeInTheDocument()
    expect(screen.getByText('brain.showHistory')).toBeInTheDocument()
  })

  const HISTORY = [
    { revision: 'rev-abcdef0123456789', committed_at_ms: 1700000000000, content: 'v1' },
    { revision: 'rev-1234567890abcdef', committed_at_ms: 1700100000000, content: 'v2' },
  ]

  it('renders revision rows when the history toggle opens a document', async () => {
    let historySpace: string | null = null
    server.use(
      http.get('/api/brain/search', () => HttpResponse.json(ENVELOPE_WITH_DOCS)),
      http.get('/api/brain/document-history', ({ request }) => {
        historySpace = new URL(request.url).searchParams.get('space')
        return HttpResponse.json(HISTORY)
      }),
    )
    render(withQueryClient(<BrainSearch space="personal" />))

    const user = userEvent.setup()
    await user.type(screen.getByLabelText('brain.searchPlaceholder'), 'oxibrain')
    await user.click(screen.getByRole('button', { name: 'brain.search' }))

    const toggle = await screen.findByRole('button', { name: 'brain.showHistory' })
    expect(screen.queryByText(/rev-abcd/)).not.toBeInTheDocument()
    await user.click(toggle)

    // Both revisions render as `date · revision.slice(0, 8)` rows, and the
    // toggle flips to the hide label.
    expect(await screen.findByText(/rev-abcd/)).toBeInTheDocument()
    expect(screen.getByText(/rev-1234/)).toBeInTheDocument()
    // Document history is space-scoped too.
    expect(historySpace).toBe('personal')
    expect(screen.getByRole('button', { name: 'brain.hideHistory' })).toBeInTheDocument()
  })

  it('shows the no-history empty path when document history is empty', async () => {
    server.use(
      http.get('/api/brain/search', () => HttpResponse.json(ENVELOPE_WITH_DOCS)),
      http.get('/api/brain/document-history', () => HttpResponse.json([])),
    )
    render(withQueryClient(<BrainSearch space="personal" />))

    const user = userEvent.setup()
    await user.type(screen.getByLabelText('brain.searchPlaceholder'), 'oxibrain')
    await user.click(screen.getByRole('button', { name: 'brain.search' }))

    await user.click(await screen.findByRole('button', { name: 'brain.showHistory' }))
    expect(await screen.findByText('brain.noHistory')).toBeInTheDocument()
  })

  it('does not issue a scoped request when no space is bound', async () => {
    let called = false
    server.use(
      http.get('/api/brain/search', () => {
        called = true
        return HttpResponse.json(ENVELOPE)
      }),
    )
    render(withQueryClient(<BrainSearch space="" />))

    const user = userEvent.setup()
    await user.type(screen.getByLabelText('brain.searchPlaceholder'), 'user')
    await user.click(screen.getByRole('button', { name: 'brain.search' }))

    // The submit resolves to the empty state without firing the scoped
    // query — the backend would 400 an unscoped search.
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent('brain.noSearchResults'),
    )
    expect(called).toBe(false)
  })
})
