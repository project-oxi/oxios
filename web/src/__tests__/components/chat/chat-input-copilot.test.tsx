// Component test: ChatInput's `variant` prop restricts the composer to the
// capability surface allowed inside the Oxios Copilot dialog. The Copilot
// boundary hides persona picker / model params / file attach / fan-out —
// the dialog posts only to the locked persona over its own WS, so the
// broader affordances must not leak into the UI.

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
})
