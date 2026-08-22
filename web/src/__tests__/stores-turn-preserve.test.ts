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

describe('out-of-order terminal chunks must not wipe the turn', () => {
  it('keeps blocks when done races ahead of usage/agent_end (live capture 2026-08-20)', () => {
    // Real WS capture: the gateway's done and the kernel bus's usage/agent_end
    // are emitted from two concurrent tasks and can interleave. When done
    // lands first it finishes + deletes the StreamProcessor; the stragglers
    // then created a FRESH processor whose patch stamped `blocks: []` over
    // the live-rendered message (bubble collapsed 255 → 39 chars live).
    const send = seed()
    useChatStore.getState().sendMessage('한 문장으로만 답해줘.')

    const h = useChatStore.getState().handleChunk
    h({ type: 'agent_start', agent_id: 'ab12', name: '한 문장으로만 답해줘.' })
    h({ type: 'model', model: 'zai-coding-plan/glm-5-turbo' })
    h({ type: 'reasoning', subtype: 'start' })
    h({ type: 'reasoning', content: '짧게 답한다', source: 'thinking' })
    h({ type: 'reasoning', subtype: 'end' })
    h({ type: 'token', content: '한 문장 답변' })
    h({ type: 'done', session_id: 's-race', phase: 'execute', duration_ms: 3200 })
    // Stragglers AFTER done — the racy order seen in production.
    h({ type: 'usage', input_tokens: 100, output_tokens: 5 })
    h({ type: 'agent_end', agent_id: 'ab12', success: true })
    // A tail token whose rAF flush lands after done; the trailing non-token
    // chunk flushes it synchronously (no timers needed in the test).
    h({ type: 'token', content: '끝.' })
    h({ type: 'usage', input_tokens: 105, output_tokens: 6 })

    const s = useChatStore.getState()
    const last = s.messages[s.messages.length - 1]!
    expect(send).toHaveBeenCalledTimes(1)
    expect(last.role).toBe('assistant')
    // The streamed text must survive — not be truncated to the straggler tail.
    expect(last.content).toBe('한 문장 답변끝.')
    const types = (last.blocks ?? []).map((b) => b.type)
    expect(types).toContain('subagent')
    expect(types).toContain('reasoning')
    expect(types).toContain('text')
    expect((last.blocks ?? []).length).toBeGreaterThan(2)
    expect(last.generating).toBe(false)
  })
})
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
