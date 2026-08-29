// Component test: RootPathsEditor — the folder-rows editor backing the
// project create/edit dialogs (design §4.2). Picker results append and
// dedupe, rows are removable, and the empty state renders its i18n key.
// The use-folder-picker hook is mocked so tests drive pick() directly.

import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { RootPathsEditor } from '@/components/project/root-paths-editor'

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}))

// Toast is invoked for duplicates; mock so tests can assert (and silence).
vi.mock('sonner', () => ({
  toast: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
}))

const mockPick = vi.fn<() => Promise<string[] | null>>()

vi.mock('@/hooks/use-folder-picker', () => ({
  useFolderPicker: () => ({
    pick: mockPick,
    picking: false,
    error: null,
    localOnly: false,
  }),
}))

describe('RootPathsEditor', () => {
  it('renders the empty-state label key when there are no folders', () => {
    render(<RootPathsEditor value={[]} onChange={() => {}} />)
    expect(screen.getByText('projects.noFoldersYet')).toBeTruthy()
  })

  it('appends picked folders and reports each row by path', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    mockPick.mockResolvedValue(['/tmp/alpha', '/tmp/beta'])
    render(<RootPathsEditor value={[]} onChange={onChange} />)

    await user.click(screen.getByRole('button', { name: 'projects.chooseFolder' }))
    expect(onChange).toHaveBeenCalledWith(['/tmp/alpha', '/tmp/beta'])
  })

  it('dedupes picked folders against existing rows', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    mockPick.mockResolvedValue(['/tmp/alpha', '/tmp/beta'])
    render(<RootPathsEditor value={['/tmp/alpha']} onChange={onChange} />)

    await user.click(screen.getByRole('button', { name: 'projects.chooseFolder' }))
    // Only the new path is appended; the duplicate is dropped.
    expect(onChange).toHaveBeenCalledWith(['/tmp/alpha', '/tmp/beta'])
  })

  it('removes a row via its remove button', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<RootPathsEditor value={['/tmp/alpha', '/tmp/beta']} onChange={onChange} />)

    const removeButtons = screen.getAllByRole('button', { name: 'projects.removeFolder' })
    // First remove button belongs to the first row (alpha).
    await user.click(removeButtons[0]!)
    expect(onChange).toHaveBeenCalledWith(['/tmp/beta'])
  })

  it('adds a manual path via the fallback input', async () => {
    const user = userEvent.setup()
    const onChange = vi.fn()
    render(<RootPathsEditor value={[]} onChange={onChange} />)

    await user.type(screen.getByLabelText('projects.pathInputLabel'), '/tmp/manual')
    await user.click(screen.getByRole('button', { name: 'common.add' }))
    expect(onChange).toHaveBeenCalledWith(['/tmp/manual'])
  })

  it('disables the picker button when disabled prop is set', () => {
    render(<RootPathsEditor value={['/tmp/alpha']} onChange={() => {}} disabled />)
    const pickButton = screen.getByRole('button', { name: 'projects.chooseFolder' })
    expect((pickButton as HTMLButtonElement).disabled).toBe(true)
  })
})
