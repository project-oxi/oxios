// Thinking block — collapsible reasoning display (ported from LobeHub).
//
// Visual tier: reasoning lies FLAT — no fill, no border, no rail. It is the
// agent's internal monologue (marginalia), not a container like a tool card.
// Hierarchy reads: answer (foreground) > tool card (bordered container) >
// reasoning (flat muted text). The title row is the only anchor; the body
// drops below it indented, in small muted type. Streaming: full muted weight
// + shiny sweep + spinner. Settled: recedes to 60% so completed thoughts fade
// into the rhythm between tool cards.

import { Brain, Loader2 } from 'lucide-react'
import { memo, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { MarkdownMessage } from '@/components/chat/markdown-message'
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from '@/components/ui/accordion'
import { cn } from '@/lib/utils'

// ── Props ──

export interface ThinkingProps {
  /** Markdown content of the reasoning block. */
  content?: string
  /** Whether the agent is currently thinking (streaming). Auto-expands. */
  thinking?: boolean
  /** Elapsed duration in milliseconds. */
  duration?: number
  /** Owning message id — forwarded to MarkdownMessage for artifact context. */
  messageId?: string
  /** Owning block id — scopes artifact identity within the message. */
  blockId?: string
  /** Extra class on the outer wrapper. */
  className?: string
}

// ── Component ──

export const Thinking = memo(function Thinking({
  content,
  thinking = false,
  duration,
  messageId = '',
  blockId = '',
  className,
}: ThinkingProps) {
  const [open, setOpen] = useState(thinking)

  // Auto-expand while streaming, collapse when done.
  useEffect(() => {
    setOpen(thinking)
  }, [thinking])

  const hasContent = !!content && content.trim().length > 0
  if (!hasContent && !thinking) return null

  return (
    <Accordion
      type="single"
      collapsible
      value={open ? 'thinking' : ''}
      onValueChange={(v) => setOpen(v === 'thinking')}
      className={cn('border-0', className)}
    >
      <AccordionItem value="thinking" className="border-0 bg-transparent">
        <AccordionTrigger className="py-1 px-1 -mx-1 rounded-sm hover:no-underline hover:bg-muted/30 transition-colors">
          <ThinkingTitle thinking={thinking} duration={duration} />
        </AccordionTrigger>
        <AccordionContent className="pb-2 pl-3">
          <MarkdownMessage
            messageId={messageId}
            blockId={blockId}
            isStreaming={thinking}
            className="text-xs"
          >
            {content ?? ''}
          </MarkdownMessage>
        </AccordionContent>
      </AccordionItem>
    </Accordion>
  )
})

// ── Title ──

function ThinkingTitle({ thinking, duration }: { thinking: boolean; duration?: number }) {
  const { t } = useTranslation()
  return (
    <div
      className={cn(
        'flex items-center gap-1.5 text-xs',
        thinking ? 'text-muted-foreground' : 'text-muted-foreground/60',
      )}
    >
      {thinking ? <Loader2 className="w-3 h-3 animate-spin" /> : <Brain className="w-3 h-3" />}
      <span className={cn('font-medium', thinking && 'thinking-shiny')}>
        {thinking ? t('chat.thinking') : t('chat.thought')}
      </span>
      {duration != null && (
        <span className="ml-auto tabular-nums text-muted-foreground/60">
          {formatDuration(duration)}
        </span>
      )}
    </div>
  )
}

// ── Helpers ──

function formatDuration(ms: number): string {
  const seconds = ms / 1000
  if (seconds < 60) return `${seconds.toFixed(1)}s`
  const minutes = Math.floor(seconds / 60)
  const secs = Math.floor(seconds % 60)
  return `${minutes}m ${secs}s`
}
