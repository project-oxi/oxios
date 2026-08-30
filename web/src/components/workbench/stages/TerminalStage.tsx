// TerminalStage — aggregated terminal tool-call view from the transcript
// (Task 7). NOT a PTY; the stage surfaces the same shell/command tool
// calls the agent already emitted so the user can scroll, copy, and
// review past command output without leaving the workbench.

import { useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { isTerminalToolName } from '@/components/workbench/terminal-aliases'
import { useChatStore } from '@/stores/chat'
import type { ChatBlock } from '@/types'

function extractText(result: unknown): string {
  if (!result) return ''
  if (typeof result === 'string') return result
  if (typeof result === 'object' && result !== null) {
    const obj = result as Record<string, unknown>
    if (typeof obj.output === 'string') return obj.output
    if (typeof obj.stdout === 'string') return obj.stdout
    if (typeof obj.text === 'string') return obj.text
    if (typeof obj.content === 'string') return obj.content
  }
  try {
    return JSON.stringify(result)
  } catch {
    return ''
  }
}

export function TerminalStage() {
  const { t } = useTranslation()
  const messages = useChatStore((s) => s.messages)

  const entries = useMemo(() => {
    const out: { id: string; command: string; output: string; at: number }[] = []
    for (const m of messages) {
      if (m?.role !== 'assistant') continue
      const blocks = (m.blocks ?? []) as ChatBlock[]
      for (const b of blocks) {
        if (b.type !== 'tool') continue
        const tool = b as unknown as {
          id: string
          apiName?: string
          arguments?: Record<string, unknown>
          result?: unknown
        }
        if (!isTerminalToolName(tool.apiName)) continue
        const cmd =
          (typeof tool.arguments?.command === 'string' && tool.arguments.command) ||
          (typeof tool.arguments?.cmd === 'string' && tool.arguments.cmd) ||
          tool.apiName ||
          ''
        out.push({
          id: tool.id,
          command: cmd,
          output: extractText(tool.result),
          at: Date.parse(m.timestamp ?? '') || 0,
        })
      }
    }
    return out
  }, [messages])

  return (
    <div className="flex h-full flex-col" data-testid="terminal-stage">
      <div className="flex items-center justify-between border-b px-3 py-2 text-xs">
        <span className="font-medium">{t('workbench.stage.terminal')}</span>
      </div>
      <div className="flex-1 overflow-y-auto">
        {entries.length === 0 ? (
          <div className="p-3 text-2xs text-muted-foreground">{t('workbench.terminal.empty')}</div>
        ) : (
          <ul className="divide-y">
            {entries.map((e) => (
              <li key={e.id} className="flex flex-col gap-1 px-3 py-2">
                <pre className="rounded bg-muted/40 px-2 py-1 text-2xs font-mono text-primary">
                  $ {e.command}
                </pre>
                {e.output && (
                  <pre className="max-h-32 overflow-auto rounded bg-muted/40 p-2 text-2xs font-mono text-foreground/80">
                    {e.output}
                  </pre>
                )}
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  )
}
