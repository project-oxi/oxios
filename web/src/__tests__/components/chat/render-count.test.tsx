// Render-count regression tests for store subscription scoping (Task 16).
//
// useAssistantActions previously destructured the whole chat store, so the
// `messages` subscription re-rendered every action bar on every streaming
// token. These tests pin the fix: an unrelated store field change must not
// re-render the hook's consumer, and `messages` must be read imperatively
// (inside regenerate) rather than reactively.

import { act, render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useAssistantActions } from '@/components/chat/messages/useAssistantActions'
import { useChatStore } from '@/stores/chat'
import type { ChatMessage } from '@/types'

function makeAssistantMessage(): ChatMessage {
  return {
    id: 'a1',
    role: 'assistant',
    content: 'hello',
    createdAt: 0,
  } as ChatMessage
}

describe('useAssistantActions subscription scope', () => {
  it('does not re-render when an unrelated store field changes', () => {
    let renders = 0
    function Probe() {
      const { actions } = useAssistantActions({ message: makeAssistantMessage() })
      renders++
      return <button type="button">{actions.length}</button>
    }
    render(<Probe />)
    const before = renders
    act(() => {
      useChatStore.setState({ activeMountIds: 'mount-1' })
    })
    expect(renders).toBe(before)
  })

  it('regenerate reads messages imperatively, not via a live subscription', () => {
    // Regression: the hook used to subscribe to `messages` to locate the
    // preceding user message. With the fix, `messages` is read from
    // useChatStore.getState() inside the handler, so appending messages
    // (streaming) must not re-render the consumer.
    let renders = 0
    function Probe() {
      useAssistantActions({ message: makeAssistantMessage() })
      renders++
      return null
    }
    render(<Probe />)
    const before = renders
    act(() => {
      useChatStore.setState((s) => ({
        messages: [...s.messages, { id: 'a2', role: 'assistant', content: 'more' } as ChatMessage],
      }))
    })
    expect(renders).toBe(before)
  })
})
