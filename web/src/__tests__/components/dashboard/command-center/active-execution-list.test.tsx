import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ActiveExecutionList } from '@/components/dashboard/command-center/active-execution-list'
import type { AgentListItem } from '@/types/agent'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock('@tanstack/react-router', () => ({
  Link: (props: { to: string; children: React.ReactNode }) => (
    <a href={props.to}>{props.children}</a>
  ),
}))

function run(overrides: Partial<AgentListItem> = {}): AgentListItem {
  return {
    id: 'aaa111-agent',
    name: 'Release QA',
    status: 'running',
    created_at: '2026-08-31T05:00:00Z',
    started_at: '2026-08-31T05:00:10Z',
    completed_at: null,
    project_id: null,
    session_id: null,
    error: null,
    steps_completed: 3,
    steps_total: 5,
    tokens_used: 1200,
    cost_usd: 0.04,
    model_id: 'provider/model',
    duration_secs: null,
    ...overrides,
  }
}

const baseProps = {
  isLoading: false,
  isError: false,
  onRetry: vi.fn(),
  onSelect: vi.fn(),
  onStop: vi.fn(),
  stoppingId: null as string | null,
}

beforeEach(() => {
  vi.clearAllMocks()
})

describe('ActiveExecutionList', () => {
  it('renders lifecycle state as icon + text + elapsed, without inventing a task summary', () => {
    render(<ActiveExecutionList {...baseProps} runs={[run()]} selectedId={null} />)
    expect(screen.getByText('Release QA')).toBeInTheDocument()
    expect(screen.getByText('aaa111')).toBeInTheDocument()
    expect(screen.getByText('commandCenter.status.running')).toBeInTheDocument()
    expect(screen.getByText(/steps/)).toBeInTheDocument()
    // No summary/phase field exists on the wire — nothing invented.
    expect(screen.queryByText(/phase/i)).not.toBeInTheDocument()
  })

  it('omits progress when the backend does not report a step total', () => {
    render(
      <ActiveExecutionList
        {...baseProps}
        runs={[run({ steps_total: null, steps_completed: 0 })]}
        selectedId={null}
      />,
    )
    expect(screen.queryByText(/steps/)).not.toBeInTheDocument()
  })

  it('selects rows on click and exposes the selection via aria', () => {
    render(
      <ActiveExecutionList
        {...baseProps}
        runs={[run(), run({ id: 'bbb222-agent', name: 'Docs crawl' })]}
        selectedId="bbb222-agent"
      />,
    )
    const options = screen.getAllByRole('option')
    expect(options).toHaveLength(2)
    expect(options[0]).toHaveAttribute('aria-selected', 'false')
    expect(options[1]).toHaveAttribute('aria-selected', 'true')

    fireEvent.click(options[0]!)
    expect(baseProps.onSelect).toHaveBeenCalledWith('aaa111-agent')
  })

  it('orders rows by lifecycle priority: running before starting before failed', () => {
    render(
      <ActiveExecutionList
        {...baseProps}
        runs={[
          run({ id: 'fail1-agent', status: 'failed', completed_at: '2026-08-31T05:59:00Z' }),
          run({ id: 'start1-agent', status: 'starting' }),
          run({ id: 'run1-agent', status: 'running' }),
        ]}
        selectedId={null}
      />,
    )
    const ids = screen.getAllByRole('option').map((o) => o.getAttribute('data-agent-id'))
    expect(ids).toEqual(['run1-agent', 'start1-agent', 'fail1-agent'])
  })

  it('offers Stop only for stoppable agents and disables it while stopping', () => {
    render(
      <ActiveExecutionList
        {...baseProps}
        runs={[
          run({ id: 'run1-agent' }),
          run({ id: 'fail1-agent', status: 'failed', completed_at: '2026-08-31T05:59:00Z' }),
        ]}
        selectedId={null}
        stoppingId="run1-agent"
      />,
    )
    // The stopping row shows the in-flight label; the failed row has no Stop at all.
    expect(
      screen.queryAllByRole('button', { name: 'commandCenter.activeExecution.stop' }),
    ).toHaveLength(0)
    const stopping = screen.getByRole('button', { name: 'commandCenter.activeExecution.stopping' })
    expect(stopping).toBeDisabled()
    fireEvent.click(stopping)
    expect(baseProps.onStop).not.toHaveBeenCalled()
  })

  it('shows a quiet empty state with a way to start work', () => {
    render(<ActiveExecutionList {...baseProps} runs={[]} selectedId={null} />)
    expect(screen.getByText('commandCenter.activeExecution.empty')).toBeInTheDocument()
    expect(
      screen.getByRole('link', { name: 'commandCenter.activeExecution.startTask' }),
    ).toHaveAttribute('href', '/studio')
  })

  it('localizes a failed agents query with retry', () => {
    render(<ActiveExecutionList {...baseProps} isError runs={undefined} selectedId={null} />)
    fireEvent.click(screen.getByRole('button', { name: 'common.retry' }))
    expect(baseProps.onRetry).toHaveBeenCalled()
  })

  it('supports keyboard selection', () => {
    render(<ActiveExecutionList {...baseProps} runs={[run()]} selectedId={null} />)
    const row = screen.getByRole('option')
    fireEvent.keyDown(row, { key: 'Enter' })
    expect(baseProps.onSelect).toHaveBeenCalledWith('aaa111-agent')
  })
})
