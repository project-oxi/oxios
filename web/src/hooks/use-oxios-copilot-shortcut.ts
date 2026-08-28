import { useEffect } from 'react'
import { useOxiosCopilotStore } from '@/stores/oxios-copilot'

/**
 * Registers the global ⌘J shortcut that opens the Oxios Copilot dialog from any
 * route. Mounted once in AppLayout alongside the other global hooks.
 */
export function useOxiosCopilotShortcut(): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'j') {
        e.preventDefault()
        useOxiosCopilotStore.getState().openCopilot()
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])
}
