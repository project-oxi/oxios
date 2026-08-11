// InlineDiffViewer — minimal line-by-line diff renderer for tool call results.
//
// RFC-044 Phase 3: when a persona exposes the `diff-viewer` capability,
// tool call results that include file edits (tool_name matches edit/write/
// patch, or args carry old_text / new_text) render an inline diff with green
// (added) / red (removed) backgrounds. The intent is to make the agent's
// edits scannable without leaving the chat surface.
//
// This is intentionally simple — a single <pre> with per-line backgrounds.
// No third-party diff library. The implementation derives the diff from the
// tool call arguments (old_text / new_text / old_str / new_str); it does not
// attempt a full Myers diff. We use a stable LCS-free diff that's adequate
// for the small hunks the agent typically produces.

import { FileCode } from 'lucide-react'
import { cn } from '@/lib/utils'

/** One rendered diff line. */
export interface DiffLine {
  /** 'add' | 'remove' | 'context'. */
  kind: 'add' | 'remove' | 'context'
  /** Line content (without the +/-/space prefix). */
  text: string
  /** 1-based line number in the new file (for adds/context). Null on removes. */
  newLine: number | null
  /** 1-based line number in the old file (for removes/context). Null on adds. */
  oldLine: number | null
}

/** Inputs the InlineDiffViewer accepts. */
export interface InlineDiffViewerProps {
  /** Path of the file being edited (shown as a header). */
  path?: string
  /** Tool name (for the header line). */
  toolName?: string
  /** Tool call arguments. We look for old_text/old_str + new_text/new_str. */
  args?: Record<string, unknown>
  /** Pre-computed diff lines. When supplied, we skip the line-splitting step. */
  lines?: DiffLine[]
  /** Optional className passthrough for the wrapper. */
  className?: string
}

/** Read a string field from an arbitrary record. */
function readStr(args: Record<string, unknown> | undefined, keys: string[]): string {
  if (!args) return ''
  for (const k of keys) {
    const v = args[k]
    if (typeof v === 'string') return v
  }
  return ''
}

/**
 * Build a simple line-by-line diff between `oldText` and `newText`.
 *
 * Algorithm: align unchanged prefix + suffix, emit a removed block for the
 * stripped middle of `oldText` and an added block for the stripped middle of
 * `newText`. This is not a full LCS — it under-reports common hunks when
 * the change is interleaved — but it preserves the visible shape of most
 * agent edits (single contiguous replacement).
 */
export function buildSimpleDiff(oldText: string, newText: string): DiffLine[] {
  const out: DiffLine[] = []

  if (oldText === newText) {
    if (oldText === '') return []
    const lines = oldText.split('\n')
    for (let i = 0; i < lines.length; i++) {
      out.push({ kind: 'context', text: lines[i] ?? '', newLine: i + 1, oldLine: i + 1 })
    }
    return out
  }

  const oldLines = oldText.split('\n')
  const newLines = newText.split('\n')

  // Common prefix length.
  let prefix = 0
  const maxPrefix = Math.min(oldLines.length, newLines.length)
  while (prefix < maxPrefix && oldLines[prefix] === newLines[prefix]) prefix++

  // Common suffix length (must not overlap the prefix).
  let suffix = 0
  const maxSuffix = Math.min(oldLines.length - prefix, newLines.length - prefix)
  while (
    suffix < maxSuffix &&
    oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
  ) {
    suffix++
  }

  const ctxStart = Math.max(0, prefix - 2)
  const ctxEndFromOld = Math.min(oldLines.length, prefix + (oldLines.length - prefix - suffix) + 2)
  const ctxEndFromNew = Math.min(newLines.length, prefix + (newLines.length - prefix - suffix) + 2)

  // Context (prefix).
  for (let i = ctxStart; i < prefix; i++) {
    out.push({
      kind: 'context',
      text: oldLines[i] ?? '',
      newLine: i + 1,
      oldLine: i + 1,
    })
  }

  // Removed block.
  const removedEnd = oldLines.length - suffix
  for (let i = prefix; i < removedEnd; i++) {
    out.push({
      kind: 'remove',
      text: oldLines[i] ?? '',
      newLine: null,
      oldLine: i + 1,
    })
  }

  // Added block.
  const addedEnd = newLines.length - suffix
  for (let i = prefix; i < addedEnd; i++) {
    out.push({
      kind: 'add',
      text: newLines[i] ?? '',
      newLine: i + 1,
      oldLine: null,
    })
  }

  // Context (suffix).
  const ctxStartNew = Math.max(0, addedEnd - 2)
  for (let i = ctxStartNew; i < addedEnd; i++) {
    out.push({
      kind: 'context',
      text: newLines[i] ?? '',
      newLine: i + 1,
      oldLine: oldLines.length - (addedEnd - i),
    })
  }
  // Suppress unused-variable warnings when nothing was added.
  void ctxEndFromOld
  void ctxEndFromNew

  return out
}

