import Placeholder from '@tiptap/extension-placeholder'
import { EditorContent, useEditor } from '@tiptap/react'
import StarterKit from '@tiptap/starter-kit'
import {
  BookOpen,
  Brain,
  Clock,
  FileText,
  HardDrive,
  Image,
  Paperclip,
  Send,
  Sparkles,
  Square,
  X,
} from 'lucide-react'
import { type ChangeEvent, type DragEvent, useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import { useBrainSearchMutation } from '@/hooks/use-brain'
import { useIsTouch } from '@/hooks/use-is-touch'
import { useKnowledgeSearch } from '@/hooks/use-knowledge'
import { useMounts } from '@/hooks/use-mounts'
import { usePersonaCapabilities } from '@/hooks/usePersonaCapabilities'
import { api } from '@/lib/api-client'
import { getInputHistory } from '@/lib/input-history-storage'
import { cn } from '@/lib/utils'
import { ApprovalModeSelector } from './approval-mode-selector'
import { FanOutButton } from './FanOutButton'
import { LiveActivityBar } from './live-activity-bar'
import { ModelParamsPopover } from './model-params-popover'
import { ModelPickerContainer } from './model-picker'

// ── Types ──

export interface AttachedFile {
  name: string
  size: number
  type: string
  dataUrl?: string
  content?: string
}

export interface ContextAttachment {
  type: 'knowledge' | 'memory' | 'file'
  id: string
  label: string
  snippet?: string
}

interface MentionResult {
  type: 'mount' | 'knowledge' | 'memory' | 'role'
  id: string
  label: string
  snippet: string
  score?: number
}

interface ChatInputProps {
  value: string
  onChange: (value: string) => void
  onSend: (content: string, contextItems: ContextAttachment[], files: AttachedFile[]) => void
  onCancel?: () => void
  disabled?: boolean
  isStreaming?: boolean
  connected?: boolean
  queuedCount?: number
  roles?: { name: string; model: string }[]
  activeRole?: string | null
  setActiveRole?: (role: string | null) => void
  activeModelId?: string | null
  setActiveModelId?: (id: string | null) => void
  activeMounts?: { id: string; label: string }[]
  onAttachMount?: (id: string) => void
  onRemoveMount?: (id: string) => void
  placeholder?: string
}

// ── Slash commands ──

interface SlashCommand {
  id: string
  label: string
  description: string
  icon: string
  action: (editor: ReturnType<typeof useEditor>) => void
}

const SLASH_COMMANDS: SlashCommand[] = [
  // ── Conversation control ──
  {
    id: 'compact',
    label: '/compact',
    description: 'Summarize the conversation to save context',
    icon: '📝',
    action: (ed) => ed?.commands.insertContent('/compact '),
  },
  {
    id: 'new-topic',
    label: '/new-topic',
    description: 'Start a new topic branch',
    icon: '🆕',
    action: (ed) => ed?.commands.insertContent('/new-topic '),
  },
  {
    id: 'clear',
    label: '/clear',
    description: 'Clear the current input',
    icon: '🗑️',
    action: (ed) => ed?.commands.clearContent(),
  },
  // ── Search & web ──
  {
    id: 'search-on',
    label: '/search',
    description: 'Toggle web search for this message',
    icon: '🌐',
    action: (ed) => ed?.commands.insertContent('/search '),
  },
  {
    id: 'web',
    label: '/web',
    description: 'Fetch a URL and use it as context',
    icon: '🔗',
    action: (ed) => ed?.commands.insertContent('/web '),
  },
  // ── Skill invocation ──
  {
    id: 'skill',
    label: '/skill',
    description: 'Invoke a skill by name',
    icon: '⚡',
    action: (ed) => ed?.commands.insertContent('/skill '),
  },
  {
    id: 'persona',
    label: '/persona',
    description: 'Switch active persona',
    icon: '🎭',
    action: (ed) => ed?.commands.insertContent('/persona '),
  },
  // ── Session ──
  {
    id: 'save',
    label: '/save',
    description: 'Save the current response to knowledge base',
    icon: '📌',
    action: (ed) => ed?.commands.insertContent('/save '),
  },
  {
    id: 'export',
    label: '/export',
    description: 'Export the conversation',
    icon: '📤',
    action: (ed) => ed?.commands.insertContent('/export '),
  },
]

// TipTap's setContent() and the `content:` option parse their argument as
// HTML via DOMParser.parseFromString, which collapses newlines and swallows
// HTML-looking fragments (`a < b` → `a b`). Since onChange now carries plain
// text, escape it and convert `\n` to <br> (hardBreak stays enabled in
// StarterKit) before every setContent/content call.
function plainTextToHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/\n/g, '<br>')
}

