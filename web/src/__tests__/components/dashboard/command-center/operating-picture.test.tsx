import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { OperatingPicture } from '@/components/dashboard/command-center/operating-picture'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

function renderPicture() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <OperatingPicture activeCount={2} />
    </QueryClientProvider>,
  )
}

function stubEndpoints(overrides: Record<string, () => Response> = {}) {
  server.use(
    http.get('/api/agents', () =>
      overrides.agents
        ? overrides.agents()
        : HttpResponse.json({
            items: [],
            total: 0,
            page: 1,
            per_page: 1,
            total_pages: 0,
            stats: { count_completed: 5, count_failed: 1, count_running: 0 },
          }),
    ),
    http.get('/api/costs/summary', () =>
      overrides.summary
        ? overrides.summary()
        : HttpResponse.json({
            total_cost_usd: 0.5,
            total_tokens: 1234,
            agent_count: 3,
            period: 'today',
            spend_limit_usd: null,
            month_to_date_spend_usd: 12,
            month_to_date_tokens: 99999,
          }),
    ),
    http.get('/api/costs/daily', () =>
      overrides.daily
        ? overrides.daily()
        : HttpResponse.json({
            items: Array.from({ length: 7 }, (_, i) => ({
              date: `2026-08-${25 + i}`,
              cost_usd: 0.1,
              tokens: 100,
              agent_count: i + 1,
            })),
          }),
    ),
  )
}

describe('OperatingPicture', () => {
  it('renders the four aggregates from real today-scoped endpoints', async () => {
    stubEndpoints()
    renderPicture()

    // Wait for the query-backed values to land before asserting anything.
    await screen.findByText('1,234')
    expect(screen.getByLabelText('commandCenter.picture.activeAgents')).toHaveTextContent('2')
    expect(screen.getByLabelText('commandCenter.picture.completedToday')).toHaveTextContent('5')
    expect(screen.getByLabelText('commandCenter.picture.tokensToday')).toHaveTextContent('1,234')
    expect(screen.getByLabelText('commandCenter.picture.spendToday')).toHaveTextContent('$0.500')
  })
  it('renders the 7-day run series with an explicit period label, never "today"', async () => {
    stubEndpoints()
    renderPicture()

    expect(await screen.findByText('commandCenter.picture.last7days')).toBeInTheDocument()
    const bars = await screen.findAllByTestId('picture-series-bar')
    expect(bars).toHaveLength(7)
  })

  it('localizes a failed metric with retry instead of failing the panel', async () => {
    stubEndpoints({ summary: () => HttpResponse.json({ error: 'boom' }, { status: 500 }) })
    renderPicture()

    const spend = await screen.findByLabelText('commandCenter.picture.spendToday')
    await waitFor(() => expect(spend).toHaveTextContent('commandCenter.picture.unavailable'))

    // Recovery: restore the healthy handler, then retry just this metric.
    stubEndpoints()
    fireEvent.click(screen.getByRole('button', { name: /commandCenter.picture.spendToday/ }))
    await waitFor(() =>
      expect(screen.getByLabelText('commandCenter.picture.spendToday')).toHaveTextContent('$0.500'),
    )
  })
})
