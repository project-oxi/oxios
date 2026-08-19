// Task 21: sub-agent forks in the chat timeline.
//
// The backend now correlates agent lifecycle events to their owning chat
// session (KernelEvent::Agent{Started,Stopped,Failed}.session_id) and
// forwards `agent_start`/`agent_end` chunks only for the active session's
// forks. The adapter turns those into subagent ChatEvents and the
// StreamProcessor renders one timeline block per fork, keyed by agent_id.
import { describe, expect, it } from 'vitest'
import { adaptChunk } from '@/lib/stream/adapter'
import { StreamProcessor } from '@/lib/stream/StreamProcessor'
import type { StreamChunk } from '@/types'
import type { SubAgentBlockData } from '@/types/chat'

function startChunk(agentId: string, name: string): StreamChunk {
  return { type: 'agent_start', agent_id: agentId, name } as StreamChunk
}

function endChunk(agentId: string, success: boolean): StreamChunk {
  return { type: 'agent_end', agent_id: agentId, success } as StreamChunk
}

describe('agent_start / agent_end chunks', () => {
  it('adapts into subagent ChatEvents', () => {
    const start = adaptChunk(startChunk('a-1', 'researcher'), { msgId: 'm1' })
    expect(start.events).toEqual([
      { kind: 'subagent.start', messageId: 'm1', agentId: 'a-1', name: 'researcher' },
    ])
    const end = adaptChunk(endChunk('a-1', true), { msgId: 'm1' })
    expect(end.events).toEqual([
      { kind: 'subagent.end', messageId: 'm1', agentId: 'a-1', success: true },
    ])
  })

  it('renders one block per fork with running → done status', () => {
    const p = new StreamProcessor('m1')
    for (const ev of adaptChunk(startChunk('a-1', 'researcher'), { msgId: 'm1' }).events) {
      p.handleEvent(ev)
    }
    let blocks = p.materialize({ id: 'm1', role: 'assistant', content: '' } as never).blocks ?? []
    expect(blocks).toHaveLength(1)
    expect((blocks[0] as SubAgentBlockData).type).toBe('subagent')
    expect(blocks[0]).toMatchObject({ agentId: 'a-1', name: 'researcher', status: 'running' })

    for (const ev of adaptChunk(endChunk('a-1', true), { msgId: 'm1' }).events) {
      p.handleEvent(ev)
    }
    blocks = p.materialize({ id: 'm1', role: 'assistant', content: '' } as never).blocks ?? []
    expect(blocks).toHaveLength(1)
    expect(blocks[0]).toMatchObject({ agentId: 'a-1', status: 'done' })
  })

  it('marks failed forks as failed', () => {
    const p = new StreamProcessor('m1')
    for (const ev of adaptChunk(startChunk('a-2', 'coder'), { msgId: 'm1' }).events) {
      p.handleEvent(ev)
    }
    for (const ev of adaptChunk(endChunk('a-2', false), { msgId: 'm1' }).events) {
      p.handleEvent(ev)
    }
    const blocks = p.materialize({ id: 'm1', role: 'assistant', content: '' } as never).blocks ?? []
    expect(blocks[0]).toMatchObject({ agentId: 'a-2', status: 'failed' })
  })

  it('keeps sibling forks as separate blocks and reopens by agent_id', () => {
    const p = new StreamProcessor('m1')
    for (const ev of [
      ...adaptChunk(startChunk('a-1', 'researcher'), { msgId: 'm1' }).events,
      ...adaptChunk(startChunk('a-2', 'coder'), { msgId: 'm1' }).events,
      ...adaptChunk(endChunk('a-1', true), { msgId: 'm1' }).events,
      ...adaptChunk(endChunk('a-2', true), { msgId: 'm1' }).events,
    ]) {
      p.handleEvent(ev)
    }
    const blocks = p.materialize({ id: 'm1', role: 'assistant', content: '' } as never).blocks ?? []
    expect(blocks).toHaveLength(2)
    expect(blocks.map((b) => b.type)).toEqual(['subagent', 'subagent'])
    expect(blocks.map((b) => b.id)).toEqual(['a-a-1', 'a-a-2'])
  })

  it('closes a still-running fork when the stream ends (cancelled turn)', () => {
    const p = new StreamProcessor('m1')
    for (const ev of adaptChunk(startChunk('a-1', 'researcher'), { msgId: 'm1' }).events) {
      p.handleEvent(ev)
    }
    p.handleEvent({ kind: 'stream.stop', messageId: 'm1', reason: 'aborted' })
    const blocks = p.materialize({ id: 'm1', role: 'assistant', content: '' } as never).blocks ?? []
    expect(blocks[0]).toMatchObject({ agentId: 'a-1', status: 'done' })
  })

  it('ignores an agent_end without a matching start', () => {
    const p = new StreamProcessor('m1')
    for (const ev of adaptChunk(endChunk('ghost', true), { msgId: 'm1' }).events) {
      p.handleEvent(ev)
    }
    const blocks = p.materialize({ id: 'm1', role: 'assistant', content: '' } as never).blocks ?? []
    expect(blocks).toEqual([])
  })
})
