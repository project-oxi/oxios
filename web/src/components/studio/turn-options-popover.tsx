// TurnOptionsPopover — Studio's collapsed one-turn override (IA design
// §8.3): "A one-turn override, if shipped, lives in a collapsed `Turn
// options` popover and resets after send."
//
// Ships exactly one override: the model used by the NEXT send only. The
// chat store consumes `turnModelOverride` in the send payload and clears
// it immediately after the frame is written — a second send goes back to
// the persistent binding. The popover NEVER touches `activeModelId`.

import { ChevronDown, SlidersHorizontal } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { useModels } from '@/hooks/use-engine'
import { useChatStore } from '@/stores/chat'

export function TurnOptionsPopover() {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const override = useChatStore((s) => s.turnModelOverride)
  const setOverride = useChatStore((s) => s.setTurnModelOverride)
  const { data: models = [] } = useModels(null)

  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          size="sm"
          data-testid="studio-turn-options"
          className="h-7 gap-1 px-2 text-xs font-normal text-muted-foreground"
          aria-label={t('studio.turnOptions.label')}
        >
          <SlidersHorizontal className="h-3.5 w-3.5" />
          {override ? (
            <span className="max-w-[140px] truncate text-status-info-on-subtle">{override}</span>
          ) : (
            t('studio.turnOptions.label')
          )}
          <ChevronDown className="h-3 w-3" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-[220px]">
        <DropdownMenuLabel>{t('studio.turnOptions.title')}</DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={() => {
            setOverride(null)
            setOpen(false)
          }}
        >
          <span className={!override ? 'font-medium' : ''}>
            {t('studio.turnOptions.noOverride')}
          </span>
        </DropdownMenuItem>
        {models.map((m) => (
          <DropdownMenuItem
            key={m.id}
            onSelect={() => {
              setOverride(m.id)
              setOpen(false)
            }}
          >
            <span className={override === m.id ? 'font-medium' : ''}>{m.id}</span>
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <p className="px-2 py-1.5 text-2xs text-muted-foreground">
          {t('studio.turnOptions.resetsHint')}
        </p>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
