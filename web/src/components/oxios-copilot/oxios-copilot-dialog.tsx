import { useRouter } from '@tanstack/react-router'
import { Check, Copy, MessageSquarePlus, X } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ChatInput } from '@/components/chat/chat-input'
import { MessageBubble } from '@/components/chat/message-bubble'
import { ToolApprovalCard } from '@/components/chat/tool-approval-card'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { ScrollArea } from '@/components/ui/scroll-area'
import { useEngineConfig } from '@/hooks/use-engine'
import { api } from '@/lib/api-client'
import { useChatStore } from '@/stores/chat'
import { buildSeedRequestBody, useOxiosCopilotStore } from '@/stores/oxios-copilot'

/**
 * OxiosCopilotDialog — global one-shot question overlay.
 *
 * Renders in AppLayout so it overlays every route. Sends `ephemeral: true`
 * over its own short-lived WS; nothing is persisted. Uses the same ChatInput
 * as the regular chat page so the UX (model picker, queue, stop, streaming)
 * is identical — the only difference is that no session is saved.
 */
export function OxiosCopilotDialog() {
  const { t } = useTranslation()
  const router = useRouter()
  const open = useOxiosCopilotStore((s) => s.open)
  const closeCopilot = useOxiosCopilotStore((s) => s.closeCopilot)
  const messages = useOxiosCopilotStore((s) => s.messages)
  const isStreaming = useOxiosCopilotStore((s) => s.isStreaming)
  const send = useOxiosCopilotStore((s) => s.send)
  const cancel = useOxiosCopilotStore((s) => s.cancel)
  const copilotModel = useOxiosCopilotStore((s) => s.copilotModel)
  const setCopilotModel = useOxiosCopilotStore((s) => s.setCopilotModel)
  const queuedCount = useOxiosCopilotStore((s) => s._pendingQueue.length)
  const lastExchange = useOxiosCopilotStore((s) => s.lastExchange)
  const activeToolApproval = useOxiosCopilotStore((s) => s.activeToolApproval)
  const resolveToolApproval = useOxiosCopilotStore((s) => s.resolveToolApproval)
  const reset = useOxiosCopilotStore((s) => s.reset)

  // Sync the engine-config one-shot model into the store (single source: Settings).
  const { data: engineConfig } = useEngineConfig()
  useEffect(() => {
    const configured = engineConfig?.quick_ask_model
    const fallback = engineConfig?.default_model || null
    setCopilotModel(configured || fallback)
  }, [engineConfig?.quick_ask_model, engineConfig?.default_model, setCopilotModel])

  const [input, setInput] = useState('')
  const [copied, setCopied] = useState(false)
  const scrollRef = useRef<HTMLDivElement>(null)

  // Auto-scroll on new content.
  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: 'smooth' })
  }, [messages, isStreaming])

  // Reset input when the dialog closes.
  useEffect(() => {
    if (!open) {
      setInput('')
      setCopied(false)
    }
  }, [open])

  const handleCopy = async () => {
    const reply = lastExchange?.reply
    if (!reply) return
    try {
      await navigator.clipboard.writeText(reply)
      setCopied(true)
      setTimeout(() => setCopied(false), 2000)
    } catch {
      toast.error(t('oxiosCopilot.copyFailed'))
    }
  }

  const handlePromote = async () => {
    if (!lastExchange) return
    try {
      const res = await api.post<{ session_id: string }>(
        '/api/chat/seed',
        buildSeedRequestBody(lastExchange),
      )
      closeCopilot()
      reset()
      router.history.push('/studio')
      // Seed the chat store so /studio shows the promoted exchange immediately.
      await useChatStore.getState().loadSession(res.session_id)
      toast.success(t('oxiosCopilot.promoted'))
    } catch {
      toast.error(t('oxiosCopilot.promoteFailed'))
    }
  }

  const hasResult = !isStreaming && lastExchange !== null
  const empty = messages.length === 0

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) closeCopilot()
      }}
    >
      <DialogContent
        showCloseButton={false}
        onOpenAutoFocus={(e) => e.preventDefault()}
        className="flex h-[80vh] max-w-2xl flex-col gap-0 p-0 sm:rounded-xl"
      >
        <DialogHeader className="flex-row items-center justify-between border-b px-5 py-3">
          <div className="flex items-center gap-2">
            <DialogTitle className="text-sm font-medium">{t('oxiosCopilot.title')}</DialogTitle>
          </div>
          <DialogDescription className="sr-only">{t('oxiosCopilot.placeholder')}</DialogDescription>
          <Button
            variant="ghost"
            size="icon"
            className="h-7 w-7"
            onClick={closeCopilot}
            aria-label={t('common.close')}
          >
            <X className="h-4 w-4" />
          </Button>
        </DialogHeader>

        <ScrollArea className="flex-1" ref={scrollRef}>
          <div className="space-y-4 px-5 py-4">
            {empty && !isStreaming && (
              <div className="py-10 text-center">
                <p className="text-sm font-medium">{t('oxiosCopilot.subtitle')}</p>
                <div className="mt-4 flex flex-wrap justify-center gap-2 px-6">
                  {(['addMcp', 'budget', 'tasks', 'model'] as const).map((k) => (
                    <button
                      key={k}
                      type="button"
                      onClick={() => send(t(`oxiosCopilot.chips.${k}`))}
                      className="rounded-full border bg-muted/40 px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-muted/70"
                    >
                      {t(`oxiosCopilot.chips.${k}`)}
                    </button>
                  ))}
                </div>
              </div>
            )}
            {messages.map((m) => (
              <MessageBubble key={m.id} message={m} />
            ))}
            {activeToolApproval && (
              <ToolApprovalCard
                toolName={activeToolApproval.toolName}
                reason={activeToolApproval.reason}
                onApprove={(remember) => resolveToolApproval(activeToolApproval.id, true, remember)}
                onDeny={() => resolveToolApproval(activeToolApproval.id, false)}
              />
            )}
          </div>
        </ScrollArea>

        {/* Result actions (copy / promote) — shown only when a turn completed. */}
        {hasResult && (
          <div className="flex items-center justify-between border-t px-5 py-1.5">
            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                size="sm"
                onClick={handleCopy}
                className="h-7 gap-1.5 px-2 text-xs"
              >
                {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
                {t('oxiosCopilot.copy')}
              </Button>
              <Button
                variant="ghost"
                size="sm"
                onClick={handlePromote}
                className="h-7 gap-1.5 px-2 text-xs"
              >
                <MessageSquarePlus className="h-3 w-3" />
                {t('oxiosCopilot.promote')}
              </Button>
            </div>
            <span className="text-[10px] text-muted-foreground">{t('oxiosCopilot.notSaved')}</span>
          </div>
        )}

        {/* Input — same ChatInput as the chat page for UX parity. */}
        <div className="border-t bg-background/95 backdrop-blur-sm shrink-0">
          <ChatInput
            value={input}
            onChange={setInput}
            onSend={(content, _ctx, _files) => send(content)}
            onCancel={cancel}
            isStreaming={isStreaming}
            connected={true}
            queuedCount={queuedCount}
            variant="copilot"
            activeModelId={copilotModel}
            setActiveModelId={setCopilotModel}
            placeholder={t('oxiosCopilot.placeholder')}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}
