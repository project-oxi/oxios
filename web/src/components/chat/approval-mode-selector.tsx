// ApprovalModeSelector — toolbar dropdown that shows the current approval mode
// (Manual / Allow list / Auto-run) and PATCHes /api/security/approval on pick.
//
// LobeHub analogue: features/ChatInput/ActionBar/ApprovalModeSelect/.

import type { LucideIcon } from 'lucide-react'
import { Check, ChevronDown, Hand, ListChecks, Zap } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useApprovalConfig, useSetApprovalMode } from '@/hooks/use-approval-config'
import { cn } from '@/lib/utils'
import type { ApprovalMode } from '@/types/approval'

const MODE_LABELS: Record<ApprovalMode, { key: string; icon: LucideIcon }> = {
  manual: { key: 'approvalMode.manual', icon: Hand },
  'allow-list': { key: 'approvalMode.allowList', icon: ListChecks },
  'auto-run': { key: 'approvalMode.autoRun', icon: Zap },
}

const MODE_ORDER: ApprovalMode[] = ['manual', 'allow-list', 'auto-run']

export function ApprovalModeSelector() {
  const { t } = useTranslation()
  const { data, isLoading } = useApprovalConfig()
  const setMode = useSetApprovalMode()

  const current: ApprovalMode = data?.mode ?? 'manual'
  const currentEntry = MODE_LABELS[current]
  const CurrentIcon = currentEntry.icon
  const isPending = setMode.isPending

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          disabled={isLoading || isPending}
          aria-label={t('approvalMode.buttonLabel')}
          className="h-8 gap-1 px-2 text-xs font-normal"
        >
          <CurrentIcon className="size-3.5" />
          <span>{t(currentEntry.key)}</span>
          <ChevronDown className="size-3 opacity-70" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top">
        {MODE_ORDER.map((mode) => {
          const entry = MODE_LABELS[mode]
          const Icon = entry.icon
          const isCurrent = mode === current
          return (
            <DropdownMenuItem
              key={mode}
              disabled={isPending || isCurrent}
              onClick={() => {
                if (mode !== current) setMode.mutate(mode)
              }}
              className={cn('cursor-pointer gap-2', isCurrent && 'opacity-70')}
            >
              <Icon className="size-3.5 text-muted-foreground" />
              <span className="flex-1">{t(entry.key)}</span>
              {isCurrent && <Check className="size-3.5 text-primary" />}
            </DropdownMenuItem>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
