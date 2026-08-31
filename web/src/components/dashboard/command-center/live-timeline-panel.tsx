import { Link } from '@tanstack/react-router'
import { ChevronRight, Radio } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { LiveActivityFeed } from '@/components/dashboard/live-activity-feed'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'

/**
 * Live timeline — command-center presentation of the existing SSE feed
 * (spec §4). Same singleton store, pause/resume, capped list, and
 * auto-scroll behavior as `LiveActivityFeed`; adds the panel header with
 * an explicit "View full timeline" link to /events and a "This run"
 * filter whenever a run is selected in the dashboard.
 */
export function LiveTimelinePanel({ selectedAgentId }: { selectedAgentId?: string | null }) {
  const { t } = useTranslation()

  return (
    <Card className="flex h-full min-w-0 flex-col">
      <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Radio className="h-4 w-4" aria-hidden="true" />
          {t('commandCenter.timeline.title')}
        </CardTitle>
        <Link
          to="/operate/events"
          className="inline-flex shrink-0 items-center gap-0.5 text-xs text-muted-foreground transition-colors hover:text-foreground"
        >
          {t('commandCenter.timeline.viewFull')}
          <ChevronRight className="h-3 w-3" aria-hidden="true" />
        </Link>
      </CardHeader>
      <CardContent className="flex-1 pt-0 min-h-[280px]">
        <LiveActivityFeed variant="bare" selectedAgentId={selectedAgentId} />
      </CardContent>
    </Card>
  )
}
