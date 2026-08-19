// MarkdownMessage — enhanced markdown renderer with syntax highlighting +
// copy button AND interactive artifact previews.
//
// Pipeline:
//   remark-gfm → rehype-raw → rehypeArtifactExtract → rehype-sanitize
//     → rehype-highlight → rehype-thinking → rehype-link-card
//
// Security: rehype-raw parses model output as HTML (needed so <think> tags and
// the <lobeArtifact> protocol work). rehype-sanitize then strips dangerous
// constructs (event handlers, scripts, iframes). rehypeArtifactExtract runs
// BEFORE sanitize so it can pull artifact code out of the sanitizable tree as
// a plain text node (sanitize would otherwise mangle the code's children).
//
// Artifacts: a fenced ```html / ```svg / ```mermaid / ```jsx block (or a
// <lobeArtifact> tag, normalised by the extract plugin into the same shape)
// renders as an interactive ArtifactCard that opens a sandboxed preview panel
// — NOT as a plain code listing. See components/chat/artifact/.

import type { Element, ElementContent, Root, RootContent } from 'hast'
import type { Schema } from 'hast-util-sanitize'
import { Check, Copy } from 'lucide-react'
import { type ComponentPropsWithoutRef, memo, type ReactNode, useCallback, useState } from 'react'
import { useTranslation } from 'react-i18next'
import ReactMarkdown from 'react-markdown'
import rehypeHighlight from 'rehype-highlight'
import rehypeRaw from 'rehype-raw'
import rehypeSanitize, { defaultSchema } from 'rehype-sanitize'
import remarkGfm from 'remark-gfm'
import { ArtifactCard, ArtifactContext } from '@/components/chat/artifact/artifact-card'
import { rehypeLinkCard } from '@/components/chat/markdown-plugins/rehype-link-card'
import { rehypeStampArtifactOrdinal } from '@/components/chat/markdown-plugins/rehype-stamp-artifact-ordinal'
import { rehypeThinking } from '@/components/chat/markdown-plugins/rehype-thinking'
import { healStreamingMarkdown, inOpenFence } from '@/lib/markdown/heal-streaming'
import { cn } from '@/lib/utils'
import { languageToArtifactType } from '@/types/artifact'
import { preprocessArtifacts } from './markdown-plugins/preprocess-artifacts'

// ── Code block with language label + copy button ──────────────────
/** Fenced blocks render this many lines inline; anything longer gets a collapse control. */
const COLLAPSE_LINES = 24

function CodeBlock({
  language,
  code,
  children,
}: {
  language?: string
  /** Raw source text — copy payload. Never derived from the rendered nodes. */
  code: string
  /** Highlighted hast children as rendered by react-markdown. */
  children: ReactNode
}) {
  const { t } = useTranslation()
  const [copied, setCopied] = useState(false)
  const [expanded, setExpanded] = useState(false)

  const lineCount = code.split('\n').length
  const collapsible = lineCount > COLLAPSE_LINES
  const collapsed = collapsible && !expanded

  const handleCopy = useCallback(() => {
    navigator.clipboard.writeText(code).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    })
  }, [code])

  return (
    <div className="group relative my-3 rounded-lg border bg-muted/50 overflow-hidden">
      <div className="flex items-center justify-between px-3 py-1.5 bg-muted border-b">
        <span className="text-xs text-muted-foreground font-mono">{language ?? 'text'}</span>
        <button
          type="button"
          onClick={handleCopy}
          className="flex items-center gap-1 text-xs text-muted-foreground transition-opacity opacity-0 group-hover:opacity-100 focus-visible:opacity-100 hover:text-foreground"
        >
          {copied ? (
            <>
              <Check className="w-3 h-3" />
              {t('common.copied')}
            </>
          ) : (
            <>
              <Copy className="w-3 h-3" />
              {t('common.copy')}
            </>
          )}
        </button>
      </div>
      <pre
        data-collapsed={collapsed || undefined}
        className={cn(
          'overflow-x-auto p-3 text-xs leading-relaxed',
          collapsed && 'max-h-96 overflow-y-hidden',
        )}
      >
        <code className={`language-${language ?? 'text'} font-mono`}>{children}</code>
      </pre>
      {collapsible && (
        <button
          type="button"
          onClick={() => setExpanded((v) => !v)}
          className="w-full border-t px-3 py-1.5 text-xs text-muted-foreground hover:text-foreground"
        >
          {expanded ? t('chat.code.collapse') : t('chat.code.expand', { count: lineCount })}
        </button>
      )}
    </div>
  )
}

