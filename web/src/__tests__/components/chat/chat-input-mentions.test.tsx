// Component test: ChatInput's `@`-mention search must resolve the brain
// binding through `resolveBrainSpace` — the same formula `sendMessage`
// uses to record the binding on the session — so an inherited default
// (activeBrainSpace === null but brainDefaults.project/global bound) runs
// the memory search against the resolved space instead of silently
// no-op'ing. Reading the raw `activeBrainSpace` would pass the inherited
// pill through while leaving the mention surface empty.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ChatInput } from '@/components/chat/chat-input'
import { useChatStore } from '@/stores/chat'
import { server } from '../../msw/server'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

vi.mock('@/components/chat/live-activity-bar', () => ({
  LiveActivityBar: () => null,
}))

vi.mock('@/components/chat/model-picker', () => ({
  ModelPickerContainer: () => null,
}))

vi.mock('@/components/chat/approval-mode-selector', () => ({
  ApprovalModeSelector: () => null,
}))

vi.mock('@/components/chat/model-params-popover', () => ({
  ModelParamsPopover: () => null,
}))

vi.mock('@/components/chat/FanOutButton', () => ({
  FanOutButton: () => null,
}))

vi.mock('@/hooks/use-is-touch', () => ({ useIsTouch: () => false }))

// Stable no-op shared by all the props/renderInput mocks; matches the
// other chat-input tests so a single identity satisfies every callback.
const noop = () => {}
const qc = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchInterval: false } },
})

const renderInput = (props: Partial<React.ComponentProps<typeof ChatInput>>) =>
  render(
    <QueryClientProvider client={qc}>
      <ChatInput value="" onChange={noop} onSend={noop} connected {...props} />
    </QueryClientProvider>,
  )

beforeAll(() => {
  if (!window.matchMedia) {
    window.matchMedia = (query: string) =>
      ({
        matches: false,
        media: query,
        addListener: noop,
        removeListener: noop,
        addEventListener: noop,
        removeEventListener: noop,
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList
  }
  for (const proto of [Text.prototype, Range.prototype] as unknown as Array<{
    getClientRects?: unknown
    getBoundingClientRect?: unknown
  }>) {
    proto.getClientRects ??= () => [new DOMRect(0, 0, 1, 1)] as unknown as DOMRectList
    proto.getBoundingClientRect ??= () => new DOMRect(0, 0, 1, 1)
  }
})

describe('ChatInput mention search — resolved brain binding', () => {
  let lastSearchSpace: string | null = null
  let lastSearchUrl: string | null = null

  beforeEach(() => {
    lastSearchSpace = null
    lastSearchUrl = null
    localStorage.clear()
    useChatStore.setState({
      activeBrainSpace: null,
      brainDefaults: { project: null, global: '' },
    })
    server.use(
      http.get('/api/brain/search', ({ request }) => {
        const url = new URL(request.url)
        lastSearchUrl = request.url
        lastSearchSpace = url.searchParams.get('space')
        return HttpResponse.json({
          memory: [
            {
              entity_id: 'ent-1',
              entity_surface: 'Mention hit',
              entity_type: 'concept',
              snippet: 'snippet',
              score: 0.9,
              source_kind: 'memory',
              source_locator: null,
            },
          ],
          documents: [],
          freshness: {
            reconciled_roots: [],
            skipped_roots: [],
            skipped_files: 0,
            stale_after_retry: [],
            dense_coverage: null,
          },
        })
      }),
    )
  })

  afterEach(() => {
    server.resetHandlers()
  })

  it('issues the memory search with the resolved project default when no explicit selection is set', async () => {
    useChatStore.setState({
      activeBrainSpace: null,
      brainDefaults: { project: 'Work', global: '' },
    })
    const { container } = renderInput({})
    const editable = container.querySelector('.ProseMirror') as HTMLElement
    expect(editable).not.toBeNull()
    editable.focus()
    ;(editable.firstElementChild as HTMLElement).textContent = '@wor'
    await waitFor(() => expect(lastSearchUrl).not.toBeNull())
    // The query was dispatched against the resolved project default,
    // not the raw null activeBrainSpace.
    expect(lastSearchSpace).toBe('Work')
  })

  it('issues the memory search with the resolved global default when only the global default is set', async () => {
    useChatStore.setState({
      activeBrainSpace: null,
      brainDefaults: { project: null, global: 'Personal' },
    })
    const { container } = renderInput({})
    const editable = container.querySelector('.ProseMirror') as HTMLElement
    expect(editable).not.toBeNull()
    editable.focus()
    ;(editable.firstElementChild as HTMLElement).textContent = '@per'
    await waitFor(() => expect(lastSearchUrl).not.toBeNull())
    expect(lastSearchSpace).toBe('Personal')
  })

  it('does not issue the memory search when no layer provides a binding (unconnected turn)', async () => {
    useChatStore.setState({
      activeBrainSpace: null,
      brainDefaults: { project: null, global: '' },
    })
    const { container } = renderInput({})
    const editable = container.querySelector('.ProseMirror') as HTMLElement
    expect(editable).not.toBeNull()
    editable.focus()
    ;(editable.firstElementChild as HTMLElement).textContent = '@any'
    // Wait a debounce window plus a margin: the search must NOT have fired.
    await new Promise((r) => setTimeout(r, 350))
    expect(lastSearchUrl).toBeNull()
  })

  it('prefers an explicit session selection over any default', async () => {
    useChatStore.setState({
      activeBrainSpace: 'Work',
      brainDefaults: { project: 'Personal', global: '' },
    })
    const { container } = renderInput({})
    const editable = container.querySelector('.ProseMirror') as HTMLElement
    expect(editable).not.toBeNull()
    editable.focus()
    ;(editable.firstElementChild as HTMLElement).textContent = '@wor'
    await waitFor(() => expect(lastSearchUrl).not.toBeNull())
    expect(lastSearchSpace).toBe('Work')
  })
})
