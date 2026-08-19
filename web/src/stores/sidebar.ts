import { create } from 'zustand'
import { persist } from 'zustand/middleware'

export type SidebarMode = 'console' | 'knowledge' | 'brain' | 'chat'

interface SidebarState {
  collapsed: boolean
  mobileOpen: boolean
  mode: SidebarMode
  toggle: () => void
  setMobileOpen: (open: boolean) => void
  setMode: (mode: SidebarMode) => void
}

export const useSidebarStore = create<SidebarState>()(
  persist(
    (set) => ({
      collapsed: false,
      mobileOpen: false,
      mode: 'console' as SidebarMode,

      toggle: () =>
        set((s) => {
          localStorage.setItem('oxios-sidebar-collapsed', String(!s.collapsed))
          return { collapsed: !s.collapsed }
        }),

      setMobileOpen: (open) => set({ mobileOpen: open }),

      setMode: (mode) => set({ mode }),
    }),
    {
      name: 'oxios-sidebar',
      partialize: (state) => ({ collapsed: state.collapsed }),
    },
  ),
)

/** Derive sidebar mode from pathname. */
export function deriveSidebarMode(pathname: string): SidebarMode {
  if (pathname.startsWith('/brain/knowledge')) return 'knowledge'
  if (pathname.startsWith('/brain')) return 'brain'
  if (pathname === '/chat') return 'chat'
  return 'console'
}
