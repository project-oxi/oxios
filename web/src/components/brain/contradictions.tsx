import { useTranslation } from 'react-i18next'
import { ErrorState } from '@/components/shared/error-state'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useBrainContradictions } from '@/hooks/use-brain'

/** Contradiction inbox: statements with both affirming and denying support. */
export function BrainContradictions() {
  const { t } = useTranslation()
  const { data, isLoading, isError, refetch } = useBrainContradictions()

  if (isLoading) return <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const items = Array.isArray(data) ? data : []
  if (items.length === 0) {
    return <p className="text-sm text-muted-foreground">{t('brain.noContradictions')}</p>
  }

  return (
    <div className="space-y-2">
      {items.map((c, i) => (
        <Card key={i}>
          <CardHeader>
            <CardTitle className="text-base">
              {String(typeof c.subject === 'string' ? c.subject : (c.id ?? `#${i + 1}`))}
            </CardTitle>
          </CardHeader>
          <CardContent>
            <pre className="max-h-64 overflow-auto rounded bg-muted p-3 text-xs">
              {JSON.stringify(c, null, 2)}
            </pre>
          </CardContent>
        </Card>
      ))}
    </div>
  )
}
