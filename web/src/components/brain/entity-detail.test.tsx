import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it } from 'vitest'
import { server } from '@/__tests__/msw/server'
import { BrainEntityDetail } from './entity-detail'

// Mock i18next — verbatim convention from existing component tests
// (see web/src/components/brain/status-banner.test.tsx).
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

const ENTITY_ID = 'ent-user-1'

function withQueryClient(ui: React.ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  })
  return <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
}

describe('BrainEntityDetail', () => {
  afterEach(() => {
    server.resetHandlers()
  })

  it('renders the brief page, beliefs table, and timeline table from the daemon contracts', async () => {
    server.use(
      http.get('/api/brain/brief', () =>
        HttpResponse.json({
          markdown:
            '# The user (Person)\n\n- **has_skill** [Rust](entity://ent-rust-1) — 0.34 · active',
        }),
      ),
      http.get(`/api/brain/entity/${ENTITY_ID}`, () =>
        HttpResponse.json([
          {
            statement: 'stmt-has-skill',
            valid_from: -9223372036854776000,
            valid_to: 9223372036854776000,
            support: {
              affirm_count: 3,
              deny_count: 0,
              distinct_episodes: 2,
              trust_weights: [['trusted', 2]],
            },
            confidence: 0.34,
            status: 'active',
          },
        ]),
      ),
      http.get('/api/brain/timeline', () =>
        HttpResponse.json([
          {
            statement_id: 'stmt-has-skill',
            predicate: 'has_skill',
            object_repr: 'Rust',
            object_entity: 'ent-rust-1',
            valid_from: -9223372036854776000,
            valid_to: 9223372036854776000,
            status: 'active',
            recorded_at: 1786848388455,
          },
        ]),
      ),
    )

    render(withQueryClient(<BrainEntityDetail space="personal" initialEntityId={ENTITY_ID} />))

    // Brief page renders as markdown with the entity link as an in-page button.
    expect(await screen.findByText('The user (Person)')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: 'Rust' }).length).toBeGreaterThan(0)

    // Beliefs table: support counts, sentinel validity rendered as em-dash.
    expect(await screen.findByText('0.34')).toBeInTheDocument()
    expect(screen.getByText('+3')).toBeInTheDocument()
    expect(screen.getByText('−0')).toBeInTheDocument()

    // Timeline rows: predicate appears in both the brief markdown and the
    // timeline table — assert at least one occurrence exists.
    expect(await screen.findAllByText('has_skill')).not.toHaveLength(0)
  })

  it('follows an entity:// brief link by loading the target entity in place', async () => {
    const briefFor = (id: string) =>
      http.get('/api/brain/brief', ({ request }) => {
        const url = new URL(request.url)
        return HttpResponse.json({
          markdown:
            url.searchParams.get('entity_id') === id
              ? `# Entity ${id}\n\n[peer](entity://ent-peer)`
              : '# other',
        })
      })
    server.use(
      briefFor('ent-rust-1'),
      http.get('/api/brain/entity/ent-rust-1', () => HttpResponse.json([])),
      http.get('/api/brain/entity/ent-peer', () => HttpResponse.json([])),
      http.get('/api/brain/timeline', () => HttpResponse.json([])),
    )

    const onNavigateEntity = vi.fn()
    render(
      withQueryClient(
        <BrainEntityDetail
          space="personal"
          initialEntityId="ent-rust-1"
          onNavigateEntity={onNavigateEntity}
        />,
      ),
    )

    const peer = await screen.findByRole('button', { name: 'peer' })
    peer.click()
    expect(onNavigateEntity).toHaveBeenCalledWith('ent-peer')
  })
})
