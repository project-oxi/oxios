import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { LiveTimelinePanel } from '@/components/dashboard/command-center/live-timeline-panel'
import { useEventStore } from '@/stores/events'
import type { OxiosEvent } from '@/types'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock('@tanstack/react-router', () => ({
  Link: (props: { to: string; children: React.ReactNode }) => (
    <a href={props.to}>{props.children}</a>
  ),
}))

const EVENTS: OxiosEvent[] = [
  { id: 'e1', type: 'tool_started', agent_id: 'agent_1', timestamp: '2026-08-31T06:00:00Z' },
  { id: 'e2', type: 'memory_stored', agent_id: 'agent_2', timestamp: '2026-08-31T06:01:00Z' },
]

beforeEach(() => {
  useEventStore.setState({ events: EVENTS, isConnected: true, error: null })
})

function renderPanel(selectedAgentId: string | null = null) {
  return render(<LiveTimelinePanel selectedAgentId={selectedAgentId} />)
}

describe('LiveTimelinePanel', () => {
  it('renders the capped event log and a view-full-timeline link to /events', () => {
    renderPanel()
    expect(screen.getByRole('log')).toBeInTheDocument()
    const link = screen.getByRole('link', { name: 'commandCenter.timeline.viewFull' })
    expect(link).toHaveAttribute('href', '/operate/events')
  })

  it('adds a "This run" filter that keeps only the selected agent events', () => {
    renderPanel('agent_1')
    const select = screen.getByLabelText('dashboard.filterEvents') as HTMLSelectElement
    const thisRun = [...select.options].find((o) => o.value === 'thisRun')
    expect(thisRun).toBeDefined()

    fireEvent.change(select, { target: { value: 'thisRun' } })
    const log = screen.getByRole('log')
    expect(log.querySelectorAll('li')).toHaveLength(1)
  })

  it('keeps the unfiltered view by default and does not offer "This run" without a selection', () => {
    renderPanel(null)
    const select = screen.getByLabelText('dashboard.filterEvents') as HTMLSelectElement
    expect([...select.options].some((o) => o.value === 'thisRun')).toBe(false)
    expect(screen.getByRole('log').querySelectorAll('li')).toHaveLength(2)
  })

  it('names the connection state in text: live, reconnecting, disconnected', () => {
    const { unmount } = renderPanel()
    expect(screen.getByText('commandCenter.timeline.connectedLive')).toBeInTheDocument()
    unmount()

    useEventStore.setState({ isConnected: false, error: null })
    const rerendered = renderPanel()
    expect(screen.getByText('commandCenter.timeline.reconnecting')).toBeInTheDocument()
    rerendered.unmount()

    useEventStore.setState({ isConnected: false, error: new Error('boom') })
    renderPanel()
    expect(screen.getByText('commandCenter.timeline.disconnected')).toBeInTheDocument()
  })
})
