// AutomationTriggerEditor — trigger configuration (manual/cron/heartbeat).
//
// Wraps CronScheduleEditor for the cron path; manual mode needs no inputs;
// heartbeat mode asks for an interval (minutes). On every change the parent
// receives the merged SetTriggerParams payload via onChange:
//
//   manual     → { trigger: 'manual' }
//   cron       → { trigger: 'cron', cronPattern, timezone? }
//   heartbeat  → { trigger: 'heartbeat', heartbeatIntervalSecs }
//
// The trigger value is controlled (parent passes current trigger); schedule
// and interval live in local state so the user can stage edits. Final commit
// happens when the parent reads the onChange callback.

import { Clock3 } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { AutomationCronEditor } from '@/components/automation/automation-cron-editor'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Select } from '@/components/ui/select'
import { DEFAULT_CRON } from '@/lib/cron-utils'
import type { AutomationTrigger, SetTriggerParams } from '@/types/automation'

export interface AutomationTriggerEditorProps {
  /** Currently persisted trigger (parent's authoritative state). */
  trigger: AutomationTrigger
  /** Persisted cron pattern when trigger === 'cron'. */
  cronPattern?: string | null
  /** Persisted heartbeat interval in seconds when trigger === 'heartbeat'. */
  heartbeatIntervalSecs?: number | null
  /**
   * Fired on every edit. Receives the full merged SetTriggerParams for the
   * current mode so callers can persist the whole shape via PUT /trigger.
   */
  onChange: (params: SetTriggerParams) => void
  id?: string
}

const TRIGGER_OPTIONS: { value: AutomationTrigger; labelKey: string }[] = [
  { value: 'manual', labelKey: 'automations.triggerManual' },
  { value: 'cron', labelKey: 'automations.triggerSchedule' },
  { value: 'heartbeat', labelKey: 'automations.triggerHeartbeat' },
]

export function AutomationTriggerEditor({
  trigger,
  cronPattern,
  heartbeatIntervalSecs,
  onChange,
  id,
}: AutomationTriggerEditorProps) {
  const { t } = useTranslation()
  // Local cron value so the editor can drive the GUI without fighting the
  // parent's persisted state on every keystroke.
  const [localCron, setLocalCron] = useState(cronPattern ?? DEFAULT_CRON)
  const [localIntervalSecs, setLocalIntervalSecs] = useState(
    Math.max(60, heartbeatIntervalSecs ?? 1800),
  )

  // Mirror persisted state when the parent's trigger flips (e.g. after a save).
  useEffect(() => {
    if (trigger === 'cron') {
      setLocalCron(cronPattern ?? DEFAULT_CRON)
    } else if (trigger === 'heartbeat') {
      setLocalIntervalSecs(Math.max(60, heartbeatIntervalSecs ?? 1800))
    }
  }, [trigger, cronPattern, heartbeatIntervalSecs])

  const handleTriggerChange = (next: AutomationTrigger) => {
    if (next === 'manual') {
      onChange({ trigger: 'manual' })
    } else if (next === 'cron') {
      onChange({ trigger: 'cron', cronPattern: localCron })
    } else {
      onChange({ trigger: 'heartbeat', heartbeatIntervalSecs: localIntervalSecs })
    }
  }

  const handleCronChange = (cron: string) => {
    setLocalCron(cron)
    onChange({ trigger: 'cron', cronPattern: cron })
  }

  const handleIntervalChange = (mins: number) => {
    const secs = Math.max(60, mins * 60)
    setLocalIntervalSecs(secs)
    onChange({ trigger: 'heartbeat', heartbeatIntervalSecs: secs })
  }

  return (
    <div className="space-y-3" data-testid={id ? `${id}-trigger-editor` : undefined}>
      <div className="flex items-center justify-between">
        <Label className="text-muted-foreground">{t('automations.triggerLabel')}</Label>
      </div>
      <Select
        id={id ? `${id}-trigger` : undefined}
        value={trigger}
        onValueChange={(v) => handleTriggerChange(v as AutomationTrigger)}
        options={TRIGGER_OPTIONS.map((o) => ({ label: t(o.labelKey), value: o.value }))}
      />

      {trigger === 'cron' && (
        <div className="rounded-lg border p-3">
          <AutomationCronEditor value={localCron} onChange={handleCronChange} id={id} />
        </div>
      )}

      {trigger === 'heartbeat' && (
        <div>
          <Label className="text-sm font-medium mb-1 block">
            {t('automations.intervalMinutes')}
          </Label>
          <div className="flex items-center gap-2">
            <Clock3 className="h-3.5 w-3.5 text-muted-foreground" />
            <Input
              type="number"
              min={1}
              value={Math.max(1, Math.round(localIntervalSecs / 60))}
              onChange={(e) => handleIntervalChange(Math.max(1, Number(e.target.value) || 1))}
              className="w-32"
            />
          </div>
        </div>
      )}
    </div>
  )
}