const KIND_STYLE: Record<DiffLine['kind'], string> = {
  add: 'bg-status-success-subtle text-status-success-on-subtle border-l-2 border-status-success',
  remove: 'bg-status-error-subtle text-status-error-on-subtle border-l-2 border-status-error',
  context: 'text-foreground/80 border-l-2 border-transparent',
}

const KIND_GUTTER: Record<DiffLine['kind'], string> = {
  add: 'text-status-success-on-subtle',
  remove: 'text-status-error-on-subtle',
  context: 'text-muted-foreground',
}

const KIND_PREFIX: Record<DiffLine['kind'], string> = {
  add: '+',
  remove: '-',
  context: ' ',
}

/** A single rendered diff line — kept simple for fast streaming updates. */
function DiffLineRow({ line, idx }: { line: DiffLine; idx: number }) {
  return (
    <div className={cn('flex font-mono text-xs leading-5', KIND_STYLE[line.kind])}>
      <span className="w-10 shrink-0 select-none px-1 text-right tabular-nums text-muted-foreground">
        {line.oldLine ?? ''}
      </span>
      <span className="w-10 shrink-0 select-none px-1 text-right tabular-nums text-muted-foreground">
        {line.newLine ?? ''}
      </span>
      <span
        className={cn('w-4 shrink-0 select-none text-center font-bold', KIND_GUTTER[line.kind])}
      >
        {KIND_PREFIX[line.kind]}
      </span>
      <span className="min-w-0 flex-1 whitespace-pre-wrap break-words pr-3">
        {line.text || '\u00A0'}
      </span>
      <span className="hidden" data-line-idx={idx} />
    </div>
  )
}

/**
 * InlineDiffViewer
 *
 * When `lines` is supplied, renders them directly. Otherwise, derives a diff
 * from `args` (`old_text`/`new_text` or `old_str`/`new_str`).
 */
export function InlineDiffViewer({
  path,
  toolName,
  args,
  lines,
  className,
}: InlineDiffViewerProps) {
  const resolvedLines = useDiffLines(lines, args)
  if (resolvedLines.length === 0) return null

  return (
    <div
      className={cn('overflow-hidden rounded-md border border-border/70 bg-muted/20', className)}
    >
      {(path || toolName) && (
        <div className="flex items-center gap-1.5 border-b border-border/60 bg-muted/40 px-2.5 py-1 text-xs text-muted-foreground">
          <FileCode className="h-3 w-3 shrink-0" />
          {toolName && <span className="font-medium text-foreground/80">{toolName}</span>}
          {path && (
            <span className="truncate font-mono text-2xs" title={path}>
              {path}
            </span>
          )}
          <span className="ml-auto text-2xs tabular-nums">
            +{resolvedLines.filter((l) => l.kind === 'add').length} −
            {resolvedLines.filter((l) => l.kind === 'remove').length}
          </span>
        </div>
      )}
      <pre className="overflow-x-auto py-1">
        {resolvedLines.map((l, i) => (
          <DiffLineRow key={`${i}-${l.kind}-${l.newLine ?? l.oldLine ?? 0}`} line={l} idx={i} />
        ))}
      </pre>
    </div>
  )
}

/** Resolve the diff lines for an InlineDiffViewer from `lines` or `args`. */
function useDiffLines(
  lines: DiffLine[] | undefined,
  args: Record<string, unknown> | undefined,
): DiffLine[] {
  if (lines && lines.length > 0) return lines
  const oldText = readStr(args, ['old_text', 'old_str', 'oldString', 'old'])
  const newText = readStr(args, ['new_text', 'new_str', 'newString', 'new', 'content'])
  if (!oldText && !newText) return []
  return buildSimpleDiff(oldText, newText)
}

/**
 * Detect whether a tool call's args look like a file edit we should diff.
 * Matches the standard edit / write / patch argument shape used across the
 * tool registry (old_text/new_text, old_str/new_str, etc.).
 */
export function isFileEditCall(
  toolName: string | undefined,
  args: Record<string, unknown> | undefined,
): boolean {
  if (!toolName || !args) return false
  const name = toolName.toLowerCase()
  const looksLikeEdit =
    name.includes('edit') ||
    name.includes('write') ||
    name.includes('patch') ||
    name.endsWith('_edit') ||
    name.endsWith('_write')
  if (!looksLikeEdit) return false
  // Need both before/after (or only after — write-style replacement).
  const hasOld = 'old_text' in args || 'old_str' in args || 'oldString' in args
  const hasNew = 'new_text' in args || 'new_str' in args || 'newString' in args || 'content' in args
  return hasOld || hasNew
}
