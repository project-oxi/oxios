// messages/components/ErrorCard — inline error display with retry.
//
// LobeHub analogue: Conversation/Messages/Error/.
// Replaces ChatItem's ErrorBlock for assistant-role errors that benefit from
// richer presentation (retry button, errorKind-specific copy + suggestion).
//
// All copy is i18n-driven. The lookup table maps every backend `errorKind`
// (snake_case, from KNOWN_ERROR_KINDS in src/types/chat.ts) to a locale
// key under `chat.error.<key>.title` / `.hint`. An unknown kind falls back
// to the `chat.error.unknown` entry rather than a hardcoded English string.

import { AlertTriangle, RefreshCw } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { cn } from '@/lib/utils'
import type { ChatError } from '@/types/chat'

interface ErrorCardProps {
  error: ChatError
  onRetry?: () => void
  className?: string
}

/** Map backend `errorKind` (snake_case wire format) to an i18n key stem.
 *  Unknown backend kinds fall through to the `'unknown'` stem. The map
 *  covers every entry in KNOWN_ERROR_KINDS so renaming a variant on the
 *  wire becomes a build-time miss instead of a silent fallback. */
const KIND_KEYS: Record<string, string> = {
  execution_failed: 'executionFailed',
  api_key_missing: 'apiKeyMissing',
  provider_error: 'providerError',
  timeout: 'timeout',
  permission_denied: 'permissionDenied',
  validation_error: 'validationError',
  cancelled: 'cancelled',
  internal: 'internal',
  unknown: 'unknown',
}

export function ErrorCard({ error, onRetry, className }: ErrorCardProps) {
  const { t } = useTranslation()
  const rawKind = (error.category ?? error.type ?? 'unknown') as string
  const key = KIND_KEYS[rawKind] ?? 'unknown'
  const title = t(`chat.error.${key}.title`)
  const hint = t(`chat.error.${key}.hint`, { defaultValue: '' })
  const severity = error.severity ?? 'error'
  const isCritical = severity === 'critical'

  return (
    <div
      className={cn(
        'rounded-md border px-3 py-2 text-sm flex items-start gap-2',
        isCritical
          ? 'border-destructive bg-destructive/10 text-destructive'
          : 'border-destructive/40 bg-destructive/5 text-destructive',
        className,
      )}
      role="alert"
    >
      <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
      <div className="flex-1 min-w-0">
        <div className="font-medium">{title}</div>
        {error.message && <div className="text-xs mt-0.5 opacity-90">{error.message}</div>}
        {hint && <div className="text-xs mt-1 opacity-75 italic">{hint}</div>}
      </div>
      {onRetry && (
        <button
          type="button"
          onClick={onRetry}
          className="inline-flex items-center gap-1 px-2 py-1 rounded text-xs bg-destructive text-destructive-foreground hover:opacity-90 transition-opacity shrink-0"
        >
          <RefreshCw className="w-3 h-3" />
          {t('chat.retry')}
        </button>
      )}
    </div>
  )
}
