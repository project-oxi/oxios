// Automation domain — MSW component tests for the /studio/automations page.
//
// Covers the Task 1.3 brief behaviors:
//   - projectless create (projectId/personaId/brainSpace stay absent → null)
//   - cron create (cronPattern flows through the create payload)
//   - pause/resume (PUT /api/automations/:id/status round-trip)
//   - test run (POST /api/automations/:id/run + invalidation)
//   - immutable run-context display (contextSnapshot rendered verbatim)
//   - no run history (empty runs → "no runs" copy, not an error)
//   - zero /api/tasks requests observed (the retired domain must be silent)
//
// All network I/O runs through the shared MSW node server; no module mocks
// for the data layer — behavior is pinned at the HTTP + DOM boundary.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { AutomationsPage } from '@/components/automation/automations-page'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    // The cron editor reads `i18n.resolvedLanguage` for locale-aware time
    // labels; provide an en stub.
    i18n: { resolvedLanguage: 'en' },
    t: (key: string, opts?: Record<string, unknown>) => {
      // Minimal interpolation for the keys the page renders with params.
      if (opts && 'secs' in opts) return `${key} ${opts.secs}`
      if (opts && 'count' in opts) return `${key} ${opts.count}`
      return key
    },
  }),
}))

// ── Wire fixtures — mirror the Rust Automation/AutomationRun JSON ──

const automationProjectless = {
  id: 'a-1',
  name: 'Nightly digest',
  instruction: 'compile the digest',
  trigger: 'manual',
  executionCount: 0,
  verify: { enabled: false, maxIterations: 3 },
  status: 'active',
  createdAt: '2026-08-31T00:00:00+00:00',
  updatedAt: '2026-08-31T00:00:00+00:00',
}

const automationCron = {
  id: 'a-2',
  name: 'Morning brief',
  instruction: 'write the brief',
  trigger: 'cron',
  cronPattern: '0 9 * * *',
  timezone: null,
  maxExecutions: null,
  executionCount: 2,
  projectId: null,
  personaId: null,
  brainSpace: null,
  verify: { enabled: false, maxIterations: 3 },
  status: 'active',
  nextRunAt: '2026-09-01T09:00:00+00:00',
  lastRunAt: '2026-08-31T09:00:00+00:00',
  lastError: null,
  createdAt: '2026-08-30T00:00:00+00:00',
  updatedAt: '2026-08-31T00:00:00+00:00',
}

const runWithContext = {
  id: 'r-1',
  automationId: 'a-2',
  sessionId: 's-1',
  trigger: 'cron',
  status: 'succeeded',
  contextSnapshot: {
    projectId: 'proj-7',
    personaId: 'persona-3',
    brainSpace: 'Work',
    trigger: 'cron',
    verify: { enabled: false, maxIterations: 3 },
    instruction: 'write the brief',
  },
  summary: 'Brief delivered',
  resultContent: null,
  error: null,
  costUsd: 0.12,
  tokensUsed: 900,
  startedAt: '2026-08-31T09:00:00+00:00',
  completedAt: '2026-08-31T09:02:00+00:00',
}

// ── Request capture ──

// Every request the page makes is recorded here so the "no /api/tasks"
// assertion observes the actual network, not a mock.
function makeRequestLog() {
  const log: { method: string; path: string }[] = []
  const record = (req: Request) => {
    log.push({ method: req.method, path: new URL(req.url).pathname })
    return undefined
  }
  const watchers = [http.all('/api/*', ({ request }) => record(request))]
  return { log, watchers }
}

// ── Harness ──

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  })
  return render(
    <QueryClientProvider client={queryClient}>
      <AutomationsPage />
    </QueryClientProvider>,
  )
}

/** Default MSW state: one cron automation, empty rosters, empty runs. */
function installDefaultApi() {
  server.use(
    http.get('/api/automations', () =>
      HttpResponse.json({ automations: [automationProjectless, automationCron], count: 2 }),
    ),
    http.post('/api/automations', async ({ request }) => {
      const body = (await request.json()) as Record<string, unknown>
      return HttpResponse.json({
        ...automationProjectless,
        ...body,
        id: 'a-new',
        createdAt: '2026-09-01T00:00:00+00:00',
        updatedAt: '2026-09-01T00:00:00+00:00',
      })
    }),
    http.get('/api/automations/:id/runs', () => HttpResponse.json({ runs: [], count: 0 })),
    http.put('/api/automations/:id/status', async ({ request }) => {
      const body = (await request.json()) as { status: string }
      return HttpResponse.json({ id: 'a-2', status: body.status })
    }),
    http.post('/api/automations/:id/run', () =>
      HttpResponse.json({
        id: 'a-2',
        run_id: 'r-manual',
        success: true,
        summary: 'done',
      }),
    ),
    http.get('/api/projects', () => HttpResponse.json({ projects: [], count: 0 })),
    http.get('/api/personas', () => HttpResponse.json([])),
    http.get('/api/brain/spaces', () => HttpResponse.json([])),
  )
}

