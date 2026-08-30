// Component test: chat brain picker — default resolution formula, pill
// rendering states, roster interaction, and container visibility against
// the msw brain-spaces roster.

import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { HttpResponse, http } from 'msw'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { BrainPicker, BrainPickerContainer } from '@/components/chat/brain-picker'
import { resolveBrainSpace } from '@/lib/brain-binding'
import { useChatStore } from '@/stores/chat'
import type { SpaceSummary } from '@/types/brain'
import { server } from '../../msw/server'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) =>
      opts && 'name' in opts ? `${key}:${String(opts.name)}` : key,
    i18n: { language: 'en' },
  }),
}))

// ids deliberately differ from names — the chat binding is the NAME
// (oxibrain ops address spaces by name), never the id.
const spaces: SpaceSummary[] = [
  { id: 'space-uuid-1', name: 'Personal', created_at: 0, entity_count: 0, episode_count: 0 },
  { id: 'space-uuid-2', name: 'Work', created_at: 0, entity_count: 0, episode_count: 0 },
]

function Wrapper({ children }: { children: React.ReactNode }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, refetchInterval: false } },
  })
  return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
}

function renderPicker(props: Partial<Parameters<typeof BrainPicker>[0]> = {}) {
  return render(
    <Wrapper>
      <BrainPicker
        spaces={spaces}
        activeBrainSpace={null}
        setActiveBrainSpace={vi.fn()}
        {...props}
      />
    </Wrapper>,
  )
}

// ─── resolveBrainSpace (Task 9 consumes the resolved value) ───

describe('resolveBrainSpace', () => {
  it('prefers the explicit session selection over any default', () => {
    expect(resolveBrainSpace('work', 'project-space', 'global-space')).toBe('work')
  })

  it('falls back to the project default when nothing is picked', () => {
    expect(resolveBrainSpace(null, 'project-space', 'global-space')).toBe('project-space')
  })

  it('falls back to the global default when the project default is missing', () => {
    expect(resolveBrainSpace(null, null, 'global-space')).toBe('global-space')
    expect(resolveBrainSpace(null, undefined, 'global-space')).toBe('global-space')
  })

  it('resolves to null (unconnected) when no layer provides a space', () => {
    expect(resolveBrainSpace(null, null, '')).toBeNull()
    expect(resolveBrainSpace(null, undefined, '')).toBeNull()
  })

  it('treats an empty global default as no default', () => {
    expect(resolveBrainSpace(null, null, '')).toBeNull()
  })
})

// ─── BrainPicker ───

describe('BrainPicker', () => {
  it('shows the off state with no selection', () => {
    renderPicker()
    expect(screen.getByRole('button', { name: 'chat.brain.label' })).toHaveTextContent(
      'chat.brain.toolsOff',
    )
  })

  it('shows the bound space name when a selection exists', () => {
    renderPicker({ activeBrainSpace: 'Work' })
    expect(screen.getByRole('button', { name: 'chat.brain.label' })).toHaveTextContent('Work')
  })

  it('displays the resolved default (name + suffix) when only defaults bind', () => {
    renderPicker({ activeBrainSpace: null, projectDefault: 'Work', globalDefault: '' })
    const pill = screen.getByRole('button', { name: 'chat.brain.label' })
    expect(pill).toHaveTextContent('Work')
    expect(pill).toHaveTextContent('chat.brain.defaultSuffix')
  })

  it('checks the resolved space row in the dropdown when inherited', () => {
    renderPicker({ activeBrainSpace: null, projectDefault: 'Work', globalDefault: '' })
    fireEvent.click(screen.getByRole('button', { name: 'chat.brain.label' }))
    // The pill also shows the resolved name — assert on the dropdown row,
    // identified by its checked (bg-accent/60) styling.
    const checkedRow = screen
      .getAllByText('Work')
      .map((el) => el.closest('button'))
      .find((r) => r?.className.includes('bg-accent/60'))
    expect(checkedRow).toBeDefined()
  })

  it('does not suffix an explicit selection', () => {
    renderPicker({ activeBrainSpace: 'Work', projectDefault: 'Personal' })
    expect(screen.getByRole('button', { name: 'chat.brain.label' })).not.toHaveTextContent(
      'chat.brain.defaultSuffix',
    )
  })

  it('lists the none row + roster and selects a space by NAME', () => {
    const setActive = vi.fn()
    renderPicker({ setActiveBrainSpace: setActive })

    fireEvent.click(screen.getByRole('button', { name: 'chat.brain.label' }))
    expect(screen.getByText('chat.brain.none')).toBeInTheDocument()

    fireEvent.click(screen.getByText('Work'))
    expect(setActive).toHaveBeenCalledWith('Work')
  })

  it('unbinds via the none row', () => {
    const setActive = vi.fn()
    renderPicker({ activeBrainSpace: 'Work', setActiveBrainSpace: setActive })

    fireEvent.click(screen.getByRole('button', { name: 'chat.brain.label' }))
    fireEvent.click(screen.getByText('chat.brain.none'))
    expect(setActive).toHaveBeenCalledWith(null)
  })
})

