import { createFileRoute } from '@tanstack/react-router'
import { Activity, BookOpen } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Habits } from '@/components/knowledge/habits'
import { TodayStats } from '@/components/knowledge/today-stats'
import { PageHeader } from '@/components/shared/page-header'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useJournalToday } from '@/hooks/use-knowledge'

export const Route = createFileRoute('/knowledge/library/journal')({
  component: LibraryJournalPage,
})

/**
 * Composes the existing journal/habits/today-stats surfaces into one Journal
 * page. Reuses `use-knowledge` journal hooks and the existing Habits and
 * TodayStats components without modifying any of their internals.
 *
 * TodayStats already renders its own Card; the Journal block uses the card
 * chrome around it to avoid double cards.
 */
function LibraryJournalPage() {
  const { t } = useTranslation()
  const { data: journalToday } = useJournalToday()
  const journalPath = journalToday?.path

  return (
    <div className="space-y-6">
      <PageHeader
        title={t('knowledge.surface.journal')}
        subtitle={t('knowledge.surface.librarySubtitle')}
      />
      <div className="grid gap-4 md:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-base">
              <BookOpen className="h-4 w-4" />
              {t('knowledge.homeJournal')}
            </CardTitle>
          </CardHeader>
          <CardContent>
            {journalPath ? (
              <p className="text-sm text-muted-foreground">{journalPath}</p>
            ) : (
              <p className="text-sm text-muted-foreground" role="status">
                {t('knowledge.homeJournalHint')}
              </p>
            )}
          </CardContent>
        </Card>
        <TodayStats />
      </div>
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-base">
            <Activity className="h-4 w-4" />
            {t('knowledge.habitsTitle')}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <Habits />
        </CardContent>
      </Card>
    </div>
  )
}
