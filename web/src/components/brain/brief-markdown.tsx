import type { ComponentPropsWithoutRef } from 'react'
import ReactMarkdown from 'react-markdown'
import rehypeSanitize, { defaultSchema } from 'rehype-sanitize'
import remarkGfm from 'remark-gfm'
import { cn } from '@/lib/utils'

// The daemon's brief pages link entities as `entity://<id>`. The default
// sanitize schema whitelists only web protocols on href and would strip
// every entity link — extend the whitelist so links survive sanitizing.
const sanitizeSchema = {
  ...defaultSchema,
  protocols: {
    ...defaultSchema.protocols,
    href: [...(defaultSchema.protocols?.href ?? []), 'entity'],
  },
}
interface BriefMarkdownProps {
  children: string
  /** Follow an in-page `entity://` link (loads that entity's page). */
  onEntityLink?: (entityId: string) => void
  className?: string
}

/**
 * Markdown renderer for daemon `brief` pages.
 *
 * Distinct from chat MarkdownMessage: no artifacts/thinking/highlight
 * machinery — just GFM + sanitize. `entity://` links become buttons that
 * navigate within the Brain tab instead of opening a broken external URL.
 */
export function BriefMarkdown({ children, onEntityLink, className }: BriefMarkdownProps) {
  return (
    <div className={cn('text-sm leading-relaxed', className)}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeSanitize, sanitizeSchema]]}
        // react-markdown's defaultUrlTransform whitelists only web
        // protocols and would zero `entity://` hrefs BEFORE the component
        // map runs (independently of rehype-sanitize). Identity transform
        // is safe: rehypeSanitize above already enforces the schema.
        urlTransform={(url) => url}
        components={{
          a({ href, children }: ComponentPropsWithoutRef<'a'>) {
            if (href?.startsWith('entity://')) {
              const id = href.slice('entity://'.length)
              return (
                <button
                  type="button"
                  onClick={() => onEntityLink?.(id)}
                  className="text-primary underline underline-offset-2 hover:opacity-80 transition-opacity"
                >
                  {children}
                </button>
              )
            }
            return (
              <a
                href={href}
                target="_blank"
                rel="noopener noreferrer"
                className="text-primary underline underline-offset-2 hover:opacity-80 transition-opacity"
              >
                {children}
              </a>
            )
          },
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  )
}
