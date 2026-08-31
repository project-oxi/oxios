import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import {
  OperateInspector,
  type OperateInspectorProps,
} from '@/components/operate/operate-inspector'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
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

// RunTraceDetail pulls trace via TanStack Query — stub it to keep the test
// pure-presentational.
vi.mock('@/components/dashboard/command-center/run-inspector', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('@/components/dashboard/command-center/run-inspector')>()
  return {
    ...actual,
    RunTraceDetail: () => <div data-testid="trace-detail" />,
  }
})

const AUTOMATION_RUN = {
  kind: 'automation',
  id: 'auto_run_1',
  name: 'Nightly sync',
  status: 'failed',
  trigger: 'schedule',
  automationId: 'auto_1',
  startedAt: '2026-08-31T05:00:00Z',
  completedAt: '2026-08-31T05:01:00Z',
  durationSecs: 60,
  error: 'disk full',
  stepsCompleted: 3,
  stepsTotal: 5,
}

const AGENT_RUN = {
  kind: 'agent',
  id: 'agent_9',
  name: 'Agent nine',
  status: 'running',
  sessionId: 'sess_42',
  projectId: 'proj_1',
  startedAt: '2026-08-31T05:00:00Z',
  stepsCompleted: 2,
  stepsTotal: 4,
  tokensUsed: 12345,
  costUsd: 0.42,
  modelId: 'model-x',
  durationSecs: 120,
}

function renderInspector(
  run: typeof AUTOMATION_RUN | typeof AGENT_RUN,
  props: Partial<OperateInspectorProps> = {},
) {
  return render(
    <OperateInspector run={run as never} onClose={() => {}} variant="panel" {...props} />,
  )
}
describe('OperateInspector', () => {
  it('renders sections in the fixed order: state → origin → scope → impact → actions → details', () => {
    const { container } = renderInspector(AGENT_RUN, { onStop: () => {} })
    const order = Array.from(container.querySelectorAll('[data-section]')).map((el) =>
      el.getAttribute('data-section'),
    )
    expect(order).toEqual(['state', 'origin', 'scope', 'actions', 'details', 'trace'])
  })

  it('omits origin rows that the source does not record', () => {
    const { container } = renderInspector(AGENT_RUN)
    // AGENT_RUN carries projectId + sessionId; projectId IS recorded so the
    // label "Project" is present, but the automation origin label must be
    // absent (no automation on agent runs).
    expect(container.textContent).not.toContain('operate.origin.automation')
    expect(container.textContent).not.toContain('operate.origin.automationId')
  })

  it('renders the Studio launch anchor only when automation origin is present', () => {
    renderInspector(AGENT_RUN, { onStop: () => {} })
    expect(
      screen.queryByRole('link', { name: 'operate.actions.openAutomation' }),
    ).not.toBeInTheDocument()
    renderInspector(AUTOMATION_RUN)
    expect(screen.getByRole('link', { name: 'operate.actions.openAutomation' })).toHaveAttribute(
      'href',
      '/studio/automations/auto_1',
    )
  })

  it('embeds the agent trace detail only for agent runs', () => {
    const { unmount } = renderInspector(AGENT_RUN, { onStop: () => {} })
    expect(screen.getByTestId('trace-detail')).toBeInTheDocument()
    unmount()
    renderInspector(AUTOMATION_RUN)
    expect(screen.queryByTestId('trace-detail')).not.toBeInTheDocument()
  })
})
