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
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
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

const runOlder = {
  id: 'r-0',
  automationId: 'a-2',
  sessionId: null,
  trigger: 'manual',
  status: 'failed',
  contextSnapshot: {
    projectId: null,
    personaId: 'persona-3',
    brainSpace: null,
    trigger: 'manual',
    verify: { enabled: false, maxIterations: 3 },
    instruction: 'write the brief',
  },
  summary: null,
  resultContent: null,
  error: 'provider timeout',
  costUsd: null,
  tokensUsed: 120,
  startedAt: '2026-08-30T09:00:00+00:00',
  completedAt: '2026-08-30T09:00:40+00:00',
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

function renderPage(queryClient?: QueryClient) {
  const qc =
    queryClient ??
    new QueryClient({
      defaultOptions: { queries: { retry: false, refetchInterval: false } },
    })
  return {
    queryClient: qc,
    ...render(
      <QueryClientProvider client={qc}>
        <AutomationsPage />
      </QueryClientProvider>,
    ),
  }
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

  it('renders the full run-history list (newest first) plus lastError in detail', async () => {
    server.use(
      http.get('/api/automations', () =>
        HttpResponse.json({
          automations: [
            automationProjectless,
            { ...automationCron, lastError: 'provider timeout' },
          ],
          count: 2,
        }),
      ),
      http.get('/api/automations/:id/runs', ({ request }) => {
        const id = new URL(request.url).pathname.split('/')[3]
        if (id === 'a-2') return HttpResponse.json({ runs: [runWithContext, runOlder], count: 2 })
        return HttpResponse.json({ runs: [], count: 0 })
      }),
    )

    renderPage()
    fireEvent.click(await screen.findByTestId(`automation-detail-${automationCron.id}`))
    const history = await screen.findByTestId('automation-run-history')
    const rows = within(history).getAllByTestId('automation-run-history-row')
    expect(rows).toHaveLength(2)
    // Newest first: succeeded cron run, then the failed manual run.
    expect(rows[0]).toHaveTextContent('succeeded')
    expect(rows[0]).toHaveTextContent('Brief delivered')
    expect(rows[1]).toHaveTextContent('failed')
    expect(rows[1]).toHaveTextContent('provider timeout')
    // The definition's lastError is surfaced in the detail dialog.
    expect(await screen.findByTestId('automation-last-error')).toHaveTextContent('provider timeout')
  })

  it('shows the quiet empty state (no history rows) when runs are empty', async () => {
    renderPage()
    fireEvent.click(await screen.findByTestId(`automation-detail-${automationCron.id}`))
    await screen.findByTestId('automation-last-run-context')
    expect(await screen.findByText('automations.noRuns')).toBeInTheDocument()
    expect(screen.queryByTestId('automation-run-history')).not.toBeInTheDocument()
  })

  it('labels the template-gallery CTA with the automation copy, not the retired Task copy', async () => {
    server.use(http.get('/api/automations', () => HttpResponse.json({ automations: [], count: 0 })))
    renderPage()
    const gallery = await screen.findByTestId('automation-template-gallery')
    const ctas = within(gallery)
      .getAllByRole('button')
      .filter((b) => /addFromTemplate|addTask/.test(b.textContent ?? ''))
    expect(ctas.length).toBeGreaterThan(0)
    for (const cta of ctas) {
      expect(cta).toHaveTextContent('automations.addFromTemplate')
      expect(cta).not.toHaveTextContent('cronJobs.templates.addTask')
    }
  })
  it('keeps the shared brain-spaces query successful when the daemon returns a null body', async () => {
    server.use(http.get('/api/brain/spaces', () => HttpResponse.json(null)))
    const qc = new QueryClient({
      defaultOptions: { queries: { retry: false, refetchInterval: false } },
    })
    renderPage(qc)
    fireEvent.click(await screen.findByTestId('automations-new'))
    // The picker hides with an empty roster...
    await waitFor(() =>
      expect(screen.queryByTestId('automation-brain-picker')).not.toBeInTheDocument(),
    )
    // ...and the shared ['brain','spaces'] query must be a successful [] —
    // not an error state that would poison useBrainSpaces consumers.
    const state = qc.getQueryState(['brain', 'spaces'])
    expect(state?.status).toBe('success')
    expect(state?.data).toEqual([])
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

  // ── Fix round 2: edit flow (double-option clears + non-disruptive trigger) ──

  it('clears the project binding via an explicit null and shows No project after', async () => {
    const bound = { ...automationCron, projectId: 'proj-1' }
    let currentList = [automationProjectless, bound]
    const putBodies: Record<string, unknown>[] = []
    server.use(
      http.get('/api/projects', () =>
        HttpResponse.json({ projects: [{ id: 'proj-1', name: 'Alpha' }], count: 1 }),
      ),
      http.get('/api/automations', () =>
        HttpResponse.json({ automations: currentList, count: currentList.length }),
      ),
      http.put('/api/automations/:id', async ({ request, params }) => {
        const body = (await request.json()) as Record<string, unknown>
        putBodies.push(body)
        const id = params.id
        currentList = currentList.map((a) => (a.id === id ? { ...a, ...body } : a))
        return HttpResponse.json({ ...bound, ...body })
      }),
    )

    renderPage()
    const card = await screen.findByTestId(`automation-card-${automationCron.id}`)
    fireEvent.click(within(card).getByRole('button', { name: 'common.edit' }))
    // The dialog preloads the bound project.
    const pill = await screen.findByTestId('automation-project-picker')
    expect(pill).toHaveTextContent('Alpha')
    // Pick "No project" and save.
    fireEvent.click(pill)
    fireEvent.click(await screen.findByTestId('automation-project-picker-none'))
    fireEvent.click(screen.getByTestId('automation-form-submit'))

    await waitFor(() => expect(putBodies.length).toBeGreaterThan(0))
    const firstPut = putBodies[0] ?? {}
    expect(firstPut.projectId).toBeNull()

    // Reopen: the picker reflects the cleared binding.
    fireEvent.click(await screen.findByTestId(`automation-card-${automationCron.id}`))
    const card2 = await screen.findByTestId(`automation-card-${automationCron.id}`)
    fireEvent.click(within(card2).getByRole('button', { name: 'common.edit' }))
    expect(await screen.findByTestId('automation-project-picker')).toHaveTextContent(
      'automations.noProject',
    )
  })

  it('does not fire PUT /trigger when editing only the name of a paused automation', async () => {
    const { log, watchers } = makeRequestLog()
    const paused = { ...automationCron, status: 'paused' }
    const triggerPuts: string[] = []
    server.use(
      ...watchers,
      http.get('/api/automations', () =>
        HttpResponse.json({ automations: [automationProjectless, paused], count: 2 }),
      ),
      http.put('/api/automations/:id/trigger', async ({ request }) => {
        triggerPuts.push(new URL(request.url).pathname)
        return HttpResponse.json({ id: paused.id, trigger: 'cron' })
      }),
      http.put('/api/automations/:id', async ({ request }) => {
        const body = (await request.json()) as Record<string, unknown>
        return HttpResponse.json({ ...paused, ...body })
      }),
    )

    renderPage()
    const card = await screen.findByTestId(`automation-card-${automationCron.id}`)
    fireEvent.click(within(card).getByRole('button', { name: 'common.edit' }))
    const nameInput = await screen.findByTestId('automation-form-name')
    fireEvent.change(nameInput, { target: { value: 'Renamed while paused' } })
    fireEvent.click(screen.getByTestId('automation-form-submit'))

    await waitFor(() =>
      expect(log.some((r) => r.method === 'PUT' && !r.path.endsWith('/trigger'))).toBe(true),
    )
    // The definition PUT went through; the trigger config was NOT re-sent.
    expect(triggerPuts).toEqual([])
    // And the paused state is untouched on the card.
    expect((await screen.findAllByText('automations.status.paused')).length).toBeGreaterThan(0)
  })
})
