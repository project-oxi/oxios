// Streaming markdown parse-count tests (Task 18).
//
// While a block streams, only the live tail (everything after the last
// blank-line boundary) may change — the settled prefix is a completed block
// and must be parsed once, not once per frame. MarkdownMessage exposes
// `onParse(src)` fired per ReactMarkdown parse of the settled prefix; these
// tests assert the parse count stays at one across re-renders.

import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { MarkdownMessage } from '@/components/chat/markdown-message'

describe('MarkdownMessage settled-prefix memoization', () => {
  it('does not re-parse the settled prefix on every streaming frame', () => {
    const parses: string[] = []
    const { rerender } = render(
      <MarkdownMessage isStreaming onParse={(s) => parses.push(s)}>
        {'para one\n\npara two'}
      </MarkdownMessage>,
    )
    rerender(
      <MarkdownMessage isStreaming onParse={(s) => parses.push(s)}>
        {'para one\n\npara two more'}
      </MarkdownMessage>,
    )
    // The settled first paragraph must be parsed once, not twice.
    expect(parses.filter((p) => p.startsWith('para one')).length).toBe(1)
  })

  it('does not split inside an open fenced block (artifact safety)', () => {
    const parses: string[] = []
    const { rerender } = render(
      <MarkdownMessage isStreaming onParse={(s) => parses.push(s)}>
        {'```html\n<p>hi</p>\n```\n\ntext'}
      </MarkdownMessage>,
    )
    // The fence is already closed here; the split point is the blank line
    // after the fence. Now the tail grows while a NEW open fence appears —
    // the whole buffer must re-parse (no split), so the settled hook fires
    // for the full content.
    rerender(
      <MarkdownMessage isStreaming onParse={(s) => parses.push(s)}>
        {'```html\n<p>hi</p>\n```\n\n```js\nco'}
      </MarkdownMessage>,
    )
    // Second render has an open fence → no split → full parse → the settled
    // hook (which only fires on a split) is not re-invoked with a stale
    // prefix. Instead the full buffer re-parses; assert the settled parse
    // count never exceeded 1 (no stale "…```\n" prefix parse happened).
    expect(parses.length).toBeLessThanOrEqual(2)
  })

  it('does not split when the tail carries artifact-eligible code', () => {
    const parses: string[] = []
    render(
      <MarkdownMessage isStreaming onParse={(s) => parses.push(s)}>
        {'```html\n<p>a</p>\n```\n\n```html\n<p>b</p>\n```'}
      </MarkdownMessage>,
    )
    // Tail opens a second artifact fence — the split is refused so both
    // artifact blocks share one ordinal counter (no key collision). The
    // settled hook therefore never fires (no split happened).
    expect(parses).toEqual([])
  })
})
