// Task 7 (project-roots persona workbench): WorkbenchShell — the
// conversation-first shell that wraps the chat canvas when the
// server-derived EffectiveProfile is in the Base/Code family.
//
// The shell renders:
//   * ProjectBar (always): project selector + roots summary + branch/dirty
//     status + active model + ⌘K hint. For a folderless coding project, the
//     roots summary collapses to a single "Add folders" action — no file
//     rail, no terminal, no changes rail, no preview rail.
//   * ActivityRail: visible rail icons (conversation + files/terminal/
//     changes/preview per affordances, in that order).
//   * Stage panel: a slide-over that opens in response to emergence
//     events (agent edit, long command, renderable artifact, user rail
//     click). Pin/dismiss/auto-dismiss rules are unit-tested in
//     workbench.test.ts; this file only verifies the rendered effect.
//
// Folderless code (CODE_FOLDERLESS): the bar exposes Add folders; the rail
// renders only the Conversation rail.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { act, cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import type { ReactNode } from 'react'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { ProjectSelectorPopover } from '@/components/workbench/ProjectSelectorPopover'
import { WorkbenchShell } from '@/components/workbench/WorkbenchShell'
import { useChatStore } from '@/stores/chat'
import { useWorkbenchStore } from '@/stores/workbench'
import type { EffectiveProfile, Project } from '@/types'
import { server } from '../../msw/server'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))
vi.mock('sonner', () => ({ toast: { error: vi.fn(), info: vi.fn(), success: vi.fn() } }))
vi.mock('@/hooks/use-is-touch', () => ({ useIsTouch: () => false }))

// `useFolderPicker` returns a fake pick() so the Add Folders button never
// touches the network (folder picking is local-only).
vi.mock('@/hooks/use-folder-picker', () => ({
  useFolderPicker: () => ({
    pick: vi.fn().mockResolvedValue(['/tmp/proj-root']),
    picking: false,
    error: null,
    localOnly: false,
  }),
}))

class FakeWebSocket {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readyState = FakeWebSocket.CONNECTING
  onopen: unknown = null
  onmessage: unknown = null
  onclose: unknown = null
  onerror: unknown = null
  send() {}
  close() {}
}

const CODE_ROOTS: EffectiveProfile = {
  persona_id: 'dev',
  tool_profile: 'code',
  affordances: ['diff', 'files', 'terminal', 'worktree-fanout'],
  presentation_lens: 'code',
}
const CODE_FOLDERLESS: EffectiveProfile = {
  persona_id: 'dev',
  tool_profile: 'code',
  affordances: ['diff'],
  presentation_lens: 'code',
}
const MINIMAL: EffectiveProfile = {
  persona_id: 'writer',
  tool_profile: 'minimal',
  affordances: [],
  presentation_lens: 'writing',
}

const PROJECT: Project = {
  id: 'p1',
  name: 'Acme',
  root_paths: ['/tmp/proj-root'],
  instructions: '',
  created_at: '2026-08-29T00:00:00Z',
  updated_at: '2026-08-29T00:00:00Z',
  last_active_at: '2026-08-29T00:00:00Z',
}

function setupServer() {
  server.use(
    http.get('/api/status', () => HttpResponse.json({ auth_enabled: false })),
    http.get('/api/engine/roles', () => HttpResponse.json({ roles: {} })),
    http.get('/api/engine/config', () => HttpResponse.json({ routingEnabled: false })),
    http.get('/api/engine/models', () => HttpResponse.json({ models: [], providers: [] })),
    http.get('/api/personas', () => HttpResponse.json([])),
    http.get('/api/personas/active', () => HttpResponse.json(null)),
    http.get('/api/sessions', () => HttpResponse.json({ items: [], total: 0 })),
    http.get('/api/projects', () => HttpResponse.json({ items: [PROJECT], total: 1 })),
    http.get(`/api/projects/${PROJECT.id}`, () => HttpResponse.json(PROJECT)),
    http.get('/api/project/workspace/status', () =>
      HttpResponse.json({ branch: 'main', dirty_count: 0 }),
    ),
    http.get('/api/project/workspace/tree', () =>
      HttpResponse.json({
        roots: ['/tmp/proj-root'],
        path: '/tmp/proj-root',
        entries: [{ name: 'README.md', path: '/tmp/proj-root/README.md', is_dir: false, size: 11 }],
      }),
    ),
    http.get('/api/project/workspace/file', () =>
      HttpResponse.json({
        path: '/tmp/proj-root/README.md',
        content: '',
        size: 0,
        truncated: false,
      }),
    ),
    http.get('/api/system/pick-folders', () => HttpResponse.json({ paths: [] })),
  )
}

