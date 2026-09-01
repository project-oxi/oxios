import { useRouterState } from '@tanstack/react-router'
import { BookOpen } from 'lucide-react'
import { useSidebarStore } from '@/stores/sidebar'
import { NavGroup, type NavItem } from './sidebar-primitives'

// ── Knowledge sidebar — local navigation groups (design §7.2) ──
//
// Groups: Memory / Brain and Library / Knowledge. The two source domains
// stay distinct. Overview targets the Knowledge home; Memory sub-views and
// Library views land with the wave-2 Knowledge worker, which appends
// NavItems to the groups below.

const GROUPS: { labelKey: string; items: NavItem[] }[] = [
  {
    labelKey: 'knowledge.groupMemory',
    items: [
      {
        labelKey: 'knowledge.overview',
        href: '/knowledge',
        icon: <BookOpen className="h-4 w-4" />,
      },
    ],
  },
  { labelKey: 'knowledge.groupLibrary', items: [] },
]

export function KnowledgeSidebar() {
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
