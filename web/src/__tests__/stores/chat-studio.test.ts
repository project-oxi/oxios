// WT-3 store contracts (IA design §8.3):
//  - bindProject changes FUTURE turns only: the live session and its
//    transcript survive; the NEXT send payload carries the new project_id.
//  - the one-turn model override is consumed by exactly one send and
//    cleared afterwards; the second send falls back to the session model.
//  - setActiveProject keeps its old reset semantics (start-fresh path).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useChatStore } from '@/stores/chat'

class CapturingSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readyState = CapturingSocket.OPEN
  sent: string[] = []
  onopen: unknown = null
  onmessage: unknown = null
  onclose: unknown = null
  onerror: unknown = null
  send(data: string) {
    this.sent.push(data)
  }
  close() {}
}

describe('chat store — studio context contracts', () => {
  beforeEach(() => {
    useChatStore.getState().disconnect()
    useChatStore.setState({
      connected: true,
      isStreaming: false,
      activeSessionId: 'sess-live',
      activeProjectId: null,
      messages: [{ id: 'm1', role: 'user', content: 'hello', timestamp: 't' }],
      activeModelId: null,
      turnModelOverride: null,
    })
  })

  afterEach(() => {
    useChatStore.getState().disconnect()
    vi.restoreAllMocks()
  })

  it('bindProject keeps the live session and transcript (future turns only)', () => {
    useChatStore.getState().bindProject('proj-1')
    expect(useChatStore.getState().activeProjectId).toBe('proj-1')
    expect(useChatStore.getState().activeSessionId).toBe('sess-live')
    expect(useChatStore.getState().messages).toHaveLength(1)
  })

  it('the next payload carries the bound project; transcript untouched', () => {
    const socket = new CapturingSocket()
    useChatStore.setState({ _ws: socket as unknown as WebSocket })
    useChatStore.getState().bindProject('proj-1')
    useChatStore.getState().sendMessage('second message')
    const frame = JSON.parse(socket.sent.at(-1)!)
    expect(frame.project_id).toBe('proj-1')
    expect(useChatStore.getState().activeSessionId).toBe('sess-live')
  })

  it('the one-turn model override rides exactly one send, then clears', () => {
    const socket = new CapturingSocket()
    useChatStore.setState({
      _ws: socket as unknown as WebSocket,
      activeModelId: 'provider/session-model',
    })
    useChatStore.getState().setTurnModelOverride('provider/one-turn')
    useChatStore.getState().sendMessage('first send')
    const first = JSON.parse(socket.sent.at(-1)!)
    expect(first.model).toBe('provider/one-turn')
    expect(useChatStore.getState().turnModelOverride).toBeNull()

    // The first send flipped isStreaming; reset so the second send takes
    // the immediate path instead of the pending queue.
    useChatStore.setState({ isStreaming: false })
    useChatStore.getState().sendMessage('second send')
    const second = JSON.parse(socket.sent.at(-1)!)
    expect(second.model).toBe('provider/session-model')
  })

  it('setActiveProject keeps its reset semantics (start-fresh path)', () => {
    useChatStore.getState().setActiveProject('proj-2')
    expect(useChatStore.getState().activeProjectId).toBe('proj-2')
    expect(useChatStore.getState().activeSessionId).toBeNull()
    expect(useChatStore.getState().messages).toHaveLength(0)
  })
})
