import { useEffect } from 'react'
import { useBrainSpaces } from '@/hooks/use-brain'
import { useConfig } from '@/hooks/use-config'
import { useProjects } from '@/hooks/use-projects'
import { useChatStore } from '@/stores/chat'

/**
 * Chat-page sync for default-brain inheritance (Task 9):
 *
 * - Seeds the store's resolution inputs (`brainDefaults`) from the active
 *   project's `default_brain_space` and the global config
 *   `brain.default_space`; sendMessage resolves the effective binding from
 *   these plus the explicit picker choice.
 * - Normalizes a stale rehydrated binding: a persisted space that is no
 *   longer on the roster (renamed/deleted in oxibrain) is cleared instead of
 *   poisoning every subsequent turn.
 *
 * Mounted once by the chat page; every input comes from the shared
 * react-query cache, so no extra network traffic.
 */
export function useBrainBindingSync() {
  const activeProjectId = useChatStore((s) => s.activeProjectId)
  const activeBrainSpace = useChatStore((s) => s.activeBrainSpace)
  const setActiveBrainSpace = useChatStore((s) => s.setActiveBrainSpace)
  const setBrainDefaults = useChatStore((s) => s.setBrainDefaults)

  const { data: config } = useConfig()
  const { data: projectsData } = useProjects()
  const spacesQuery = useBrainSpaces()
  const spaces = Array.isArray(spacesQuery.data) ? spacesQuery.data : []

  useEffect(() => {
    const brain = (config?.brain ?? {}) as Record<string, unknown>
    const globalDefault = typeof brain.default_space === 'string' ? brain.default_space : ''
    const project = projectsData?.items?.find((p) => p.id === activeProjectId) ?? null
    setBrainDefaults({
      project: project?.default_brain_space ?? null,
      global: globalDefault,
    })
  }, [activeProjectId, config, projectsData, setBrainDefaults])

  useEffect(() => {
    // Guard: only normalize against a roster we actually received — an empty
    // roster (brain degraded / unconfigured) must not wipe the persisted
    // binding.
    if (spaces.length > 0 && activeBrainSpace && !spaces.some((s) => s.name === activeBrainSpace)) {
      setActiveBrainSpace(null)
    }
  }, [activeBrainSpace, setActiveBrainSpace, spaces])
}
