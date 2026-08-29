import { useCallback, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { ApiError, api } from '@/lib/api-client'

interface PickFoldersResponse {
  paths: string[]
  cancelled?: boolean
}

/**
 * Native macOS directory picker over `POST /api/system/pick-folders`.
 *
 * Contract (design §4.4 / §9):
 * - `200 { paths: [...] }`          → resolved paths
 * - `200 { paths: [], cancelled }`  → user cancelled: `null` silently
 * - `403` (remote client)           → localized local-only toast, `null`
 * - `501` (non-macOS)               → localized unavailable toast, `null`
 * - anything else                   → localized failure toast, `null`
 *
 * After a 403 the hook flips `localOnly` so callers can disable the picker
 * button with an explanatory tooltip instead of letting the user retry into
 * the same rejection.
 */
export function useFolderPicker(): {
  pick: () => Promise<string[] | null>
  picking: boolean
  error: string | null
  localOnly: boolean
} {
  const { t } = useTranslation()
  const [picking, setPicking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [localOnly, setLocalOnly] = useState(false)

  const pick = useCallback(async (): Promise<string[] | null> => {
    setPicking(true)
    setError(null)
    try {
      const res = await api.post<PickFoldersResponse>('/api/system/pick-folders', {})
      if (res.cancelled || res.paths.length === 0) return null
      return res.paths
    } catch (err) {
      if (err instanceof ApiError && err.status === 403) {
        setLocalOnly(true)
        const msg = t('projects.folderPickerLocalOnly')
        setError(msg)
        toast.error(msg)
      } else if (err instanceof ApiError && err.status === 501) {
        const msg = t('projects.folderPickerUnavailable')
        setError(msg)
        toast.error(msg)
      } else {
        const msg = t('projects.folderPickerFailed')
        setError(msg)
        toast.error(msg)
      }
      return null
    } finally {
      setPicking(false)
    }
  }, [t])

  return { pick, picking, error, localOnly }
}