// ── Component ──

export function ChatInput({
  value,
  onChange,
  onSend,
  onCancel,
  disabled,
  isStreaming,
  connected,
  queuedCount = 0,
  roles = [],
  activeRole = null,
  setActiveRole = () => {},
  activeModelId = null,
  setActiveModelId = () => {},
  activeMounts = [],
  onAttachMount = () => {},
  onRemoveMount = () => {},
  placeholder,
}: ChatInputProps) {
  const { t } = useTranslation()
  const isTouch = useIsTouch()

  // State
  const [contextAttachments, setContextAttachments] = useState<ContextAttachment[]>([])
  const [attachedFiles, setAttachedFiles] = useState<AttachedFile[]>([])
  const [isDragOver, setIsDragOver] = useState(false)
  const dragCounter = useRef(0)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const maxFileSize = 10 * 1024 * 1024
  const [showSlashMenu, setShowSlashMenu] = useState(false)
  const [slashFilter, setSlashFilter] = useState('')
  const [mentionQuery, setMentionQuery] = useState<string | null>(null)
  const [mentionIndex, setMentionIndex] = useState(0)
  const [mentionResults, setMentionResults] = useState<MentionResult[]>([])
  const mentionSearchTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Search hooks
  const knowledgeSearch = useKnowledgeSearch()
  const memorySearch = useBrainSearchMutation()
  const { data: mountsData } = useMounts()
  // RFC-044 Phase 3: persona capabilities drive composer affordances.
  const { capabilities } = usePersonaCapabilities()

  // Mention search
  // searchMentions must stay referentially stable: it is a dependency of the
  // mention effect below. useMutation() returns a NEW result object every
  // render, and `roles` is a fresh array each parent render — so listing them
  // as useCallback deps recreated searchMentions every render, re-running the
  // effect every render. Its `setMentionResults([])` (a new array ref) then
  // never bailed, producing a setState→render loop (React #185). Mirror the
  // latest inputs in a ref and read them inside a stable callback.
  const searchInputsRef = useRef({ knowledgeSearch, memorySearch, mountsData, roles })
  searchInputsRef.current = { knowledgeSearch, memorySearch, mountsData, roles }

  const searchMentions = useCallback(async (query: string): Promise<MentionResult[]> => {
    const {
      knowledgeSearch: k,
      memorySearch: mem,
      mountsData: mounts,
      roles: roleList,
    } = searchInputsRef.current
    const results: MentionResult[] = []
    try {
      const kRes = await k.mutateAsync({ query, limit: 5 })
      for (const hit of kRes.results)
        results.push({
          type: 'knowledge',
          id: hit.path,
          label: hit.name,
          snippet: hit.snippet.slice(0, 80),
        })
    } catch {
      /* offline */
    }
    try {
      const mRes = await mem.mutateAsync({ query, limit: 5 })
      for (const hit of mRes.items ?? []) {
        const id = hit.target?.id ?? ''
        results.push({
          type: 'memory',
          id,
          label: id.slice(0, 12),
          snippet: `${hit.target?.kind ?? 'memory'} · score ${hit.fused_score?.toFixed(3) ?? '—'}`,
          score: hit.fused_score,
        })
      }
    } catch {
      /* offline */
    }
    const mq = query.toLowerCase()
    for (const m of mounts?.items ?? []) {
      if (m.name.toLowerCase().includes(mq) || m.auto_description.toLowerCase().includes(mq))
        results.push({
          type: 'mount',
          id: m.id,
          label: m.name,
          snippet: m.auto_description.slice(0, 80),
        })
    }
    for (const r of roleList) {
      if (r.name.toLowerCase().includes(mq))
        results.push({ type: 'role', id: r.model, label: r.name, snippet: r.model })
    }
    const kindRank = (t: MentionResult['type']) => {
      switch (t) {
        case 'role':
          return 0
        case 'mount':
          return 1
        case 'knowledge':
          return 2
        default:
          return 3
      }
    }
    results.sort((a, b) => kindRank(a.type) - kindRank(b.type) || (b.score ?? 0) - (a.score ?? 0))
    return results.slice(0, 8)
  }, [])

  // Enter-to-send runs through ProseMirror's handleKeyDown (it fires before the
  // keymap, so the newline never lands in the doc). editorProps is bound once at
  // editor creation, so the changing values are mirrored into a ref and read fresh
  // on each keypress.
  const sendGate = useRef({ isTouch, showSlashMenu, mentionQuery, handleSend: () => {} })
  // Input history — terminal-style ArrowUp/Down navigation through past prompts.
  // Mirrors the sendGate ref pattern: the editorProps handler is bound once, so
  // mutable state is read fresh from a ref.
  const [historyPopup, setHistoryPopup] = useState<{ items: string[]; index: number } | null>(null)
  const historyGate = useRef({
    items: [] as string[],
    index: -1,
    original: '',
    applyingHistory: false,
    apply: (_text: string) => {},
  })
  // Placeholder text depends on connection state and i18n. useEditor captures
  // its config once at mount (no deps array), so the resolved string is also
  // pushed to the Placeholder extension imperatively when it changes (below).
  const placeholderText =
    placeholder ?? (connected ? t('chat.inputPlaceholder') : t('chat.waitingForConnection'))
  // Editor
  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // Plain-text input for the AI — every formatting mark/node is
        // disabled so the editor cannot produce bold/italic/code/list output
        // the model would never see anyway. document/paragraph/text/hardBreak
        // and history (undo/redo) remain. Typing ``` or ** stays literal text.
        heading: false,
        bold: false,
        italic: false,
        strike: false,
        code: false,
        codeBlock: false,
        bulletList: false,
        orderedList: false,
        listItem: false,
        blockquote: false,
        horizontalRule: false,
      }),
      Placeholder.configure({
        placeholder: placeholderText,
      }),
    ],
    content: plainTextToHtml(value),
    editable: !disabled && !!connected,
    editorProps: {
      handleKeyDown: (_view, event) => {
        // Shift+Enter → newline; IME-composition Enter confirms a candidate
        // (Korean/CJK) instead of sending.
        const composing = event.isComposing || event.keyCode === 229
        if (
          event.key === 'Enter' &&
          !event.shiftKey &&
          !composing &&
          !sendGate.current.isTouch &&
          !sendGate.current.showSlashMenu &&
          sendGate.current.mentionQuery === null
        ) {
          event.preventDefault()
          sendGate.current.handleSend()
          return true
        }
        // ArrowUp/Down — input history navigation (terminal-style), only when
        // no slash/mention menu is open and not composing.
        const hg = historyGate.current
        const menusOpen = sendGate.current.showSlashMenu || sendGate.current.mentionQuery !== null
        if (!composing && !menusOpen) {
          if (event.key === 'ArrowUp') {
            const text = _view.state.doc.textBetween(0, _view.state.doc.content.size, '\n')
            if (text.length === 0 || hg.index >= 0) {
              event.preventDefault()
              if (hg.index < 0) {
                hg.items = getInputHistory()
                if (hg.items.length === 0) return true
                hg.original = text
              }
              hg.index = Math.min(hg.index + 1, hg.items.length - 1)
              hg.apply(hg.items[hg.index]!)
              setHistoryPopup({ items: [...hg.items], index: hg.index })
              return true
            }
          }
          if (event.key === 'ArrowDown' && hg.index >= 0) {
            event.preventDefault()
            hg.index -= 1
            if (hg.index < 0) {
              hg.apply(hg.original)
              setHistoryPopup(null)
            } else {
              hg.apply(hg.items[hg.index]!)
              setHistoryPopup({ items: [...hg.items], index: hg.index })
            }
            return true
          }
          if (event.key === 'Escape' && hg.index >= 0) {
            event.preventDefault()
            hg.index = -1
            hg.apply(hg.original)
            setHistoryPopup(null)
            return true
          }
        }
        return false
      },
    },
    onUpdate: ({ editor }) => {
      const text = editor.getText()
      // Send plain text — formatting marks are disabled so getText() carries
      // everything the model needs (slash commands, @mentions, code fences
      // typed verbatim). getHTML() previously sent markup that getContent()
      // below then stripped back to text on send: a zero-value round-trip.
      onChange(text)
      // Exit input-history mode when the user types normally (not via apply).
      if (!historyGate.current.applyingHistory && historyGate.current.index >= 0) {
        historyGate.current.index = -1
        setHistoryPopup(null)
      }
      const anchor = editor.state.selection.anchor
      const textBefore = text.slice(0, anchor)
      // /commands
      const slashMatch = textBefore.match(/(?:^|\n)\/(\w*)$/)
      if (slashMatch) {
        setShowSlashMenu(true)
        setSlashFilter(slashMatch[1] || '')
      } else {
        setShowSlashMenu(false)
      }
      // @mentions
      const mentionMatch = textBefore.match(/@(\S*)$/)
      if (mentionMatch) {
        setMentionQuery(mentionMatch[1] || '')
      } else {
        setMentionQuery(null)
        setMentionResults([])
      }
    },
  })
  // Sync editable when connection state changes after mount
  useEffect(() => {
    if (editor) editor.setEditable(!disabled && !!connected)
  }, [editor, connected, disabled])
  // Sync placeholder text when connection state / i18n change after mount.
  // useEditor captures the Placeholder extension's option once at creation;
  // without this the empty-input hint stays frozen at the mount-time value
  // (e.g. "연결 대기 중...") even after the WebSocket connects. The Placeholder
  // plugin only rebuilds its decoration on doc/selection change, so we mutate
  // the option in place and touch the selection to force a rescan.
  useEffect(() => {
    if (!editor) return
    const ext = editor.extensionManager.extensions.find((e) => e.name === 'placeholder')
    if (!ext || ext.options.placeholder === placeholderText) return
    ext.options.placeholder = placeholderText
    const { state, view } = editor
    // setSelection marks the transaction selectionSet even when unchanged,
    // which is what makes the placeholder state field rescan decorations.
    view.dispatch(state.tr.setSelection(state.selection))
  }, [editor, placeholderText])

  // Sync external value (draft restore, history nav) into the editor.
  // onChange now emits plain text, so compare against getText() — comparing
  // against getHTML() would never match and re-trigger setContent each render.
  useEffect(() => {
    if (!editor) return
    if ((value || '') !== editor.getText()) editor.commands.setContent(plainTextToHtml(value || ''))
  }, [value, editor])

  // Wire the input-history apply callback now that the editor exists.
  useEffect(() => {
    if (!editor) return
    historyGate.current.apply = (text: string) => {
      historyGate.current.applyingHistory = true
      editor.commands.setContent(plainTextToHtml(text))
      editor.commands.focus('end')
      historyGate.current.applyingHistory = false
    }
  }, [editor])

  // Mention search effect
  useEffect(() => {
    if (mentionQuery === null) {
      setMentionResults((prev) => (prev.length === 0 ? prev : []))
      return
    }
    clearTimeout(mentionSearchTimer.current!)
    mentionSearchTimer.current = setTimeout(async () => {
      const results = await searchMentions(mentionQuery)
      setMentionResults(results)
      setMentionIndex(0)
    }, 200)
    return () => {
      clearTimeout(mentionSearchTimer.current!)
    }
  }, [mentionQuery, searchMentions])

  const readFile = useCallback(async (file: File): Promise<AttachedFile> => {
    const result: AttachedFile = { name: file.name, size: file.size, type: file.type }
    if (file.type.startsWith('image/')) {
      // Upload to unified asset store — keeps message payloads lean and
      // makes the image persistent + reusable. Falls back to base64 on error.
      try {
        const fd = new FormData()
        fd.append('file', file)
        fd.append('source', 'chat-attach')
        const asset = await api.upload<{ url: string }>('/api/assets', fd)
        result.dataUrl = asset.url
      } catch {
        result.dataUrl = await new Promise<string>((resolve) => {
          const r = new FileReader()
          r.onload = () => resolve(r.result as string)
          r.readAsDataURL(file)
        })
      }
    } else if (/\.(md|json|txt|csv|yml|yaml|toml|xml|log|rs|ts|js|py|html|css)$/i.test(file.name)) {
      result.content = await file.text()
    }
    return result
  }, [])
  const addFiles = useCallback(
    async (fileList: FileList | File[]) => {
      const files = Array.from(fileList)
        .filter((f) => f.size <= maxFileSize)
        .slice(0, 5)
      if (files.length === 0) return
      const results = await Promise.all(files.map(readFile))
      setAttachedFiles((prev) => [...prev, ...results].slice(0, 10))
    },
    [maxFileSize, readFile],
  )
  const removeFile = useCallback(
    (index: number) => setAttachedFiles((prev) => prev.filter((_, i) => i !== index)),
    [],
  )
  const handleFilePick = useCallback(
    (e: ChangeEvent<HTMLInputElement>) => {
      if (e.target.files) addFiles(e.target.files)
      // Reset so the same file can be re-selected
      e.target.value = ''
    },
    [addFiles],
  )

  // Drag-drop
  const handleDragEnter = useCallback((e: DragEvent) => {
    e.preventDefault()
    e.stopPropagation()
    dragCounter.current++
    if (e.dataTransfer?.types.includes('Files')) setIsDragOver(true)
  }, [])
  const handleDragLeave = useCallback((e: DragEvent) => {
    e.preventDefault()
    e.stopPropagation()
    dragCounter.current--
    if (dragCounter.current <= 0) {
      dragCounter.current = 0
      setIsDragOver(false)
    }
  }, [])
  const handleDragOver = useCallback((e: DragEvent) => {
    e.preventDefault()
    e.stopPropagation()
  }, [])
  const handleDrop = useCallback(
    (e: DragEvent) => {
      e.preventDefault()
      e.stopPropagation()
      dragCounter.current = 0
      setIsDragOver(false)
      if (e.dataTransfer?.files?.length) addFiles(e.dataTransfer.files)
    },
    [addFiles],
  )

  // Send
  const getContent = useCallback(() => editor?.getText() ?? '', [editor])
  const handleSend = useCallback(() => {
    const content = getContent()
    if (!content.trim() || !connected) return
    onSend(content, contextAttachments, attachedFiles)
    editor?.commands.clearContent()
    setContextAttachments([])
    setAttachedFiles([])
  }, [getContent, connected, contextAttachments, attachedFiles, onSend, editor])
  sendGate.current = { isTouch, showSlashMenu, mentionQuery, handleSend }

  const canSend = editor?.getText().trim() && connected

  const filteredCommands = SLASH_COMMANDS.filter(
    (c) => c.id.includes(slashFilter) || c.label.includes(slashFilter),
  )

  return (
    <div className="w-full max-w-3xl mx-auto px-4 pb-4 pt-2 relative">
      {/* File chips */}
      {attachedFiles.length > 0 && (
        <div className="flex flex-wrap gap-1.5 mb-2">
          {attachedFiles.map((file, i) => (
            <span
              key={`${file.name}-${i}`}
              className="inline-flex items-center gap-1 rounded-full bg-status-info-subtle border border-info-subtle-border px-2.5 py-0.5 text-xs text-status-info-on-subtle"
            >
              {file.type.startsWith('image/') ? (
                <Image className="h-3 w-3" />
              ) : (
                <Paperclip className="h-3 w-3" />
              )}
              <span className="truncate max-w-[140px]">{file.name}</span>
              <button
                type="button"
                onClick={() => removeFile(i)}
                className="ml-0.5 -mr-1 rounded-full p-0.5 hover:bg-status-info-muted"
              >
                <X className="h-2.5 w-2.5" />
              </button>
            </span>
          ))}
        </div>
      )}
      {/* Context chips */}
      {(activeMounts.length > 0 || contextAttachments.length > 0) && (
        <div className="flex flex-wrap gap-1.5 mb-2">
          {activeMounts.map((m) => (
            <span
              key={`mount-${m.id}`}
              className="inline-flex items-center gap-1 rounded-full bg-primary/10 border border-primary/20 px-2.5 py-0.5 text-xs text-primary"
            >
              <HardDrive className="h-3 w-3" />
              <span className="truncate max-w-[140px]">{m.label}</span>
              <button
                type="button"
                onClick={() => onRemoveMount(m.id)}
                className="ml-0.5 -mr-1 rounded-full p-0.5 hover:bg-primary/20"
              >
                <X className="h-2.5 w-2.5" />
              </button>
            </span>
          ))}
          {contextAttachments.map((ctx) => (
            <span
              key={`${ctx.type}-${ctx.id}`}
              className="inline-flex items-center gap-1 rounded-full bg-muted/80 px-2.5 py-0.5 text-xs text-foreground"
            >
              {ctx.type === 'knowledge' ? (
                <BookOpen className="h-3 w-3 text-status-info" />
              ) : (
                <Brain className="h-3 w-3 text-hue-purple" />
              )}
              <span className="truncate max-w-[140px]">{ctx.label}</span>
              <button
                type="button"
                onClick={() => setContextAttachments((prev) => prev.filter((a) => a.id !== ctx.id))}
                className="ml-0.5 -mr-1 rounded-full p-0.5 hover:bg-muted-foreground/20"
              >
                <X className="h-2.5 w-2.5" />
              </button>
            </span>
          ))}
        </div>
      )}
      {/* @mention Popover */}
      {mentionQuery !== null && (
        <div className="absolute bottom-full left-4 right-4 z-50 mb-1 rounded-xl border bg-popover shadow-lg">
          <div className="p-1.5 max-h-64 overflow-y-auto">
            {mentionResults.length > 0 ? (
              mentionResults.map((result, idx) => (
                <button
                  key={`${result.type}-${result.id}`}
                  type="button"
                  onClick={() => {
                    if (result.type === 'mount') {
                      onAttachMount(result.id)
                    } else if (result.type === 'role') {
                      setActiveRole(result.label)
                    } else {
                      const ctx: ContextAttachment = {
                        type: result.type as 'knowledge' | 'memory',
                        id: result.id,
                        label: result.label,
                        snippet: result.snippet,
                      }
                      setContextAttachments((prev) =>
                        prev.some((a) => a.id === ctx.id && a.type === ctx.type)
                          ? prev
                          : [...prev, ctx],
                      )
                    }
                    setMentionQuery(null)
                    editor?.commands.focus()
                  }}
                  className={cn(
                    'flex items-start gap-2.5 w-full rounded-lg px-2.5 py-2 text-left transition-colors',
                    idx === mentionIndex
                      ? 'bg-accent text-accent-foreground'
                      : 'hover:bg-accent/50',
                  )}
                >
                  {result.type === 'mount' ? (
                    <HardDrive className="h-4 w-4 mt-0.5 shrink-0 text-status-success" />
                  ) : result.type === 'knowledge' ? (
                    <FileText className="h-4 w-4 mt-0.5 shrink-0 text-status-info" />
                  ) : result.type === 'role' ? (
                    <Sparkles className="h-4 w-4 mt-0.5 shrink-0 text-status-warning" />
                  ) : (
                    <Brain className="h-4 w-4 mt-0.5 shrink-0 text-hue-purple" />
                  )}
                  <div className="min-w-0 flex-1">
                    <p className="text-sm font-medium truncate">{result.label}</p>
                    {result.snippet && (
                      <p className="text-xs text-muted-foreground truncate">{result.snippet}</p>
                    )}
                  </div>
                  <span className="text-2xs text-muted-foreground/60 shrink-0 mt-0.5">
                    {result.type === 'mount'
                      ? 'Mount'
                      : result.type === 'knowledge'
                        ? 'KB'
                        : result.type === 'role'
                          ? 'Agent'
                          : 'Memory'}
                  </span>
                </button>
              ))
            ) : (
              <p className="px-2.5 py-3 text-xs text-muted-foreground text-center">
                {mentionQuery === '' ? 'Type to search...' : 'No results'}
              </p>
            )}
          </div>
        </div>
      )}
      {/* Slash command menu */}
      {showSlashMenu && (
        <div className="absolute bottom-full left-4 z-50 mb-1 rounded-xl border bg-popover shadow-lg w-64">
          <div className="p-1.5">
            {filteredCommands.map((cmd) => (
              <button
                key={cmd.id}
                type="button"
                onClick={() => {
                  cmd.action(editor)
                  setShowSlashMenu(false)
                }}
                className="flex items-center gap-2.5 w-full rounded-lg px-2.5 py-2 text-left hover:bg-accent/50 transition-colors"
              >
                <span className="text-sm">{cmd.icon}</span>
                <div className="min-w-0 flex-1">
                  <p className="text-sm font-medium">{cmd.label}</p>
                  <p className="text-xs text-muted-foreground">{cmd.description}</p>
                </div>
              </button>
            ))}
          </div>
        </div>
      )}
      {/* Input */}
      <div
        onDragEnter={handleDragEnter}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}
        className={cn(
          'relative rounded-lg border bg-background shadow-sm transition-all',
          'focus-within:shadow-md focus-within:border-primary/40 focus-within:ring-1 focus-within:ring-ring/30',
          !connected && 'opacity-60',
          isStreaming && 'border-destructive/30',
          isDragOver && 'border-primary ring-2 ring-primary/30',
        )}
      >
        {isDragOver && (
          <div className="absolute inset-0 z-10 flex items-center justify-center rounded-lg bg-primary/5 backdrop-blur-[1px] pointer-events-none">
            <span className="text-sm text-primary font-medium">Drop files to attach</span>
          </div>
        )}
        {historyPopup && (
          <div className="absolute bottom-full left-0 right-0 z-20 mb-1 max-h-56 overflow-y-auto rounded-lg border bg-popover p-1 shadow-md">
            {historyPopup.items.slice(0, 8).map((item, i) => (
              <button
                key={i}
                type="button"
                className={cn(
                  'block w-full truncate rounded px-2 py-1.5 text-left text-xs transition-colors',
                  i === historyPopup.index
                    ? 'bg-accent font-medium text-accent-foreground'
                    : 'text-muted-foreground hover:bg-muted',
                )}
              >
                {item.replace(/<[^>]+>/g, '').slice(0, 120)}
              </button>
            ))}
          </div>
        )}
        <LiveActivityBar />
        <div className="px-4 pt-3 pb-2.5">
          <EditorContent
            editor={editor}
            className="prose prose-sm dark:prose-invert max-w-none [&_.ProseMirror]:outline-none [&_.ProseMirror]:min-h-[1.5em] [&_.ProseMirror]:max-h-[280px] [&_.ProseMirror]:overflow-y-auto [&_.ProseMirror_p.is-editor-empty:first-child::before]:text-muted-foreground/70 [&_.ProseMirror_p.is-editor-empty:first-child::before]:content-[attr(data-placeholder)] [&_.ProseMirror_p.is-editor-empty:first-child::before]:float-left [&_.ProseMirror_p.is-editor-empty:first-child::before]:pointer-events-none [&_.ProseMirror_p.is-editor-empty:first-child::before]:h-0"
          />
        </div>
        <div className="flex items-center justify-between gap-2 px-4 pb-3 pt-1">
          {/* Left: model + context tools */}
          <div className="flex items-center gap-1 min-w-0 flex-1">
            <ModelPickerContainer
              activeModelId={activeModelId}
              setActiveModelId={setActiveModelId}
              roles={roles}
              activeRole={activeRole}
              setActiveRole={setActiveRole}
            />
            <ApprovalModeSelector />
            <input
              ref={fileInputRef}
              type="file"
              multiple
              onChange={handleFilePick}
              className="hidden"
              aria-label={t('chat.attachFiles')}
            />
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-input text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              title={t('chat.attachFiles')}
            >
              <Paperclip className="h-3.5 w-3.5" />
            </button>
            <ModelParamsPopover />
            {capabilities.has('worktree-fanout') && <FanOutButton />}
          </div>
          {/* Right: queue + send */}
          <div className="flex items-center shrink-0 gap-1.5">
            {queuedCount > 0 && (
              <span className="mr-0.5 flex items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-2xs text-muted-foreground">
                <Clock className="h-3 w-3" />
                {t('chat.queued', { count: queuedCount, defaultValue: '{{count}} queued' })}
              </span>
            )}
            {isStreaming ? (
              <Button
                onClick={onCancel}
                variant="destructive"
                size="sm"
                className="h-8 rounded-lg px-3 text-xs gap-1.5"
              >
                <Square className="h-3 w-3 fill-current" />
                {t('chat.stop')}
              </Button>
            ) : (
              canSend &&
              !isTouch && (
                <kbd
                  className="hidden h-5 items-center rounded border bg-muted/60 px-1.5 font-mono text-2xs text-muted-foreground/70 sm:inline-flex"
                  title={t('chat.send')}
                >
                  ⏎
                </kbd>
              )
            )}
            <Button
              onClick={handleSend}
              disabled={!canSend}
              size="icon"
              className={cn(
                'h-8 w-8 rounded-lg transition-all',
                canSend
                  ? 'bg-primary text-primary-foreground hover:bg-primary/90 shadow-sm'
                  : 'bg-muted text-muted-foreground',
              )}
            >
              <Send className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>
    </div>
  )
}
