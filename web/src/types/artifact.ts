// Artifact type definitions — LobeHub analogue adapted to Oxios (shadcn/ui).
//
// An "artifact" is a renderable code block the model produces (HTML, SVG,
// Mermaid, React) that gets an interactive preview panel instead of a plain
// code listing. Detection has two paths, both converging on the same render
// path (the `code` component in markdown-message.tsx):
//
//   1. Language auto-detect — a fenced ```html / ```svg / ```mermaid / ```jsx
//      block. Immune to rehype-sanitize (code is already text inside <code>).
//      Works with any model, zero prompt changes.
//   2. Tag protocol — the model emits <lobeArtifact type="..." title="...">
//      CODE </lobeArtifact>. preprocessArtifacts() (markdown-plugins/
//      preprocess-artifacts.ts) rewrites the tag into a fenced block at the
//      STRING level, before markdown parsing — so the inner code is captured
//      as a substring and never parsed as HTML (sanitize-safe).

/** Renderable artifact categories. */
export enum ArtifactType {
  Html = 'html',
  Svg = 'svg',
  Mermaid = 'mermaid',
  React = 'react',
}

/** How an artifact was detected. */
export type ArtifactSource = 'language' | 'tag'

/** Metadata identifying an artifact instance within a message. */
export interface ArtifactMeta {
  /** Owning chat message id (for streaming/live-content coordination). */
  messageId: string
  /** Render category. */
  type: ArtifactType
  /** Human title (from tag attr or a leading comment line). */
  title?: string
  /** Raw code-fence language token (html, svg, mermaid, jsx, tsx, ...). */
  language?: string
  /** Detection path. */
  source: ArtifactSource
  /** Zero-based index of this artifact within its owning message. Assigned
   *  by `ArtifactCard` from a per-message counter in `ArtifactContext`, so
   *  two untitled artifacts of the same type in one message get distinct
   *  identity keys (and therefore distinct panel entries). */
  ordinal: number
}

/** Panel display mode — code source vs live preview. */
export type ArtifactDisplayMode = 'code' | 'preview'

/** The currently-open artifact in the side panel. */
export interface ActiveArtifact extends ArtifactMeta {
  /** Live code content (updated during streaming by the active card). */
  content: string
  displayMode: ArtifactDisplayMode
}

// ── Language → type mapping ──────────────────────────────────────────────

const LANG_MAP: Record<string, ArtifactType> = {
  htm: ArtifactType.Html,
  html: ArtifactType.Html,
  jsx: ArtifactType.React,
  mermaid: ArtifactType.Mermaid,
  react: ArtifactType.React,
  svg: ArtifactType.Svg,
  tsx: ArtifactType.React,
  xml: ArtifactType.Svg,
}

/**
 * Resolve a fenced-code language token to a renderable artifact type.
 * Returns `undefined` for non-renderable languages (bash, rust, json, ...),
 * in which case the block stays a normal code listing.
 */
export function languageToArtifactType(lang: string | undefined): ArtifactType | undefined {
  if (!lang) return undefined
  return LANG_MAP[lang.toLowerCase()]
}

/**
 * Map an artifact type back to a code-fence language token for the tag-protocol
 * extraction (so the synthesised block flows through the same render path as
 * language-detected blocks).
 */
export function artifactTypeToLanguage(type: ArtifactType): string {
  switch (type) {
    case ArtifactType.Html:
      return 'html'
    case ArtifactType.Svg:
      return 'svg'
    case ArtifactType.Mermaid:
      return 'mermaid'
    case ArtifactType.React:
      return 'tsx'
  }
}

/**
 * Resolve a tag-protocol `type` attribute (MIME-ish, LobeHub-compatible) to a
 * render category. Unknown / code types resolve to `undefined` (rendered as a
 * plain non-preview block).
 *
 * Examples: "image/svg+xml" → svg, "text/markdown" → (none, keep as code),
 * "application/lobe.artifacts.react" → react, "html" → html.
 */
export function tagTypeToArtifactType(tagType: string | undefined): ArtifactType | undefined {
  if (!tagType) return undefined
  const t = tagType.toLowerCase()
  if (t.includes('svg')) return ArtifactType.Svg
  if (t.includes('react')) return ArtifactType.React
  if (t.includes('mermaid')) return ArtifactType.Mermaid
  if (t === 'html' || t.includes('text/html')) return ArtifactType.Html
  return undefined
}

// ── Title extraction ──────────────────────────────────────────────────────

/** Result of splitting a leading title directive from artifact code. */
export interface ParsedArtifactCode {
  title?: string
  /** Code with any title directive line stripped (for preview rendering). */
  content: string
  /** Full original code including the directive (for the code view). */
  raw: string
}

/**
 * Extract an optional title from the first line of artifact code and return
 * the content with that line stripped (for preview rendering).
 *
 * Recognised directives (so the directive never leaks into the rendered
 * preview): a leading `# Title`, `// Title`, or `<!-- Title -->`. Used by the
 * tag-protocol path where the synthesised block prepends a title line.
 */
export function parseArtifactCode(code: string): ParsedArtifactCode {
  const firstLine = code.split('\n')[0] ?? ''

  const hashTitle = firstLine.match(/^#\s+(.+?)\s*$/)?.[1]
  if (hashTitle) return splitTitle(code, hashTitle)

  const slashTitle = firstLine.match(/^\/\/\s+(.+?)\s*$/)?.[1]
  if (slashTitle) return splitTitle(code, slashTitle)

  const commentTitle = firstLine.match(/^<!--\s*(.+?)\s*-->\s*$/)?.[1]
  if (commentTitle) return splitTitle(code, commentTitle)

  return { content: code, raw: code }
}

function splitTitle(code: string, title: string): ParsedArtifactCode {
  const nl = code.indexOf('\n')
  const content = nl === -1 ? '' : code.slice(nl + 1)
  return { title, content, raw: code }
}
