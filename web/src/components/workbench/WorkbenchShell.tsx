// WorkbenchShell — the conversation-first shell that wraps the chat
// canvas when the server-derived EffectiveProfile is in the
// Base/Code family (Task 7, design §7.1).
//
// Layout:
//   ┌─────────────────────────────────────────────────┐
//   │ ProjectBar (project · roots · status · ⌘K)     │
//   ├─────┬───────────────────────────────────────────┤
//   │     │                                           │
//   │ R   │   <children> — chat transcript + composer │
//   │ a   │                                           │
//   │ i   │       ┌─────────────────────┐             │
//   │ l   │       │  Stage panel        │ ← optional  │
//   │     │       │  (slide-over)       │             │
//   └─────┴───────────────────────────────────────────┘
//
// Folderless coding projects: NO rail, NO trees. The ProjectBar
// collapses to the project selector + Add folders action.
//
// ⌘P is bound on the chat route only — see useWorkbenchShortcut hook
// below; it calls preventDefault so the browser print dialog never
// opens.

import { X } from 'lucide-react'
import { type ReactNode, useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { ActivityRail } from '@/components/workbench/ActivityRail'
import { FileEditorModal } from '@/components/workbench/FileEditorModal'
import { ProjectBar } from '@/components/workbench/ProjectBar'
import { QuickOpen } from '@/components/workbench/QuickOpen'
import { DiffStage } from '@/components/workbench/stages/DiffStage'
import { PreviewStage } from '@/components/workbench/stages/PreviewStage'
import { TerminalStage } from '@/components/workbench/stages/TerminalStage'
import { useEffectiveProfile } from '@/hooks/use-effective-profile'
import { useProject } from '@/hooks/use-projects'
import { useStageEmergence } from '@/hooks/use-stage-emergence'
import { isCodingFamily } from '@/lib/affordances'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'
import { type Rail, useWorkbenchStore } from '@/stores/workbench'

interface WorkbenchShellProps {
  /** The shared chat canvas (transcript + composer). */
  children: ReactNode
}

function StagePanel({ rail }: { rail: Rail }) {
  if (rail === 'changes') return <DiffStage />
  if (rail === 'preview') return <PreviewStage />
  if (rail === 'terminal') return <TerminalStage />
  // Files + Conversation: no stage panel needed; the rail click on Files
  // opens the QuickOpen dialog instead (handled in QuickOpen).
  return null
}

/**
 * Bind ⌘P on the chat route — CODING PROFILES ONLY (fix F4). A non-coding
 * persona must never swallow browser print. Also clears a stale
 * quickOpenOpen when the profile loses the coding family mid-flight.
 */
function useWorkbenchShortcut(active: boolean): void {
  const setQuickOpen = useWorkbenchStore((s) => s.setQuickOpen)
  useEffect(() => {
    if (!active) {
      // Losing the coding family closes the dialog and stops listening —
      // ⌘P reverts to the browser's own binding (print).
      setQuickOpen(false)
      return
    }
    // Read the current pathname from window.location so this hook works
    // outside a RouterProvider (tests, embedded routes). The Studio route
    // is the only legitimate activation context for ⌘P. (The
    // use-tab-shortcuts registry only covers ⌃1-3 route switching, so the
    // bespoke listener stays; it is gated on `active` here.)
    const onKey = (e: KeyboardEvent) => {
      const onStudio = window.location.pathname.startsWith('/studio')
      if (!onStudio) return
      if (!e.metaKey && !e.ctrlKey) return
      if (e.shiftKey || e.altKey) return
      if (e.code !== 'KeyP' && e.key.toLowerCase() !== 'p') return
      e.preventDefault()
      setQuickOpen(true)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, setQuickOpen])
}

export function WorkbenchShell({ children }: WorkbenchShellProps) {
  const { t } = useTranslation()
  const profile = useEffectiveProfile()
  const isCoding = isCodingFamily(profile)
  useWorkbenchShortcut(isCoding)
  useStageEmergence(profile)
  const activeProjectId = useChatStore((s) => s.activeProjectId)
  const { data: project } = useProject(activeProjectId)
  const stageOpen = useWorkbenchStore((s) => s.stageOpen)
  const activeRail = useWorkbenchStore((s) => s.activeRail)
  const dismissStage = useWorkbenchStore((s) => s.dismissStage)

  const folderless = isCoding && !!project && project.root_paths.length === 0

  // Tree-shape stability: `children` (the chat transcript) must occupy the
  // SAME slot in the element tree regardless of persona — a profile switch
  // (e.g. code → minimal) toggles the chrome via `false` placeholder slots
  // and NEVER swaps the return type. An early `return <>{children}</>` for
  // non-coding profiles would remount the transcript and destroy the
  // scroll/DOM state the Task 6 contract protects.
  return (
    <div className="flex h-full flex-col" data-testid="workbench-shell">
      {isCoding && <ProjectBar isCoding={isCoding} />}
      <div className="flex flex-1 min-h-0 overflow-hidden">
        {isCoding && !folderless && <ActivityRail profile={profile} />}
        <main className="relative flex flex-1 min-w-0 flex-col overflow-hidden">
          {children}
          {isCoding && stageOpen && activeRail !== 'conversation' && (
            <aside
              data-testid="workbench-stage"
              className={cn(
                'absolute right-0 top-0 z-30 flex h-full w-[420px] max-w-full flex-col',
                'border-l bg-background/98 shadow-xl backdrop-blur-sm',
              )}
            >
              <div className="flex items-center justify-end border-b px-2 py-1">
                <button
                  type="button"
                  onClick={() => dismissStage()}
                  aria-label={t('workbench.stage.close')}
                  className="flex items-center gap-1 text-2xs text-muted-foreground hover:text-foreground"
                >
                  <X className="h-3 w-3" />
                  {t('workbench.stage.close')}
                </button>
              </div>
              <div className="flex-1 min-h-0 overflow-hidden">
                <StagePanel rail={activeRail} />
              </div>
            </aside>
          )}
        </main>
      </div>
      {isCoding && <QuickOpen />}
      {isCoding && <FileEditorModal />}
    </div>
  )
}
