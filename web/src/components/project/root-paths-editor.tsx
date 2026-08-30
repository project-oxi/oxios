import { FolderOpen, Plus, X } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { useFolderPicker } from '@/hooks/use-folder-picker'

interface RootPathsEditorProps {
  value: string[]
  onChange: (next: string[]) => void
  disabled?: boolean
}

/**
 * Repeatable project-root rows backed by the native macOS directory picker
 * (design §4.2). Picked folders append and dedupe; each row is removable.
 * A manual path row is the fallback for remote sessions, where the picker
 * button is disabled with a local-only tooltip (403 mapping).
 */
export function RootPathsEditor({ value, onChange, disabled }: RootPathsEditorProps) {
  const { t } = useTranslation()
  const { pick, picking, localOnly } = useFolderPicker()
  const [manualPath, setManualPath] = useState('')

  const appendPaths = (picked: string[]) => {
    const fresh = picked.filter((p) => !value.includes(p))
    if (fresh.length < picked.length) toast.info(t('projects.duplicateFolder'))
    if (fresh.length === 0) return
    onChange([...value, ...fresh])
  }

  const handlePick = async () => {
    const picked = await pick()
    if (!picked || picked.length === 0) return
    appendPaths(picked)
  }

  const addManual = () => {
    const path = manualPath.trim()
    if (!path) return
    appendPaths([path])
    setManualPath('')
  }

  const pickerButton = (
    <Button
      type="button"
      variant="outline"
      size="sm"
      disabled={disabled || picking || localOnly}
      onClick={handlePick}
    >
      <FolderOpen className="h-4 w-4" />
      {t('projects.chooseFolder')}
    </Button>
  )

  return (
    <div className="space-y-2">
      {value.length === 0 ? (
        <p className="text-xs text-muted-foreground rounded-md border border-dashed p-3 text-center">
          {t('projects.noFoldersYet')}
        </p>
      ) : (
        <div className="space-y-1.5">
          {value.map((path) => (
            <div
              key={path}
              className="flex items-center gap-2 rounded-md border bg-card px-2.5 py-1.5"
            >
              <FolderOpen className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
              <code className="text-xs font-mono truncate flex-1" title={path}>
                {path}
              </code>
              <button
                type="button"
                disabled={disabled}
                onClick={() => onChange(value.filter((p) => p !== path))}
                className="shrink-0 rounded p-1 text-muted-foreground hover:bg-muted hover:text-destructive focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                aria-label={t('projects.removeFolder')}
              >
                <X className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
        </div>
      )}

      <div className="flex items-center gap-2">
        {localOnly ? (
          <TooltipProvider>
            <Tooltip>
              <TooltipTrigger asChild>
                {/* pointer-events-none lets the span receive hover while the
                    disabled button swallows clicks (tooltip on a disabled control). */}
                <span className="pointer-events-none">{pickerButton}</span>
              </TooltipTrigger>
              <TooltipContent side="top">{t('chat.folderPickerLocalOnly')}</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        ) : (
          pickerButton
        )}
        <div className="flex flex-1 items-center gap-1.5 min-w-0">
          <Input
            value={manualPath}
            onChange={(e) => setManualPath(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') {
                e.preventDefault()
                addManual()
              }
            }}
            placeholder="/absolute/path/to/folder"
            disabled={disabled}
            className="h-8 text-xs font-mono"
            aria-label={t('projects.pathInputLabel')}
          />
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-8 shrink-0 px-2"
            disabled={disabled || !manualPath.trim()}
            onClick={addManual}
            aria-label={t('common.add')}
          >
            <Plus className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </div>
  )
}
