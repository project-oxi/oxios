import { ShieldAlert } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { useApprovalConfig } from '@/hooks/use-approval-config'

interface ToolApprovalCardProps {
  toolName: string
  reason: string
  onApprove: (remember: boolean) => void
  onDeny: () => void
  disabled?: boolean
}

/**
 * Inline tool approval card shown in the chat when an agent tries
 * a tool it doesn't have CSpace capability for (RFC-017).
 */
export function ToolApprovalCard({
  toolName,
  reason,
  onApprove,
  onDeny,
  disabled,
}: ToolApprovalCardProps) {
  const { t, i18n } = useTranslation()
  const { data } = useApprovalConfig()
  const { mode } = data ?? {}
  const [remember, setRemember] = useState(false)
  const rememberLabel = i18n.language?.startsWith('ko')
    ? '이 도구는 다시 묻지 않기'
    : "Don't ask again for this tool"
  return (
    <div className="flex gap-3 my-1.5">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-warning text-warning-foreground">
        <ShieldAlert className="h-4 w-4" />
      </div>
      <div className="max-w-[80%] flex-1">
        <div className="rounded-xl border bg-card shadow-sm">
          <div className="flex items-center gap-2 px-4 py-3 border-b">
            <ShieldAlert className="h-4 w-4 text-warning shrink-0" />
            <span className="text-sm font-medium">{t('chat.toolApproval.title')}</span>
            <span className="ml-auto px-2 py-0.5 rounded bg-muted text-xs font-mono">
              {toolName}
            </span>
          </div>
          <div className="px-4 py-3">
            <p className="text-sm text-muted-foreground">{reason}</p>
            <p className="text-xs text-muted-foreground mt-2">
              {t('chat.toolApproval.description')}
            </p>
          </div>
          <div className="flex items-center justify-between gap-3 px-4 py-3 border-t">
            {mode === 'manual' || mode === 'allow-list' ? (
              <label className="flex items-center gap-2 text-xs text-muted-foreground cursor-pointer">
                <Checkbox
                  checked={remember}
                  onCheckedChange={(checked) => setRemember(checked === true)}
                  disabled={disabled}
                />
                {rememberLabel}
              </label>
            ) : (
              <span />
            )}
            <div className="flex justify-end gap-2">
              <Button onClick={onDeny} variant="ghost" size="sm" disabled={disabled}>
                {t('chat.toolApproval.deny')}
              </Button>
              <Button
                onClick={() => {
                  onApprove(remember)
                  setRemember(false)
                }}
                size="sm"
                disabled={disabled}
                className="bg-success/90 hover:bg-success text-success-foreground"
              >
                {t('chat.toolApproval.approve')}
              </Button>
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
