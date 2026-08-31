// StudioContextBar — the ONE persistent selector for Studio session
// defaults (IA design §8.3): project · persona · model · Brain.
//
// Every control reads and writes the canonical chat-store bindings the
// send path already consumes (`project_id`, `persona_id`, `model`,
// `brain_space`). No second Studio-only copy of that state exists. Each
// change applies from the NEXT turn — the bar says so explicitly — and a
// project attach never resets the live conversation (bindProject).
//
// The composer does not repeat these selectors (chat-input variant
// 'studio'); this bar is their only home.

import { Unplug } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import { BrainPickerContainer } from '@/components/chat/brain-picker'
import { ModelPickerContainer } from '@/components/chat/model-picker'
import { PersonaPickerContainer } from '@/components/chat/persona-picker'
import { ProjectSelectorPopover } from '@/components/workbench/ProjectSelectorPopover'
import { useBrainSpaces } from '@/hooks/use-brain'
import { useRoles } from '@/hooks/use-engine'
import { useProjects } from '@/hooks/use-projects'
import { useChatStore } from '@/stores/chat'

export function StudioContextBar() {
  const { t } = useTranslation()
  const activeProjectId = useChatStore((s) => s.activeProjectId)
  const activeModelId = useChatStore((s) => s.activeModelId)
  const setActiveModelId = useChatStore((s) => s.setActiveModelId)

  const { data: projectsData } = useProjects()
  const activeProject = (projectsData?.items ?? []).find((p) => p.id === activeProjectId) ?? null

  const { data: rolesData } = useRoles()
  const roles = Object.entries(rolesData?.roles ?? {}).map(([name, model]) => ({
    name,
    model,
  }))

  // Unbound Brain stays a neutral `Not connected` chip (IA design §8.3) —
  // never an error and never an implicit fallback space. When the brain
  // has no spaces at all the picker hides itself, so the bar renders the
  // neutral chip in its place.
  const { data: spaces } = useBrainSpaces()
  const brainAvailable = Array.isArray(spaces) && spaces.length > 0

  return (
    <div
      data-testid="studio-context-bar"
      className="flex h-10 shrink-0 items-center gap-1 border-b bg-background/95 px-3"
    >
      <ProjectSelectorPopover activeProject={activeProject} onSelect={() => {}} />
      <PersonaPickerContainer />
      <ModelPickerContainer
        activeModelId={activeModelId}
        setActiveModelId={setActiveModelId}
        roles={roles}
        activeRole={null}
        setActiveRole={() => {}}
      />
      {brainAvailable ? (
        <BrainPickerContainer />
      ) : (
        <span
          data-testid="studio-context-brain-unbound"
          className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground"
          title={t('studio.context.brainNotConnected')}
        >
          <Unplug className="h-3.5 w-3.5" />
          {t('studio.context.brainNotConnected')}
        </span>
      )}
      <span className="ml-auto truncate pl-2 text-2xs text-muted-foreground/80">
        {t('studio.context.nextTurnHint')}
      </span>
    </div>
  )
}
