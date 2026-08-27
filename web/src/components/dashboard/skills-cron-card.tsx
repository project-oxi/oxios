import { useQuery } from '@tanstack/react-query'
import { Link } from '@tanstack/react-router'
import { Clock, Layers, Zap } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { api } from '@/lib/api-client'
import { formatRelativeDate } from '@/lib/utils'
import type { Skill } from '@/types'
import type { Task } from '@/types/task'

/**
 * Combined Skills & Cron Jobs overview card.
 *
 * Shows active skills count and active cron jobs
 * with next-run time. Uses shared formatRelativeDate util.
 */
export function SkillsCronCard({ className }: { className?: string }) {
  const { t } = useTranslation()

  const { data: skillsData } = useQuery({
    queryKey: ['skills'],
    queryFn: () => api.get<{ items: Skill[] }>('/api/skills'),
    staleTime: 60_000,
  })

  const { data: tasksData } = useQuery({
    queryKey: ['tasks'],
    queryFn: () => api.get<{ tasks: Task[] }>('/api/tasks'),
    staleTime: 30_000,
  })

  const skills = Array.isArray(skillsData?.items) ? skillsData.items : []
  const activeSkills = skills.filter((s) => s.eligible)
  const scheduledTasks = (tasksData?.tasks ?? []).filter(
    (tk) => tk.automationMode === 'schedule' && tk.status !== 'canceled',
  )
  const nextTask = scheduledTasks
    .filter((tk) => tk.nextRunAt)
    .sort((a, b) => new Date(a.nextRunAt!).getTime() - new Date(b.nextRunAt!).getTime())[0]

  return (
    <Card className={className}>
      <CardHeader className="flex flex-row items-center justify-between pb-2">
        <CardTitle className="flex items-center gap-2 text-base">
          <Layers className="h-4 w-4" />
          {t('dashboard.skillsCron')}
        </CardTitle>
      </CardHeader>
      <CardContent className="pt-0 space-y-3">
        {/* Skills */}
        <div>
          <div className="flex items-center justify-between text-xs">
            <div className="flex items-center gap-1.5 text-muted-foreground">
              <Zap className="h-3 w-3" />
              <span>{t('dashboard.activeSkills')}</span>
            </div>
            <Link
              to="/skills"
              search={{ tab: 'installed' }}
              className="font-semibold hover:underline"
            >
              {activeSkills.length}
            </Link>
          </div>
        </div>

        {/* Scheduled tasks */}
        <div>
          <div className="flex items-center justify-between text-xs">
            <div className="flex items-center gap-1.5 text-muted-foreground">
              <Clock className="h-3 w-3" />
              <span>{t('dashboard.scheduledTasks')}</span>
            </div>
            <Link to="/tasks" className="font-semibold hover:underline">
              {scheduledTasks.length} {t('dashboard.active')}
            </Link>
          </div>
          {nextTask?.nextRunAt && (
            <p className="text-2xs text-muted-foreground mt-1 pl-5">
              {t('dashboard.nextRun')}: {formatRelativeDate(nextTask.nextRunAt, t)}
            </p>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
