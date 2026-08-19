// SearchGrounding — citation cards + image search results (ported from LobeHub)
//
// LobeHub original: /tmp/lobehub/src/features/Conversation/Messages/components/SearchGrounding.tsx
// Dependencies removed: @lobehub/ui (SearchResultCards, Flexbox), antd-style
// Replaced with: Tailwind utility classes

import { ChevronDown, Globe, Image } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'

// ── Types ──

interface Citation {
  favicon?: string
  id?: string
  title?: string
  url: string
}

interface ImageCitation {
  domain?: string
  imageUri?: string
  sourceUri?: string
  title?: string
}

interface SearchGroundingData {
  citations?: Citation[]
  imageResults?: ImageCitation[]
  imageSearchQueries?: string[]
  searchQueries?: string[]
}

// ── Component ──

interface SearchGroundingProps {
  search: SearchGroundingData
  className?: string
}

export function SearchGrounding({ search, className }: SearchGroundingProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(true)
  const hasCitations = search.citations && search.citations.length > 0
  const hasImages = search.imageResults && search.imageResults.length > 0

  if (!hasCitations && !hasImages) return null

  return (
    <div className={cn('rounded-lg border bg-muted/30 overflow-hidden', className)}>
      {/* Header */}
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="flex w-full items-center gap-2 px-3 py-2 text-sm hover:bg-muted/50 transition-colors"
      >
        <Globe className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
        <span className="font-medium text-muted-foreground">
          {hasCitations
            ? t('search.sources', { count: search.citations!.length })
            : t('search.results')}
        </span>
        {search.searchQueries && search.searchQueries.length > 0 && (
          <span className="text-xs text-muted-foreground/60 ml-2 truncate">
            {search.searchQueries[0]}
          </span>
        )}
        <ChevronDown
          className={cn(
            'w-4 h-4 ml-auto transition-transform duration-200',
            !expanded && '-rotate-90',
          )}
        />
      </button>

      {/* Body */}
      {expanded && (
        <div className="border-t px-3 py-2 space-y-1.5">
          {/* Search queries */}
          {search.searchQueries && search.searchQueries.length > 0 && (
            <div className="flex flex-wrap gap-1 mb-2">
              {search.searchQueries.map((q, i) => (
                <span
                  key={i}
                  className="text-[10px] px-1.5 py-0.5 rounded-full bg-muted text-muted-foreground"
                >
                  {q}
                </span>
              ))}
            </div>
          )}

          {/* Citations */}
          {hasCitations &&
            search.citations!.map((c, i) => (
              <a
                key={c.id ?? i}
                href={c.url}
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-start gap-2 px-2 py-1.5 rounded hover:bg-muted transition-colors group"
              >
                {c.favicon ? (
                  <img
                    src={c.favicon}
                    alt=""
                    className="w-4 h-4 rounded mt-0.5 shrink-0"
                    onError={(e) => {
                      ;(e.target as HTMLImageElement).style.display = 'none'
                    }}
                  />
                ) : (
                  <Globe className="w-4 h-4 text-muted-foreground mt-0.5 shrink-0" />
                )}
                <div className="min-w-0">
                  <div className="text-xs font-medium truncate group-hover:text-primary transition-colors">
                    {c.title || c.url}
                  </div>
                  <div className="text-[10px] text-muted-foreground truncate mt-0.5">{c.url}</div>
                </div>
              </a>
            ))}

          {/* Image results */}
          {hasImages && (
            <div className="mt-2">
              <div className="flex items-center gap-1.5 mb-1.5">
                <Image className="w-3.5 h-3.5 text-muted-foreground" />
                <span className="text-xs font-medium text-muted-foreground">
                  {t('search.images', { count: search.imageResults!.length })}
                </span>
              </div>
              <div className="grid grid-cols-3 gap-1.5">
                {search.imageResults!.slice(0, 9).map((img, i) => (
                  <a
                    key={i}
                    href={img.sourceUri ?? img.imageUri ?? '#'}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="relative aspect-square rounded overflow-hidden border bg-muted group/img"
                  >
                    {img.imageUri ? (
                      <img
                        src={img.imageUri}
                        alt={img.title ?? ''}
                        className="w-full h-full object-cover"
                        loading="lazy"
                      />
                    ) : (
                      <div className="w-full h-full flex items-center justify-center text-muted-foreground">
                        <Image className="w-5 h-5" />
                      </div>
                    )}
                    {img.domain && (
                      <div className="absolute bottom-0 left-0 right-0 bg-scrim px-1 py-0.5 text-[9px] text-scrim-foreground truncate opacity-0 group-hover/img:opacity-100 transition-opacity">
                        {img.domain}
                      </div>
                    )}
                  </a>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}
