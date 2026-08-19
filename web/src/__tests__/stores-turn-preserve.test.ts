// Contract pin: a full sub-agent turn sequence must preserve its streamed
// content through completion (2026-08-20). Replays the exact frame order
// captured from the live WS: agent_start → model → reasoning start/delta/end
// → token → usage → agent_end → done. Guards the done handler and the
// StreamProcessor lifecycle against regressing into a content-losing path
// (e.g. a future done-phase rewrite that drops blocks or content).

import { describe, expect, it, type Mock, vi } from 'vitest'
import { __clearStreamProcessorsForTesting, useChatStore } from '@/stores/chat'

function seed(): Mock {
  localStorage.clear()
  sessionStorage.clear()
  __clearStreamProcessorsForTesting()
  const send: Mock = vi.fn()
  useChatStore.setState({
    messages: [],
    isStreaming: false,
    streamStartedAt: null,
    connected: true,
    _ws: { readyState: 1, send, close: vi.fn() } as unknown as WebSocket,
    _pendingQueue: [],
    _reconnectTimer: null,
    _pingTimer: null,
    activeSessionId: null,
    activeModelId: 'gpt-test',
    pendingModel: null,
  })
  return send
}

describe('turn completion preserves streamed content', () => {
  it('keeps reasoning + answer blocks after done (sub-agent turn)', () => {
    const send = seed()
    useChatStore.getState().sendMessage('9와 10을 곱한 결과만 알려줘.')

    const h = useChatStore.getState().handleChunk
    h({ type: 'agent_start', agent_id: '2f3073ae', name: '9와 10을 곱한 결과만 알려줘.' })
    h({ type: 'model', model: 'zai-coding-plan/glm-5-turbo' })
    h({ type: 'reasoning', subtype: 'start' })
    h({ type: 'reasoning', content: '9 곱하기 10은 90', source: 'thinking' })
    h({ type: 'reasoning', subtype: 'end' })
    h({ type: 'token', content: '90' })
    h({ type: 'token', content: '' })
    h({ type: 'usage', input_tokens: 4243, output_tokens: 4 })
    h({ type: 'token', content: '' })
    h({ type: 'agent_end', agent_id: '2f3073ae', success: true })
    h({ type: 'done', session_id: 's-live', phase: 'execute', duration_ms: 2613 })

    const s = useChatStore.getState()
    const last = s.messages[s.messages.length - 1]!
    expect(send).toHaveBeenCalledTimes(1)
    // Content must survive the done phase intact.
    expect(last.role).toBe('assistant')
    expect(last.content).toBe('90')
    expect((last.blocks ?? []).length).toBeGreaterThan(0)
    expect(last.generating).toBe(false)
  })
})
