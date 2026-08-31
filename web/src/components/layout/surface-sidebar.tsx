import { useRouterState } from '@tanstack/react-router'
import { deriveSurface } from '@/types/surfaces'
import { KnowledgeSidebar } from './knowledge-sidebar'
import { OperateSidebar } from './operate-sidebar'
import { StudioSidebar } from './studio-sidebar'

/**
 * SurfaceSidebar — the sidebar body, selected strictly from the route's
 * surface. Each surface sidebar owns only its local navigation groups;
 * no sidebar renders global quick links from a legacy mode.
 */
export function SurfaceSidebar() {
  const router = useRouterState()
  const surface = deriveSurface(router.location.pathname)

  switch (surface) {
    case 'knowledge':
      return <KnowledgeSidebar />
    case 'studio':
      return <StudioSidebar />
    case 'operate':
      return <OperateSidebar />
  }
}
