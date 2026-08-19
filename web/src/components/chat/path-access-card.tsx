import { FolderPlus, ShieldCheck, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'

interface PathAccessCardProps {
  path: string
  mode: string
  toolName: string
  reason: string
  onMount: () => void
  onTempAllow: () => void
  onDeny: () => void
  disabled?: boolean
}

/**
 * Inline path-access card shown when an agent tries to read or write a
 * file outside its `allowed_paths`. Offers three choices:
 *   - Create Mount (persistent path alias, survives restart)
 *   - Temporarily allow (session-scoped `allowed_paths` entry)
 *   - Deny (return the error to the agent)
 *
 * Mirrors ToolApprovalCard's visual structure.
 */
export function PathAccessCard({
  path,
  mode,
  toolName,
  reason,
  onMount,
  onTempAllow,
  onDeny,
  disabled,
}: PathAccessCardProps) {
  const { t } = useTranslation()

  return (
    <div className="flex gap-3 my-1.5">
      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-warning text-warning-foreground">
        <ShieldCheck className="h-4 w-4" />
      </div>
      <div className="max-w-[80%] flex-1">
        <div className="rounded-xl border bg-card shadow-sm">
          <div className="flex items-center gap-2 px-4 py-3 border-b">
            <ShieldCheck className="h-4 w-4 text-warning shrink-0" />
            <span className="text-sm font-medium">{t('pathAccess.title')}</span>
            <span className="ml-auto px-2 py-0.5 rounded bg-muted text-xs font-mono">
              {mode === 'write' ? 'write' : 'read'}
            </span>
          </div>
          <div className="px-4 py-3">
            <p className="text-sm text-muted-foreground break-all font-mono text-xs">{path}</p>
            {reason && <p className="text-xs text-muted-foreground mt-2">{reason}</p>}
            <p className="text-xs text-muted-foreground mt-1">
              {t('pathAccess.prompt', { toolName })}
            </p>
          </div>
          <div className="flex items-center justify-end gap-2 px-4 py-3 border-t">
            <Button onClick={onDeny} variant="ghost" size="sm" disabled={disabled}>
              <X className="h-3.5 w-3.5 mr-1" />
              {t('pathAccess.deny')}
            </Button>
            <Button onClick={onTempAllow} variant="outline" size="sm" disabled={disabled}>
              {t('pathAccess.allowOnce')}
            </Button>
            <Button
              onClick={onMount}
              size="sm"
              disabled={disabled}
              className="bg-success/90 hover:bg-success text-success-foreground"
            >
              <FolderPlus className="h-3.5 w-3.5 mr-1" />
              {t('pathAccess.createMount')}
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
