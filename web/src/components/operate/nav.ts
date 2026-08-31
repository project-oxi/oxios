/**
 * Operate nav data (design §6) — typed source for the shell integration
 * that will wire /operate/* into the sidebar / command palette. NOT
 * mounted anywhere on this branch; the shell workstream consumes it.
 * `labelKey` values are single-namespace flat i18n keys; `to` values are
 * literal route paths under /operate/*.
 */
export interface OperateNavItem {
  labelKey: string
  to: string
}

export interface OperateNavGroup {
  id: 'now' | 'projects' | 'runs' | 'system'
  items: OperateNavItem[]
}

export const OPERATE_NAV_GROUPS: OperateNavGroup[] = [
  {
    id: 'now',
    items: [
      { labelKey: 'operate.attention', to: '/operate' },
      { labelKey: 'operate.recentActivity', to: '/operate/events' },
    ],
  },
  {
    id: 'projects',
    items: [{ labelKey: 'operate.allProjects', to: '/operate/projects' }],
  },
  {
    id: 'runs',
    items: [
      { labelKey: 'operate.runCenter', to: '/operate/runs' },
      { labelKey: 'operate.events', to: '/operate/events' },
    ],
  },
  {
    id: 'system',
    items: [
      { labelKey: 'operate.capabilities', to: '/operate/capabilities' },
      { labelKey: 'operate.connections', to: '/operate/system/connections' },
      { labelKey: 'operate.skillsAndMcp', to: '/operate/system/skills' },
      { labelKey: 'operate.mcp', to: '/operate/system/mcp' },
      { labelKey: 'common.security', to: '/operate/system/security' },
      { labelKey: 'operate.resourcesAndCost', to: '/operate/system/resources' },
      { labelKey: 'common.settings', to: '/operate/system/settings' },
    ],
  },
]
