import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from '@tanstack/react-router'
import { render, screen, waitFor, within } from '@testing-library/react'
import type { ReactNode } from 'react'
import { describe, expect, it } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import i18n, { initI18n } from '@/i18n'
import { BottomNav } from './bottom-nav'
import { SurfaceSwitcher } from './surface-switcher'

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

function renderAt(path: string, ui: ReactNode) {
  testSlot = ui
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

/** Wait for the router's initial async transition to settle. */
async function nav() {
  return await screen.findByRole('navigation', { name: 'Surface navigation' })
}

describe('SurfaceSwitcher (expanded)', () => {
  it('renders exactly one switcher with three surface links', async () => {
    renderAt('/operate', <SurfaceSwitcher />)
    const navEl = await nav()
    expect(navEl.querySelectorAll('a')).toHaveLength(3)
    expect(within(navEl).getByRole('link', { name: /Operate/ })).toHaveAttribute('href', '/operate')
    expect(within(navEl).getByRole('link', { name: /Knowledge/ })).toHaveAttribute(
      'href',
      '/knowledge',
    )
    expect(within(navEl).getByRole('link', { name: /Studio/ })).toHaveAttribute('href', '/studio')
  })

  it('marks the active surface from the route', async () => {
    renderAt('/knowledge', <SurfaceSwitcher />)
    const navEl = await nav()
    expect(within(navEl).getByRole('link', { name: /Knowledge/ })).toHaveAttribute(
      'aria-current',
      'page',
    )
    expect(within(navEl).getByRole('link', { name: /Operate/ })).not.toHaveAttribute('aria-current')
    expect(within(navEl).getByRole('link', { name: /Studio/ })).not.toHaveAttribute('aria-current')
  })

  it('never renders a legacy surface label', async () => {
    renderAt('/operate', <SurfaceSwitcher />)
    await nav()
    for (const legacy of ['Console', 'Brain', 'Chat']) {
      expect(screen.queryByText(legacy)).not.toBeInTheDocument()
    }
  })
})

describe('SurfaceSwitcher (collapsed icon rail)', () => {
  it('renders icon-only links with accessible labels', async () => {
    renderAt('/studio', <SurfaceSwitcher collapsed />)
    const navEl = await nav()
    expect(navEl.querySelectorAll('a')).toHaveLength(3)
    const links = navEl.querySelectorAll('a')
    expect(links[0]).toHaveAttribute('href', '/operate')
    expect(links[0]).toHaveAttribute('aria-label', 'Operate')
    expect(links[2]).toHaveAttribute('href', '/studio')
    expect(links[2]).toHaveAttribute('aria-current', 'page')
  })
})

describe('BottomNav (mobile)', () => {
  it('mirrors the same three surfaces and active state', async () => {
    renderAt('/operate', <BottomNav />)
    const navEl = await nav()
    expect(navEl.querySelectorAll('a')).toHaveLength(3)
    expect(within(navEl).getByRole('link', { name: /Operate/ })).toHaveAttribute(
      'aria-current',
      'page',
    )
    expect(within(navEl).getByRole('link', { name: /Knowledge/ })).not.toHaveAttribute(
      'aria-current',
    )
  })

  it('stays hidden on desktop viewports', async () => {
    renderAt('/operate', <BottomNav />)
    const navEl = await nav()
    expect(navEl.className).toContain('lg:hidden')
  })
})

describe('surface isolation', () => {
  it('renders one switcher per viewport mechanism, never more', async () => {
    renderAt(
      '/knowledge',
      <>
        <SurfaceSwitcher />
        <BottomNav />
      </>,
    )
    const navEls = await screen.findAllByRole('navigation', { name: 'Surface navigation' })
    await waitFor(() => {
      expect(navEls).toHaveLength(2)
      for (const navEl of navEls) expect(navEl.querySelectorAll('a')).toHaveLength(3)
    })
  })
})
