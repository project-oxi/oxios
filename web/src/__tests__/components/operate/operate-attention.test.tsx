import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor, within } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { OperateAttention } from '@/components/operate/operate-attention'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock('@tanstack/react-router', () => ({
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

function stubAttention(items: unknown[]) {
  server.use(
    http.get('/api/operate/attention', () =>
      HttpResponse.json({ items, generatedAt: '2026-08-31T00:00:00Z' }),
    ),
  )
}

function stubApprovals(items: unknown[]) {
  server.use(http.get('/api/approvals', () => HttpResponse.json(items)))
}

function renderPage() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <OperateAttention />
    </QueryClientProvider>,
  )
}

const WAITING_INPUT = {
  kind: 'waiting_input',
  id: 'wi_1',
  state: 'waiting_user',
  summary: 'Agent asks: proceed with deploy?',
  origin: { agentId: 'agent_9' },
  timestamp: '2026-08-31T06:00:00Z',
  actions: ['open_agent'],
}

const FAILED_RUN = {
  kind: 'failed_run',
  id: 'run_1',
  state: 'failed',
  summary: 'Task nightly-sync failed',
  origin: { automationId: 'task_1', automationName: 'Nightly sync' },
  timestamp: '2026-08-31T05:30:00Z',
  actions: ['open_run', 'open_automation'],
  detailRoute: '/studio/automations/task_1',
}

describe('OperateAttention', () => {
  it('renders waiting_input and failed_run rows with recorded origin and safe actions', async () => {
    stubApprovals([])
    stubAttention([
      WAITING_INPUT,
      FAILED_RUN,
      // endpoint approvals are NOT duplicated here — canonical queue is AttentionQueue
      {
        kind: 'pending_approval',
        id: 'apr_1',
        state: 'pending',
        summary: 'endpoint approval row',
        origin: {},
        timestamp: '2026-08-31T05:00:00Z',
        actions: ['approve', 'reject'],
      },
    ])
    renderPage()

    await waitFor(() =>
      expect(screen.getByTestId('attention-group-waiting_input')).toBeInTheDocument(),
    )
    const failedGroup = screen.getByTestId('attention-group-failed_run')

    expect(screen.getByText('Agent asks: proceed with deploy?')).toBeInTheDocument()
    expect(
      within(failedGroup).getAllByText(
        (_, node) => node?.textContent?.includes('Nightly sync') ?? false,
      ).length,
    ).toBeGreaterThan(0)
    // no duplicated endpoint-approval rows from /api/operate/attention
    expect(screen.queryByText('endpoint approval row')).not.toBeInTheDocument()
    // safe actions
    expect(screen.getByRole('link', { name: 'operate.actions.openAgent' })).toHaveAttribute(
      'href',
      '/operate/runs',
    )
    expect(
      within(failedGroup).getByRole('link', { name: 'operate.actions.openAutomation' }),
    ).toHaveAttribute('href', '/studio/automations/task_1')
  })

  it('renders no panel for empty categories (endpoint approvals alone are not shown here)', async () => {
    stubApprovals([])
    stubAttention([
      {
        kind: 'pending_approval',
        id: 'apr_2',
        state: 'pending',
        summary: 'an approval',
        origin: {},
        timestamp: '2026-08-31T05:00:00Z',
        actions: ['approve'],
      },
    ])
    renderPage()

    await waitFor(() => expect(screen.getByText('operate.attentionEmpty')).toBeInTheDocument())
    expect(screen.queryByTestId('attention-group-waiting_input')).not.toBeInTheDocument()
    expect(screen.queryByTestId('attention-group-failed_run')).not.toBeInTheDocument()
  })

  it('isolates an endpoint failure to an error panel and keeps the live timeline', async () => {
    stubApprovals([])
    server.use(http.get('/api/operate/attention', () => HttpResponse.json({}, { status: 500 })))
    renderPage()

    await waitFor(() =>
      expect(screen.getByText('operate.attentionUnavailable')).toBeInTheDocument(),
    )
    expect(screen.getByRole('button', { name: 'common.retry' })).toBeInTheDocument()
    // singleton SSE timeline still present
    expect(screen.getByRole('log')).toBeInTheDocument()
  })

  it('shows one quiet line with a run-center link when nothing needs attention', async () => {
    stubApprovals([])
    stubAttention([])
    renderPage()

    await waitFor(() => expect(screen.getByText('operate.attentionEmpty')).toBeInTheDocument())
    expect(screen.getByRole('link', { name: 'operate.runCenter' })).toHaveAttribute(
      'href',
      '/operate/runs',
    )
    // and the timeline stays
    expect(screen.getByRole('log')).toBeInTheDocument()
  })
})
