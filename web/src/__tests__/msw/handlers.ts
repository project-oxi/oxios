import { HttpResponse, http } from 'msw'

// ---------------------------------------------------------------------------
// Default MSW handlers — explicit stubs for every endpoint the app touches
// at boot (so tests don't need to know the surface). Tests that need a
// specific response should override a handler with `server.use(...)` in their
// own setup; the per-test override replaces the default and the default
// handler is restored by `server.resetHandlers()` between tests.
// ---------------------------------------------------------------------------

/**
 * Scoped brain endpoints require the `space` query parameter (Task 2
 * contract). Mirror the backend's 400 so tests cannot accidentally depend
 * on unscoped calls. Returns the 400 response when space is missing.
 */
function missingSpace(request: Request): Response | null {
  const space = new URL(request.url).searchParams.get('space')
  return space ? null : HttpResponse.json({ error: 'space parameter required' }, { status: 400 })
}

export const handlers = [
  // ── Budget ──────────────────────────────────────────────────────────────
  http.get('/api/budget', () =>
    HttpResponse.json({
      items: [],
      total: 0,
      page: 1,
      limit: 100,
    }),
  ),

  // ── A2A (agent-to-agent) ────────────────────────────────────────────────
  http.get('/api/a2a/agents', () => HttpResponse.json({ agents: [] })),
  http.get('/api/a2a/messages', () => HttpResponse.json({ messages: [] })),
  http.get('/api/a2a/topology', () => HttpResponse.json({ nodes: [], edges: [] })),

  // ── Skills ──────────────────────────────────────────────────────────────
  http.get('/api/skills', () => HttpResponse.json({ skills: [] })),
  // ── Config ──────────────────────────────────────────────────────────────
  // The brain tab's default-brain selector reads `brain.default_space` via
  // GET /api/config and persists it with a deep-merge PATCH.
  http.get('/api/config', () => HttpResponse.json({})),
  http.patch('/api/config', () =>
    HttpResponse.json({
      config: {},
      hot_reload: { applied_immediately: [], requires_restart: [], total_changed: 0 },
    }),
  ),

  // ── Brain daemon (RFC-047, brain-chat binding) — default responses
  //    matching the daemon wire contracts (bare arrays, resource
  //    payloads). Scoped endpoints REQUIRE `space` (400 without it);
  //    `/status` and `/spaces` are global. Tests that need a populated
  //    surface should override with `server.use(...)`.
  http.get('/api/brain/status', () =>
    HttpResponse.json({
      available: true,
      pending_extraction: null,
      binary: { installed: true, path: '/usr/bin/oxibrain', version: '0.8.0' },
    }),
  ),
  http.get(
    '/api/brain/stats',
    ({ request }) =>
      missingSpace(request) ??
      HttpResponse.json({ episodes: 0, entities: 0, statements: 0, contradictions: 0 }),
  ),
  http.get(
    '/api/brain/search',
    ({ request }) =>
      missingSpace(request) ??
      HttpResponse.json({
        memory: [],
        documents: [],
        freshness: {
          reconciled_roots: [],
          skipped_roots: [],
          skipped_files: 0,
          stale_after_retry: [],
          dense_coverage: null,
        },
      }),
  ),
  http.get(
    '/api/brain/document-history',
    ({ request }) => missingSpace(request) ?? HttpResponse.json([]),
  ),
  http.get(
    '/api/brain/contradictions',
    ({ request }) => missingSpace(request) ?? HttpResponse.json([]),
  ),
  http.get(
    '/api/brain/entity/:id',
    ({ request }) => missingSpace(request) ?? HttpResponse.json([]),
  ),
  http.get('/api/brain/timeline', ({ request }) => missingSpace(request) ?? HttpResponse.json([])),
  http.get(
    '/api/brain/why/:statement_id',
    ({ request }) => missingSpace(request) ?? HttpResponse.json(null),
  ),
  http.get(
    '/api/brain/brief',
    ({ request }) => missingSpace(request) ?? HttpResponse.json({ markdown: null }),
  ),
  http.get('/api/brain/graph', ({ request }) => missingSpace(request) ?? HttpResponse.json(null)),
  http.get(
    '/api/brain/operations/:section',
    ({ request }) => missingSpace(request) ?? HttpResponse.json([]),
  ),
  http.get('/api/brain/spaces', () => HttpResponse.json([])),
  http.get(
    '/api/brain/space',
    ({ request }) =>
      missingSpace(request) ??
      HttpResponse.json({
        space: new URL(request.url).searchParams.get('space'),
        space_id: null,
        entity_count: 0,
        episode_count: 0,
        contradiction_count: 0,
        recent_entities: [],
      }),
  ),
  http.post('/api/brain/recall', async ({ request }) => {
    const body = (await request.json()) as { space?: string }
    if (!body.space) {
      return HttpResponse.json({ error: 'space parameter required' }, { status: 400 })
    }
    return HttpResponse.json({ context: null })
  }),

  // ── Auth ────────────────────────────────────────────────────────────────
  // The dev token endpoint mirrors the daemon's `POST /api/auth/token`.
  // Stub it so any code path that auto-requests a dev token (e.g. on
  // session restore) does not blow up the `onUnhandledRequest: 'error'`
  // policy.
  http.post('/api/auth/token', () => HttpResponse.json({ token: 'dev-token', expiresIn: 3600 })),

  // ── Health / version ────────────────────────────────────────────────────
  http.get('/api/health', () => HttpResponse.json({ status: 'ok' })),
  http.get('/api/version', () => HttpResponse.json({ version: '0.0.0-test' })),

  // ── Sessions / projects ─────────────────────────────────────────────────
  // Used by the chat sidebar and project picker. Returning empty arrays
  // keeps boot-time queries deterministic.
  http.get('/api/sessions', () => HttpResponse.json({ sessions: [] })),
  http.get('/api/projects', () => HttpResponse.json({ projects: [] })),

  // ── Calendar / email / cron ─────────────────────────────────────────────
  // RFC-018 subsystems — these default to empty lists. Components that
  // surface them must work without data.
  http.get('/api/calendar/events', () => HttpResponse.json({ events: [] })),
  http.get('/api/email/inbox', () => HttpResponse.json({ messages: [] })),
  http.get('/api/cron/jobs', () => HttpResponse.json({ jobs: [] })),

  // ── Knowledge / persona ──────────────────────────────────────────────────
  http.get('/api/knowledge', () => HttpResponse.json({ items: [] })),
  http.get('/api/persona', () => HttpResponse.json({ persona: null })),

  // ── Models ──────────────────────────────────────────────────────────────
  http.get('/api/models', () => HttpResponse.json({ providers: [], models: [], default: null })),

  // ── Cost / token usage ──────────────────────────────────────────────────
  http.get('/api/cost', () =>
    HttpResponse.json({
      total: 0,
      byDay: [],
      byModel: [],
    }),
  ),
  http.get('/api/token-usage', () => HttpResponse.json({ total: 0, sessions: [] })),

  // ── Notifications ───────────────────────────────────────────────────────
  http.get('/api/notifications', () => HttpResponse.json({ items: [] })),

  // ── Oxios Copilot (one-shot dialog stub) ─────────────────────────────────
  http.get('/api/quick-ask', () => HttpResponse.json({ items: [] })),
]
