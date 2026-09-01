import type { TFunction } from 'i18next'
import { Clock3, Code2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select } from '@/components/ui/select'
import {
  cronToSimple,
  DEFAULT_SCHEDULE,
  type FrequencyMode,
  formatTime,
  HOUR_INTERVAL_OPTIONS,
  isValidCron,
  MINUTE_INTERVAL_OPTIONS,
  parseTime,
  type SimpleSchedule,
  simpleToCron,
} from '@/lib/cron-utils'
import { cn } from '@/lib/utils'

interface AutomationCronEditorProps {
  /** Current cron expression (5-field). */
  value: string
  /** Fired with the new cron expression whenever the GUI or raw input changes. */
  onChange: (cron: string) => void
  id?: string
}

const FREQUENCY_OPTIONS: { mode: FrequencyMode; labelKey: string }[] = [
  { mode: 'minutes', labelKey: 'cronJobs.freqMinutes' },
  { mode: 'hourly', labelKey: 'cronJobs.freqHourly' },
  { mode: 'daily', labelKey: 'cronJobs.freqDaily' },
  { mode: 'weekly', labelKey: 'cronJobs.freqWeekly' },
  { mode: 'monthly', labelKey: 'cronJobs.freqMonthly' },
]

const WEEKDAY_KEYS = [
  'common.sun',
  'common.mon',
  'common.tue',
  'common.wed',
  'common.thu',
  'common.fri',
  'common.sat',
] as const

function dayOfMonthOptions() {
  return Array.from({ length: 31 }, (_, i) => ({ label: String(i + 1), value: String(i + 1) }))
}

/** Locale-aware friendly time label (e.g. "오전 9:00" / "9:00 AM"). */
function formatTimeLabel(hour: number, minute: number, ko: boolean): string {
  const mm = minute.toString().padStart(2, '0')
  const isPM = hour >= 12
  const h12 = hour % 12 === 0 ? 12 : hour % 12
  return ko ? `${isPM ? '오후' : '오전'} ${h12}:${mm}` : `${h12}:${mm} ${isPM ? 'PM' : 'AM'}`
}

/**
 * Build a localized human-readable description directly from the structured
 * {@link SimpleSchedule}. Describes exactly the 5 shapes the GUI can emit, so
 * no cron-string round-trip is needed.
 */
function describeSchedule(s: SimpleSchedule, t: TFunction, ko: boolean): string {
  switch (s.mode) {
    case 'minutes':
      return t('cronJobs.descMinutes', { count: s.minuteInterval })
    case 'hourly':
      return t('cronJobs.descHourly', {
        count: s.hourInterval,
        minute: String(s.atMinute).padStart(2, '0'),
      })
    case 'daily':
      return t('cronJobs.descDaily', { time: formatTimeLabel(s.hour, s.minute, ko) })
    case 'weekly': {
      const days = [...s.weekdays]
        .sort((a, b) => a - b)
        .map((d) => t(WEEKDAY_KEYS[d] as string))
        .join(', ')
      return t('cronJobs.descWeekly', { days, time: formatTimeLabel(s.hour, s.minute, ko) })
    }
    case 'monthly':
      return t('cronJobs.descMonthly', {
        day: s.dayOfMonth,
        time: formatTimeLabel(s.hour, s.minute, ko),
      })
  }
}

/**
 * Human-friendly cron schedule editor.
 *
 * Two modes:
 *  - **Simple** (default): structured GUI that emits canonical cron. When an
 *    incoming expression matches a supported pattern it is parsed back into GUI
 *    state; otherwise the editor auto-switches to advanced mode so an existing
 *    job's schedule is never misrepresented.
 *  - **Advanced**: raw 5-field cron input for power users.
 *
 * The live preview (cron expression + locale-aware human description) updates on
 * every keystroke / control change in both modes. Descriptions are derived
 * directly from the structured schedule state — no external parsing library.
 */
