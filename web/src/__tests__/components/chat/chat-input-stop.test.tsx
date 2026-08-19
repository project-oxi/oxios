// Component test: ChatInput's composer swaps the send affordance for a
// destructive Stop button while isStreaming is true. This guards the
// "cancel a turn from the composer" contract (T24 scenario 4) against
// regressions in the render branch.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
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

vi.mock('@/components/chat/model-picker-container', () => ({
  ModelPickerContainer: () => null,
}))

vi.mock('@/components/chat/approval-mode-selector', () => ({
  ApprovalModeSelector: () => null,
}))

vi.mock('@/components/chat/model-params-popover', () => ({
  ModelParamsPopover: () => null,
}))

vi.mock('@/components/chat/fanout-button', () => ({
  FanOutButton: () => null,
}))

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
})

describe('ChatInput streaming controls', () => {
  it('renders the Stop button while isStreaming is true', () => {
    renderInput({ isStreaming: true, onCancel: vi.fn() })
    expect(screen.getByText('chat.stop')).toBeTruthy()
    expect(screen.queryByTitle('chat.send')).toBeNull()
  })

  it('does not render the Stop button when idle', () => {
    renderInput({})
    expect(screen.queryByText('chat.stop')).toBeNull()
  })
})