// ── External link ─────────────────────────────────────────────────

function ExternalLink({ href, children, ...props }: ComponentPropsWithoutRef<'a'>) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noopener noreferrer"
      className="text-primary underline underline-offset-2 hover:opacity-80 transition-opacity"
      {...props}
    >
      {children}
    </a>
  )
}

// ── Inline code ───────────────────────────────────────────────────

function InlineCode({ children }: ComponentPropsWithoutRef<'code'>) {
  return <code className="px-1.5 py-0.5 rounded bg-muted text-[0.85em] font-mono">{children}</code>
}

// ── Sanitize schema (extending default) ───────────────────────────
//
// defaultSchema already strips scripts/event handlers. We extend it to:
//   • allow class names on code/pre/span (syntax highlight + CodeBlock)
//   • allow summary/details (for thinking-block rewrite)
//   • keep the safe-by-default denylist for iframes, embeds, etc.
//
// Artifact code needs NO extra allowance here: rehypeArtifactExtract converts
// <lobeArtifact> into a normal <pre><code class="language-*"> whose child is a
// text node, and text + className are already permitted by default.

const sanitizeSchema: Schema = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    code: [...(defaultSchema.attributes?.code ?? []), ['className']],
    pre: [...(defaultSchema.attributes?.pre ?? []), ['className']],
    span: [...(defaultSchema.attributes?.span ?? []), ['className']],
    div: [...(defaultSchema.attributes?.div ?? []), ['className']],
    details: [...(defaultSchema.attributes?.details ?? []), ['className']],
    summary: [...(defaultSchema.attributes?.summary ?? []), ['className']],
    img: [...(defaultSchema.attributes?.img ?? []), ['alt']],
  },
  tagNames: [...(defaultSchema.tagNames ?? []), 'details', 'summary'],
}

// ── Component map for react-markdown ──────────────────────────────

/** Recursively extract raw text from a hast node (immune to highlight spans). */
function hastToText(node: ElementContent | undefined | null): string {
  if (!node) return ''
  if (node.type === 'text') return node.value
  if (node.type === 'element') return node.children.map((c) => hastToText(c)).join('')
  // comment / doctype / raw carry no printable code text we want.
  return ''
}

/** react-markdown v10 dropped the `inline` prop (grep the published lib: zero
 *  occurrences), so the component map has no way to tell `` `x` `` from a
 *  fenced block — every inline span was rendering as a full CodeBlock card,
 *  emitting a `<div>` inside a `<p>`. Block code is *always* `<pre><code>`;
 *  mark everything else so the `code` component can branch on structure. */
function rehypeMarkInlineCode() {
  return (tree: Root) => {
    const walk = (node: Root | RootContent, parentTag?: string): void => {
      if (node.type === 'element' && node.tagName === 'code' && parentTag !== 'pre') {
        node.properties = { ...node.properties, dataInlineCode: 'true' }
      }
      if (node.type !== 'element' && node.type !== 'root') return
      for (const child of node.children) {
        walk(child, node.type === 'element' ? node.tagName : undefined)
      }
    }
    walk(tree)
  }
}

const markdownComponents = {
  pre: ({ children }: ComponentPropsWithoutRef<'pre'>) => <>{children}</>,
  code({ node, className, children }: ComponentPropsWithoutRef<'code'> & { node?: Element }) {
    if (node?.properties?.dataInlineCode === 'true') return <InlineCode>{children}</InlineCode>

    const langMatch = /language-(\w+)/.exec(className ?? '')
    const language = langMatch ? langMatch[1] : undefined

    // Renderable language → interactive artifact preview card.
    const artifactType = languageToArtifactType(language)
    if (artifactType) {
      // Read raw text from the hast node so syntax-highlight spans never
      // corrupt the artifact content.
      const raw = hastToText(node) || extractText(children)
      const ordinal = Number(node?.properties?.dataArtifactOrdinal ?? 0)
      return (
        <ArtifactCard type={artifactType} language={language} source="language" ordinal={ordinal}>
          {raw}
        </ArtifactCard>
      )
    }

    // rehype-highlight has already wrapped every token in `<span class="hljs-*">`.
    // Render those children so highlighting survives, but take the copy payload
    // from the hast node — flattening React children drops every token.
    return (
      <CodeBlock language={language} code={hastToText(node)}>
        {children}
      </CodeBlock>
    )
  },
  a: ExternalLink,
}