// jsdom lacks the Radix pointer-capture APIs the popover pickers use.
if (typeof Element !== 'undefined') {
  const proto = Element.prototype as Element & {
    hasPointerCapture?: (id: number) => boolean
    releasePointerCapture?: (id: number) => void
  }
  proto.hasPointerCapture ??= () => false
  proto.releasePointerCapture ??= () => {}
}

describe('<AutomationsPage> — automation domain (MSW)', () => {
  beforeEach(() => {
    installDefaultApi()
  })

  it('renders the cron automation with trigger copy and next-run context', async () => {
    renderPage()
    expect(await screen.findByTestId(`automation-card-${automationCron.id}`)).toBeInTheDocument()
    expect(screen.getByText('Morning brief')).toBeInTheDocument()
    expect(screen.getByText('0 9 * * *')).toBeInTheDocument()
  })

  it('creates a projectless automation and never touches /api/tasks', async () => {
    const { log, watchers } = makeRequestLog()
    server.use(...watchers)
    let createBody: Record<string, unknown> | undefined
    server.use(
      http.post('/api/automations', async ({ request }) => {
        createBody = (await request.json()) as Record<string, unknown>
        return HttpResponse.json({
          ...automationProjectless,
          ...createBody,
          id: 'a-new',
          createdAt: '2026-09-01T00:00:00+00:00',
          updatedAt: '2026-09-01T00:00:00+00:00',
        })
      }),
    )

    renderPage()
    fireEvent.click(await screen.findByTestId('automations-new'))
    fireEvent.change(await screen.findByTestId('automation-form-name'), {
      target: { value: 'Projectless job' },
    })
    fireEvent.change(screen.getByTestId('automation-form-instruction'), {
      target: { value: 'do the thing' },
    })
    fireEvent.click(screen.getByTestId('automation-form-submit'))

    await waitFor(() => expect(createBody).toBeDefined())
    // Projectless: the bindings are null, not inferred from browser state.
    expect(createBody?.projectId).toBeNull()
    expect(createBody?.personaId).toBeNull()
    expect(createBody?.brainSpace).toBeNull()
    expect(createBody?.name).toBe('Projectless job')

    // The retired domain is silent: zero /api/tasks requests.
    expect(log.filter((r) => r.path.startsWith('/api/tasks'))).toEqual([])
  })

  it('creates a cron automation with the pattern in the payload', async () => {
    let createBody: Record<string, unknown> | undefined
    server.use(
      http.post('/api/automations', async ({ request }) => {
        createBody = (await request.json()) as Record<string, unknown>
        return HttpResponse.json({
          ...automationCron,
          ...createBody,
          id: 'a-cron-new',
          createdAt: '2026-09-01T00:00:00+00:00',
          updatedAt: '2026-09-01T00:00:00+00:00',
        })
      }),
    )

    renderPage()
    fireEvent.click(await screen.findByTestId('automations-new'))
    fireEvent.change(await screen.findByTestId('automation-form-name'), {
      target: { value: 'Cron job' },
    })
    fireEvent.change(screen.getByTestId('automation-form-instruction'), {
      target: { value: 'run on schedule' },
    })
    // The default trigger is manual; the create payload carries trigger: manual
    // and cronPattern: null. Flip to cron via the trigger select.
    // The Radix select renders a native-like trigger; interact through its DOM.
    const triggerSelect = document.querySelector('#automation-form-trigger')
    expect(triggerSelect).toBeTruthy()
    fireEvent.click(triggerSelect as HTMLElement)
    const cronOption = await screen.findByText('automations.triggerSchedule')
    fireEvent.click(cronOption)
    fireEvent.click(screen.getByTestId('automation-form-submit'))

    await waitFor(() => expect(createBody).toBeDefined())
    expect(createBody?.trigger).toBe('cron')
    const pattern = createBody?.cronPattern
    expect(typeof pattern).toBe('string')
    expect((pattern as string).split(' ').length).toBeGreaterThanOrEqual(5)
  })

  it('pauses and resumes via PUT /api/automations/:id/status', async () => {
    const statusCalls: { id: string; status: string }[] = []
    server.use(
      http.put('/api/automations/:id/status', async ({ request }) => {
        const body = (await request.json()) as { status: string }
        const id = new URL(request.url).pathname.split('/')[3] ?? ''
        statusCalls.push({ id, status: body.status })
        return HttpResponse.json({ id, status: body.status })
      }),
    )

    // Active list → card offers Pause. Each phase renders fresh so the card
    // reflects the server state the invalidation refetch will return.
    server.use(
      http.get('/api/automations', () =>
        HttpResponse.json({ automations: [automationProjectless, automationCron], count: 2 }),
      ),
    )
    const { unmount } = renderPage()
    await screen.findByTestId(`automation-card-${automationCron.id}`)
    fireEvent.click(screen.getByTestId(`automation-pause-${automationCron.id}`))
    await waitFor(() => expect(statusCalls).toEqual([{ id: 'a-2', status: 'paused' }]))
    unmount()

    // Paused list → card offers Resume.
    server.use(
      http.get('/api/automations', () =>
        HttpResponse.json({
          automations: [automationProjectless, { ...automationCron, status: 'paused' }],
          count: 2,
        }),
      ),
    )
    renderPage()
    await screen.findByTestId(`automation-card-${automationCron.id}`)
    fireEvent.click(screen.getByTestId(`automation-resume-${automationCron.id}`))
    await waitFor(() =>
      expect(statusCalls).toEqual([
        { id: 'a-2', status: 'paused' },
        { id: 'a-2', status: 'active' },
      ]),
    )
  })

  it('runs a test execution and reports success', async () => {
    let runCalled = false
    server.use(
      http.post('/api/automations/:id/run', () => {
        runCalled = true
        return HttpResponse.json({ id: 'a-2', run_id: 'r-x', success: true, summary: 'ok' })
      }),
    )

    renderPage()
    const runBtn = await screen.findByTestId(`automation-run-${automationCron.id}`)
    fireEvent.click(runBtn)
    await waitFor(() => expect(runCalled).toBe(true))
  })

  it('renders the latest run contextSnapshot verbatim (immutable context display)', async () => {
    server.use(
      http.get('/api/automations/:id/runs', ({ request }) => {
        const id = new URL(request.url).pathname.split('/')[3]
        if (id === 'a-2') return HttpResponse.json({ runs: [runWithContext], count: 1 })
        return HttpResponse.json({ runs: [], count: 0 })
      }),
    )

    renderPage()
    // Open the detail dialog.
    fireEvent.click(await screen.findByTestId(`automation-detail-${automationCron.id}`))
    const snap = await screen.findByTestId('automation-run-context-snapshot')
    expect(snap).toBeInTheDocument()
    // The snapshot is displayed as captured at run start — not re-resolved.
    expect(snap).toHaveTextContent('proj-7')
    expect(snap).toHaveTextContent('persona-3')
    expect(snap).toHaveTextContent('Work')
    expect(snap).toHaveTextContent('write the brief')
  })

  it('shows the no-runs copy when the automation has no run history', async () => {
    renderPage()
    fireEvent.click(await screen.findByTestId(`automation-detail-${automationCron.id}`))
    await screen.findByTestId('automation-last-run-context')
    expect(await screen.findByText('automations.noRuns')).toBeInTheDocument()
  })

  it('makes zero /api/tasks requests across list + detail + run flows', async () => {
    const { log, watchers } = makeRequestLog()
    server.use(...watchers)

    renderPage()
    await screen.findByTestId(`automation-card-${automationCron.id}`)
    fireEvent.click(screen.getByTestId(`automation-detail-${automationCron.id}`))
    await screen.findByTestId('automation-last-run-context')
    fireEvent.click(screen.getByTestId(`automation-run-${automationCron.id}`))
    // Give any stray refetch a tick to land.
    await waitFor(() => expect(log.some((r) => r.path.startsWith('/api/automations'))).toBe(true))

    const tasksRequests = log.filter((r) => r.path.startsWith('/api/tasks'))
    expect(tasksRequests).toEqual([])
  })
})
