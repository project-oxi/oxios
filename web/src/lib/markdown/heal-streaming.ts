// Streaming markdown healing.
//
// A partially streamed buffer is fed to the parser every frame. CommonMark
// already handles an unterminated fence gracefully (code block to EOF, which
// is exactly what the artifact card needs), but a GFM table without its
// delimiter row renders as raw pipes, and an unclosed `**`/`` ` `` renders
// literally — both flicker distractingly until the closing token arrives.
//
// Idempotent: complete input is returned unchanged.

/** Whether the buffer ends inside an unterminated fenced block. */
export function inOpenFence(lines: string[]): boolean {
  let open = false
  for (const line of lines) {
    if (/^\s{0,3}(```|~~~)/.test(line)) open = !open
  }
  return open
}

const DELIMITER_ROW = /^\s*\|?\s*:?-{1,}:?\s*(\|\s*:?-{1,}:?\s*)*\|?\s*$/

export function healStreamingMarkdown(src: string): string {
  const lines = src.split('\n')
  if (inOpenFence(lines)) return src

  let out = src

  // 1. Header-only GFM table → synthesize the delimiter row.
  const lastIdx = lines.length - 1
  const last = lines[lastIdx] ?? ''
  const prev = lines[lastIdx - 1] ?? ''
  const isHeaderRow = (l: string) => l.trim().startsWith('|') && l.trim().endsWith('|')
  if (isHeaderRow(last) && !DELIMITER_ROW.test(prev)) {
    const cells = last.trim().slice(1, -1).split('|').length
    out = `${out}\n| ${Array(cells).fill('---').join(' | ')} |`
  }

  // 2. Unclosed inline tokens on the final line, longest marker first so
  //    `**` is not mistaken for two `*`.
  const tail = out.split('\n').at(-1) ?? ''
  for (const marker of ['**', '`', '*', '_']) {
    const count = tail.split(marker).length - 1
    if (count % 2 === 1) out += marker
  }

  return out
}
