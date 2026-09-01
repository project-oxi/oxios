// StudioHandoff — typed intent builder for handing a Knowledge source over to
// the Studio surface (design §7.3). WT-3 owns the canonical
// `KnowledgeContextRef` / `StudioLaunchIntent` types; integration will
// switch this file to import from WT-3's canonical location. Until then
// the type lives locally so the workspace can wire it without coupling.

import { useNavigate } from '@tanstack/react-router'
import { Sparkles, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { ProjectSelectorPopover } from '@/components/workbench/ProjectSelectorPopover'
import { cn } from '@/lib/utils'
import { useChatStore } from '@/stores/chat'
import type { Project } from '@/types'

export type KnowledgeContextRef =
  | { kind: 'brain-memory'; id: string }
  | { kind: 'knowledge-document'; path: string }
  | { kind: 'citation'; turnId: string; citationId: string }
  | { kind: 'artifact'; id: string }

export interface StudioLaunchIntent {
  source: 'knowledge'
  projectId?: string
  sessionId?: string
  contextRefs: KnowledgeContextRef[]
  initialPrompt?: string
}

interface StudioHandoffSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Refs the inspector currently carries. Unavailable refs appear as
   *  removable chips; available refs flow into the intent unchanged. */
  contextRefs: KnowledgeContextRef[]
  /** Resolver deciding which refs are "available" — returns true when the
   *  backing record resolves against the caller's current queries.
   *  Unresolvable refs render as unavailable chips and are excluded from
   *  the intent when removed. */
  isAvailable: (ref: KnowledgeContextRef) => boolean
  /** Element to restore focus to on close (knowledge-workspace row). */
  triggerRef?: React.RefObject<HTMLElement | null>
  /** Optional initial prompt carried in the intent (the user's note). */
  initialPrompt?: string
}

type HandoffTarget = 'projectless' | 'currentSession' | 'project'

