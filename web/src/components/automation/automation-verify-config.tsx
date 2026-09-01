// AutomationVerifyConfig — verify-gate section for the automation detail dialog.
//
// Controlled subcomponent: receives the current verify state and an `onSave`
// callback that performs the mutation (typically useSetAutomationVerify.mutate).
// "Save" sends the merged SetVerifyParams payload — every field is sent so
// the store sees the current snapshot. Toggling enabled is staged in local
// state and only committed when the user presses Save.

import { ShieldCheck } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import type { SetVerifyParams } from '@/types/automation'

export interface AutomationVerifyConfigProps {
  /** Current persisted values from the automation. */
  enabled: boolean
  requirement?: string
  maxIterations: number
  /** Pending state from the parent mutation. */
  isPending?: boolean
  /**
   * Called with a complete SetVerifyParams payload when the user clicks Save.
   * The component merges local edits into the current persisted state so
   * every field is sent.
   */
  onSave: (params: SetVerifyParams) => void
}

export function AutomationVerifyConfig({
  enabled,
  requirement,
  maxIterations,
  isPending,
  onSave,
}: AutomationVerifyConfigProps) {
  const { t } = useTranslation()
  const [localEnabled, setLocalEnabled] = useState(enabled)
  const [localRequirement, setLocalRequirement] = useState(requirement ?? '')
  const [localMaxIterations, setLocalMaxIterations] = useState(maxIterations)

  // Staging mirrors persisted state when the parent's `enabled` flips back
  // (e.g. after a save from elsewhere). Keeps the UI honest.
  const dirty =
    localEnabled !== enabled ||
    (localRequirement || undefined) !== requirement ||
    localMaxIterations !== maxIterations

  const handleSave = () => {
    onSave({
      enabled: localEnabled,
      requirement: localRequirement.trim() || null,
      maxIterations: localMaxIterations,
    })
  }

  return (
    <div className="space-y-3 rounded-lg border p-3" data-testid="automation-verify-config">
      <div className="flex items-center gap-1.5">
        <ShieldCheck className="h-3.5 w-3.5 text-muted-foreground" />
        <Label className="text-xs font-medium text-muted-foreground">
          {t('automations.verifyTitle')}
        </Label>
      </div>

      <div className="flex items-center justify-between gap-2">
        <Label htmlFor="verify-enabled" className="text-sm">
          {t('automations.verifyEnabled')}
        </Label>
        <Switch
          id="verify-enabled"
          checked={localEnabled}
          onCheckedChange={setLocalEnabled}
          size="sm"
          aria-label={t('automations.verifyEnabled')}
        />
      </div>

      <div>
        <Label className="text-xs font-medium mb-1 block text-muted-foreground">
          {t('automations.verifyRequirement')}
        </Label>
        <Textarea
          value={localRequirement}
          onChange={(e) => setLocalRequirement(e.target.value)}
          placeholder={t('automations.verifyRequirementPlaceholder')}
          rows={3}
          disabled={!localEnabled}
        />
      </div>

      <div>
        <Label
          htmlFor="verify-max-iterations"
          className="text-xs font-medium mb-1 block text-muted-foreground"
        >
          {t('automations.verifyMaxIterations')}
        </Label>
        <Input
          id="verify-max-iterations"
          type="number"
          min={1}
          value={localMaxIterations}
          onChange={(e) => setLocalMaxIterations(Math.max(1, Number(e.target.value) || 1))}
          className="w-24"
          disabled={!localEnabled}
        />
      </div>

      <div className="flex justify-end">
        <Button
          size="sm"
          variant="outline"
          onClick={handleSave}
          disabled={!localEnabled || !dirty || isPending}
        >
          {isPending ? t('automations.saving') : t('automations.verifySave')}
        </Button>
      </div>
    </div>
  )
}
