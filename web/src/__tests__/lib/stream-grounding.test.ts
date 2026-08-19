// Regression: the `grounding` WS chunk (backend chat.rs grounding_from_event)
// had no case in adaptChunk, so it fell through to `default: {events: []}`.
// message.search was never populated and SearchGrounding was dead code.
import { describe, expect, it } from 'vitest'
import { adaptChunk } from '@/lib/stream/adapter'
import { StreamProcessor } from '@/lib/stream/StreamProcessor'
import type { StreamChunk } from '@/types'

function groundingChunk(urls: string[]): StreamChunk {
  return {
    type: 'grounding',
    citations: urls.map((url) => ({ url, title: `T ${url}`, favicon: `${url}/f.ico` })),
    tool_name: 'web_search',
  } as StreamChunk
}

describe('grounding chunk', () => {
  it('adapts into a grounding ChatEvent', () => {
    const { events } = adaptChunk(groundingChunk(['https://a.dev']), { msgId: 'm1' })
    expect(events).toEqual([
      {
        kind: 'grounding',
        messageId: 'm1',
        search: {
          citations: [
            { url: 'https://a.dev', title: 'T https://a.dev', favicon: 'https://a.dev/f.ico' },
          ],
        },
      },
    ])
  })

  it('drops empty citation payloads', () => {
    expect(adaptChunk(groundingChunk([]), { msgId: 'm1' }).events).toEqual([])
  })

  it('accumulates citations across searches and dedupes by url', () => {
    const p = new StreamProcessor('m1')
    for (const chunk of [
      groundingChunk(['https://a.dev']),
      groundingChunk(['https://a.dev', 'https://b.dev']),
    ]) {
      for (const ev of adaptChunk(chunk, { msgId: 'm1' }).events) p.handleEvent(ev)
    }
    const last = p.handleEvent({ kind: 'stream.stop', messageId: 'm1', reason: 'done' })
    expect(last.finished).toBe(true)
    const final = p.materialize({ id: 'm1', role: 'assistant', content: '' } as never)
    expect(final.search?.citations?.map((c) => c.url)).toEqual(['https://a.dev', 'https://b.dev'])
  })
})
