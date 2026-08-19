// markdown-plugins — rehype plugin that stamps artifact-eligible code nodes
// with their document-order ordinal.
//
// WHY (defect D6 remediation): artifact identity keys embed an ordinal so two
// untitled artifacts of the same type in one message do not collide. The first
// implementation assigned the ordinal from a per-render closure counter in
// MarkdownMessage's ArtifactContext (nextOrdinal()). That was unsound:
//
//   1. ArtifactCard subscribes to the portal store, so a card can re-render
//      (any stack change) without MarkdownMessage re-rendering. The closure
//      counter then advanced past the card's real position, the key drifted,
//      and clicking the card pushed a duplicate panel entry instead of
//      closing.
//   2. The counter was per-MarkdownMessage while BlockStream renders one
//      MarkdownMessage per text block sharing the same messageId — two
//      same-type untitled artifacts in different blocks of one message still
//      collided.
//
// Fix: stamp the ordinal ONTO the hast node at parse time, derived from the
// node's position in the tree (document order), which is deterministic and
// stable across re-renders. The `code` component in markdown-message.tsx
// reads `node.properties.dataArtifactOrdinal` and passes it to ArtifactCard
// as a plain prop. Block identity is handled separately by `blockId` in the
// context (BlockStream passes its block id).
//
// Runs AFTER rehypeSanitize (default schema strips unknown properties), like
// rehypeMarkInlineCode.

import type { Element, ElementContent, Root } from 'hast'
import type { Plugin } from 'unified'
import { languageToArtifactType } from '@/types/artifact'

const LANGUAGE_RE = /(?:^|\s)language-(\w+)/

export const rehypeStampArtifactOrdinal: Plugin<[], Root> = () => {
  return (tree) => {
    let ordinal = 0
    stamp(tree, () => ordinal++)
  }
}

/** Depth-first document-order walk; stamps artifact-eligible code elements.
 *  Mirrors markdown-message's `code` component branch exactly: block code is
 *  always `<pre><code>`, and the artifact branch only runs when the node is
 *  NOT inline (`dataInlineCode` handled by rehypeMarkInlineCode) and its
 *  language maps to a renderable artifact type. */
function stamp(node: Root | ElementContent, next: () => number, parentTag?: string): void {
  if (node.type !== 'element' && node.type !== 'root') return

  if (node.type === 'element' && isArtifactCode(node, parentTag)) {
    node.properties = {
      ...node.properties,
      dataArtifactOrdinal: String(next()),
    }
  }

  const children = (node as Element | Root).children
  if (Array.isArray(children)) {
    for (const c of children)
      stamp(c as ElementContent, next, node.type === 'element' ? node.tagName : undefined)
  }
}

/** A fenced code block whose language maps to a renderable artifact type.
 *  Inline code (parent not `pre`) is excluded — the component's artifact
 *  branch never sees it either. */
function isArtifactCode(node: Element, parentTag?: string): boolean {
  if (node.tagName !== 'code' || parentTag !== 'pre') return false
  const cls = node.properties?.className
  const joined = Array.isArray(cls) ? cls.join(' ') : typeof cls === 'string' ? cls : ''
  const m = LANGUAGE_RE.exec(joined)
  if (!m?.[1]) return false
  return languageToArtifactType(m[1]) != null
}
