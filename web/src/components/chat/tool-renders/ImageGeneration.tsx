// ImageGeneration render — shows images produced by the `image_generation` tool.
//
// The tool result is a JSON string: { images:[{url,width?,height?}], prompt,
// provider, model, revised_prompt? }. While running, a spinner is shown; on
// error the raw message is displayed. Each image opens full-size in a new tab.

import { Download, Image as ImageIcon, Loader2 } from 'lucide-react'
import { useState } from 'react'
import type { ToolRenderComponent } from './registry'

interface GeneratedImage {
  url: string
  width?: number
  height?: number
}

interface ImageGenResult {
  images?: GeneratedImage[]
  prompt?: string
  provider?: string
  model?: string
  revised_prompt?: string
}

export const ImageGenerationRender: ToolRenderComponent = ({ args, result, isRunning }) => {
  const prompt = String(args?.prompt ?? '')
  const model = String(args?.model ?? '')
  const n = Number(args?.n ?? 1)

  if (isRunning) {
    return (
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="w-3.5 h-3.5 animate-spin" />
        <span>Generating image{n > 1 ? 's' : ''}…</span>
      </div>
    )
  }

  // Result is either the JSON payload (success) or a plain error string.
  let parsed: ImageGenResult | null = null
  let errorMsg: string | null = null
  if (typeof result === 'string' && result.length > 0) {
    try {
      const obj = JSON.parse(result) as ImageGenResult
      if (obj && Array.isArray(obj.images)) parsed = obj
      else errorMsg = result
    } catch {
      errorMsg = result
    }
  }

  if (errorMsg) {
    return (
      <div className="space-y-1 text-sm">
        <div className="flex items-center gap-2 text-xs">
          <ImageIcon className="w-3.5 h-3.5 text-muted-foreground" />
          <span className="truncate font-medium">{prompt || '(no prompt)'}</span>
          <span className="ml-auto shrink-0 text-status-error-on-surface">failed</span>
        </div>
        <p className="line-clamp-3 text-xs text-status-error-on-surface/80">{errorMsg}</p>
      </div>
    )
  }

  const images = parsed?.images ?? []
  if (images.length === 0) return null

  return (
    <div className="space-y-2 text-sm">
      <div className="flex items-center gap-2 text-xs">
        <ImageIcon className="w-3.5 h-3.5 text-muted-foreground" />
        <span className="truncate font-medium">{prompt || '(no prompt)'}</span>
        <span className="ml-auto shrink-0 text-status-success-on-surface">
          {images.length} image{images.length === 1 ? '' : 's'}
        </span>
      </div>
      {model && <div className="text-xs text-muted-foreground">{model}</div>}
      <div className="grid grid-cols-2 gap-2">
        {images.map((img, i) => (
          <ImagePreview key={`${img.url}-${i}`} url={img.url} />
        ))}
      </div>
    </div>
  )
}

function ImagePreview({ url }: { url: string }) {
  const [loaded, setLoaded] = useState(false)
  return (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      className="group relative block aspect-square overflow-hidden rounded-lg border bg-muted"
    >
      {!loaded && (
        <Loader2 className="absolute inset-0 m-auto h-5 w-5 animate-spin text-muted-foreground" />
      )}
      <img
        src={url}
        alt="generated"
        loading="lazy"
        onLoad={() => setLoaded(true)}
        className="h-full w-full object-cover transition-opacity"
        style={{ opacity: loaded ? 1 : 0 }}
      />
      <span className="absolute bottom-1 right-1 rounded bg-black/60 p-1 text-white opacity-0 transition-opacity group-hover:opacity-100">
        <Download className="h-3 w-3" />
      </span>
    </a>
  )
}
