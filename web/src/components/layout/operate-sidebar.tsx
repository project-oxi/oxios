import { useRouterState } from '@tanstack/react-router'
import { LayoutDashboard } from 'lucide-react'
import { useSidebarStore } from '@/stores/sidebar'
import { NavGroup, type NavItem } from './sidebar-primitives'

// ── Operate sidebar — local navigation groups (design §6.2) ────
//
// Groups: Now, Projects, Runs, System. Items appear as their canonical
// routes land; empty groups render nothing, so the wave-2 Operate worker
// populates Projects/Runs/System by appending NavItems here.

const GROUPS: { labelKey: string; items: NavItem[] }[] = [
  {
    labelKey: 'operate.groupNow',
    items: [
      {
        labelKey: 'operate.attention',
        href: '/operate',
        icon: <LayoutDashboard className="h-4 w-4" />,
      },
    ],
  },
  { labelKey: 'operate.groupProjects', items: [] },
  { labelKey: 'operate.groupRuns', items: [] },
  { labelKey: 'operate.groupSystem', items: [] },
]

export function OperateSidebar() {
  const router = useRouterState()
  const currentPath = router.location.pathname
  const { collapsed } = useSidebarStore()

  return (
    <>
      {GROUPS.map((group) => (
        <NavGroup
          key={group.labelKey}
          labelKey={group.labelKey}
          items={group.items}
          currentPath={currentPath}
          collapsed={collapsed}
        />
      ))}
    </>
  )
}
