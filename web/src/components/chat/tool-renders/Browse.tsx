// Browse render — browsed page as formatted markdown (replaces WebFetchRender).
//
// Renders the result of browse/browse_extract/browse_session/browse_script
// tool calls: a header with URL link + HTTP status, the full page markdown
// via MarkdownMessage, and a footer to open the Search Panel.

import { Globe } from 'lucide-react'
import { MarkdownMessage } from '@/components/chat/markdown-message'
import { usePortalStore } from '@/stores/portal'
import type { ToolRenderComponent } from './registry'

interface BrowseResult {
  url?: string
  title?: string
  markdown?: string
  text?: string
  content?: string
  html?: string
  status?: number
}

function tryJson(s: string): BrowseResult | null {
  try {
    return JSON.parse(s) as BrowseResult
  } catch {
    return null
  }
}

function domain(url: string): string {
  try {
    return new URL(url).hostname
  } catch {
    return url
  }
}

export const BrowseRender: ToolRenderComponent = ({ args, result, isRunning }) => {
  const url = (args?.url ?? args?.uri ?? '') as string
  const parsed: BrowseResult =
    typeof result === 'string' ? (tryJson(result) ?? { content: result }) : (result as BrowseResult)

  const title = parsed.title ?? domain(parsed.url ?? url)
  const href = parsed.url ?? url
  const body = parsed.markdown ?? parsed.text ?? parsed.content ?? ''

  return (
    <div className="space-y-2 text-sm">
      {/* Header: icon + title link + status badge */}
      <div className="flex items-center gap-2">
        <Globe className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="text-primary hover:underline truncate font-medium"
        >
          {title}
        </a>
        {parsed.status !== undefined && (
          <span
            className={`text-xs tabular-nums shrink-0 ${
              parsed.status === 200
                ? 'text-status-success-on-surface'
                : parsed.status < 400
                  ? 'text-status-warning-on-surface'
                  : 'text-destructive'
            }`}
          >
            {parsed.status}
          </span>
        )}
      </div>

      {/* Body: markdown content */}
      {isRunning ? (
        <div className="text-xs text-muted-foreground animate-pulse">Loading page…</div>
      ) : body ? (
        <div className="max-h-80 overflow-y-auto rounded bg-muted/40 p-2">
          <MarkdownMessage messageId="" isStreaming={false}>
            {body}
          </MarkdownMessage>
        </div>
      ) : result != null ? (
        <pre className="p-2 rounded bg-muted text-xs overflow-x-auto max-h-48 whitespace-pre-wrap">
          {typeof result === 'string' ? result.slice(0, 3000) : JSON.stringify(result, null, 2)}
        </pre>
      ) : null}

      {/* Footer: open in panel */}
      {body && (
        <div className="flex justify-end">
          <button
            type="button"
            className="text-xs text-muted-foreground hover:text-primary transition-colors px-1 py-0.5"
            onClick={() => usePortalStore.getState().pushView({ type: 'search', query: title })}
          >
            Open in Panel
          </button>
        </div>
      )}
    </div>
  )
}