beforeAll(() => {
  vi.stubGlobal('WebSocket', FakeWebSocket)
  if (!window.matchMedia) {
    window.matchMedia = (query: string) =>
      ({
        matches: false,
        media: query,
        addListener: () => {},
        removeListener: () => {},
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }) as unknown as MediaQueryList
  }
  // cmdk (QuickOpen) needs ResizeObserver in jsdom.
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  )
  // cmdk scrolls the selected item into view; jsdom lacks the API.
  Element.prototype.scrollIntoView ??= () => {}
  for (const proto of [Text.prototype, Range.prototype] as unknown as Array<{
    getClientRects?: unknown
    getBoundingClientRect?: unknown
  }>) {
    proto.getClientRects ??= () => [new DOMRect(0, 0, 1, 1)] as unknown as DOMRectList
    proto.getBoundingClientRect ??= () => new DOMRect(0, 0, 1, 1)
  }
})

afterAll(() => vi.unstubAllGlobals())

beforeEach(() => {
  // WorkbenchShell binds ⌘P only on the /studio route (verified by
  // preventDefault); jsdom defaults to "/" so set the route first.
  window.history.replaceState(null, '', '/studio')
  setupServer()
  useChatStore.getState().disconnect()
  useChatStore.setState({
    activeSessionId: 'sess-shell',
    activeProjectId: PROJECT.id,
    effectiveProfile: null,
    messages: [],
    isLoadingSession: false,
    sessionLoadError: null,
  })
  useWorkbenchStore.getState().reset()
})

afterEach(() => {
  cleanup()
})

function renderShell() {
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  return render(
    <WorkbenchShell>{<div data-testid="chat-canvas">transcript</div>}</WorkbenchShell>,
    {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    },
  )
}

function setProfile(p: EffectiveProfile) {
  act(() => {
    useChatStore.setState({ effectiveProfile: p })
  })
}

const TRANSCRIPT = '[data-testid="chat-canvas"]'

describe('WorkbenchShell — affordance gating', () => {
  it('renders the project bar with roots summary for code + roots', async () => {
    setProfile(CODE_ROOTS)
    renderShell()
    // Project query resolves async — wait for the project name.
    await waitFor(() => expect(screen.queryByText(PROJECT.name)).not.toBeNull())
    // Roots summary: basename of root_paths[0].
    expect(screen.getByText(/proj-root/)).toBeTruthy()
  })

  it('folderless code shows the Add Folders action and no file rails', async () => {
    // Folderless = a bound project whose root_paths is EMPTY (design
    // §7.3). Override the project endpoint so the bound project has no
    // roots; the Add Folders action replaces the roots summary and the
    // file/terminal rails are suppressed.
    server.use(
      http.get(`/api/projects/${PROJECT.id}`, () =>
        HttpResponse.json({ ...PROJECT, root_paths: [] }),
      ),
    )
    setProfile(CODE_FOLDERLESS)
    renderShell()
    await waitFor(() =>
      expect(screen.queryByRole('button', { name: 'workbench.addFolders' })).not.toBeNull(),
    )
    // No Files / Terminal / Changes / Preview rail buttons.
    expect(screen.queryByRole('button', { name: 'workbench.rail.files' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'workbench.rail.terminal' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'workbench.rail.changes' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'workbench.rail.preview' })).toBeNull()
  })

  it('code + roots shows all 5 rails in design order (conversation, files, terminal, changes, preview)', async () => {
    setProfile(CODE_ROOTS)
    renderShell()
    await waitFor(() => expect(screen.queryByTestId('project-bar')).not.toBeNull())
    for (const r of ['conversation', 'files', 'terminal', 'changes', 'preview'] as const) {
      expect(screen.getByRole('button', { name: `workbench.rail.${r}` })).toBeTruthy()
    }
  })

  it('minimal profile renders no workbench shell at all', async () => {
    setProfile(MINIMAL)
    renderShell()
    // The shell falls through to the plain chat canvas: no project bar,
    // no rails. (A minimal persona never renders coding affordances.)
    await waitFor(() => expect(screen.queryByTestId('project-bar')).toBeNull())
    expect(screen.queryByRole('button', { name: 'workbench.rail.files' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'workbench.rail.terminal' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'workbench.rail.changes' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'workbench.rail.preview' })).toBeNull()
  })

  it('transcript is preserved across stage toggles (no unmount)', async () => {
    setProfile(CODE_ROOTS)
    const { container } = renderShell()
    await waitFor(() => expect(screen.queryByTestId('project-bar')).not.toBeNull())
    const before = container.querySelector(TRANSCRIPT)
    expect(before).not.toBeNull()
    // Toggle the changes rail → stage opens.
    await userEvent.click(screen.getByRole('button', { name: 'workbench.rail.changes' }))
    await waitFor(() =>
      expect(
        useWorkbenchStore.getState().stageOpen &&
          useWorkbenchStore.getState().activeRail === 'changes',
      ).toBe(true),
    )
    // Toggle back to conversation → stage dismisses.
    await userEvent.click(screen.getByRole('button', { name: 'workbench.rail.conversation' }))
    await waitFor(() => expect(useWorkbenchStore.getState().activeRail).toBe('conversation'))
    // The same transcript DOM node survives.
    expect(container.querySelector(TRANSCRIPT)).toBe(before)
  })
})

