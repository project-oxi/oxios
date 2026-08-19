// Muted "the turn ended without a fault" footer. Deliberately NOT the
// destructive ErrorCard styling: cancelling is a normal action, and a
// dropped socket is a recoverable failure, not a fault.
//
// `reason` selects the copy:
//   - 'cancelled'   (default): user pressed Stop   → chat.interrupted
//   - 'interrupted'          : socket dropped      → chat.connectionLost
import { CircleSlash } from 'lucide-react'
import { useTranslation } from 'react-i18next'

export type InterruptedNoticeReason = 'cancelled' | 'interrupted'

interface InterruptedNoticeProps {
  reason?: InterruptedNoticeReason
}

export function InterruptedNotice({ reason = 'cancelled' }: InterruptedNoticeProps = {}) {
  const { t } = useTranslation()
  const messageKey = reason === 'interrupted' ? 'chat.connectionLost' : 'chat.interrupted'
  return (
    <div className="mt-1 flex items-center gap-1.5 text-2xs text-muted-foreground" role="status">
      <CircleSlash className="h-3 w-3 shrink-0" />
      {t(messageKey)}
    </div>
  )
}
