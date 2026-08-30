import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { WhyDetails } from '@/components/brain/why-details'
import { ErrorState } from '@/components/shared/error-state'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { useBrainContradictions } from '@/hooks/use-brain'
import type { ContradictionDetail } from '@/types/brain'

function ContradictionCard({
  detail,
  space,
  onSelectEntity,
}: {
  detail: ContradictionDetail
  space: string
  onSelectEntity?: (id: string) => void
}) {
  const { t } = useTranslation()
  const [whyOpen, setWhyOpen] = useState(false)
  return (
    <Card>
      <CardContent className="pt-6 space-y-3">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-sm">
          <button
            type="button"
            className="font-medium text-primary underline underline-offset-2 hover:opacity-80 transition-opacity"
            onClick={() => onSelectEntity?.(detail.subject_id)}
          >
            {detail.subject_surface}
          </button>
          <span className="text-muted-foreground">{detail.predicate}</span>
          <span className="font-medium">
            {detail.object_kind === 'entity' ? (
              <button
                type="button"
                className="text-primary underline underline-offset-2 hover:opacity-80 transition-opacity"
                onClick={() =>
                  onSelectEntity?.(detail.object_kind === 'entity' ? detail.object_value : '')
                }
              >
                {detail.object_value}
              </button>
            ) : (
              detail.object_value
            )}
          </span>
          <span className="text-2xs text-muted-foreground/70">{detail.subject_type}</span>
        </div>

        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs">
          <span className="text-status-success">
            {t('brain.affirmCount', { count: detail.affirm_episodes.length })}
          </span>
          <span className="text-status-error">
            {t('brain.denyCount', { count: detail.deny_episodes.length })}
          </span>
          <Button variant="ghost" size="sm" onClick={() => setWhyOpen((v) => !v)}>
            {whyOpen ? t('brain.hideWhy') : t('brain.why')}
          </Button>
        </div>

        {whyOpen && <WhyDetails statementId={detail.statement_id} space={space} />}
      </CardContent>
    </Card>
  )
}

/** Contradiction inbox: statements with both affirming and denying support. */
export function BrainContradictions({
  space,
  onSelectEntity,
}: {
  /** Space the scoped brain calls are bound to (required server-side). */
  space: string
  onSelectEntity?: (id: string) => void
}) {
  const { t } = useTranslation()
  const { data, isLoading, isError, refetch } = useBrainContradictions(space)

  if (isLoading) return <p className="text-sm text-muted-foreground">{t('brain.loading')}</p>
  if (isError) return <ErrorState onRetry={() => refetch()} />

  const items = Array.isArray(data) ? data : []
  if (items.length === 0) {
    return <p className="text-sm text-muted-foreground">{t('brain.noContradictions')}</p>
  }

  return (
    <div className="space-y-2">
      {items.map((c) => (
        <ContradictionCard
          key={c.statement_id}
          detail={c}
          space={space}
          onSelectEntity={onSelectEntity}
        />
      ))}
    </div>
  )
}
