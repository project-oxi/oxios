// SendEmail render — email sending result card.

import { Mail } from 'lucide-react'
import type { ToolRenderComponent } from './registry'

export const SendEmailRender: ToolRenderComponent = ({ args, result, isRunning }) => {
  const to = (args?.to ?? args?.recipient ?? '') as string
  const subject = (args?.subject ?? '') as string

  let status = 'sending'
  let messageId: string | undefined
  if (!isRunning && typeof result === 'string') {
    try {
      const parsed = JSON.parse(result)
      status = parsed.status ?? 'sent'
      messageId = parsed.message_id
    } catch {
      status = result.includes('sent') || result.includes('success') ? 'sent' : 'error'
    }
  }

  return (
    <div className="space-y-1.5 text-sm">
      <div className="flex items-center gap-2 text-xs">
        <Mail className="w-3.5 h-3.5 text-muted-foreground" />
        <span className="font-medium truncate">{subject || '(no subject)'}</span>
        <span
          className={`ml-auto shrink-0 ${status === 'sent' ? 'text-status-success-on-surface' : 'text-status-warning-on-surface'}`}
        >
          {status}
        </span>
      </div>
      {to && <div className="text-xs text-muted-foreground">To: {to}</div>}
      {messageId && <div className="text-xs text-muted-foreground font-mono">ID: {messageId}</div>}
    </div>
  )
}
