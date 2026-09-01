// Task 4.2 — StudioHandoffSheet (Add to Studio typed handoff).
//
// Covers the plan's acceptance points:
//   * projectless handoff: default target, payload carries
//     source:'knowledge' + contextRefs, no projectId.
//   * explicit chosen-project handoff: picker selection lands projectId
//     in the payload.
//   * stale source chip: unresolvable ref renders a removable
//     "unavailable" chip; payload excludes it after removal.
//   * keyboard focus return to the triggering row after close.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { HttpResponse, http } from 'msw'
import { useRef, useState } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { server } from '@/__tests__/msw/server'
import {
  type KnowledgeContextRef,
  StudioHandoffSheet,
  type StudioLaunchIntent,
} from '@/components/knowledge/studio-handoff'
import { useChatStore } from '@/stores/chat'
import type { Project } from '@/types'

const { navigateSpy } = vi.hoisted(() => ({ navigateSpy: vi.fn() }))

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}))

vi.mock('@tanstack/react-router', () => ({
  useNavigate: () => navigateSpy,
}))

vi.mock('sonner', () => ({ toast: { error: vi.fn(), info: vi.fn(), success: vi.fn() } }))

vi.mock('@/hooks/use-folder-picker', () => ({
  useFolderPicker: () => ({
    pick: vi.fn().mockResolvedValue(['/tmp/proj-root']),
    picking: false,
    error: null,
    localOnly: false,
  }),
}))

const PROJECT: Project = {
  id: 'proj_1',
  name: 'Oxios',
  root_paths: ['/tmp/oxios'],
  instructions: '',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  last_active_at: '2026-01-01T00:00:00Z',
}

function stubProjects() {
  server.use(http.get('/api/projects', () => HttpResponse.json({ items: [PROJECT], total: 1 })))
}

const DOC_REF: KnowledgeContextRef = { kind: 'knowledge-document', path: 'notes/alpha.md' }

function renderSheet(
  contextRefs: KnowledgeContextRef[],
  isAvailable: (ref: KnowledgeContextRef) => boolean = () => true,
  triggerRef?: React.RefObject<HTMLElement | null>,
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return render(
    <QueryClientProvider client={queryClient}>
      <StudioHandoffSheet
        open
        onOpenChange={() => {}}
        contextRefs={contextRefs}
        isAvailable={isAvailable}
        triggerRef={triggerRef}
      />
    </QueryClientProvider>,
  )
}

function lastIntent(): StudioLaunchIntent {
  const call = navigateSpy.mock.calls.at(-1)?.[0] as
    | { to: string; search?: { launch?: string } }
    | undefined
  expect(call?.to).toBe('/studio')
  return JSON.parse(decodeURIComponent(call!.search!.launch!)) as StudioLaunchIntent
}

describe('StudioHandoffSheet', () => {
  afterEach(() => {
    server.resetHandlers()
    navigateSpy.mockClear()
    useChatStore.setState({ activeSessionId: null, activeProjectId: null })
  })

  it('builds a projectless intent by default: source, contextRefs, no projectId', async () => {
    stubProjects()
    renderSheet([DOC_REF, { kind: 'brain-memory', id: 'ent-user-1' }])

    // Default target: projectless radio checked.
    expect(screen.getByTestId('handoff-target-projectless')).toBeChecked()

    await userEvent.click(screen.getByTestId('handoff-submit'))
    const intent = lastIntent()
    expect(intent.source).toBe('knowledge')
    expect(intent.contextRefs).toEqual([
      { kind: 'knowledge-document', path: 'notes/alpha.md' },
      { kind: 'brain-memory', id: 'ent-user-1' },
    ])
    expect(intent.projectId).toBeUndefined()
    expect(intent.sessionId).toBeUndefined()
  })

  it('lands the picker-selected projectId in the payload', async () => {
    stubProjects()
    renderSheet([DOC_REF])

    await userEvent.click(screen.getByTestId('handoff-target-project'))
    // Open the reused ProjectSelectorPopover and pick the project.
    await userEvent.click(screen.getByRole('button', { name: 'workbench.projectSelector.label' }))
    await userEvent.click(await screen.findByText(PROJECT.name))
    await userEvent.click(screen.getByTestId('handoff-submit'))

    const intent = lastIntent()
    expect(intent.projectId).toBe(PROJECT.id)
    expect(intent.source).toBe('knowledge')
    expect(intent.contextRefs).toEqual([DOC_REF])
  })

  it('renders an unresolvable ref as a removable unavailable chip and excludes it from the payload', async () => {
    stubProjects()
    const STALE: KnowledgeContextRef = { kind: 'brain-memory', id: 'ent-gone' }
    renderSheet([DOC_REF, STALE], (ref) => ref.kind === 'knowledge-document')

    // The stale ref renders an unavailable chip…
    const chip = screen.getByRole('button', {
      name: /knowledge\.workspace\.unavailableChip/,
    })
    expect(chip).toBeInTheDocument()

    // …and is excluded only after explicit removal.
    await userEvent.click(chip)
    await waitFor(() =>
      expect(
        screen.queryByRole('button', { name: /knowledge\.workspace\.unavailableChip/ }),
      ).not.toBeInTheDocument(),
    )

    await userEvent.click(screen.getByTestId('handoff-submit'))
    const intent = lastIntent()
    expect(intent.contextRefs).toEqual([DOC_REF])
  })

  it('restores focus to the triggering row after the sheet closes', async () => {
    stubProjects()
    function Harness() {
      const triggerRef = useRef<HTMLButtonElement | null>(null)
      const [open, setOpen] = useState(false)
      return (
        <QueryClientProvider
          client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
        >
          <button
            type="button"
            ref={triggerRef}
            data-testid="trigger-row"
            onClick={() => setOpen(true)}
          >
            trigger-row
          </button>
          <StudioHandoffSheet
            open={open}
            onOpenChange={setOpen}
            contextRefs={[DOC_REF]}
            isAvailable={() => true}
            triggerRef={triggerRef}
          />
        </QueryClientProvider>
      )
    }
    render(<Harness />)

    // The row holds focus BEFORE the sheet opens (real user flow) — the
    // dialog's focus trap is not mounted yet, so the focus sticks.
    const row = screen.getByTestId('trigger-row')
    await userEvent.click(row)
    expect(screen.getByTestId('handoff-target-projectless')).toBeInTheDocument()

    // Close → focus returns to the triggering row.
    await userEvent.click(
      screen.getByRole('button', { name: 'knowledge.workspace.handoff.cancel' }),
    )
    await waitFor(() => expect(row).toHaveFocus())
  })
})
