import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { RunCenter } from '@/components/operate/run-center'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

vi.mock('@tanstack/react-router', () => ({
  createFileRoute: () => () => ({}),
  Link: (props: { to: string; params?: Record<string, string>; children: React.ReactNode }) => (
    <a
      href={Object.entries(props.params ?? {}).reduce(
        (p, [k, v]) => p.replace(`$${k}`, v ?? ''),
        props.to,
      )}
    >
      {props.children}
    </a>
  ),
}))

vi.mock('@/hooks/use-media-query', () => ({
  useMediaQuery: () => true,
}))

const AUTOMATION = {
  kind: 'automation',
  id: 'run_a',
  name: 'Nightly sync',
  status: 'failed',
  trigger: 'schedule',
  automationId: 'auto_1',
  startedAt: '2026-08-31T05:00:00Z',
  completedAt: '2026-08-31T05:01:00Z',
  durationSecs: 60,
  error: 'boom',
}

const AGENT = {
  kind: 'agent',
  id: 'agent_9',
  name: 'Agent nine',
  status: 'running',
  startedAt: '2026-08-31T05:00:00Z',
}

const STOPPABLE = {
  kind: 'agent',
  id: 'agent_stop',
  name: 'Agent stop',
  status: 'running',
}

function stubRuns(automationRows: unknown[], agentRows: unknown[]) {
  server.use(
    http.get('/api/operate/runs', () =>
      HttpResponse.json({
        runs: [...automationRows, ...agentRows],
        total: automationRows.length + agentRows.length,
      }),
    ),
  )
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <RunCenter />
    </QueryClientProvider>,
  )
}

describe('RunCenter', () => {
  afterEach(() => server.resetHandlers())

  it('renders automation + agent rows, with plain anchor to /studio/automations/{id}', async () => {
    stubRuns([AUTOMATION], [AGENT])
    // /api/agents drives the active-execution list — return empty.
    server.use(http.get('/api/agents', () => HttpResponse.json({ items: [] })))
    renderPage()
    await waitFor(() => expect(screen.getByTestId('automation-runs-table')).toBeInTheDocument())
    const automation = screen.getByTestId('automation-runs-table')
    expect(within(automation).getByText('Nightly sync')).toBeInTheDocument()
    expect(within(automation).getByText('operate.runs.trigger.schedule')).toBeInTheDocument()
    expect(
      within(automation).getByRole('link', { name: 'operate.actions.openAutomation' }),
    ).toHaveAttribute('href', '/studio/automations/auto_1')
    // no trigger editing controls (no combobox inside the trigger cell)
    expect(within(automation).queryByRole('combobox')).not.toBeInTheDocument()
    expect(screen.getByTestId('agent-runs-table')).toBeInTheDocument()
  })

  it('clicking an agent row opens the operate inspector (section sections visible)', async () => {
    stubRuns([], [AGENT])
    server.use(http.get('/api/agents', () => HttpResponse.json({ items: [] })))
    renderPage()
    await waitFor(() => screen.getByTestId('agent-runs-table'))
    const row = screen.getByLabelText('commandCenter.activeExecution.selectRun')
    fireEvent.click(row)
    // inspector (panel variant on desktop) shows fixed section order
    await waitFor(() => expect(screen.getByLabelText('operate.sections.state')).toBeInTheDocument())
    expect(screen.getByLabelText('operate.sections.details')).toBeInTheDocument()
  })

  it('agent row stop affordance only for stoppable rows', async () => {
    stubRuns(
      [],
      [
        AGENT,
        { ...STOPPABLE },
        { kind: 'agent', id: 'agent_done', name: 'Done', status: 'completed' },
      ],
    )
    server.use(http.get('/api/agents', () => HttpResponse.json({ items: [] })))
    renderPage()
    await waitFor(() => screen.getByTestId('agent-runs-table'))
    const table = screen.getByTestId('agent-runs-table')
    const stops = within(table).getAllByRole('button', { name: 'commandCenter.inspector.stop' })
    // only AGENT + STOPPABLE are running/starting; agent_done is completed (not stoppable)
    expect(stops).toHaveLength(2)
  })
})
