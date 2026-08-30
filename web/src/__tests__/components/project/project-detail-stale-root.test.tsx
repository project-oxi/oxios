// Task 8 (design §9): a stale project root is surfaced on the project
// detail page — each row reflects the backend's per-root exists/is_dir
// status (GET /api/projects/:id/roots/status) with a visible badge, and is
// never silently swapped for an ambient workspace.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { describe, expect, it, vi } from 'vitest'
import { ProjectDetailsCard } from '@/routes/projects/$projectId'
import type { Project } from '@/types'
import { server } from '../../msw/server'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))
const PROJECT: Project = {
  id: 'proj-1',
  name: 'Alpha',
  root_paths: ['/tmp/alive', '/tmp/gone'],
  instructions: '',
  created_at: new Date('2026-01-01').toISOString(),
  updated_at: new Date('2026-01-02').toISOString(),
  last_active_at: new Date('2026-01-03').toISOString(),
} as unknown as Project

function renderCard() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <ProjectDetailsCard project={PROJECT} />
    </QueryClientProvider>,
  )
}

function mockStatus(status: Array<{ path: string; exists: boolean; is_dir: boolean }>) {
  server.use(http.get('/api/projects/:id/roots/status', () => HttpResponse.json(status)))
}

describe('project details stale-root badge', () => {
  it('renders every root without a badge while all roots are live', async () => {
    mockStatus([
      { path: '/tmp/alive', exists: true, is_dir: true },
      { path: '/tmp/gone', exists: true, is_dir: true },
    ])
    renderCard()
    await waitFor(() => expect(screen.getByText('/tmp/alive')).toBeTruthy())
    expect(screen.getByText('/tmp/gone')).toBeTruthy()
    expect(screen.queryByText('projects.staleRoot')).toBeNull()
  })

  it('flags a vanished root with the stale badge and hint', async () => {
    mockStatus([
      { path: '/tmp/alive', exists: true, is_dir: true },
      { path: '/tmp/gone', exists: false, is_dir: false },
    ])
    renderCard()
    await waitFor(() => expect(screen.getByText('projects.staleRoot')).toBeTruthy())
    const badge = screen.getByText('projects.staleRoot')
    expect(badge.getAttribute('title')).toBe('projects.staleRootHint')
    // The live root is NOT flagged — only the row the backend reported stale.
    expect(screen.getAllByText('projects.staleRoot')).toHaveLength(1)
  })

  it('flags a root replaced by a regular file (exists but not a dir)', async () => {
    mockStatus([
      { path: '/tmp/alive', exists: true, is_dir: true },
      { path: '/tmp/gone', exists: true, is_dir: false },
    ])
    renderCard()
    await waitFor(() => expect(screen.getByText('projects.staleRoot')).toBeTruthy())
  })
})