// ── Main ──────────────────────────────────────────────────────────

interface MarkdownMessageProps {
  children: string
  className?: string
  /** Owning message id — lets artifact cards coordinate with the panel store. */
  messageId?: string
  /** Owning block id (BlockStream passes its block id). Scopes artifact
   *  identity so same-type untitled artifacts in different blocks of one
   *  message do not collide. */
  blockId?: string
  /** Whether the owning message is still streaming (drives live preview). */
  isStreaming?: boolean
  /** Test hook: fired once per ReactMarkdown parse of the settled prefix. */
  onParse?: (src: string) => void
}

/** Shared parse/render pipeline. */
function MarkdownCore({
  children,
  isStreaming = false,
}: {
  children: string
  isStreaming?: boolean
}) {
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      rehypePlugins={[
        [rehypeRaw, { allowDangerousHtml: true }],
        [rehypeSanitize, sanitizeSchema],
        // After sanitize — defaultSchema strips unknown properties.
        rehypeMarkInlineCode,
        // Also after sanitize: stamps artifact ordinals onto code nodes.
        rehypeStampArtifactOrdinal,
        rehypeHighlight,
        rehypeThinking,
        rehypeLinkCard,
      ]}
      components={markdownComponents}
    >
      {preprocessArtifacts(isStreaming ? healStreamingMarkdown(children) : children)}
    </ReactMarkdown>
  )
}

/** Settled (completed) prefix of a streaming block. Memoized so it is only
 *  re-parsed when the prefix actually grows — the live tail re-parses per
 *  frame, the settled prefix must not. */
const SettledMarkdown = memo(
  function SettledMarkdown({ src, onParse }: { src: string; onParse?: (src: string) => void }) {
    onParse?.(src)
    return <MarkdownCore>{src}</MarkdownCore>
  },
  // The props that matter for identity: src only. A per-render onParse
  // closure (test hook) must not defeat the memo.
  (prev, next) => prev.src === next.src,
)

/** Split a streaming buffer at the last blank-line boundary: everything
 *  before it is a completed block-level construct, everything after is the
 *  live tail. Returns null when splitting would be unsound:
 *  - buffer ends inside an open fence (split point would land mid-code)
 *  - the tail carries artifact-eligible code — it renders through a separate
 *    pipeline whose artifact ordinal counter restarts at 0, which would
 *    collide with the settled subtree's ordinals (same defect class as the
 *    pre-blockId key collision). */
function splitStreaming(
  src: string,
  isStreaming: boolean,
): { settled: string; tail: string } | null {
  if (!isStreaming) return null
  if (inOpenFence(src.split('\n'))) return null
  const splitAt = src.lastIndexOf('\n\n')
  if (splitAt <= 0) return null
  const tail = src.slice(splitAt)
  if (tailHasArtifactCode(tail)) return null
  return { settled: src.slice(0, splitAt), tail }
}

/** True when the buffer contains a fenced code block whose language maps to
 *  a renderable artifact type, or a <lobeArtifact> tag. */
function tailHasArtifactCode(src: string): boolean {
  if (src.includes('<lobeArtifact')) return true
  for (const line of src.split('\n')) {
    const m = /^\s{0,3}```(\w+)/.exec(line)
    if (m && languageToArtifactType(m[1]) != null) return true
  }
  return false
}

export const MarkdownMessage = memo(function MarkdownMessage({
  children,
  className,
  messageId = '',
  blockId = '',
  isStreaming = false,
  onParse,
}: MarkdownMessageProps) {
  const split = splitStreaming(children, isStreaming)
  return (
    <ArtifactContext.Provider value={{ messageId, blockId, isStreaming }}>
      <div className={cn('prose prose-sm max-w-none', className)}>
        {split ? (
          <>
            <SettledMarkdown src={split.settled} onParse={onParse} />
            <MarkdownCore isStreaming>{split.tail}</MarkdownCore>
          </>
        ) : (
          <MarkdownCore isStreaming={isStreaming}>{children}</MarkdownCore>
        )}
      </div>
    </ArtifactContext.Provider>
  )
})

function extractText(node: React.ReactNode): string {
  if (typeof node === 'string') return node
  if (Array.isArray(node)) return node.map((c) => (typeof c === 'string' ? c : '')).join('')
  return String(node ?? '')
}
