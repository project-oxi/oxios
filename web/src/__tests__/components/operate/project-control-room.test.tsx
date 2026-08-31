import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { ProjectControlRoom } from '@/components/operate/project-control-room'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

vi.mock('@tanstack/react-router', () => ({
  Link: (props: {
    to: string
    params?: Record<string, string>
    'aria-label'?: string
    children?: React.ReactNode
  }) => (
    <a
      aria-label={props['aria-label']}
      href={Object.entries(props.params ?? {}).reduce(
        (p, [k, v]) => p.replace(`$${k}`, v ?? ''),
        props.to,
      )}
    >
      {props.children}
    </a>
  ),
}))

const BASE_CONTEXT = {
  project: {
    id: 'p1',
    name: 'Alpha',
    rootPaths: ['/tmp/alpha', '/tmp/beta'],
    instructions: 'be focused',
  },
  issuesOpen: [
    { id: 'i1', title: 'Issue one', state: 'open' },
    { id: 'i2', title: 'Issue two', state: 'open' },
  ],
  milestones: [{ slug: 'm1', openCount: 2, closedCount: 3 }],
  activeRuns: [
    {
      kind: 'agent',
      id: 'agent_x',
      name: 'Agent X',
      status: 'running',
      projectId: 'p1',
    },
  ],
}

function stubContext(body: Record<string, unknown>, status = 200) {
  server.use(
    http.get('/api/operate/projects/:id/context', () => HttpResponse.json(body, { status })),
    // Project tabs fetch their own canonical data.
    http.get('/api/projects/:id/issues', () => HttpResponse.json({ items: [] })),
    http.get('/api/projects/:id/milestones', () => HttpResponse.json({ milestones: [] })),
  )
}

function renderRoom(id: string) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <ProjectControlRoom projectId={id} />
    </QueryClientProvider>,
  )
}

describe('ProjectControlRoom', () => {
  afterEach(() => server.resetHandlers())

  it('renders project header, status line, active runs and tabs', async () => {
    stubContext(BASE_CONTEXT)
    renderRoom('p1')
    await waitFor(() => expect(screen.getByText('Alpha')).toBeInTheDocument())
    expect(screen.getByText('operate.project.statusLine')).toBeInTheDocument()
    expect(screen.getByText('/tmp/alpha')).toBeInTheDocument()
    expect(screen.getByText('be focused')).toBeInTheDocument()
    expect(screen.getByTestId('active-runs')).toBeInTheDocument()
    expect(screen.getByText('projects.tabs.issues')).toBeInTheDocument()
    expect(screen.getByText('projects.tabs.milestones')).toBeInTheDocument()
  })

  it('omits relations not recorded by the backend (no invented sections)', async () => {
    stubContext({
      project: { id: 'p1', name: 'Empty', rootPaths: [] },
      issuesOpen: [],
      milestones: [],
      activeRuns: [],
    })
    renderRoom('p1')
    await waitFor(() => expect(screen.getByText('Empty')).toBeInTheDocument())
    expect(screen.queryByTestId('active-runs')).not.toBeInTheDocument()
    expect(screen.queryByText('operate.project.roots')).not.toBeInTheDocument()
    expect(screen.queryByText('be focused')).not.toBeInTheDocument()
  })

  it('renders an error state with a back link on 404', async () => {
    stubContext({}, 404)
    renderRoom('missing')
    await waitFor(() => expect(screen.getByRole('alert')).toBeInTheDocument())
    const backLink = screen.getByRole('link', { name: 'operate.allProjects' })
    expect(backLink).toHaveAttribute('href', '/operate/projects')
  })
})
