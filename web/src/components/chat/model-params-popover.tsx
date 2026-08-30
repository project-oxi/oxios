// ModelParamsPopover — temperature + max_tokens controls for the next message.
//
// LobeHub analogue: features/ChatInput/ActionBar/Params/ (sliders popover).
// Oxios version: a popover with a styled Radix Slider for temperature and
// preset chips + direct input for max tokens (a 256-step slider over a
// 256–32768 range had no usable granularity).
// Values flow into the WS payload as `temperature` and `max_tokens`; the
// backend reads them from IncomingMessage metadata and threads them
// through `MsgCtx` → `ExecEnv::model_params` → `AgentConfig`.
//
// When a control is reset to the provider default (null), the field is
// omitted from the WS payload and the agent runtime falls back to its
// built-in defaults (0.7 / 8192).

import { ChevronDown, RotateCcw, SlidersHorizontal } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Slider } from '@/components/ui/slider'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'

/** Runtime default temperature mirrored from agent_runtime.rs (0.7). */
const TEMP_DEFAULT = 0.7
const MAX_TOKENS_CEILING = 32768

/** Sensible output-length presets; the last one doubles as "long form". */
const TOKEN_PRESETS = [1024, 4096, 8192, 16384, 32768] as const

function formatTokens(n: number): string {
  return n >= 1024 && n % 1024 === 0 ? `${n / 1024}K` : n.toLocaleString()
}

export function ModelParamsPopover() {
  const { t } = useTranslation()
  const temperature = useChatStore((s) => s.temperature)
  const maxTokens = useChatStore((s) => s.maxTokens)
  const setTemperature = useChatStore((s) => s.setTemperature)
  const setMaxTokens = useChatStore((s) => s.setMaxTokens)
  const [open, setOpen] = useState(false)
  // Local text mirror for the token input so partial typing doesn't fight the
  // store; committed on blur/Enter.
  const [tokenDraft, setTokenDraft] = useState('')
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [open])

  const hasOverrides = temperature != null || maxTokens != null

  const commitTokenDraft = () => {
    const parsed = Number(tokenDraft)
    if (tokenDraft.trim() === '' || !Number.isFinite(parsed)) {
      setTokenDraft('')
      return
    }
    setMaxTokens(Math.max(256, Math.min(MAX_TOKENS_CEILING, Math.round(parsed))))
    setTokenDraft('')
  }

  return (
    <div ref={ref} className="relative">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className={cn(
          'flex h-8 items-center gap-1 rounded-md border px-2 text-xs transition-colors',
          hasOverrides
            ? 'border-primary/40 bg-primary/5 text-primary'
            : 'border-input text-muted-foreground hover:bg-accent hover:text-foreground',
        )}
        aria-label={t('chat.modelParams')}
        aria-expanded={open}
      >
        <SlidersHorizontal className="size-3.5" />
        <ChevronDown className="size-3" />
      </button>
      {open && (
        <div className="absolute bottom-full right-0 z-20 mb-1 w-80 rounded-lg border bg-popover p-4 shadow-lg">
          {/* ── Temperature ─────────────────────────────────────── */}
          <div className="mb-4">
            <div className="mb-2 flex items-center justify-between">
              <label htmlFor="temp-slider" className="text-xs font-medium">
                {t('chat.temperature')}
              </label>
              <div className="flex items-center gap-1.5">
                <span className="text-xs tabular-nums text-foreground">
                  {temperature != null ? temperature.toFixed(1) : t('chat.default')}
                </span>
                {temperature != null && (
                  <button
                    type="button"
                    onClick={() => setTemperature(null)}
                    aria-label={t('chat.reset')}
                    className="text-muted-foreground hover:text-foreground"
                  >
                    <RotateCcw className="size-3" />
                  </button>
                )}
              </div>
            </div>
            <Slider
              id="temp-slider"
              min={0}
              max={2}
              step={0.1}
              value={[temperature ?? TEMP_DEFAULT]}
              onValueChange={(v) => v[0] !== undefined && setTemperature(v[0])}
            />
            <div className="mt-1 flex justify-between text-2xs text-muted-foreground">
              <span>{t('chat.tempPrecise')}</span>
              <span>{t('chat.tempDefault')}</span>
              <span>{t('chat.tempCreative')}</span>
            </div>
          </div>

          {/* ── Max tokens ──────────────────────────────────────── */}
          <div>
            <div className="mb-2 flex items-center justify-between">
              <label htmlFor="maxtok-input" className="text-xs font-medium">
                {t('chat.maxTokens')}
              </label>
              <div className="flex items-center gap-1.5">
                <span className="text-xs tabular-nums text-foreground">
                  {maxTokens != null ? formatTokens(maxTokens) : t('chat.default')}
                </span>
                {maxTokens != null && (
                  <button
                    type="button"
                    onClick={() => setMaxTokens(null)}
                    aria-label={t('chat.reset')}
                    className="text-muted-foreground hover:text-foreground"
                  >
                    <RotateCcw className="size-3" />
                  </button>
                )}
              </div>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {TOKEN_PRESETS.map((preset) => (
                <button
                  key={preset}
                  type="button"
                  onClick={() => setMaxTokens(preset)}
                  className={cn(
                    'rounded-md border px-2 py-1 text-2xs tabular-nums transition-colors',
                    maxTokens === preset
                      ? 'border-primary/40 bg-primary/5 text-primary'
                      : 'border-input text-muted-foreground hover:bg-accent hover:text-foreground',
                  )}
                >
                  {formatTokens(preset)}
                </button>
              ))}
              <input
                id="maxtok-input"
                type="number"
                inputMode="numeric"
                min={256}
                max={MAX_TOKENS_CEILING}
                placeholder={t('chat.custom')}
                value={tokenDraft}
                onChange={(e) => setTokenDraft(e.target.value)}
                onBlur={commitTokenDraft}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') {
                    e.preventDefault()
                    commitTokenDraft()
                  }
                }}
                className={cn(
                  'h-6 w-20 rounded-md border border-input bg-transparent px-2 text-2xs tabular-nums',
                  'placeholder:text-muted-foreground focus:outline-none focus-visible:ring-1 focus-visible:ring-ring',
                )}
              />
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
