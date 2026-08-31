import { useRouter } from '@tanstack/react-router'
import { useEffect } from 'react'
import { SURFACE_TABS } from '@/components/layout/surface-switcher'

/**
 * Registers global ⌃1 / ⌃2 / ⌃3 shortcuts for switching between the three
 * top-level surfaces (Operate / Knowledge / Studio).
 *
 * Uses Control+number (not ⌘+number) because every major browser binds
 * ⌘1-⌘9 to browser-tab switching — a menu-level shortcut that fires
 * before the DOM keydown event and cannot be suppressed by
 * preventDefault(). Control+number has no browser or macOS binding,
 * matching the iTerm2 / Terminal convention for tab switching.
 *
 * Uses e.code ('Digit1' etc.) instead of e.key so it works regardless
 * of keyboard layout or IME state. Uses router.history.push() (same
 * pattern as OxiosCopilotDialog) instead of useNavigate() for reliability
 * in event-handler context.
 *
 * Routes are derived from SURFACE_TABS (surface-switcher.tsx) so the hook,
 * the kbd hints in SurfaceSwitcher, and the sidebar all share one source of
 * truth. If the array is reordered, shortcuts and hints stay in sync.
 */
export function useTabShortcuts(): void {
  const router = useRouter()
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return
      const idx = ['Digit1', 'Digit2', 'Digit3'].indexOf(e.code)
      if (idx === -1 || idx >= SURFACE_TABS.length) return
      const surface = SURFACE_TABS[idx]
      if (!surface) return
      e.preventDefault()
      router.history.push(`/${surface.key}`)
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [router])
}
