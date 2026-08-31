import { create } from 'zustand'
import { persist } from 'zustand/middleware'

/**
 * Sidebar chrome state only. The active surface is derived from the route
 * (`deriveSurface`), never stored — there is no persisted surface mode.
 */
interface SidebarState {
  collapsed: boolean
  mobileOpen: boolean
  toggle: () => void
  setMobileOpen: (open: boolean) => void
}

export const useSidebarStore = create<SidebarState>()(
  persist(
    (set) => ({
      collapsed: false,
      mobileOpen: false,

      toggle: () =>
        set((s) => {
          localStorage.setItem('oxios-sidebar-collapsed', String(!s.collapsed))
          return { collapsed: !s.collapsed }
        }),

      setMobileOpen: (open) => set({ mobileOpen: open }),
    }),
    {
      name: 'oxios-sidebar',
      partialize: (state) => ({ collapsed: state.collapsed }),
    },
  ),
)
