import { describe, expect, it } from 'vitest'
import { adaptChunk } from './adapter'

describe('adaptChunk — reasoning delta threading', () => {
  it('forwards `source` on a no-subtype reasoning delta', () => {
    // A regular reasoning chunk (no `subtype`) with `source: 'compaction'` must
    // produce a `reasoning.delta` event that carries `source` so the
    // StreamProcessor can stamp the resulting block.
    const result = adaptChunk(
      { type: 'reasoning', content: 'compaction complete', source: 'compaction' },
      { msgId: 'm1' },
    )
    expect(result.events).toEqual([
      {
        kind: 'reasoning.delta',
        messageId: 'm1',
        text: 'compaction complete',
        source: 'compaction',
      },
    ])
  })

  it('omits `source` for reasoning deltas without one', () => {
    const result = adaptChunk({ type: 'reasoning', content: 'thinking…' }, { msgId: 'm1' })
    expect(result.events).toEqual([{ kind: 'reasoning.delta', messageId: 'm1', text: 'thinking…' }])
  })

  it('drops unknown source values (only thinking|compaction reach the event)', () => {
    // The wire format allows free-form `source`; the adapter narrows to the
    // closed `ReasoningBlock['source']` union so the processor contract holds.
    const result = adaptChunk(
      { type: 'reasoning', content: 'x', source: 'something_else' },
      { msgId: 'm1' },
    )
    expect(result.events).toEqual([{ kind: 'reasoning.delta', messageId: 'm1', text: 'x' }])
  })

  it('does not emit a source on reasoning.start / reasoning.end', () => {
    // Lifecycle subtype markers don't carry source — only real deltas do.
    const start = adaptChunk({ type: 'reasoning', subtype: 'start' }, { msgId: 'm1' })
    const end = adaptChunk({ type: 'reasoning', subtype: 'end' }, { msgId: 'm1' })
    expect(start.events).toEqual([{ kind: 'reasoning.start', messageId: 'm1' }])
    expect(end.events).toEqual([{ kind: 'reasoning.end', messageId: 'm1' }])
  })
})

describe('adaptChunk — tool_end error reason', () => {
  it('carries output_summary as the error message for gate denials', () => {
    // Exact wire shape of a GatedTool denial: no `tool_result`, reason only in
    // `output_summary`. The card's error paragraph must show the 🔒 reason,
    // not the generic "Tool error" fallback.
    const result = adaptChunk(
      {
        type: 'tool_end',
        tool_name: 'exec',
        tool_call_id: 't1',
        duration_ms: 0,
        is_error: true,
        output_summary:
          "🔒 Access denied: CSpace lacks EXECUTE capability for tool 'exec' [CSpace]",
      },
      { msgId: 'm1' },
    )
    const end = result.events.find((e) => e.kind === 'tool.end') as {
      error?: { message?: string }
    }
    expect(end.error?.message).toContain('🔒 Access denied')
  })

  it('keeps a structured tool_result string as the error message', () => {
    const result = adaptChunk(
      { type: 'tool_end', tool_call_id: 't2', is_error: true, tool_result: 'boom' },
      { msgId: 'm1' },
    )
    const end = result.events.find((e) => e.kind === 'tool.end') as {
      error?: { message?: string }
    }
    expect(end.error?.message).toBe('boom')
  })
})
