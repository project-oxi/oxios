import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { CommandCenterPage } from '@/components/dashboard/command-center/command-center-page'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock('@/hooks/use-media-query', () => ({
  useMediaQuery: () => true,
}))

vi.mock('@/routes/agents', () => ({ defaultAgentSearch: {} }))
vi.mock('@tanstack/react-router', () => ({
  Link: (props: { to: string; params?: Record<string, string>; children: React.ReactNode }) => (
    <a href={props.params ? props.to.replace('$agentId', props.params.agentId ?? '') : props.to}>
      {props.children}
    </a>
  ),
}))

function agent(id: string, overrides: Record<string, unknown> = {}) {
  return {
    id,
    name: `Agent ${id.slice(0, 4)}`,
    status: 'running',
    created_at: '2026-08-31T05:00:00Z',
    started_at: '2026-08-31T05:00:10Z',
    completed_at: null,
    project_id: null,
    session_id: 'sess_1',
    error: null,
    steps_completed: 2,
    steps_total: 4,
    tokens_used: 500,
    cost_usd: 0.01,
    model_id: 'm1',
    duration_secs: null,
    ...overrides,
  }
}

const AGENTS = [
  agent('run1-agent'),
  agent('start-agent', { status: 'starting' }),
  agent('fail-agent', { status: 'failed', completed_at: '2026-08-31T05:59:00Z' }),
  agent('idle-agent', { status: 'idle' }), // excluded from active execution
  agent('old-fail-agent', {
    status: 'failed',
    completed_at: '2026-08-29T05:00:00Z', // >24h old → excluded
  }),
]

const STATUS = {
  service: 'oxios',
  status: 'ok',
  version: '1.45.0',
  web_version: '1.6.0',
  channels: [],
  uptime: '1h',
  components: {
    state_store: { healthy: true },
    event_bus: { healthy: true },
    brain: { healthy: true },
  },
}

function stubApi() {
  server.use(
    http.get('/api/status', () => HttpResponse.json(STATUS)),
    http.get('/api/agents', ({ request }) => {
      const url = new URL(request.url)
      if (url.searchParams.has('date_from')) {
        // completed-today projection query — only stats matter.
        return HttpResponse.json({
          items: [],
          total: 0,
          page: 1,
          per_page: 1,
          total_pages: 0,
          stats: { count_completed: 7, count_failed: 0, count_running: 0 },
        })
      }
      return HttpResponse.json({
        items: AGENTS,
        total: AGENTS.length,
        page: 1,
        per_page: 50,
        total_pages: 1,
        stats: { count_completed: 7, count_failed: 1, count_running: 2 },
      })
    }),
    http.get('/api/agents/:id', ({ params }) =>
      HttpResponse.json({
        id: params.id,
        name: 'Agent run1',
        status: 'running',
        session_id: 'sess_1',
        project_id: null,
        created_at: '2026-08-31T05:00:00Z',
        started_at: '2026-08-31T05:00:10Z',
        completed_at: null,
        error: null,
        steps_completed: 2,
        steps_total: 4,
        tokens_used: 500,
        cost_usd: 0.01,
        model_id: 'model-x',
      }),
    ),
    http.get('/api/agents/:id/trace', () =>
      HttpResponse.json({
        agent_id: 'run1-agent',
        steps: [
          {
            index: 0,
            kind: 'tool',
            tool_name: 'bash',
            action: 'run tests',
            input: null,
            output: null,
            started_at: '2026-08-31T05:10:00Z',
            duration_ms: 1200,
            status: 'completed',
          },
        ],
        completed_at: null,
      }),
    ),
    http.post('/api/agents/:id/kill', () => HttpResponse.json({ ok: true })),
    http.get('/api/approvals', () =>
      HttpResponse.json([
        {
          id: 'apr_1',
          subject: 'agent_x',
          action: 'bash.exec',
          resource: 'make install',
          reason: 'needs confirmation',
          created_at: '2026-08-31T05:55:00Z',
          status: 'pending',
        },
      ]),
    ),
    http.get('/api/costs/summary', () =>
      HttpResponse.json({
        total_cost_usd: 0.25,
        total_tokens: 42_000,
        agent_count: 3,
        period: 'today',
        spend_limit_usd: null,
        month_to_date_spend_usd: 3,
        month_to_date_tokens: 300_000,
      }),
    ),
    http.get('/api/costs/daily', () =>
      HttpResponse.json({
        items: Array.from({ length: 7 }, (_, i) => ({
          date: `2026-08-${25 + i}`,
          cost_usd: 0.1,
          tokens: 100,
          agent_count: i,
        })),
      }),
    ),
  )
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <CommandCenterPage />
    </QueryClientProvider>,
  )
}

describe('CommandCenterPage', () => {
  it('orders the operational sections: attention before active execution before passive panels', async () => {
    stubApi()
    const { container } = renderPage()

    await screen.findByText('bash.exec')
    const text = container.textContent ?? ''
    const attention = text.indexOf('commandCenter.attention.title')
    const execution = text.indexOf('commandCenter.activeExecution.title')
    const timeline = text.indexOf('commandCenter.timeline.title')
    const picture = text.indexOf('commandCenter.picture.title')
    const system = text.indexOf('commandCenter.system.title')
    expect(attention).toBeGreaterThanOrEqual(0)
    expect(attention).toBeLessThan(execution)
    expect(execution).toBeLessThan(timeline)
    expect(timeline).toBeLessThan(picture)
    expect(picture).toBeLessThan(system)
  })

  it('lists only live and recently failed runs in priority order', async () => {
    stubApi()
    renderPage()

    await screen.findByText(/Agent run1/)
    await waitFor(() => {
      const ids = screen
        .queryAllByRole('option')
        .map((o) => o.getAttribute('data-agent-id'))
        .filter(Boolean)
      expect(ids).toEqual(['run1-agent', 'start-agent', 'fail-agent'])
    })
  })

  it('selecting a run opens the desktop inspector without navigation', async () => {
    stubApi()
    renderPage()

    await screen.findByText(/Agent run1/)
    fireEvent.click(document.querySelector<HTMLElement>('[data-agent-id="run1-agent"]')!)

    const inspector = await screen.findByRole('complementary', {
      name: 'commandCenter.inspector.title',
    })
    expect(inspector).toBeInTheDocument()
    expect(await screen.findByText('model-x')).toBeInTheDocument()
  })

  it('shows the health verdict and degrades only the health chip when /api/status fails', async () => {
    stubApi()
    server.use(http.get('/api/status', () => HttpResponse.json({ error: 'down' }, { status: 500 })))
    renderPage()

    expect(await screen.findByText('commandCenter.health.degraded')).toBeInTheDocument()
    // Localized failure: the rest of the command center still renders.
    expect(await screen.findByText(/Agent run1/)).toBeInTheDocument()
    expect(screen.getByRole('log')).toBeInTheDocument()
  })

  it('renders the quiet state when there is no active work', async () => {
    stubApi()
    server.use(
      http.get('/api/agents', ({ request }) => {
        const url = new URL(request.url)
        const empty = {
          items: [],
          total: 0,
          page: 1,
          total_pages: 0,
          stats: { count_completed: 0, count_failed: 0, count_running: 0 },
        }
        if (url.searchParams.has('date_from')) {
          return HttpResponse.json({ ...empty, per_page: 1 })
        }
        return HttpResponse.json({ ...empty, per_page: 50 })
      }),
    )
    renderPage()

    expect(await screen.findByText('commandCenter.activeExecution.empty')).toBeInTheDocument()
  })
})
