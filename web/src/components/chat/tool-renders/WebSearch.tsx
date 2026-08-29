// WebSearch render — Claude-Desktop-style search card:
// query header + result count badge + collapsible result list (favicon,
// title, domain, snippet). Results arrive as a structured array from the
// kernel's search tools (`tool_end.results`); falls back to URL extraction
// from plain-text summaries for legacy paths.
import { ChevronDown, Globe, Search } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { domainOf, faviconUrl } from '@/lib/favicon'
import { usePortalStore } from '@/stores/portal'
import type { ToolRenderComponent } from './registry'

const COLLAPSED_COUNT = 3

export const WebSearchRender: ToolRenderComponent = ({ args, result, isRunning }) => {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const query = (args?.query ?? args?.search_query ?? '') as string
  const results = parseResults(result)
  const visible = expanded ? results : results.slice(0, COLLAPSED_COUNT)

  return (
    <div className="space-y-2 text-sm">
      {/* Query header + result count badge */}
      <div className="flex items-center gap-2 text-xs">
        <Globe className="w-3.5 h-3.5 text-muted-foreground" />
        <span className="text-muted-foreground italic truncate max-w-[48ch]">
          {query.length > 80 ? `${query.slice(0, 80)}...` : query}
        </span>
        {!isRunning && results.length > 0 && (
          <span className="ml-auto shrink-0 rounded-full bg-muted px-2 py-0.5 text-[10px] text-muted-foreground">
            {t('chat.transparency.resultCount', { count: results.length })}
          </span>
        )}
      </div>

      {/* Results */}
      {isRunning ? (
        <div className="flex items-center gap-2 text-muted-foreground">
          <span className="inline-block w-2 h-2 rounded-full bg-status-warning animate-pulse" />
          {t('chat.transparency.searching')}
        </div>
      ) : results.length > 0 ? (
        <div className="space-y-1.5">
          {visible.map((r, i) => (
            <a
              key={i}
              href={r.url}
              target="_blank"
              rel="noopener noreferrer"
              className="block px-2 py-1.5 rounded hover:bg-muted transition-colors group"
            >
              <div className="flex items-start gap-2">
                <img
                  src={r.favicon || faviconUrl(r.url)}
                  alt=""
                  className="w-4 h-4 rounded mt-0.5 shrink-0"
                  onError={(e) => {
                    ;(e.target as HTMLImageElement).style.display = 'none'
                  }}
                />
                <div className="min-w-0">
                  <div className="text-xs font-medium truncate group-hover:text-primary transition-colors">
                    {r.title || r.url}
                  </div>
                  {r.snippet && (
                    <div className="text-xs text-muted-foreground line-clamp-2 mt-0.5">
                      {r.snippet}
                    </div>
                  )}
                  <div className="text-[10px] text-muted-foreground/60 truncate mt-0.5">
                    {domainOf(r.url)}
                  </div>
                </div>
              </div>
            </a>
          ))}
          {results.length > COLLAPSED_COUNT && (
            <div className="flex justify-between pt-1">
              <button
                type="button"
                className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-primary transition-colors"
                onClick={() => setExpanded(!expanded)}
              >
                <ChevronDown
                  className={`w-3 h-3 transition-transform ${expanded ? 'rotate-180' : ''}`}
                />
                {expanded
                  ? t('chat.transparency.showFewerResults')
                  : t('chat.transparency.showAllResults', { count: results.length })}
              </button>
              <button
                type="button"
                className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-primary transition-colors"
                onClick={() => usePortalStore.getState().pushView({ type: 'search' })}
              >
                <Search className="w-3 h-3" />
                Open in Panel
              </button>
            </div>
          )}
        </div>
      ) : result != null ? (
        <pre className="p-2 rounded bg-muted text-xs overflow-x-auto max-h-48 whitespace-pre-wrap">
          {typeof result === 'string' ? result.slice(0, 3000) : JSON.stringify(result, null, 2)}
        </pre>
      ) : null}
    </div>
  )
}

// ── Helpers ──

interface ParsedResult {
  url: string
  title?: string
  snippet?: string
  favicon?: string
}

function parseResults(raw: unknown): ParsedResult[] {
  if (!raw) return []

  // Try structured array
  if (Array.isArray(raw)) {
    return raw.map((item) => {
      if (typeof item === 'string') return { url: item }
      return {
        url: item?.url ?? item?.link ?? '',
        title: item?.title,
        snippet: item?.snippet ?? item?.description,
        favicon: item?.favicon,
      }
    })
  }

  // Try string — extract URLs
  if (typeof raw === 'string') {
    const urlRegex = /https?:\/\/[^\s<>"{}|\\^`[\]]+/g
    const urls = raw.match(urlRegex)
    if (urls) return urls.map((url) => ({ url }))
  }

  return []
}
