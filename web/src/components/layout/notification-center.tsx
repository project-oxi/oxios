import { useNavigate } from '@tanstack/react-router'
import type { TFunction } from 'i18next'
import { Bell, Check, Plus, X } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { EventDetail } from '@/components/calendar/event-detail'
import { EventEditor } from '@/components/calendar/event-editor'
import { MiniCalendar } from '@/components/calendar/mini-calendar'
import { Button } from '@/components/ui/button'
import {
  useCalendarCreate,
  useCalendarDelete,
  useCalendarEvents,
  useCalendarUpdate,
} from '@/hooks/use-calendar'
import { isSubsystemUnavailable } from '@/lib/api-client'
import { cn } from '@/lib/utils'
import { useNotificationCenter } from '@/stores/notification-center'
import {
  type Notification,
  type NotificationSeverity,
  useNotificationStore,
} from '@/stores/notifications'
import { useHour12 } from '@/stores/ui-prefs'
import type { CalendarEvent, CreateEventRequest, UpdateEventRequest } from '@/types/calendar'

// ─── Notification helpers (ported from the old inline bell dropdown) ──────

const SEVERITY_DOT: Record<NotificationSeverity, string> = {
  info: 'bg-info',
  warning: 'bg-warning',
  error: 'bg-destructive',
  success: 'bg-success',
}

/** i18n-aware relative time formatter. */
function timeAgo(iso: string, t: TFunction): string {
  const diff = Date.now() - new Date(iso).getTime()
  const sec = Math.floor(diff / 1000)
  if (sec < 60) return t('common.justNow')
  const min = Math.floor(sec / 60)
  if (min < 60) return t('common.minutesAgo', { count: min })
  const hr = Math.floor(min / 60)
  if (hr < 24) return t('common.hoursAgo', { count: hr })
  const day = Math.floor(hr / 24)
  return t('common.daysAgo', { count: day })
}

// ─── Date helpers ─────────────────────────────────────────────────────────

function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  )
}

/** First cell (Sunday) of the 6×7 grid for the month of `anchor`. */
function gridStart(anchor: Date): Date {
  const first = new Date(anchor.getFullYear(), anchor.getMonth(), 1)
  const start = new Date(first)
  start.setDate(start.getDate() - first.getDay())
  return start
}