describe('WorkbenchShell — §7.2 emergence (event-driven from the transcript)', () => {
  it('a new file-change tool call opens the changes stage pinned', async () => {
    setProfile(CODE_ROOTS)
    // Seed the transcript BEFORE render — the observer must fire on it.
    act(() => {
      useChatStore.setState({
        messages: [
          {
            id: 'm1',
            role: 'assistant',
            content: '',
            blocks: [
              {
                type: 'tool',
                id: 't1',
                identifier: 'kernel',
                apiName: 'edit',
                arguments: { path: '/repo/a.ts', old_text: 'a', new_text: 'b' },
                status: 'success',
              },
            ],
          },
        ],
      })
    })
    renderShell()
    await waitFor(() => expect(useWorkbenchStore.getState().stageOpen).toBe(true))
    const s = useWorkbenchStore.getState()
    expect(s.activeRail).toBe('changes')
    expect(s.stagePinned).toBe(true) // pendingCount 1 > 0
    // The diff stage is actually rendered in the slide-over.
    expect(screen.queryByTestId('diff-stage')).not.toBeNull()
  })

  it('a terminal-alias tool running >5s opens the terminal stage', async () => {
    setProfile(CODE_ROOTS)
    act(() => {
      useChatStore.setState({
        messages: [
          {
            id: 'm1',
            role: 'assistant',
            content: '',
            blocks: [
              {
                type: 'tool',
                id: 'b1',
                identifier: 'kernel',
                apiName: 'Bash',
                arguments: { command: 'sleep 30' },
                status: 'loading',
                startedAt: Date.now() - 6_000,
              },
            ],
          },
        ],
      })
    })
    renderShell()
    await waitFor(
      () => {
        const s = useWorkbenchStore.getState()
        expect(s.stageOpen && s.activeRail === 'terminal').toBe(true)
      },
      { timeout: 4000 },
    )
    expect(screen.queryByTestId('terminal-stage')).not.toBeNull()
  })

  it('a completed artifact opens the preview stage; the next one auto-dismisses unless pinned', async () => {
    setProfile(CODE_ROOTS)
    const artifact = (id: string, mark: string) => ({
      id,
      role: 'assistant' as const,
      generating: false,
      content: `\`\`\`svg\n<svg xmlns="http://www.w3.org/2000/svg"><text>${mark}</text></svg>\n\`\`\``,
    })
    act(() => {
      useChatStore.setState({ messages: [artifact('a1', 'ONE')] })
    })
    renderShell()
    await waitFor(() => {
      const s = useWorkbenchStore.getState()
      expect(s.stageOpen && s.activeRail === 'preview').toBe(true)
    })
    // The renderer output is in the stage.
    expect(screen.queryByTestId('preview-stage-artifact')).not.toBeNull()

    // Second completed artifact auto-dismisses the unpinned preview stage.
    act(() => {
      useChatStore.setState({
        messages: [artifact('a1', 'ONE'), artifact('a2', 'TWO')],
      })
    })
    await waitFor(() => expect(useWorkbenchStore.getState().stageOpen).toBe(false))

    // Pinned variant: reopen preview, pin it, then a third artifact keeps it open.
    act(() => {
      useWorkbenchStore.getState().openStage('preview')
      useWorkbenchStore.getState().togglePin()
    })
    act(() => {
      useChatStore.setState({
        messages: [artifact('a1', 'ONE'), artifact('a2', 'TWO'), artifact('a3', 'THREE')],
      })
    })
    await waitFor(() => {
      const s = useWorkbenchStore.getState()
      expect(s.stageOpen && s.activeRail === 'preview' && s.stagePinned).toBe(true)
    })
  })
})

