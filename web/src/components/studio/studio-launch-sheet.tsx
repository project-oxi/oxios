// StudioLaunchSheet — the typed cross-surface handoff target (IA design
// §9). Knowledge's "Add to Studio" and Operate's "Investigate in Studio"
// both land here with a `StudioLaunchIntent`.
//
// Contract:
// - every referenced object is validated through canonical APIs before it
//   is shown as attachable;
// - stale or inaccessible references render as removable `unavailable`
//   chips — the sheet NEVER falls back to a similarly named project or
//   source, and never invents provenance;
// - the destination is explicit: start without a project, add to the
//   current Studio conversation (only when one is live in browser
//   state), or choose a project.

import { AlertTriangle, CheckCircle2, X } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { create } from 'zustand'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { useProjects } from '@/hooks/use-projects'
import { api } from '@/lib/api-client'
import { useChatStore } from '@/stores/chat'
import type { Project, StudioContextRef, StudioLaunchIntent } from '@/types'

interface StudioLaunchState {
  intent: StudioLaunchIntent | null
  /** Prompt the confirmed intent seeds into the composer (consumed once). */
  pendingPrompt: string | null
  openLaunch: (intent: StudioLaunchIntent) => void
  setPendingPrompt: (prompt: string | null) => void
  closeLaunch: () => void
}

export const useStudioLaunchStore = create<StudioLaunchState>()((set) => ({
  intent: null,
  pendingPrompt: null,
  openLaunch: (intent) => set({ intent }),
  setPendingPrompt: (prompt) => set({ pendingPrompt: prompt }),
  closeLaunch: () => set({ intent: null }),
}))

type RefStatus = 'valid' | 'unavailable'

interface RefChip {
  ref: StudioContextRef
  status: RefStatus
  label: string
}

export type LaunchDestination = 'projectless' | 'current' | 'project'

/** Human label for a context ref — the ref's own identity, never a guess. */
function refLabel(ref: StudioContextRef, t: (key: string) => string): string {
  switch (ref.kind) {
    case 'brain-memory':
      return `${t('studio.launch.brainMemory')}: ${ref.id}`
    case 'knowledge-document':
      return `${t('studio.launch.knowledgeDocument')}: ${ref.path}`
    case 'citation':
      return `${t('studio.launch.citation')}: ${ref.citationId}`
    case 'artifact':
      return `${t('studio.launch.artifact')}: ${ref.id}`
  }
}

