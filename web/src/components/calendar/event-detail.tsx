import { useNavigate } from '@tanstack/react-router'
import { FileText, Unlink } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { useKnowledgeStore } from '@/stores/knowledge'
import { useNotificationCenter } from '@/stores/notification-center'
import type { CalendarEvent } from '@/types/calendar'

interface Props {
  event: CalendarEvent
  onEdit?: () => void
  onDelete?: () => void
  onUnlinkNote?: () => void
  onClose: () => void
}

export function EventDetail({ event, onEdit, onDelete, onUnlinkNote, onClose }: Props) {
  const { t, i18n } = useTranslation()
  const navigate = useNavigate()
  const openFile = useKnowledgeStore((s) => s.openFile)
  const closeCenter = useNotificationCenter((s) => s.closeCenter)

  const SOURCE_LABELS: Record<CalendarEvent['source'], string> = {
    agent: t('calendar.sourceAgent'),
    user: t('calendar.sourceUser'),
    cron: t('calendar.sourceCron'),
  }

  function formatDateTime(iso: string): string {
    const d = new Date(iso)
    return d.toLocaleString(i18n.language, {
      year: 'numeric',
      month: 'long',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
      weekday: 'short',
    })
  }

  const noteName = event.note_path
    ? (event.note_path.split('/').pop()?.replace(/\.md$/, '') ?? event.note_path)
    : ''

  function handleOpenNote() {
    if (!event.note_path) return
    openFile(event.note_path)
    closeCenter()
    navigate({ to: '/brain/knowledge' })
    onClose()
  }

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="text-xl">{event.title}</DialogTitle>
        </DialogHeader>

        <div className="space-y-4">
          {/* Time */}
          <div className="space-y-1">
            <p className="text-sm font-medium text-muted-foreground">{t('calendar.time')}</p>
            <p className="text-sm">
              {formatDateTime(event.start)}
              <span className="mx-2 text-muted-foreground">→</span>
              {formatDateTime(event.end)}
            </p>
          </div>

          {/* Location */}
          {event.location && (
            <div className="space-y-1">
              <p className="text-sm font-medium text-muted-foreground">{t('calendar.location')}</p>
              <p className="text-sm">{event.location}</p>
            </div>
          )}

          {/* Description */}
          {event.description && (
            <div className="space-y-1">
              <p className="text-sm font-medium text-muted-foreground">
                {t('calendar.description')}
              </p>
              <p className="text-sm whitespace-pre-wrap">{event.description}</p>
            </div>
          )}

          {/* Repeat (raw RRULE) */}
          {event.rrule && (
            <div className="space-y-1">
              <p className="text-sm font-medium text-muted-foreground">{t('calendar.repeat')}</p>
              <p className="text-sm font-mono text-xs">{event.rrule}</p>
            </div>
          )}

          {/* Linked knowledge note */}
          {event.note_path && (
            <div className="space-y-1">
              <p className="text-sm font-medium text-muted-foreground">
                {t('calendar.linkedNote')}
              </p>
              <div className="flex items-center gap-2">
                <Button variant="outline" size="sm" onClick={handleOpenNote}>
                  <FileText className="mr-1.5 h-3.5 w-3.5" />
                  {noteName}
                </Button>
                {onUnlinkNote && (
                  <Button variant="ghost" size="sm" onClick={onUnlinkNote}>
                    <Unlink className="mr-1.5 h-3.5 w-3.5" />
                    {t('calendar.unlink')}
                  </Button>
                )}
              </div>
            </div>
          )}

          {/* Source badge */}
          <div className="flex items-center gap-2">
            <Badge variant="outline">{SOURCE_LABELS[event.source]}</Badge>
          </div>
        </div>

        {/* Actions */}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            {t('calendar.close')}
          </Button>
          {onEdit && (
            <Button variant="outline" onClick={onEdit}>
              {t('calendar.edit')}
            </Button>
          )}
          {onDelete && (
            <Button variant="destructive" onClick={onDelete}>
              {t('calendar.delete')}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
