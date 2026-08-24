import { Search } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Select } from '@/components/ui/select'
import { useBrainSearch } from '@/hooks/use-brain'
import type { BrainSearchHit } from '@/types/brain'

// Match the oxibrain daemon's RetrievalMode enum
// (oxibrain-mcp/server.rs: enum values hybrid|lexical|lexical-vector|graph|community).
const MODES = ['hybrid', 'lexical', 'lexical-vector', 'graph', 'community'] as const
const MODE_OPTIONS = MODES.map((m) => ({ label: m, value: m }))

function HitRow({ hit, onSelect }: { hit: BrainSearchHit; onSelect: (id: string) => void }) {
  return (
    <button
      type="button"
      onClick={() => onSelect(hit.entity_id)}
      className="w-full text-left rounded-lg border p-3 transition-colors hover:bg-accent/50 focus-visible:bg-accent/50"
    >
      <div className="flex items-baseline justify-between gap-3">
        <span className="font-medium truncate">{hit.entity_surface}</span>
        <span className="text-xs text-muted-foreground tabular-nums shrink-0">
          {hit.score.toFixed(3)}
        </span>
      </div>
      {hit.snippet && (
        <p className="mt-1 text-sm text-muted-foreground line-clamp-2">{hit.snippet}</p>
      )}
      <span className="mt-1 inline-block text-2xs text-muted-foreground/70">{hit.entity_type}</span>
    </button>
  )
}

/** Hybrid (and mode-specific) search over the brain; hits open the entity page. */
export function BrainSearch({ onSelectEntity }: { onSelectEntity?: (id: string) => void }) {
  const { t } = useTranslation()
  const [q, setQ] = useState('')
  const [mode, setMode] = useState<string>('hybrid')
  const [submitted, setSubmitted] = useState('')
  const { data, isLoading } = useBrainSearch(submitted, mode, 20, true)

  const items = Array.isArray(data) ? data : []

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <Input
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => e.key === 'Enter' && setSubmitted(q.trim())}
          placeholder={t('brain.searchPlaceholder')}
          aria-label={t('brain.searchPlaceholder')}
          className="flex-1"
        />
        <Select
          value={mode}
          onValueChange={setMode}
          options={MODE_OPTIONS}
          placeholder={t('brain.mode')}
          aria-label={t('brain.mode')}
          className="w-40"
        />
        <Button onClick={() => setSubmitted(q.trim())}>
          <Search className="h-4 w-4" />
          {t('brain.search')}
        </Button>
      </div>

      <div role="status" aria-live="polite">
        {isLoading && <p className="text-sm text-muted-foreground">{t('brain.searching')}</p>}

        {!isLoading && submitted && items.length === 0 && (
          <p className="text-sm text-muted-foreground">{t('brain.noSearchResults')}</p>
        )}
      </div>

      {items.length > 0 && (
        <Card>
          <CardContent className="pt-6">
            <div className="space-y-2">
              {items.map((hit) => (
                <HitRow key={hit.entity_id} hit={hit} onSelect={onSelectEntity ?? (() => {})} />
              ))}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  )
}
