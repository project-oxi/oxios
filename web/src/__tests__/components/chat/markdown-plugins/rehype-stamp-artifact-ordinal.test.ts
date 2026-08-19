// Unit tests for rehypeStampArtifactOrdinal — the plugin that stamps
// document-order ordinals onto artifact-eligible code nodes at parse time.
//
// These tests drive the plugin directly on hast trees (no markdown parse
// needed — the plugin's contract is purely tree-shaped): every fenced
// ```html ```svg ```mermaid ```jsx ```tsx block gets 0,1,2,… in document
// order, and non-artifact code (plain text blocks, inline code, unknown
// languages) is left unstamped.

import type { Element, Root } from 'hast'
import { describe, expect, it } from 'vitest'
import { rehypeStampArtifactOrdinal } from '@/components/chat/markdown-plugins/rehype-stamp-artifact-ordinal'

function code(language: string): Element {
  return {
    type: 'element',
    tagName: 'code',
    properties: { className: [`language-${language}`] },
    children: [{ type: 'text', value: `{ /* ${language} */ }` }],
  }
}

function pre(children: Element): Element {
  return { type: 'element', tagName: 'pre', properties: {}, children: [children] }
}

function p(children: Element): Element {
  return { type: 'element', tagName: 'p', properties: {}, children: [children] }
}

/** Run the plugin, then return the stamped ordinal of every element with one. */
function run(tree: Root): Array<{ lang: string; ordinal?: string }> {
  // The plugin's transformer ignores file/next — call it as a plain function.
  // (unified's Plugin type binds `this: Processor`; cast the factory to dodge it.)
  const plugin = rehypeStampArtifactOrdinal as unknown as () => (t: Root) => void
  const stamp = plugin()
  stamp(tree)
  const out: Array<{ lang: string; ordinal?: string }> = []
  const walk = (node: Element | Root): void => {
    if (node.type === 'element') {
      const cls = node.properties?.className
      const lang = Array.isArray(cls)
        ? (cls.find((c) => String(c).startsWith('language-')) ?? '')
        : ''
      out.push({
        lang: String(lang),
        ordinal: node.properties?.dataArtifactOrdinal as string | undefined,
      })
    }
    if (node.type === 'element' || node.type === 'root') {
      for (const child of node.children) walk(child as Element)
    }
  }
  walk(tree)
  return out
}

describe('rehypeStampArtifactOrdinal', () => {
  it('stamps ordinals in document order across same-type artifact blocks', () => {
    const tree: Root = {
      type: 'root',
      children: [pre(code('html')), pre(code('html')), pre(code('html'))],
    }
    const stamped = run(tree).filter((n) => n.ordinal !== undefined)
    expect(stamped).toEqual([
      { lang: 'language-html', ordinal: '0' },
      { lang: 'language-html', ordinal: '1' },
      { lang: 'language-html', ordinal: '2' },
    ])
  })

  it('counts only artifact-eligible languages, not plain or inline code', () => {
    const tree: Root = {
      type: 'root',
      children: [
        pre(code('text')), // not artifact-eligible → no stamp
        pre(code('html')), // ordinal 0
        p(code('html')), // inline code (parent p) → no stamp
        pre(code('jsx')), // ordinal 1
        pre(code('unknownlang')), // no stamp
      ],
    }
    const stamped = run(tree).filter((n) => n.ordinal !== undefined)
    expect(stamped).toEqual([
      { lang: 'language-html', ordinal: '0' },
      { lang: 'language-jsx', ordinal: '1' },
    ])
  })

  it('is idempotent across repeated runs (stable identity for re-renders)', () => {
    const tree: Root = {
      type: 'root',
      children: [pre(code('mermaid')), pre(code('svg'))],
    }
    const first = run(tree).filter((n) => n.ordinal !== undefined)
    // Run again — the transformer would re-stamp, but values must not shift
    // (the tree is not re-parsed between renders in production; this guards
    // against double-application shifting ordinals).
    const second = run(tree).filter((n) => n.ordinal !== undefined)
    expect(first).toEqual(second)
  })
})
