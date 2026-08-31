// Task 4.2 — KnowledgeWorkspace (three-pane memory/library surface).
//
// Covers the plan's acceptance points:
//   * source type labeling: memory rows carry the Memory/Brain badge,
//     library rows the Library/Knowledge badge, each row keeps its
//     kind + backing id/path (data-testid encodes both).
//   * unbound Brain: neutral "not connected" state and NO memory
//     search/recall network calls fired.
//   * missing metadata: rows omit date/excerpt instead of fabricating
//     placeholder text.
//   * panel-local failure: brain search 500 isolates the memory group
//     error while the library group stays usable.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { KnowledgeWorkspace } from '@/components/knowledge/knowledge-workspace'
import type { KnowledgeTreeNode } from '@/types/knowledge'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => vi.fn(),
}))

const TREE: KnowledgeTreeNode[] = [
  {
    name: 'notes',
    path: 'notes',
    is_dir: true,
    ctime: 0,
    display_name: 'notes',
    has_content: true,
    children: [
      {
        name: 'alpha.md',
        path: 'notes/alpha.md',
        is_dir: false,
        ctime: 1700000000000,
        display_name: 'alpha',
        has_content: true,
        children: [],
      },
      {
        name: 'empty-note.md',
        path: 'notes/empty-note.md',
        is_dir: false,
        ctime: 1700000000000,
        display_name: 'empty-note',
        has_content: false,
        children: [],
      },
    ],
  },
]

const MEMORY_HIT = {
  entity_id: 'ent-user-1',
  entity_surface: 'The user',
  entity_type: 'Person',
  score: 0.812,
  snippet: 'has_skill Rust',
}

const ENVELOPE = {
  memory: [MEMORY_HIT],
  documents: [],
  freshness: {
    reconciled_roots: [],
    skipped_roots: [],
    skipped_files: 0,
    stale_after_retry: [],
    dense_coverage: null,
  },
}

function withQueryClient(ui: React.ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  })
  return <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
}

function stubTree() {
  server.use(
    http.get('/api/knowledge/tree', ({ request }) => {
      const url = new URL(request.url)
      if (url.searchParams.get('recursive') === 'true') return HttpResponse.json(TREE)
      return HttpResponse.json([])
    }),
  )
}

async function submitMemorySearch(q: string) {
  const input = screen.getByLabelText('brain.searchPlaceholder')
  await userEvent.type(input, q)
  await userEvent.keyboard('{Enter}')
}

describe('KnowledgeWorkspace', () => {
  afterEach(() => {
    server.resetHandlers()
    window.localStorage.clear()
  })

  it('labels each source row with its kind badge and backing id/path', async () => {
    stubTree()
    const searchUrls: string[] = []
    server.use(
      http.get('/api/brain/search', ({ request }) => {
        searchUrls.push(request.url)
        return HttpResponse.json(ENVELOPE)
      }),
    )
    render(withQueryClient(<KnowledgeWorkspace space="work" />))

    // Library row: badge + path-encoded test id.
    const libRow = await screen.findByTestId('source-row-library:notes/alpha.md')
    expect(libRow).toHaveAttribute('data-kind', 'library')
    expect(within(libRow).getByText('knowledge.workspace.kindBadge.library')).toBeInTheDocument()

    // Memory row appears after a memory search is submitted.
    await submitMemorySearch('rust')
    const memRow = await screen.findByTestId('source-row-memory:ent-user-1')
    expect(memRow).toHaveAttribute('data-kind', 'memory')
    expect(within(memRow).getByText('knowledge.workspace.kindBadge.memory')).toBeInTheDocument()

    // Every memory search request carried the bound space.
    expect(searchUrls.length).toBeGreaterThan(0)
    for (const url of searchUrls) {
      expect(new URL(url).searchParams.get('space')).toBe('work')
    }
  })

  it('renders the neutral not-connected state and fires no memory calls when unbound', async () => {
    stubTree()
    let searchCalls = 0
    let recallCalls = 0
    server.use(
      http.get('/api/brain/search', () => {
        searchCalls += 1
        return HttpResponse.json(ENVELOPE)
      }),
      http.post('/api/brain/recall', () => {
        recallCalls += 1
        return HttpResponse.json({ context: null })
      }),
    )
    render(withQueryClient(<KnowledgeWorkspace />))

    expect(screen.getByTestId('brain-not-connected')).toBeInTheDocument()
    expect(screen.getByText('knowledge.workspace.brainNotConnected')).toBeInTheDocument()

    // Let every query settle, then assert neither search nor recall fired.
    await screen.findByTestId('source-row-library:notes/alpha.md')
    await waitFor(() => {
      expect(screen.queryByTestId('source-row-memory:ent-user-1')).not.toBeInTheDocument()
    })
    expect(searchCalls).toBe(0)
    expect(recallCalls).toBe(0)
  })

  it('omits missing metadata instead of fabricating placeholders', async () => {
    stubTree()
    server.use(
      http.get('/api/brain/search', () =>
        HttpResponse.json({
          ...ENVELOPE,
          memory: [{ ...MEMORY_HIT, snippet: undefined }],
        }),
      ),
    )
    render(withQueryClient(<KnowledgeWorkspace space="work" />))

    // Empty placeholder note → missingMeta marker on its row.
    const emptyRow = await screen.findByTestId('source-row-library:notes/empty-note.md')
    expect(within(emptyRow).getByText('knowledge.workspace.missingMeta')).toBeInTheDocument()

    // Memory hit without a snippet → no excerpt/date placeholder on its row.
    await submitMemorySearch('rust')
    const memRow = await screen.findByTestId('source-row-memory:ent-user-1')
    expect(within(memRow).queryByText('knowledge.workspace.missingMeta')).not.toBeInTheDocument()
    expect(within(memRow).queryByText('—')).not.toBeInTheDocument()
  })

  it('isolates a brain API failure to the memory group and keeps the library group usable', async () => {
    stubTree()
    server.use(http.get('/api/brain/search', () => HttpResponse.json({}, { status: 500 })))
    render(withQueryClient(<KnowledgeWorkspace space="work" />))

    // Library rows still render.
    expect(await screen.findByTestId('source-row-library:notes/alpha.md')).toBeInTheDocument()

    await submitMemorySearch('rust')
    // Memory group surfaces the error (ErrorState uses role="alert").
    await screen.findByRole('alert')
    // Library group remains rendered.
    expect(screen.getByTestId('source-row-library:notes/alpha.md')).toBeInTheDocument()
  })
})
