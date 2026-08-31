import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { CapabilityHub } from '@/components/operate/capability-hub'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

vi.mock('@tanstack/react-router', () => ({
  Link: () => null,
}))

const FAMILIES = [
  {
    family: 'mcp',
    id: 'm1',
    name: 'Filesystem',
    status: 'connected',
    deepRoute: '/operate/system/mcp',
  },
  {
    family: 'skill',
    id: 's1',
    name: 'Bash skill',
    status: 'ready',
    scope: 'managed',
    deepRoute: '/operate/system/skills',
  },
  {
    family: 'engine',
    id: 'e1',
    name: 'OpenAI',
    status: 'configured',
    deepRoute: '/operate/system/settings',
  },
]

function stubCapabilities(families: unknown[]) {
  server.use(http.get('/api/operate/capabilities', () => HttpResponse.json({ families })))
}

function renderHub() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={client}>
      <CapabilityHub />
    </QueryClientProvider>,
  )
}

describe('CapabilityHub', () => {
  afterEach(() => server.resetHandlers())

  it('groups rows by family and exposes deep links to /operate/system/*', async () => {
    stubCapabilities(FAMILIES)
    renderHub()
    await waitFor(() => expect(screen.getByTestId('capability-family-mcp')).toBeInTheDocument())
    // Empty families render no panel (channel + security absent in stub).
    expect(screen.queryByTestId('capability-family-channel')).not.toBeInTheDocument()
    expect(screen.queryByTestId('capability-family-security')).not.toBeInTheDocument()

    const mcpLink = screen.getByText('Filesystem').closest('a')
    expect(mcpLink).toHaveAttribute('href', '/operate/system/mcp')

    expect(screen.getByTestId('capability-family-skill')).toBeInTheDocument()
    expect(screen.getByTestId('capability-family-engine')).toBeInTheDocument()
  })

  it('isolates an endpoint failure to an error state while keeping the page mounted', async () => {
    server.use(http.get('/api/operate/capabilities', () => HttpResponse.json({}, { status: 500 })))
    renderHub()
    await waitFor(() => expect(screen.getByRole('alert')).toBeInTheDocument())
    // Page header still renders
    expect(screen.getByText('operate.capabilities')).toBeInTheDocument()
  })
})