/** `YYYY-MM-DD` local key (own local time, no TZ shift). */
function dateKey(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

// ─── Shell ─────────────────────────────────────────────────────────────────

/**
 * Notification Center — macOS-style right slide-over.
 *
 * A single unified scrolling view: calendar widget at the top, notification
 * cards stacked below. No tabs. Frosted-glass panel, rounded cards.
 */
export function NotificationCenter() {
  const { t } = useTranslation()
  const open = useNotificationCenter((s) => s.open)
  const closeCenter = useNotificationCenter((s) => s.closeCenter)

  const scrollRef = useRef<HTMLDivElement>(null)

  // Escape closes — only while open.
  useEffect(() => {
    if (!open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') closeCenter()
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [open, closeCenter])

  // Reset scroll to top when the panel opens.
  useEffect(() => {
    if (!open) return
    const id = requestAnimationFrame(() => scrollRef.current?.scrollTo({ top: 0 }))
    return () => cancelAnimationFrame(id)
  }, [open])

  return (
    <>
      {/* Backdrop */}
      <div
        role="presentation"
        aria-hidden={!open}
        onClick={closeCenter}
        className={cn(
          'fixed inset-0 z-40 bg-black/40 backdrop-blur-[2px]',
          'transition-opacity duration-300 ease-[var(--animate-in-easing)]',
          open ? 'opacity-100' : 'pointer-events-none opacity-0',
        )}
      />

      {/* Slide-over panel — frosted glass like macOS NC */}
      <aside
        role="dialog"
        aria-modal="false"
        aria-label={t('notificationCenter.title')}
        className={cn(
          'fixed inset-y-0 right-0 z-50 flex w-[380px] max-w-[calc(100vw-1.5rem)] flex-col',
          'border-l border-border/50 bg-background/80 backdrop-blur-xl shadow-2xl',
          'transition-transform duration-300 ease-[var(--animate-in-easing)] will-change-transform',
          'pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)]',
          open ? 'translate-x-0' : 'pointer-events-none translate-x-full',
        )}
      >
        {/* Title bar */}
        <div className="flex items-center justify-between px-4 py-3">
          <h2 className="text-sm font-semibold tracking-tight">{t('notificationCenter.title')}</h2>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={closeCenter}>
            <X className="h-4 w-4" />
          </Button>
        </div>

        {/* Unified scroll view — calendar widget on top, notifications below */}
        <div ref={scrollRef} className="flex-1 overflow-y-auto overscroll-contain px-3 pb-4">
          <ScheduleWidget />
          <div className="mt-4">
            <NotificationsSection />
          </div>
        </div>
      </aside>
    </>
  )
}

// ─── Schedule widget (calendar card) ──────────────────────────────────────

function ScheduleWidget() {
  const { t, i18n } = useTranslation()
  const hour12 = useHour12()
  const now = useMemo(() => new Date(), [])
  const [viewAnchor, setViewAnchor] = useState(() => new Date())
  const [selectedDate, setSelectedDate] = useState<Date>(now)
  const [editorOpen, setEditorOpen] = useState(false)
  const [editingEvent, setEditingEvent] = useState<CalendarEvent | undefined>()
  const [defaultStart, setDefaultStart] = useState<Date | undefined>()
  const [detailEvent, setDetailEvent] = useState<CalendarEvent | null>(null)

  // Query the full 6×7 grid span so every visible cell has event data.
  const { from, to } = useMemo(() => {
    const start = gridStart(viewAnchor)
    const end = new Date(start)
    end.setDate(start.getDate() + 42)
    return { from: start.toISOString(), to: end.toISOString() }
  }, [viewAnchor])

  const { data, isLoading, error } = useCalendarEvents(from, to)
  const unavailable = isSubsystemUnavailable(error)
  const navigate = useNavigate()
  const events = useMemo(() => (Array.isArray(data?.events) ? data.events : []), [data])

  const createMutation = useCalendarCreate()
  const updateMutation = useCalendarUpdate()
  const deleteMutation = useCalendarDelete()

  // Agenda: events on the selected day, sorted by start time.
  const dayEvents = useMemo(() => {
    const key = dateKey(selectedDate)
    return events
      .filter((e) => dateKey(new Date(e.start)) === key)
      .sort((a, b) => new Date(a.start).getTime() - new Date(b.start).getTime())
  }, [events, selectedDate])

  // Next upcoming event across the loaded window.
  const nextEvent = useMemo(() => {
    const upcoming = events
      .filter((e) => new Date(e.start).getTime() >= now.getTime())
      .sort((a, b) => new Date(a.start).getTime() - new Date(b.start).getTime())
    return upcoming[0]
  }, [events, now])

  const openCreate = (date?: Date) => {
    setEditingEvent(undefined)
    setDefaultStart(date ?? selectedDate)
    setEditorOpen(true)
  }

  const handleSubmit = (data: CreateEventRequest | UpdateEventRequest) => {
    if (editingEvent) {
      updateMutation.mutate(
        { uid: editingEvent.uid, ...(data as UpdateEventRequest) },
        { onSuccess: () => setEditorOpen(false) },
      )
    } else {
      createMutation.mutate(data as CreateEventRequest, { onSuccess: () => setEditorOpen(false) })
    }
  }

  const isToday = isSameDay(selectedDate, now)
  const selectedLabel = selectedDate.toLocaleDateString(i18n.language, {
    month: 'long',
    day: 'numeric',
    weekday: 'long',
  })

  if (unavailable) {
    return (
      <div className="rounded-2xl border border-border/40 bg-card/50 p-3 shadow-sm">
        <p className="text-xs text-muted-foreground">{t('notificationCenter.calendarDisabled')}</p>
        <Button
          variant="link"
          size="sm"
          className="mt-1 h-auto p-0 text-xs"
          onClick={() =>
            navigate({ to: '/operate/system/settings', search: { section: 'calendar' } })
          }
        >
          {t('notificationCenter.enableCalendar')}
        </Button>
      </div>
    )
  }
  return (
    // ── Calendar widget card ──
    <div className="rounded-2xl border border-border/40 bg-card/50 p-3 shadow-sm">
      <MiniCalendar
        events={events}
        viewAnchor={viewAnchor}
        onViewAnchorChange={setViewAnchor}
        selectedDate={selectedDate}
        onSelectDate={setSelectedDate}
      />

      {/* Next event banner */}
      {nextEvent && (
        <div className="mt-3 rounded-xl bg-accent/40 px-3 py-2">
          <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
            {t('notificationCenter.nextEvent')}
          </p>
          <p className="mt-0.5 truncate text-sm font-medium">{nextEvent.title}</p>
          <p className="text-xs text-muted-foreground">
            {new Date(nextEvent.start).toLocaleString(i18n.language, {
              month: 'short',
              day: 'numeric',
              hour: '2-digit',
              minute: '2-digit',
              hour12,
            })}
          </p>
        </div>
      )}

      {/* Day agenda */}
      <div className="mt-3">
        <div className="mb-1.5 flex items-center justify-between">
          <span className="text-xs font-medium text-muted-foreground">
            {isToday ? t('calendar.today') : selectedLabel}
          </span>
          <Button
            variant="ghost"
            size="sm"
            className="h-6 px-2 text-xs"
            onClick={() => openCreate()}
          >
            <Plus className="mr-1 h-3 w-3" /> {t('calendar.newEvent')}
          </Button>
        </div>

        {isLoading ? (
          <p className="py-4 text-center text-sm text-muted-foreground">{t('calendar.loading')}</p>
        ) : dayEvents.length === 0 ? (
          <p className="py-4 text-center text-sm text-muted-foreground">
            {t('notificationCenter.noUpcoming')}
          </p>
        ) : (
          <div className="space-y-0.5">
            {dayEvents.map((ev) => (
              <button
                key={ev.uid}
                type="button"
                onClick={() => setDetailEvent(ev)}
                className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-accent/40 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              >
                <span
                  className={cn(
                    'h-2 w-2 shrink-0 rounded-full',
                    ev.source === 'agent'
                      ? 'bg-info'
                      : ev.source === 'cron'
                        ? 'bg-warning'
                        : 'bg-primary',
                  )}
                />
                <span className="shrink-0 text-xs tabular-nums text-muted-foreground">
                  {ev.all_day
                    ? t('calendar.allDay')
                    : new Date(ev.start).toLocaleTimeString(i18n.language, {
                        hour: '2-digit',
                        minute: '2-digit',
                        hour12,
                      })}
                </span>
                <span className="min-w-0 flex-1 truncate text-sm">{ev.title}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      <EventEditor
        open={editorOpen}
        onClose={() => setEditorOpen(false)}
        event={editingEvent}
        defaultStart={defaultStart}
        onSubmit={handleSubmit}
        isLoading={createMutation.isPending || updateMutation.isPending}
      />

      {detailEvent && (
        <EventDetail
          event={detailEvent}
          onEdit={() => {
            setEditingEvent(detailEvent)
            setDefaultStart(new Date(detailEvent.start))
            setDetailEvent(null)
            setEditorOpen(true)
          }}
          onDelete={() => {
            deleteMutation.mutate(detailEvent.uid, { onSuccess: () => setDetailEvent(null) })
          }}
          onUnlinkNote={
            detailEvent.note_path
              ? () =>
                  updateMutation.mutate(
                    { uid: detailEvent.uid, note_path: null },
                    { onSuccess: () => setDetailEvent(null) },
                  )
              : undefined
          }
          onClose={() => setDetailEvent(null)}
        />
      )}
    </div>
  )
}

// ─── Notifications section ────────────────────────────────────────────────

function NotificationsSection() {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const closeCenter = useNotificationCenter((s) => s.closeCenter)

  const notifications = useNotificationStore((s) => s.notifications)
  const unreadCount = useNotificationStore((s) => s.unreadCount)
  const markRead = useNotificationStore((s) => s.markRead)
  const markAllRead = useNotificationStore((s) => s.markAllRead)
  const dismiss = useNotificationStore((s) => s.dismiss)

  const handleClick = (n: Notification) => {
    markRead(n.id)
    if (n.link) {
      closeCenter()
      navigate({ to: n.link })
    }
  }

  return (
    <div>
      {/* Section header */}
      <div className="mb-2 flex items-center justify-between px-1">
        <span className="text-2xs font-semibold uppercase tracking-wider text-muted-foreground">
          {t('notificationCenter.notifications')}
        </span>
        {unreadCount > 0 && (
          <button
            type="button"
            onClick={markAllRead}
            className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-2xs text-muted-foreground transition-colors hover:bg-accent/50 hover:text-foreground"
          >
            <Check className="h-3 w-3" /> {t('notifications.markAllRead')}
          </button>
        )}
      </div>

      {notifications.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-2 py-8 text-muted-foreground">
          <Bell className="h-7 w-7 opacity-25" />
          <p className="text-xs">{t('notifications.noNotifications')}</p>
        </div>
      ) : (
        <div className="space-y-2">
          {notifications.map((n) => (
            // biome-ignore lint/a11y/useSemanticElements: card with nested dismiss button
            <div
              key={n.id}
              className={cn(
                'group relative cursor-pointer overflow-hidden rounded-2xl border border-border/40 p-3 shadow-sm transition-all hover:bg-card/80 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring',
                !n.read ? 'bg-card/70' : 'bg-card/40',
              )}
              onClick={() => handleClick(n)}
              role="button"
              tabIndex={0}
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleClick(n)
              }}
            >
              {/* Unread accent bar */}
              {!n.read && <span className="absolute inset-y-0 left-0 w-0.5 bg-primary" />}

              <div className="flex gap-2.5 pl-1">
                <div
                  className={cn('mt-0.5 h-2 w-2 shrink-0 rounded-full', SEVERITY_DOT[n.severity])}
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium leading-tight">{n.title}</p>
                  {n.message && (
                    <p className="mt-0.5 line-clamp-2 text-xs text-muted-foreground">{n.message}</p>
                  )}
                  <p className="mt-1 text-2xs text-muted-foreground/60">
                    {timeAgo(n.timestamp, t)}
                  </p>
                </div>
              </div>

              {/* Dismiss — appears on hover */}
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation()
                  dismiss(n.id)
                }}
                className="absolute right-2 top-2 rounded-full bg-background/60 p-0.5 text-muted-foreground opacity-0 transition-all hover:bg-muted hover:text-foreground group-hover:opacity-100"
                aria-label={t('common.dismiss')}
              >
                <X className="h-3 w-3" />
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}