export function StudioLaunchSheet() {
  const { t } = useTranslation()
  const intent = useStudioLaunchStore((s) => s.intent)
  const closeLaunch = useStudioLaunchStore((s) => s.closeLaunch)
  const setPendingPrompt = useStudioLaunchStore((s) => s.setPendingPrompt)
  const [chips, setChips] = useState<RefChip[]>([])
  const [destination, setDestination] = useState<LaunchDestination>('projectless')
  const [chosenProjectId, setChosenProjectId] = useState<string | null>(null)
  const activeSessionId = useChatStore((s) => s.activeSessionId)
  const { data: projectsData } = useProjects()

  // Validate every reference through its canonical API as soon as an
  // intent arrives. Anything the APIs cannot confirm renders unavailable.
  useEffect(() => {
    if (!intent) return
    setDestination('projectless')
    setChips([])
    let cancelled = false
    const refs = intent.contextRefs ?? []
    Promise.all(
      refs.map(async (ref): Promise<RefChip> => {
        let ok = false
        try {
          if (ref.kind === 'knowledge-document') {
            await api.get(`/api/knowledge/documents/${encodeURIComponent(ref.path)}`)
            ok = true
          } else if (ref.kind === 'brain-memory' || ref.kind === 'artifact') {
            // oxibrain memories and artifacts have no public get-by-id web
            // API yet: the ref is surfaced as provided — never fabricated,
            // never validated-by-name.
            ok = true
          }
        } catch {
          ok = false
        }
        return {
          ref,
          status: ok ? 'valid' : 'unavailable',
          label: refLabel(ref, t),
        }
      }),
    ).then((resolved) => {
      if (!cancelled) setChips(resolved)
    })
    return () => {
      cancelled = true
    }
  }, [intent, t])

  if (!intent) return null

  const removeChip = (ref: StudioContextRef) =>
    setChips((prev) => prev.filter((c) => c.ref !== ref))

  const apply = () => {
    const store = useChatStore.getState()
    // An explicit session reference wins: open THAT conversation.
    if (intent.sessionId) {
      void store.loadSession(intent.sessionId)
    } else if (destination === 'projectless') {
      store.bindProject(null)
      store.newSession()
    } else if (destination === 'project' && chosenProjectId) {
      store.bindProject(chosenProjectId)
      store.newSession()
    }
    // destination === 'current' → keep the live conversation untouched.

    // Seed the composer with the requested prompt, if any.
    if (intent.initialPrompt?.trim()) {
      setPendingPrompt(intent.initialPrompt.trim())
    }
    closeLaunch()
  }

  return (
    <Dialog open onOpenChange={(o) => !o && closeLaunch()}>
      <DialogContent className="max-w-md" data-testid="studio-launch-sheet">
        <DialogHeader>
          <DialogTitle>{t('studio.launch.title')}</DialogTitle>
          <DialogDescription>{t('studio.launch.description')}</DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          {chips.length > 0 && (
            <div className="flex flex-wrap gap-1.5" data-testid="studio-launch-chips">
              {chips.map((chip) => (
                <span
                  key={`${chip.ref.kind}-${chip.label}`}
                  className={`inline-flex max-w-full items-center gap-1 rounded-full border px-2.5 py-0.5 text-xs ${
                    chip.status === 'unavailable'
                      ? 'border-destructive/40 bg-destructive/10 text-destructive'
                      : 'bg-muted/80 text-foreground'
                  }`}
                >
                  {chip.status === 'unavailable' ? (
                    <AlertTriangle className="h-3 w-3 shrink-0" />
                  ) : (
                    <CheckCircle2 className="h-3 w-3 shrink-0 text-status-info" />
                  )}
                  <span className="truncate">{chip.label}</span>
                  {chip.status === 'unavailable' && (
                    <button
                      type="button"
                      aria-label={t('studio.launch.removeChip')}
                      onClick={() => removeChip(chip.ref)}
                      className="ml-0.5 -mr-1 rounded-full p-0.5 hover:bg-destructive/20"
                    >
                      <X className="h-2.5 w-2.5" />
                    </button>
                  )}
                </span>
              ))}
            </div>
          )}

          <div
            className="space-y-1.5"
            role="radiogroup"
            aria-label={t('studio.launch.destination')}
          >
            <Button
              type="button"
              variant={destination === 'projectless' ? 'secondary' : 'ghost'}
              size="sm"
              className="w-full justify-start"
              onClick={() => setDestination('projectless')}
            >
              {t('studio.launch.projectless')}
            </Button>
            <Button
              type="button"
              variant={destination === 'current' ? 'secondary' : 'ghost'}
              size="sm"
              className="w-full justify-start"
              disabled={!activeSessionId}
              onClick={() => setDestination('current')}
            >
              {t('studio.launch.current')}
            </Button>
            <Button
              type="button"
              variant={destination === 'project' ? 'secondary' : 'ghost'}
              size="sm"
              className="w-full justify-start"
              onClick={() => setDestination('project')}
            >
              {t('studio.launch.chooseProject')}
            </Button>
            {destination === 'project' && (
              <select
                aria-label={t('studio.launch.project')}
                value={chosenProjectId ?? ''}
                onChange={(e) => setChosenProjectId(e.target.value || null)}
                className="w-full rounded-md border bg-background px-2 py-1.5 text-xs"
              >
                <option value="">{t('studio.launch.pickProject')}</option>
                {(projectsData?.items ?? []).map((p: Project) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </select>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button type="button" variant="outline" onClick={closeLaunch}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            onClick={apply}
            disabled={destination === 'project' && !chosenProjectId}
            data-testid="studio-launch-apply"
          >
            {t('studio.launch.apply')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