describe('WorkbenchShell — quick open', () => {
  it('⌘P opens the quick-open dialog without shadowing browser print (preventDefault registered)', async () => {
    setProfile(CODE_ROOTS)
    renderShell()
    await waitFor(() => expect(screen.queryByTestId('project-bar')).not.toBeNull())
    // Before the keystroke the dialog is closed.
    expect(useWorkbenchStore.getState().quickOpenOpen).toBe(false)
    // Simulate ⌘P via window keydown — the chat route must call
    // preventDefault so the browser print dialog never opens.
    const evt = new KeyboardEvent('keydown', {
      key: 'p',
      code: 'KeyP',
      metaKey: true,
      bubbles: true,
      cancelable: true,
    })
    // MDN: dispatchEvent returns false when a cancelable event's listener
    // called preventDefault — exactly what the shell must do for ⌘P.
    expect(window.dispatchEvent(evt)).toBe(false)
    expect(evt.defaultPrevented).toBe(true)
    await waitFor(() => expect(useWorkbenchStore.getState().quickOpenOpen).toBe(true))
  })

  it('⌘P is inert for a non-coding persona (browser print untouched)', async () => {
    setProfile(MINIMAL)
    renderShell()
    await waitFor(() => expect(screen.queryByTestId('project-bar')).toBeNull())
    const evt = new KeyboardEvent('keydown', {
      key: 'p',
      code: 'KeyP',
      metaKey: true,
      bubbles: true,
      cancelable: true,
    })
    // No listener consumed the event: not prevented, no dialog.
    expect(window.dispatchEvent(evt)).toBe(true)
    expect(evt.defaultPrevented).toBe(false)
    expect(useWorkbenchStore.getState().quickOpenOpen).toBe(false)
  })

  it('quick-open lists the MSW tree entry and selecting it opens the editor modal', async () => {
    setProfile(CODE_ROOTS)
    renderShell()
    await waitFor(() => expect(screen.queryByTestId('project-bar')).not.toBeNull())
    act(() => {
      useWorkbenchStore.getState().setQuickOpen(true)
    })
    await waitFor(() => expect(screen.queryByTestId('quick-open-input')).not.toBeNull())
    // The rendered ENTRY comes from the MSW workspace tree data…
    const item = await screen.findByText('README.md')
    await userEvent.click(item)
    // …and completing list → select opens the editor modal for that path.
    await waitFor(() =>
      expect(useWorkbenchStore.getState().editingPath).toBe('/tmp/proj-root/README.md'),
    )
    await waitFor(() => expect(screen.queryByTestId('file-editor-modal')).not.toBeNull())
  })
})

describe('ProjectSelectorPopover — project binding (IA design §8.3)', () => {
  it('clicking a project entry binds it for future turns', async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    useChatStore.setState({ activeProjectId: null })
    render(<ProjectSelectorPopover activeProject={null} onSelect={() => {}} />, {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    })
    // Open the dropdown.
    await userEvent.click(screen.getByRole('button', { name: 'workbench.projectSelector.label' }))
    // Click the rendered project entry (from MSW /api/projects).
    await userEvent.click(await screen.findByText(PROJECT.name))
    await waitFor(() => expect(useChatStore.getState().activeProjectId).toBe(PROJECT.id))
  })

  it('clicking "No project" unbinds future turns and KEEPS the live session', async () => {
    const qc = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    useChatStore.setState({
      activeProjectId: PROJECT.id,
      activeSessionId: 'sess-shell',
      messages: [{ id: 'm1', role: 'user', content: 'hello', timestamp: 't' }],
    })
    render(<ProjectSelectorPopover activeProject={PROJECT} onSelect={() => {}} />, {
      wrapper: ({ children }: { children: ReactNode }) => (
        <QueryClientProvider client={qc}>{children}</QueryClientProvider>
      ),
    })
    await userEvent.click(screen.getByRole('button', { name: 'workbench.projectSelector.label' }))
    await userEvent.click(await screen.findByText('workbench.noProject'))
    await waitFor(() => expect(useChatStore.getState().activeProjectId).toBeNull())
    // Future-turns-only (IA design §8.3): the live conversation and its
    // transcript are never reset by a binding change.
    expect(useChatStore.getState().activeSessionId).toBe('sess-shell')
    expect(useChatStore.getState().messages).toHaveLength(1)
  })
})
