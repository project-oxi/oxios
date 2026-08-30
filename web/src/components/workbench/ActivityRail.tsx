// ActivityRail — vertical rail of conversation / files / terminal /
// changes / preview affordances (Task 7, design §7.1).
//
// Affordance → rail mapping (mirrored from the kernel derive_affordances
// rule + design §6.1 / §7.3):
//   * Conversation — always visible
//   * Files         — visible iff `files` affordance
//   * Terminal      — visible iff `terminal` affordance
//   * Changes       — visible iff `diff` affordance
//   * Preview       — visible iff `diff` affordance
//
// Folderless coding projects: only Conversation renders (the rails for
// filesystem affordances are suppressed even when the kernel profile
// grants the affordance — the visible affordance is `diff` only, so
// Changes + Preview still render but the terminal/files rail does not).
//
// Click behavior:
//   * Clicking a rail sets activeRail AND opens the corresponding stage
//     (if the stage is closed). The Conversation rail never opens a stage
//     — clicking it just dismisses any open stage so the user can return
//     to the focused chat canvas.
//   * The active rail is highlighted with an accent border.

import {
  Eye,
  FileText,
  GitPullRequest,
  MessageSquare,
  Terminal as TerminalIcon,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { has } from '@/lib/affordances'
import { cn } from '@/lib/utils'
import { type Rail, useWorkbenchStore } from '@/stores/workbench'
import type { EffectiveProfile } from '@/types'

interface ActivityRailProps {
  profile: EffectiveProfile | null
}

const RAILS: { rail: Rail; icon: typeof MessageSquare; key: string }[] = [
  { rail: 'conversation', icon: MessageSquare, key: 'conversation' },
  { rail: 'files', icon: FileText, key: 'files' },
  { rail: 'terminal', icon: TerminalIcon, key: 'terminal' },
  { rail: 'changes', icon: GitPullRequest, key: 'changes' },
  { rail: 'preview', icon: Eye, key: 'preview' },
]

function railVisible(rail: Rail, profile: EffectiveProfile | null): boolean {
  if (rail === 'conversation') return true
  if (rail === 'files') return has(profile, 'files')
  if (rail === 'terminal') return has(profile, 'terminal')
  if (rail === 'changes' || rail === 'preview') return has(profile, 'diff')
  return false
}

export function ActivityRail({ profile }: ActivityRailProps) {
  const { t } = useTranslation()
  const activeRail = useWorkbenchStore((s) => s.activeRail)
  const stageOpen = useWorkbenchStore((s) => s.stageOpen)
  const setActiveRail = useWorkbenchStore((s) => s.setActiveRail)
  const userSelectRail = useWorkbenchStore((s) => s.userSelectRail)
  const dismissStage = useWorkbenchStore((s) => s.dismissStage)

  const handleClick = (rail: Rail) => {
    if (rail === 'conversation') {
      // The conversation rail is the chat itself — select it and drop the
      // stage so the transcript regains full width.
      setActiveRail('conversation')
      dismissStage()
      return
    }
    // Explicit user override: fresh, unpinned stage for the target rail
    // (a stale emergence pin must not reattach — fix F5).
    userSelectRail(rail)
  }

  return (
    <nav
      data-testid="activity-rail"
      aria-label={t('workbench.rail.label')}
      className="flex h-full w-12 shrink-0 flex-col items-center gap-1 border-r bg-muted/30 py-2"
    >
      {RAILS.filter((r) => railVisible(r.rail, profile)).map(({ rail, icon: Icon, key }) => {
        const isActive = activeRail === rail && stageOpen
        return (
          <button
            key={rail}
            type="button"
            onClick={() => handleClick(rail)}
            aria-label={t(`workbench.rail.${key}`)}
            title={t(`workbench.rail.${key}`)}
            data-rail={rail}
            className={cn(
              'flex h-9 w-9 items-center justify-center rounded-md transition-colors',
              isActive
                ? 'bg-primary/10 text-primary'
                : 'text-muted-foreground hover:bg-muted hover:text-foreground',
            )}
          >
            <Icon className="h-4 w-4" />
          </button>
        )
      })}
    </nav>
  )
}
