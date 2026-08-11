// AI Image Generation Settings (ported from LobeHub)
// Default image count slider + image model configuration.

import { Image as ImageIcon, Loader2 } from 'lucide-react'
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ModelSelect } from '@/components/engine/model-select'
import { Label } from '@/components/ui/label'
import { Slider } from '@/components/ui/slider'
import { useModels } from '@/hooks/use-engine'
import { cn } from '@/lib/utils'

const MIN_DEFAULT_IMAGE_NUM = 1
const MAX_DEFAULT_IMAGE_NUM = 20

// ── Props ──

interface ImageGenerationSettingsProps {
  /** Default number of images to generate per task (1-20). */
  defaultImageNum: number
  onDefaultImageNumChange: (n: number) => void
  /** Model id for image generation (e.g. "openai/dall-e-3"). */
  imageModel?: string | null
  onImageModelChange?: (modelId: string) => void
  className?: string
}

// ── Component ──

export function ImageGenerationSettings({
  defaultImageNum,
  onDefaultImageNumChange,
  imageModel,
  onImageModelChange,
  className,
}: ImageGenerationSettingsProps) {
  const { t } = useTranslation()
  const [isUpdating, setIsUpdating] = useState(false)
  const { data: models = [] } = useModels(null)
  // Filter to image-capable models (heuristic: name contains dall-e, stable, image, etc.)
  const imageModels = models.filter((m) => {
    const name = m.name.toLowerCase()
    return name.includes('dall-e') || name.includes('image') || name.includes('stable')
  })

  return (
    <div className={cn('space-y-4', className)}>
      <div className="flex items-center gap-2">
        <div className="w-8 h-8 rounded-md bg-muted flex items-center justify-center">
          <ImageIcon className="w-4 h-4 text-muted-foreground" />
        </div>
        <div>
          <h3 className="text-sm font-semibold">{t('settings.imageGeneration.title')}</h3>
          <p className="text-xs text-muted-foreground">
            {t('settings.imageGeneration.description')}
          </p>
        </div>
      </div>

      {/* Default image count */}
      <div className="rounded-lg border bg-card p-4">
        <div className="flex items-center justify-between mb-2">
          <Label htmlFor="default-image-num" className="text-sm font-medium">
            {t('settings.imageGeneration.defaultCount')}
          </Label>
          {isUpdating && <Loader2 className="w-3 h-3 animate-spin text-muted-foreground" />}
        </div>
        <p className="text-xs text-muted-foreground mb-3">
          {t('settings.imageGeneration.countHint', {
            min: MIN_DEFAULT_IMAGE_NUM,
            max: MAX_DEFAULT_IMAGE_NUM,
          })}
        </p>
        <div className="flex items-center gap-4">
          <Slider
            id="default-image-num"
            value={[defaultImageNum]}
            min={MIN_DEFAULT_IMAGE_NUM}
            max={MAX_DEFAULT_IMAGE_NUM}
            step={1}
            onValueChange={(vals) => {
              const n = vals[0] ?? defaultImageNum
              setIsUpdating(true)
              onDefaultImageNumChange(n)
              setTimeout(() => setIsUpdating(false), 300)
            }}
            className="flex-1"
          />
          <span className="text-sm font-medium tabular-nums w-8 text-center">
            {defaultImageNum}
          </span>
        </div>
      </div>

      {/* Image model */}
      {onImageModelChange && (
        <div className="rounded-lg border bg-card p-4">
          <Label className="text-sm font-medium mb-2 block">
            {t('settings.imageGeneration.model')}
          </Label>
          <p className="text-xs text-muted-foreground mb-3">
            {t('settings.imageGeneration.modelHint')}
          </p>
          <ModelSelect
            models={imageModels.length > 0 ? imageModels : models}
            value={imageModel ?? null}
            onValueChange={(id: string) => onImageModelChange(id)}
          />
          {imageModels.length === 0 && (
            <p className="text-xs text-status-warning-on-surface mt-2">
              {t('settings.imageGeneration.noImageModels')}
            </p>
          )}
        </div>
      )}
    </div>
  )
}
