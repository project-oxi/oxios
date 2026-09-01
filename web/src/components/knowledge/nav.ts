/**
 * Knowledge nav data (design §7.3) — typed source for the shell
 * integration that will wire /knowledge/* into the sidebar / command
 * palette. NOT mounted anywhere on this branch; the shell workstream
 * consumes it. `labelKey` values are single-namespace flat i18n keys;
 * `to` values are literal route paths under /knowledge/*.
 */
export interface KnowledgeNavItem {
  labelKey: string
  to: string
}

export interface KnowledgeNavGroup {
  id: 'memory' | 'library'
  items: KnowledgeNavItem[]
}

export const MEMORY_NAV_GROUPS: KnowledgeNavGroup[] = [
  {
    id: 'memory',
    items: [
      { labelKey: 'knowledge.surface.memory', to: '/knowledge/memory' },
      { labelKey: 'brain.search', to: '/knowledge/memory/search' },
      { labelKey: 'brain.entity', to: '/knowledge/memory/entities' },
      { labelKey: 'brain.contradictions', to: '/knowledge/memory/contradictions' },
      { labelKey: 'knowledge.surface.timeline', to: '/knowledge/memory/timeline' },
    ],
  },
]

export const LIBRARY_NAV_GROUPS: KnowledgeNavGroup[] = [
  {
    id: 'library',
    items: [
      { labelKey: 'knowledge.surface.library', to: '/knowledge/library' },
      { labelKey: 'knowledge.surface.journal', to: '/knowledge/library/journal' },
      { labelKey: 'knowledge.linkGraphTitle', to: '/knowledge/library/graph' },
      { labelKey: 'knowledge.surface.assets', to: '/knowledge/library/assets' },
    ],
  },
]

/** Workspace home (/knowledge) — both groups interleaved by the shell. */
export const KNOWLEDGE_NAV_GROUPS: KnowledgeNavGroup[] = [
  ...MEMORY_NAV_GROUPS,
  ...LIBRARY_NAV_GROUPS,
]
