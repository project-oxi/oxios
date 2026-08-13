import { HttpResponse, http } from 'msw'

// ---------------------------------------------------------------------------
// Default MSW handlers — explicit stubs for every endpoint the app touches
// at boot (so tests don't need to know the surface). Tests that need a
// specific response should override a handler with `server.use(...)` in their
// own setup; the per-test override replaces the default and the default
// handler is restored by `server.resetHandlers()` between tests.
// ---------------------------------------------------------------------------

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

  // ── Brain daemon (RFC-047) — default responses. Tests that need a
  //    populated surface should override with `server.use(...)`.
  http.get('/api/brain/status', () =>
    HttpResponse.json({ available: true, space: 'personal', episodes: 0 }),
  ),
  http.get('/api/brain/stats', () =>
    HttpResponse.json({ episodes: 0, entities: 0, statements: 0, contradictions: 0 }),
  ),
  http.get('/api/brain/search', () => HttpResponse.json({ items: [], total_found: 0 })),
  http.get('/api/brain/contradictions', () => HttpResponse.json([])),
  http.get('/api/brain/entity/:id', () => HttpResponse.json(null)),
  http.get('/api/brain/timeline', () => HttpResponse.json([])),
  http.get('/api/brain/why/:statement_id', () => HttpResponse.json(null)),
  http.post('/api/brain/recall', () => HttpResponse.json({ context: null })),

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

  // ── Knowledge / persona / mounts ────────────────────────────────────────
  http.get('/api/knowledge', () => HttpResponse.json({ items: [] })),
  http.get('/api/persona', () => HttpResponse.json({ persona: null })),
  http.get('/api/mounts', () => HttpResponse.json({ mounts: [] })),

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

  // ── Quick-ask (sidebar mini chat) ───────────────────────────────────────
  http.get('/api/quick-ask', () => HttpResponse.json({ items: [] })),
]
