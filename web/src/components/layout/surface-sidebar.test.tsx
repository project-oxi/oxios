import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from '@tanstack/react-router'
import { render, screen } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import type { ReactNode } from 'react'
import { describe, expect, it } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { TooltipProvider } from '@/components/ui/tooltip'
import i18n, { initI18n } from '@/i18n'
import { SurfaceSidebar } from './surface-sidebar'

await initI18n()
await i18n.changeLanguage('en')

const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })

/** Slot the component under test into a minimal three-surface route tree. */
let testSlot: ReactNode = null
const rootRoute = createRootRoute({ component: () => testSlot })
const tree = rootRoute.addChildren(
  ['/operate', '/knowledge', '/studio'].map((routePath) =>
    createRoute({ getParentRoute: () => rootRoute, path: routePath, component: () => null }),
  ),
)

function renderAt(path: string) {
  testSlot = <SurfaceSidebar />
  const router = createRouter({
    routeTree: tree,
    history: createMemoryHistory({ initialEntries: [path] }),
  })
  return render(
    <TooltipProvider>
      <QueryClientProvider client={client}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </TooltipProvider>,
  )
}

describe('SurfaceSidebar', () => {
  it('selects the Operate sidebar on /operate', async () => {
    renderAt('/operate')
    const attention = await screen.findByRole('link', { name: 'Attention' })
    expect(attention).toHaveAttribute('href', '/operate')
    // No legacy quick links on any surface sidebar.
    expect(screen.queryByRole('link', { name: 'Agents' })).not.toBeInTheDocument()
    expect(screen.queryByRole('link', { name: 'Settings' })).not.toBeInTheDocument()
  })

  it('selects the Knowledge sidebar on /knowledge', async () => {
    renderAt('/knowledge')
    expect(await screen.findByRole('link', { name: 'Overview' })).toHaveAttribute(
      'href',
      '/knowledge',
    )
    expect(screen.getByText('Memory · Brain')).toBeInTheDocument()
    // Empty groups hide their headers; the Library group's items (and header)
    // arrive with the wave-2 Knowledge worker.
    expect(screen.queryByText('Library · Knowledge')).not.toBeInTheDocument()
  })
  it('renders the Studio session tree with a No project group', async () => {
    server.use(
      http.get('/api/sessions', () =>
        HttpResponse.json({
          items: [
            { id: 's1', title: 'Alpha', project_id: null, created_at: '2026-08-31T00:00:00Z' },
          ],
          total: 1,
        }),
      ),
      http.get('/api/projects', () => HttpResponse.json({ items: [], total: 0 })),
    )
    renderAt('/studio')
    expect(await screen.findByText('No project')).toBeInTheDocument()
    expect(screen.getByText('Alpha')).toBeInTheDocument()
  })

  it('falls back to the Operate sidebar on unknown paths', async () => {
    // Router error rendering maps unknown paths to operate.
    renderAt('/operate/projects')
    expect(await screen.findByRole('link', { name: 'Attention' })).toBeInTheDocument()
  })
})