export function AutomationCronEditor({ value, onChange, id }: AutomationCronEditorProps) {
  const { t, i18n } = useTranslation()
  const ko = i18n.resolvedLanguage?.startsWith('ko') ?? false

  const [schedule, setSchedule] = useState<SimpleSchedule>(
    () => cronToSimple(value) ?? DEFAULT_SCHEDULE,
  )
  const [advanced, setAdvanced] = useState<boolean>(
    () => value.trim() !== '' && cronToSimple(value) === null,
  )

  // Avoid re-parsing our own emitted value (idempotent anyway, but skips a setState).
  const lastEmitted = useRef(value)

  useEffect(() => {
    if (value === lastEmitted.current) return
    lastEmitted.current = value
    const parsed = cronToSimple(value)
    if (parsed) {
      setSchedule(parsed)
    } else {
      setAdvanced(true)
    }
  }, [value])

  // ── Simple mode ────────────────────────────────────────────────────────────
  const updateSchedule = (patch: Partial<SimpleSchedule>) => {
    const next = { ...schedule, ...patch }
    setSchedule(next)
    const cron = simpleToCron(next)
    lastEmitted.current = cron
    onChange(cron)
  }

  const toggleWeekday = (day: number) => {
    const set = new Set(schedule.weekdays)
    if (set.has(day)) {
      if (set.size <= 1) return // keep at least one day selected
      set.delete(day)
    } else {
      set.add(day)
    }
    updateSchedule({ weekdays: [...set] })
  }

  // ── Advanced mode ──────────────────────────────────────────────────────────
  const handleRawChange = (raw: string) => {
    lastEmitted.current = raw
    onChange(raw)
  }

  // When switching back to simple, re-emit the current GUI schedule so the
  // preview and stored value stay consistent even if advanced held an
  // unparseable expression.
  const handleToggleAdvanced = (next: boolean) => {
    setAdvanced(next)
    if (!next) {
      const cron = simpleToCron(schedule)
      lastEmitted.current = cron
      onChange(cron)
    }
  }

  // ── Preview ────────────────────────────────────────────────────────────────
  const trimmed = value.trim()
  const valid = isValidCron(trimmed)
  // In simple mode `schedule` always describes the value; in advanced mode,
  // still describe when the raw expression happens to parse.
  const parsedForDesc = advanced ? cronToSimple(trimmed) : schedule
  const description = parsedForDesc
    ? describeSchedule(parsedForDesc, t, ko)
    : valid
      ? t('cronJobs.customSchedule')
      : null

  return (
    <div className="space-y-3" data-testid={id ? `${id}-cron-editor` : undefined}>
      {/* Mode switch */}
      <div className="flex items-center justify-between">
        <Label className="text-muted-foreground">{t('cronJobs.repeat')}</Label>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          onClick={() => handleToggleAdvanced(!advanced)}
          className="h-7 gap-1 text-xs text-muted-foreground"
        >
          {advanced ? <Clock3 className="h-3.5 w-3.5" /> : <Code2 className="h-3.5 w-3.5" />}
          {advanced ? t('cronJobs.simpleMode') : t('cronJobs.advancedMode')}
        </Button>
      </div>

      {advanced ? (
        <div className="space-y-1">
          <Input
            id={id}
            value={value}
            onChange={(e) => handleRawChange(e.target.value)}
            placeholder="0 */6 * * *"
            className="font-mono"
            aria-invalid={!valid}
          />
          {!valid && <p className="text-xs text-destructive">{t('cronJobs.invalidCron')}</p>}
        </div>
      ) : (
        <div className="space-y-3">
          <Select
            id={id ? `${id}-freq` : undefined}
            value={schedule.mode}
            onValueChange={(m) => updateSchedule({ mode: m as FrequencyMode })}
            options={FREQUENCY_OPTIONS.map((o) => ({ label: t(o.labelKey), value: o.mode }))}
          />

          {schedule.mode === 'minutes' && (
            <Row label={t('cronJobs.every')}>
              <Select
                value={String(schedule.minuteInterval)}
                onValueChange={(v) => updateSchedule({ minuteInterval: Number(v) })}
                options={MINUTE_INTERVAL_OPTIONS.map((n) => ({
                  label: t('cronJobs.everyNMinutes', { count: n }),
                  value: String(n),
                }))}
              />
            </Row>
          )}

          {schedule.mode === 'hourly' && (
            <>
              <Row label={t('cronJobs.every')}>
                <Select
                  value={String(schedule.hourInterval)}
                  onValueChange={(v) => updateSchedule({ hourInterval: Number(v) })}
                  options={HOUR_INTERVAL_OPTIONS.map((n) => ({
                    label: t('cronJobs.everyNHours', { count: n }),
                    value: String(n),
                  }))}
                />
              </Row>
              <Row label={t('cronJobs.atMinute')}>
                <Select
                  value={String(schedule.atMinute)}
                  onValueChange={(v) => updateSchedule({ atMinute: Number(v) })}
                  options={Array.from({ length: 12 }, (_, i) => i * 5).map((mm) => ({
                    label: String(mm),
                    value: String(mm),
                  }))}
                />
              </Row>
            </>
          )}

          {schedule.mode === 'daily' && (
            <Row label={t('cronJobs.atTime')}>
              <Input
                type="time"
                value={formatTime(schedule.hour, schedule.minute)}
                onChange={(e) => {
                  const [h, m] = parseTime(e.target.value)
                  updateSchedule({ hour: h, minute: m })
                }}
                className="max-w-[8rem]"
              />
            </Row>
          )}

          {schedule.mode === 'weekly' && (
            <>
              <Row label={t('cronJobs.onDays')}>
                <div className="flex flex-wrap gap-1">
                  {WEEKDAY_KEYS.map((key, idx) => {
                    const active = schedule.weekdays.includes(idx)
                    return (
                      <Button
                        key={key}
                        type="button"
                        variant={active ? 'default' : 'outline'}
                        size="sm"
                        onClick={() => toggleWeekday(idx)}
                        className={cn('h-8 w-9 p-0 text-xs')}
                        aria-pressed={active}
                      >
                        {t(key)}
                      </Button>
                    )
                  })}
                </div>
              </Row>
              <Row label={t('cronJobs.atTime')}>
                <Input
                  type="time"
                  value={formatTime(schedule.hour, schedule.minute)}
                  onChange={(e) => {
                    const [h, m] = parseTime(e.target.value)
                    updateSchedule({ hour: h, minute: m })
                  }}
                  className="max-w-[8rem]"
                />
              </Row>
            </>
          )}

          {schedule.mode === 'monthly' && (
            <>
              <Row label={t('cronJobs.onDay')}>
                <Select
                  value={String(schedule.dayOfMonth)}
                  onValueChange={(v) => updateSchedule({ dayOfMonth: Number(v) })}
                  options={dayOfMonthOptions()}
                />
              </Row>
              <Row label={t('cronJobs.atTime')}>
                <Input
                  type="time"
                  value={formatTime(schedule.hour, schedule.minute)}
                  onChange={(e) => {
                    const [h, m] = parseTime(e.target.value)
                    updateSchedule({ hour: h, minute: m })
                  }}
                  className="max-w-[8rem]"
                />
              </Row>
            </>
          )}
        </div>
      )}

      {/* Live preview */}
      <div className="flex flex-col gap-1 rounded-md border bg-muted/40 px-3 py-2">
        <div className="flex items-center gap-2">
          <code className="text-xs font-semibold">{trimmed || '—'}</code>
          {description && (
            <Badge variant="secondary" className="text-[10px]">
              {t('cronJobs.preview')}
            </Badge>
          )}
        </div>
        {description ? (
          <p className="text-xs text-muted-foreground">{description}</p>
        ) : (
          <p className="text-xs text-muted-foreground/60">{t('cronJobs.previewEmpty')}</p>
        )}
      </div>
    </div>
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <Label className="shrink-0 text-muted-foreground">{label}</Label>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  )
}
