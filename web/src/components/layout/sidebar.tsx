import { PanelLeft, PanelLeftClose } from 'lucide-react'
import { Separator } from '@/components/ui/separator'
import { cn } from '@/lib/utils'
import { useSidebarStore } from '@/stores/sidebar'
import { SidebarFooter } from './sidebar-footer'
import { SurfaceSidebar } from './surface-sidebar'
import { SurfaceSwitcher } from './surface-switcher'

/**
 * Sidebar — one shared shell for all three surfaces.
 *
 * `SurfaceSwitcher` sits immediately beneath the Oxios identity and is the
 * single desktop surface control; the body is `SurfaceSidebar`, selected
 * strictly from the current route's surface. No legacy mode state exists:
 * the active surface is derived from the pathname (`deriveSurface`).
 */
export function Sidebar() {
  const { collapsed, toggle, mobileOpen } = useSidebarStore()

  return (
    <aside
      className={cn(
        'flex h-full w-72 max-w-[85vw] flex-col overflow-hidden border-r bg-sidebar text-sidebar-foreground transition-[width] duration-300 ease-[var(--animate-in-easing)]',
        collapsed ? 'lg:w-16 lg:max-w-none' : 'lg:w-60 lg:max-w-none',
      )}
    >
      <div
        className={cn(
          'flex h-14 items-center px-3',
          collapsed && !mobileOpen ? 'justify-center' : 'justify-between',
        )}
      >
        {!(collapsed && !mobileOpen) && (
          <div className="flex items-center gap-2">
            <img src="/favicon.png" alt="" className="h-6 w-6 rounded-md shrink-0" />
            <span className="font-bold text-lg">Oxios</span>
          </div>
        )}
        <button
          type="button"
          onClick={toggle}
          className="hidden lg:block rounded-md p-1.5 hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
          aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        >
          {collapsed ? <PanelLeft className="h-4 w-4" /> : <PanelLeftClose className="h-4 w-4" />}
        </button>
      </div>

      <div className="hidden lg:block px-2 pb-2">
        <SurfaceSwitcher collapsed={collapsed} />
      </div>

      <Separator />

      <nav className="flex-1 overflow-y-auto p-2">
        <SurfaceSidebar />
      </nav>

      <Separator />
      <SidebarFooter collapsed={collapsed && !mobileOpen} />
    </aside>
  )
}
