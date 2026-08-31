import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { RunInspector } from '@/components/dashboard/command-center/run-inspector'
import type { AgentDetail, AgentListItem, AgentTrace, AgentTraceStep } from '@/types/agent'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock('@tanstack/react-router', () => ({
  Link: (props: { to: string; params?: Record<string, string>; children: React.ReactNode }) => (
    <a href={props.params ? props.to.replace('$agentId', props.params.agentId ?? '') : props.to}>
      {props.children}
    </a>
  ),
}))

let detail: AgentDetail | undefined
let trace: AgentTrace | undefined
let traceLoading = false

vi.mock('@/hooks/use-agent-trace', () => ({
  useAgentDetail: () => ({ data: detail, isLoading: traceLoading }),
  useAgentTrace: () => ({ data: trace, isLoading: traceLoading }),
}))

function step(overrides: Partial<AgentTraceStep> = {}): AgentTraceStep {
  return {
    index: 0,
    kind: 'tool',
    tool_name: 'bash',
    action: 'run tests',
    input: null,
    output: null,
    started_at: '2026-08-31T05:10:00Z',
    duration_ms: 1200,
    status: 'completed',
    ...overrides,
  }
}

function run(overrides: Partial<AgentListItem> = {}): AgentListItem {
  return {
    id: 'aaa111-agent',
    name: 'Release QA',
    status: 'running',
    created_at: '2026-08-31T05:00:00Z',
    started_at: '2026-08-31T05:00:10Z',
    completed_at: null,
    project_id: null,
    session_id: 'sess_123456789',
    error: null,
    steps_completed: 3,
    steps_total: 5,
    tokens_used: 1200,
    cost_usd: 0.04,
    model_id: 'provider/model-x',
    duration_secs: null,
    ...overrides,
  }
}

const baseProps = {
  onClose: vi.fn(),
  onStop: vi.fn(),
  stopping: false,
  now: Date.parse('2026-08-31T06:00:00Z'),
}

beforeEach(() => {
  vi.clearAllMocks()
  detail = {
    id: 'aaa111-agent',
    name: 'Release QA',
    status: 'running',
    session_id: 'sess_123456789',
    project_id: null,
    created_at: '2026-08-31T05:00:00Z',
    started_at: '2026-08-31T05:00:10Z',
    completed_at: null,
    error: null,
    steps_completed: 3,
    steps_total: 5,
    tokens_used: 1200,
    cost_usd: 0.04,
    model_id: 'provider/model-x',
  }
  trace = { agent_id: 'aaa111-agent', steps: [step()], completed_at: null }
  traceLoading = false
})

describe('RunInspector', () => {
  it('renders run identity, model, session, and elapsed without navigating', () => {
    render(<RunInspector {...baseProps} run={run()} stale={false} variant="panel" />)
    expect(screen.getByText('Release QA')).toBeInTheDocument()
    expect(screen.getByText('provider/model-x')).toBeInTheDocument()
    expect(screen.getByText(/sess_1234/)).toBeInTheDocument()
    // Elapsed caps at the shared clock: 05:00:10Z → 06:00:00Z = 59m 50s.
    expect(screen.getByText('59m 50s')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: 'commandCenter.inspector.openRun' })).toHaveAttribute(
      'href',
      '/agents/aaa111-agent',
    )
  })

  it('lists execution steps with states, capped to the most recent 10', () => {
    trace = {
      agent_id: 'x',
      completed_at: null,
      steps: Array.from({ length: 14 }, (_, i) =>
        step({
          index: i,
          tool_name: `tool_${i}`,
          started_at: `2026-08-31T05:${String(10 + i).padStart(2, '0')}:00Z`,
        }),
      ),
    }
    render(<RunInspector {...baseProps} run={run()} stale={false} variant="panel" />)
    // 10 shown, newest first (index 13 at the top), 4 capped away.
    expect(screen.getAllByTestId('inspector-step')).toHaveLength(10)
    expect(screen.getAllByTestId('inspector-step')[0]).toHaveTextContent('tool_13')
  })

  it('shows a bounded recent-tool-call list with outcomes', () => {
    trace = {
      agent_id: 'x',
      completed_at: null,
      steps: [
        step({ tool_name: 'bash', status: 'completed' }),
        step({ tool_name: 'web_search', status: 'failed' }),
        step({ kind: 'reasoning', tool_name: null, action: 'thinking' }),
        step({ tool_name: 'file_read', status: 'completed' }),
      ],
    }
    render(<RunInspector {...baseProps} run={run()} stale={false} variant="panel" />)
    const tools = screen.getAllByTestId('inspector-tool')
    expect(tools).toHaveLength(3) // reasoning excluded
    expect(tools[0]).toHaveTextContent('file_read')
    expect(tools).toEqual([
      expect.objectContaining({ textContent: expect.stringContaining('file_read') }),
      expect.objectContaining({ textContent: expect.stringContaining('web_search') }),
      expect.objectContaining({ textContent: expect.stringContaining('bash') }),
    ])
  })

  it('offers Stop only while the run can be stopped and disables it in flight', () => {
    const { rerender } = render(
      <RunInspector {...baseProps} run={run()} stale={false} variant="panel" />,
    )
    expect(screen.getByRole('button', { name: 'commandCenter.inspector.stop' })).toBeEnabled()

    rerender(
      <RunInspector
        {...baseProps}
        run={run({ status: 'failed', completed_at: '2026-08-31T05:30:00Z' })}
        stale={false}
        variant="panel"
      />,
    )
    expect(screen.queryByRole('button', { name: 'commandCenter.inspector.stop' })).toBeNull()
    expect(screen.getByText('commandCenter.inspector.finished')).toBeInTheDocument()

    rerender(<RunInspector {...baseProps} run={run()} stale={false} variant="panel" stopping />)
    expect(screen.getByRole('button', { name: 'commandCenter.inspector.stop' })).toBeDisabled()
  })

  it('announces a stale selection instead of silently showing live data', () => {
    render(<RunInspector {...baseProps} run={run({ id: 'gone1-agent' })} stale variant="panel" />)
    expect(screen.getByText('commandCenter.inspector.noLongerActive')).toBeInTheDocument()
  })

  it('renders as a dialog on small screens and closes via the dialog', () => {
    render(<RunInspector {...baseProps} run={run()} stale={false} variant="dialog" />)
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    fireEvent.click(screen.getByRole('button', { name: 'Close' }))
    expect(baseProps.onClose).toHaveBeenCalled()
  })

  it('does not render as a dialog in the panel variant', () => {
    render(<RunInspector {...baseProps} run={run()} stale={false} variant="panel" />)
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('shows a quiet state while detail and trace have no data yet', () => {
    traceLoading = true
    detail = undefined
    trace = undefined
    render(<RunInspector {...baseProps} run={run()} stale={false} variant="panel" />)
    expect(screen.queryByTestId('inspector-step')).toBeNull()
    expect(screen.queryByTestId('inspector-tool')).toBeNull()
  })

  it('renders cached steps while refetching (stale-while-revalidate)', () => {
    traceLoading = true
    render(<RunInspector {...baseProps} run={run()} stale={false} variant="panel" />)
    // Cached data stays visible during a refetch — the spec keeps the
    // inspector briefly alive when a run ends.
    expect(screen.getAllByTestId('inspector-step')).toHaveLength(1)
  })
})
