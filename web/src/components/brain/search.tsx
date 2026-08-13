import { Search } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Select } from '@/components/ui/select'
import { useBrainSearch } from '@/hooks/use-brain'

const MODES = ['hybrid', 'lexical', 'semantic', 'graph', 'community'] as const
const MODE_OPTIONS = MODES.map((m) => ({ label: m, value: m }))

/** Hybrid (and mode-specific) search over the brain. */
export function BrainSearch() {
  const { t } = useTranslation()
  const [q, setQ] = useState('')
  const [mode, setMode] = useState<string>('hybrid')
  const [submitted, setSubmitted] = useState('')
  const { data, isLoading } = useBrainSearch(submitted, mode, 20, true)

  const items = Array.isArray(data?.items) ? data.items : []

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <Input
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') setSubmitted(q)
          }}
          placeholder={t('brain.searchPlaceholder')}
          className="flex-1"
        />
        <Select
          value={mode}
          onValueChange={setMode}
          options={MODE_OPTIONS}
          placeholder={t('brain.mode')}
          className="w-40"
        />
        <Button onClick={() => setSubmitted(q)}>
          <Search className="h-4 w-4" />
          {t('brain.search')}
        </Button>
      </div>

      {isLoading && <p className="text-sm text-muted-foreground">{t('brain.searching')}</p>}

      {!isLoading && submitted && items.length === 0 && (
        <p className="text-sm text-muted-foreground">{t('brain.noSearchResults')}</p>
      )}

      {items.length > 0 && (
        <div className="space-y-2">
          {items.map((hit, i) => (
            <Card key={`${hit.target?.id}-${i}`}>
              <CardContent className="py-3">
                <div className="flex items-center justify-between text-sm">
                  <span className="font-medium">
                    {hit.target?.kind}: {hit.target?.id}
                  </span>
                  <span className="text-muted-foreground">
                    {hit.fused_score != null && `score ${hit.fused_score.toFixed(3)}`}
                    {hit.salience != null && ` · salience ${hit.salience.toFixed(2)}`}
                  </span>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  )
}
