// HTML renderer — sandboxed iframe via srcdoc.
//
// SECURITY: sandbox="allow-scripts" WITHOUT allow-same-origin. The iframe runs
// in an opaque origin, so its scripts cannot touch the parent window, cookies,
// localStorage, or (critically for Oxios) the local API auth token held in the
// parent's memory. allow-popups lets <a target> still work. We deliberately do
// NOT add allow-same-origin or allow-top-navigation for model output.

import { memo, useMemo } from 'react'

interface HtmlRendererProps {
  content: string
}

/** Wrap a bare HTML fragment in a full document if needed. */
function ensureHtmlDoc(html: string): string {
  if (/<html[\s>]/i.test(html)) return html
  return `<!DOCTYPE html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <style>
      html, body { margin: 0; padding: 0; font-family: system-ui, sans-serif; }
      body { padding: 16px; }
    </style>
  </head>
  <body>${html}</body>
</html>`
}

export const HtmlRenderer = memo(function HtmlRenderer({ content }: HtmlRendererProps) {
  const srcDoc = useMemo(() => ensureHtmlDoc(content), [content])
  return (
    <iframe
      title="artifact-html-preview"
      sandbox="allow-scripts allow-popups"
      srcDoc={srcDoc}
      className="h-full w-full border-0 bg-background"
    />
  )
})
