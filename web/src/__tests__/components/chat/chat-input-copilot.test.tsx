// Component test: ChatInput's `variant` prop restricts the composer to the
// capability surface allowed inside the Oxios Copilot dialog. The Copilot
// boundary hides persona picker / model params / file attach / fan-out —
// the dialog posts only to the locked persona over its own WS, so the
// broader affordances must not leak into the UI.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { ChatInput } from '@/components/chat/chat-input'

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

// Enter-to-send is desktop keyboard behavior, but this jsdom window reports
// `ontouchstart`, which useIsTouch would read as a touch device and block
// the Enter path under test.
vi.mock('@/hooks/use-is-touch', () => ({ useIsTouch: () => false }))

const noop = () => {}
const qc = new QueryClient()

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
  // jsdom has no layout: Text nodes and Ranges lack the geometry methods
  // ProseMirror's scrollToSelection reads when the DOMObserver applies a
  // typed change (coordsAtPos → singleRect). Return a 1×1 rect at the
  // origin so the scroll math stays finite.
  for (const proto of [Text.prototype, Range.prototype] as unknown as Array<{
    getClientRects?: unknown
    getBoundingClientRect?: unknown
  }>) {
    proto.getClientRects ??= () => [new DOMRect(0, 0, 1, 1)] as unknown as DOMRectList
    proto.getBoundingClientRect ??= () => new DOMRect(0, 0, 1, 1)
  }
})

describe('ChatInput variant restrictions', () => {
  it('chat variant shows the file attach button', () => {
    renderInput({})
    expect(screen.getByTitle('chat.attachFiles')).toBeTruthy()
  })

  it('copilot variant hides the file attach button', () => {
    const { container } = renderInput({ variant: 'copilot' })
    expect(screen.queryByTitle('chat.attachFiles')).toBeNull()
    expect(container.querySelector('input[type="file"]')).toBeNull()
  })

  it('copilot variant hides the persona picker and model params popover', () => {
    renderInput({ variant: 'copilot' })
    expect(screen.queryByLabelText('chat.persona.label')).toBeNull()
    expect(screen.queryByLabelText('chat.modelParams')).toBeNull()
  })

  it('copilot variant keeps the mention popover closed and Enter sends a draft ending in "@x"', async () => {
    const onSend = vi.fn()
    const onChange = vi.fn()
    const { container } = renderInput({ variant: 'copilot', onSend, onChange })
    const editable = container.querySelector('.ProseMirror') as HTMLElement
    expect(editable).not.toBeNull()
    // Simulate typing "@x" the way the browser would: mutate the
    // contenteditable DOM and let ProseMirror's DOMObserver turn the change
    // into a document transaction, firing ChatInput's onUpdate — the path
    // that (un-gated) derives mentionQuery from the /@(\S*)$/ match.
    editable.focus()
    ;(editable.firstElementChild as HTMLElement).textContent = '@x'
    await waitFor(() => expect(onChange).toHaveBeenCalledWith('@x'))
    // No stray "no results"/searching popover may open in the copilot variant.
    expect(screen.queryByText('mention.noResults')).toBeNull()
    expect(screen.queryByText('mention.searchPlaceholder')).toBeNull()
    // Enter must still send: the mentionQuery gate must not be tripped.
    fireEvent.keyDown(editable, { key: 'Enter' })
    expect(onSend).toHaveBeenCalledWith('@x', [], [])
  })
})
