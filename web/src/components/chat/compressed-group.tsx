// CompressedGroup — collapsible panel with Summary/History tabs for older
// messages in long conversations (LobeHub CompressedGroup port).

import { ChevronDown, ChevronRight, FileText, History, MessagesSquare } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { buildCompressedDigest } from '@/lib/compressed-summary'
import { cn } from '@/lib/utils'
import type { ChatMessage, CompressionInfo } from '@/types'

interface CompressedGroupProps {
  /** Number of messages hidden while collapsed. */
  count: number
  expanded: boolean
  onToggle: () => void
  foldedMessages: ChatMessage[]
  compression: CompressionInfo | null
  className?: string
}

type Tab = 'summary' | 'history'

export function CompressedGroup({
  count,
  expanded,
  onToggle,
  foldedMessages,
  compression,
  className,
}: CompressedGroupProps) {
  const { t } = useTranslation()
  const [tab, setTab] = useState<Tab>('summary')

  return (
    <div className={cn('w-full', className)}>
      {/* Toggle bar */}
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-center gap-2 rounded-lg border border-dashed bg-muted/30 px-3 py-2 text-xs text-muted-foreground transition-colors hover:bg-muted/60"
      >
        {expanded ? (
          <ChevronDown className="size-3.5 shrink-0" />
        ) : (
          <ChevronRight className="size-3.5 shrink-0" />
        )}
        <MessagesSquare className="size-3.5 shrink-0" />
        <span>
          {expanded ? t('chat.compressedExpanded') : t('chat.compressedCollapsed', { count })}
        </span>
      </button>

      {/* Tabbed panel (only when expanded) */}
      {expanded && (
        <div className="mt-1 rounded-lg border bg-card">
          {/* Tab headers */}
          <div className="flex border-b px-2">
            <TabButton active={tab === 'summary'} onClick={() => setTab('summary')}>
              <FileText className="size-3" />
              {t('chat.compression.summaryTab')}
            </TabButton>
            <TabButton active={tab === 'history'} onClick={() => setTab('history')}>
              <History className="size-3" />
              {t('chat.compression.historyTab')}
            </TabButton>
          </div>

          {/* Tab content */}
          <div className="max-h-80 overflow-y-auto p-3 text-sm">
            {tab === 'summary' ? (
              <SummaryContent compression={compression} messages={foldedMessages} />
            ) : (
              <HistoryContent messages={foldedMessages} />
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function TabButton({
  active,
  onClick,
  children,
}: {
  active: boolean
  onClick: () => void
  children: React.ReactNode
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'flex items-center gap-1 border-b-2 px-3 py-1.5 text-xs font-medium transition-colors',
        active
          ? 'border-primary text-foreground'
          : 'border-transparent text-muted-foreground hover:text-foreground',
      )}
    >
      {children}
    </button>
  )
}

function SummaryContent({
  compression,
  messages,
}: {
  compression: CompressionInfo | null
  messages: ChatMessage[]
}) {
  const { t } = useTranslation()

  if (compression?.status === 'generating') {
    return (
      <div className="space-y-2">
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="inline-block size-2 animate-pulse rounded-full bg-primary" />
          {t('chat.compression.generating')}
        </div>
        {compression.summary && (
          <div className="whitespace-pre-wrap text-sm">{compression.summary}</div>
        )}
      </div>
    )
  }

  if (compression?.status === 'done' && compression.summary) {
    return (
      <div className="prose prose-sm max-w-none whitespace-pre-wrap">{compression.summary}</div>
    )
  }

  if (compression?.status === 'failed') {
    return (
      <div className="space-y-2">
        <p className="text-xs text-destructive">{t('chat.compression.failed')}</p>
        <DigestFallback messages={messages} />
      </div>
    )
  }

  // No compression yet — show statistical digest.
  return <DigestFallback messages={messages} />
}

function DigestFallback({ messages }: { messages: ChatMessage[] }) {
  const { t } = useTranslation()
  const d = buildCompressedDigest(messages)
  return (
    <div className="flex flex-wrap gap-3 text-xs text-muted-foreground">
      <span>{t('chat.compression.digest.messages', { count: d.total })}</span>
      <span>{t('chat.compression.digest.userMessages', { count: d.userCount })}</span>
      <span>{t('chat.compression.digest.assistantMessages', { count: d.assistantCount })}</span>
      <span>{t('chat.compression.digest.toolCalls', { count: d.toolCallCount })}</span>
    </div>
  )
}

function HistoryContent({ messages }: { messages: ChatMessage[] }) {
  if (messages.length === 0) {
    return null
  }

  return (
    <div className="space-y-2">
      {messages.map((m) => (
        <div key={m.id} className="flex gap-2 text-xs">
          <span className="shrink-0 font-medium text-muted-foreground">
            {m.role === 'user' ? '👤' : '🤖'}
          </span>
          <span className="line-clamp-2 text-foreground/80">{m.content}</span>
        </div>
      ))}
    </div>
  )
}