/** Studio handoff sheet — exactly the three destinations from design §7.3. */
export function StudioHandoffSheet({
  open,
  onOpenChange,
  contextRefs,
  isAvailable,
  initialPrompt,
  triggerRef,
}: StudioHandoffSheetProps) {
  const { t } = useTranslation()
  const navigate = useNavigate()
  const activeSessionId = useChatStore((s) => s.activeSessionId)
  const activeProjectId = useChatStore((s) => s.activeProjectId)

  // Track which refs the user has explicitly removed (in addition to the
  // unavailable set). Removing either an unavailable or user-dismissed
  // chip takes it out of the intent payload.
  const [dismissed, setDismissed] = useState<Set<number>>(new Set())
  const [target, setTarget] = useState<HandoffTarget>('projectless')
  const [chosenProject, setChosenProject] = useState<Project | null>(null)

  // Reset transient state every time the sheet re-opens.
  const handleOpenChange = (next: boolean) => {
    if (!next) {
      setDismissed(new Set())
      setTarget('projectless')
      setChosenProject(null)
    }
    onOpenChange(next)
    if (!next) {
      // Return focus to the originating row after the dialog's own focus
      // cleanup runs (setTimeout 0 lands after radix's unmount effects).
      setTimeout(() => {
        triggerRef?.current?.focus()
      }, 0)
    }
  }

  const availableRefs = useMemo(() => {
    return contextRefs.filter((ref, i) => {
      if (dismissed.has(i)) return false
      return isAvailable(ref)
    })
  }, [contextRefs, dismissed, isAvailable])

  const unavailableIndexes = useMemo(() => {
    const out: number[] = []
    contextRefs.forEach((ref, i) => {
      if (dismissed.has(i)) return
      if (!isAvailable(ref)) out.push(i)
    })
    return out
  }, [contextRefs, dismissed, isAvailable])

  const handleSubmit = () => {
    const intent: StudioLaunchIntent = {
      source: 'knowledge',
      contextRefs: availableRefs,
      ...(initialPrompt ? { initialPrompt } : {}),
    }
    if (target === 'project' && chosenProject) {
      intent.projectId = chosenProject.id
    } else if (target === 'currentSession' && activeSessionId) {
      intent.sessionId = activeSessionId
    }
    // Encode the intent into ?launch=<URI-encoded JSON>. Studio route does
    // not exist on this branch (WT-3 owns it) — the navigation lands on a
    // plain 404, which is the agreed interim behavior.
    const encoded = encodeURIComponent(JSON.stringify(intent))
    void navigate({ to: '/studio', search: { launch: encoded } } as never)
    handleOpenChange(false)
  }

  const canSubmit =
    availableRefs.length > 0 &&
    (target !== 'project' || chosenProject !== null) &&
    (target !== 'currentSession' || activeSessionId !== null)

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-lg" mobileSheet aria-describedby="studio-handoff-desc">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Sparkles className="h-5 w-5" />
            {t('knowledge.workspace.handoff.title')}
          </DialogTitle>
          <DialogDescription id="studio-handoff-desc">
            {t('knowledge.workspace.handoff.subtitle')}
          </DialogDescription>
        </DialogHeader>

        {/* Ref chips (available + unavailable) */}
        <div className="space-y-2">
          <p className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
            {t('knowledge.workspace.contextList')}
          </p>
          {availableRefs.length === 0 && unavailableIndexes.length === 0 && (
            <p className="text-sm text-muted-foreground">{t('knowledge.workspace.noSources')}</p>
          )}
          <ul className="flex flex-wrap gap-2">
            {availableRefs.map((ref, i) => (
              <RefChip key={`a-${i}`} ref={ref} kind="available" />
            ))}
            {unavailableIndexes.map((i) => (
              <li key={`u-${i}`}>
                <button
                  type="button"
                  onClick={() =>
                    setDismissed((prev) => {
                      const next = new Set(prev)
                      next.add(i)
                      return next
                    })
                  }
                  className={cn(
                    'inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs',
                    'border-status-warning-subtle-border bg-status-warning-subtle text-status-warning-on-subtle',
                    'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                  )}
                  aria-label={`${t('knowledge.workspace.unavailableChip')} — ${t(
                    'knowledge.workspace.removeUnavailable',
                  )}`}
                >
                  <X className="h-3 w-3" aria-hidden="true" />
                  <span>{t('knowledge.workspace.unavailableChip')}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>

        {/* Target picker (exactly three destinations) */}
        <fieldset className="space-y-2">
          <legend className="text-xs font-medium text-muted-foreground uppercase tracking-wide">
            {t('knowledge.workspace.handoff.title')}
          </legend>
          <TargetOption
            value="projectless"
            selected={target === 'projectless'}
            onSelect={() => setTarget('projectless')}
            label={t('knowledge.workspace.handoff.projectless')}
            description={t('knowledge.workspace.handoff.projectlessHint')}
            testId="handoff-target-projectless"
          />
          <TargetOption
            value="currentSession"
            selected={target === 'currentSession'}
            onSelect={() => setTarget('currentSession')}
            label={t('knowledge.workspace.handoff.currentSession')}
            description={
              activeSessionId
                ? t('knowledge.workspace.handoff.currentSessionHint')
                : t('brain.selectSpace')
            }
            disabled={!activeSessionId}
            testId="handoff-target-current-session"
          />
          <div
            className={cn(
              'rounded-lg border p-3 transition-colors',
              target === 'project'
                ? 'border-primary bg-primary/5'
                : 'border-border hover:bg-accent/30',
            )}
          >
            <label className="flex items-start gap-2 cursor-pointer">
              <input
                type="radio"
                name="handoff-target"
                value="project"
                checked={target === 'project'}
                onChange={() => setTarget('project')}
                className="mt-1"
                data-testid="handoff-target-project"
              />
              <span className="flex-1">
                <span className="block text-sm font-medium">
                  {t('knowledge.workspace.handoff.chooseProject')}
                </span>
                <span className="block text-xs text-muted-foreground">
                  {t('knowledge.workspace.handoff.chooseProjectHint')}
                </span>
              </span>
            </label>
            {target === 'project' && (
              <div className="mt-2 pl-6">
                <ProjectSelectorPopover
                  activeProject={chosenProject}
                  onSelect={(project) => setChosenProject(project)}
                />
                {!activeProjectId && (
                  <p className="mt-2 text-2xs text-muted-foreground">
                    {t('knowledge.workspace.handoff.noProject')}
                  </p>
                )}
              </div>
            )}
          </div>
        </fieldset>

        <DialogFooter>
          <Button variant="outline" onClick={() => handleOpenChange(false)}>
            {t('knowledge.workspace.handoff.cancel')}
          </Button>
          <Button onClick={handleSubmit} disabled={!canSubmit} data-testid="handoff-submit">
            {t('knowledge.workspace.handoff.submit')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function TargetOption({
  value,
  selected,
  onSelect,
  label,
  description,
  disabled,
  testId,
}: {
  value: HandoffTarget
  selected: boolean
  onSelect: () => void
  label: string
  description: string
  disabled?: boolean
  testId?: string
}) {
  return (
    <label
      className={cn(
        'flex items-start gap-2 rounded-lg border p-3 transition-colors',
        selected ? 'border-primary bg-primary/5' : 'border-border hover:bg-accent/30',
        disabled && 'opacity-50 pointer-events-none',
      )}
    >
      <input
        type="radio"
        name="handoff-target"
        value={value}
        checked={selected}
        onChange={onSelect}
        disabled={disabled}
        className="mt-1"
        data-testid={testId}
      />
      <span className="flex-1">
        <span className="block text-sm font-medium">{label}</span>
        <span className="block text-xs text-muted-foreground">{description}</span>
      </span>
    </label>
  )
}

function RefChip({
  ref: r,
  kind,
}: {
  ref: KnowledgeContextRef
  kind: 'available' | 'unavailable'
}) {
  const label = chipLabel(r)
  return (
    <li>
      <span
        className={cn(
          'inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs',
          kind === 'available'
            ? 'border-status-info-subtle-border bg-status-info-subtle text-status-info-on-subtle'
            : 'border-status-warning-subtle-border bg-status-warning-subtle text-status-warning-on-subtle',
        )}
      >
        <span className="font-medium">{label.kind}</span>
        <span className="font-mono text-[10px]">{label.value}</span>
      </span>
    </li>
  )
}

function chipLabel(r: KnowledgeContextRef): { kind: string; value: string } {
  switch (r.kind) {
    case 'brain-memory':
      return { kind: 'brain', value: r.id }
    case 'knowledge-document':
      return { kind: 'library', value: r.path }
    case 'citation':
      return { kind: 'citation', value: `${r.turnId}/${r.citationId}` }
    case 'artifact':
      return { kind: 'artifact', value: r.id }
  }
  return { kind: 'source', value: '' }
}
