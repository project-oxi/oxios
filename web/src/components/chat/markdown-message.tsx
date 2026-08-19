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
import ReactMarkdown from 'react-markdown'
import rehypeHighlight from 'rehype-highlight'
import rehypeRaw from 'rehype-raw'
import rehypeSanitize, { defaultSchema } from 'rehype-sanitize'
import remarkGfm from 'remark-gfm'
import { ArtifactCard, ArtifactContext } from '@/components/chat/artifact/artifact-card'
import { rehypeLinkCard } from '@/components/chat/markdown-plugins/rehype-link-card'
import { rehypeThinking } from '@/components/chat/markdown-plugins/rehype-thinking'
import { cn } from '@/lib/utils'
import { languageToArtifactType } from '@/types/artifact'
import { preprocessArtifacts } from './markdown-plugins/preprocess-artifacts'

// ── Code block with language label + copy button ──────────────────

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
  const [copied, setCopied] = useState(false)

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
              Copied
            </>
          ) : (
            <>
              <Copy className="w-3 h-3" />
              Copy
            </>
          )}
        </button>
      </div>
      <pre className="overflow-x-auto p-3 text-xs leading-relaxed">
        <code className={`language-${language ?? 'text'} font-mono`}>{children}</code>
      </pre>
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
      return (
        <ArtifactCard type={artifactType} language={language} source="language">
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
  /** Whether the owning message is still streaming (drives live preview). */
  isStreaming?: boolean
}

export const MarkdownMessage = memo(function MarkdownMessage({
  children,
  className,
  messageId = '',
  isStreaming = false,
}: MarkdownMessageProps) {
  return (
    <ArtifactContext.Provider value={{ messageId, isStreaming }}>
      <div className={cn('prose prose-sm dark:prose-invert max-w-none', className)}>
        <ReactMarkdown
          remarkPlugins={[remarkGfm]}
          rehypePlugins={[
            [rehypeRaw, { allowDangerousHtml: true }],
            [rehypeSanitize, sanitizeSchema],
            // After sanitize — defaultSchema strips unknown properties.
            rehypeMarkInlineCode,
            rehypeHighlight,
            rehypeThinking,
            rehypeLinkCard,
          ]}
          components={markdownComponents}
        >
          {preprocessArtifacts(children)}
        </ReactMarkdown>
      </div>
    </ArtifactContext.Provider>
  )
})

function extractText(node: React.ReactNode): string {
  if (typeof node === 'string') return node
  if (Array.isArray(node)) return node.map((c) => (typeof c === 'string' ? c : '')).join('')
  return String(node ?? '')
}