// ─── BrainPickerContainer ───

describe('BrainPickerContainer', () => {
  beforeEach(() => {
    localStorage.clear()
    useChatStore.setState({
      activeBrainSpace: null,
      brainDefaults: { project: null, global: '' },
    })
  })
  afterEach(() => server.resetHandlers())

  it('renders nothing when the brain exposes no spaces (default msw: [])', async () => {
    const { container } = render(
      <Wrapper>
        <BrainPickerContainer />
      </Wrapper>,
    )
    await waitFor(() => expect(container).toBeEmptyDOMElement())
  })

  it('renders the pill bound to the store selection', async () => {
    server.use(http.get('/api/brain/spaces', () => HttpResponse.json(spaces)))
    useChatStore.setState({ activeBrainSpace: 'Work' })
    render(
      <Wrapper>
        <BrainPickerContainer />
      </Wrapper>,
    )
    const pill = await screen.findByRole('button', { name: 'chat.brain.label' })
    expect(pill).toHaveTextContent('Work')
  })

  it('normalizes a stale binding (not on the roster) to the off state', async () => {
    server.use(http.get('/api/brain/spaces', () => HttpResponse.json(spaces)))
    useChatStore.setState({ activeBrainSpace: 'Ghost' })
    render(
      <Wrapper>
        <BrainPickerContainer />
      </Wrapper>,
    )
    const pill = await screen.findByRole('button', { name: 'chat.brain.label' })
    expect(pill).toHaveTextContent('chat.brain.toolsOff')
    expect(pill).not.toHaveTextContent('Ghost')
  })

  it('marks an inherited project default from the store brainDefaults', async () => {
    server.use(http.get('/api/brain/spaces', () => HttpResponse.json(spaces)))
    useChatStore.setState({
      activeBrainSpace: null,
      brainDefaults: { project: 'Work', global: '' },
    })
    render(
      <Wrapper>
        <BrainPickerContainer />
      </Wrapper>,
    )
    const pill = await screen.findByRole('button', { name: 'chat.brain.label' })
    expect(pill).toHaveTextContent('Work')
    expect(pill).toHaveTextContent('chat.brain.defaultSuffix')
  })

  it('falls back to the global default when the project has none', async () => {
    server.use(http.get('/api/brain/spaces', () => HttpResponse.json(spaces)))
    useChatStore.setState({
      activeBrainSpace: null,
      brainDefaults: { project: null, global: 'Personal' },
    })
    render(
      <Wrapper>
        <BrainPickerContainer />
      </Wrapper>,
    )
    const pill = await screen.findByRole('button', { name: 'chat.brain.label' })
    expect(pill).toHaveTextContent('Personal')
    expect(pill).toHaveTextContent('chat.brain.defaultSuffix')
  })
})
