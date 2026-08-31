// messages/CommandNotice — slash-command feedback row (`role: 'system'`).
//
// `/compact` and `/persona` execute server-side without a turn; the reply
// arrives as a `command_result` frame which the store appends as a system
// message. Rendered as a compact centered pill — distinct from assistant
// bubbles, mirroring Claude Code's inline command output. Runtime-only:
// never persisted, so it disappears on session reload.

import { cn } from '@/lib/utils'
import type { ChatMessage } from '@/types'

export function CommandNotice({ message }: { message: ChatMessage }) {
  const isError = message.metadata?.status === 'error'
  return (
    <div className="flex justify-center py-1" data-testid="command-notice">
      <div
        className={cn(
          'max-w-[85%] rounded-full border px-3 py-1 text-xs whitespace-pre-wrap text-center',
          isError
            ? 'border-destructive/30 bg-destructive/5 text-destructive'
            : 'border-border bg-muted/50 text-muted-foreground',
        )}
      >
        {message.content}
      </div>
    </div>
  )
}
