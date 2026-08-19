import { healStreamingMarkdown } from '@/lib/markdown/heal-streaming'

describe('healStreamingMarkdown', () => {
  it('is identity for complete markdown', () => {
    const src = '| a | b |\n| --- | --- |\n| 1 | 2 |\n'
    expect(healStreamingMarkdown(src)).toBe(src)
  })

  it('adds the delimiter row to a header-only table', () => {
    expect(healStreamingMarkdown('| a | b |')).toBe('| a | b |\n| --- | --- |')
  })

  it('closes trailing inline emphasis', () => {
    expect(healStreamingMarkdown('a **bold')).toBe('a **bold**')
    expect(healStreamingMarkdown('a `code')).toBe('a `code`')
  })

  it('leaves an unterminated fence alone', () => {
    const src = '```rust\nfn main() {}'
    expect(healStreamingMarkdown(src)).toBe(src)
  })

  it('does not heal inside a fenced block', () => {
    const src = '```\n| a | b |\n**x'
    expect(healStreamingMarkdown(src)).toBe(src)
  })
})
