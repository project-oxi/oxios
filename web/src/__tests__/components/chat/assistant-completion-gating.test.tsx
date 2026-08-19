// Component test: post-completion affordance gating (2026-08-20 design).
//
// While an assistant turn is still streaming (`message.generating`), the
// message must not offer turn-scoped actions: the hover action bar
// (copy/regenerate/delete — meaningless or hazardous mid-stream), the
// reactions/rating row, and the save-to-knowledge button. FollowUpChips
// already suppresses itself the same way (`enabled: !generating`).
// All three must appear once the turn completes.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { AssistantMessage } from '@/components/chat/messages/AssistantMessage'
import { ensureLastAssistant, useChatStore } from '@/stores/chat'
import type { ChatMessage } from '@/types'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

vi.mock('@/components/chat/messages/components/BlockStream', () => ({
  BlockStream: () => null,
}))

vi.mock('@/components/chat/follow-up-chips', () => ({
  FollowUpChips: () => null,
}))

vi.mock('@/components/chat/messages/components/reaction-picker', () => ({
  ReactionPicker: () => <div data-testid="reaction-picker" />,
}))

function makeMessage(generating: boolean): ChatMessage {
  return {
    id: 'a1',
    role: 'assistant',
    content: 'hello',
    createdAt: 0,
    generating,
    model: 'test/model',
  } as ChatMessage
}

function renderAssistant(generating: boolean) {
  // AssistantMessage reads the chat store imperatively (regenerate path);
  // seed a user message so the store is in a realistic state.
  useChatStore.setState({
    messages: [
      { id: 'u1', role: 'user', content: 'q', createdAt: 0 } as ChatMessage,
      makeMessage(generating),
    ],
  })
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <QueryClientProvider client={qc}>
      <AssistantMessage message={makeMessage(generating)} sessionId="s1" assistantIndex={1} />
    </QueryClientProvider>,
  )
}

describe('AssistantMessage completion gating', () => {
  it('hides action bar, reactions and save button while generating', () => {
    renderAssistant(true)
    expect(screen.queryByLabelText('common.copy')).toBeNull()
    expect(screen.queryByLabelText('chat.rateUp')).toBeNull()
    expect(screen.queryByText('chat.knowledgeSave')).toBeNull()
  })

  it('shows action bar, reactions and save button once complete', () => {
    renderAssistant(false)
    expect(screen.getByLabelText('common.copy')).toBeTruthy()
    expect(screen.getByLabelText('chat.rateUp')).toBeTruthy()
    expect(screen.getByText('chat.knowledgeSave')).toBeTruthy()
  })
})

describe('ensureLastAssistant placeholder lifecycle', () => {
  it('mounts the placeholder as generating (turn in flight)', () => {
    // Regression: agent_start arrives seconds before the first content
    // chunk (LLM latency). A non-generating placeholder flashed the
    // post-completion action row on an empty bubble for that whole window.
    const { messages } = ensureLastAssistant(
      [{ id: 'u1', role: 'user', content: 'q', createdAt: 0 } as ChatMessage],
      {},
    )
    expect(messages).toHaveLength(2)
    expect(messages[1]!.role).toBe('assistant')
    expect(messages[1]!.generating).toBe(true)
  })
})
