// Component test: SpaceSelect / DefaultBrainSelect / ProjectBrainSelect
// option filtering. oxibrain ops address spaces by NAME (ADR-013,
// `space://{name}`); a nameless roster entry would forward its id as the
// `space` query parameter, which the kernel rejects with no actionable
// error and the routed brain tab shows as empty data. The picker must
// exclude nameless spaces — same exclusion the chat picker applies
// (`spaces.some(s => s.name === resolved)`). This test pins that
// contract for the three call sites that share `useSpaceOptions`.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { afterEach, describe, expect, it } from 'vitest'
import { server } from '@/__tests__/msw/server'
import type { SpaceSummary } from '@/types/brain'
import { DefaultBrainSelect, ProjectBrainSelect, SpaceSelect } from './space-select'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

// jsdom does not implement `Element.hasPointerCapture`/`releasePointerCapture`,
// but @radix-ui/react-select's pointerdown handler relies on them. Stub
// them on the Element prototype so opening the dropdown does not throw.
if (typeof Element !== 'undefined') {
  const proto = Element.prototype as Element & {
    hasPointerCapture?: () => boolean
    releasePointerCapture?: () => void
  }
  proto.hasPointerCapture ??= () => false
  proto.releasePointerCapture ??= () => {}
}

// A nameless roster entry would be picked by `spaceParam`'s
// `sp.name || sp.id` fallback (the id is `space-uuid-orphan`); the test
// fails if it surfaces as a selectable option in any of the pickers.
const spaces: SpaceSummary[] = [
  { id: 'space-uuid-1', name: 'Personal', created_at: 0, entity_count: 0, episode_count: 0 },
  { id: 'space-uuid-2', name: 'Work', created_at: 0, entity_count: 0, episode_count: 0 },
  { id: 'space-uuid-orphan', name: '', created_at: 0, entity_count: 0, episode_count: 0 },
]

type UserEvent = ReturnType<typeof userEvent.setup>

function withQueryClient(ui: React.ReactNode) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  })
  return <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
}

async function openAndCollectLabels(user: UserEvent): Promise<string[]> {
  // The trigger carries `data-slot="select-trigger"` (Radix). Items
  // carry `data-slot="select-item"` and are rendered into a Radix
  // portal — query the document body once the dropdown opens.
  await user.click(document.querySelector('[data-slot="select-trigger"]') as HTMLElement)
  // Radix Select mounts items lazily after open; wait one tick.
  await waitFor(() => {
    expect(document.querySelector('[data-slot="select-item"]')).toBeTruthy()
  })
  const labels: string[] = []
  for (const node of document.querySelectorAll('[data-slot="select-item"]')) {
    const text = node.textContent?.trim() ?? ''
    if (text) labels.push(text)
  }
  return labels
}

async function waitForTrigger() {
  await waitFor(() => {
    expect(document.querySelector('[data-slot="select-trigger"]')).toBeTruthy()
  })
}

describe('space-select — nameless spaces are excluded', () => {
  afterEach(() => {
    server.resetHandlers()
  })

  it('SpaceSelect lists only named roster entries', async () => {
    const user = userEvent.setup()
    server.use(http.get('/api/brain/spaces', () => HttpResponse.json(spaces)))
    render(withQueryClient(<SpaceSelect value="" onValueChange={() => {}} />))
    await waitForTrigger()
    const labels = await openAndCollectLabels(user)
    expect(labels).toEqual(['Personal', 'Work'])
    // The orphan id must not be a selectable option in any form
    // (neither the id nor a fallback label).
    expect(labels).not.toContain('space-uuid-orphan')
    expect(labels.some((l) => l.includes('orphan'))).toBe(false)
  })

  it('DefaultBrainSelect lists the none sentinel plus only named roster entries', async () => {
    const user = userEvent.setup()
    server.use(http.get('/api/brain/spaces', () => HttpResponse.json(spaces)))
    render(withQueryClient(<DefaultBrainSelect />))
    await waitForTrigger()
    const labels = await openAndCollectLabels(user)
    expect(labels).toEqual(['brain.none', 'Personal', 'Work'])
    expect(labels).not.toContain('space-uuid-orphan')
  })

  it('ProjectBrainSelect lists only named roster entries', async () => {
    const user = userEvent.setup()
    server.use(http.get('/api/brain/spaces', () => HttpResponse.json(spaces)))
    render(withQueryClient(<ProjectBrainSelect value={null} onValueChange={() => {}} />))
    await waitForTrigger()
    const labels = await openAndCollectLabels(user)
    expect(labels).toEqual(['brain.none', 'Personal', 'Work'])
    expect(labels).not.toContain('space-uuid-orphan')
  })
})
