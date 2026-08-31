import { fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { AttentionQueue } from '@/components/dashboard/command-center/attention-queue'
import type { Approval } from '@/types'

const mockMutate = vi.fn()
let pendingItems: Approval[] = []
let isLoading = false
let isError = false
const mockRefetch = vi.fn()
let approvePending = false

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}))

vi.mock('@/hooks/use-approvals', () => ({
  usePendingApprovals: () => ({ items: pendingItems, isLoading, isError, refetch: mockRefetch }),
  useApproveApproval: () => ({ mutate: mockMutate, isPending: approvePending }),
  useRejectApproval: () => ({ mutate: mockMutate, isPending: approvePending }),
}))

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}))

function approval(overrides: Partial<Approval> = {}): Approval {
  return {
    id: 'apr_1',
    subject: 'agent_1',
    action: 'bash.exec',
    resource: 'rm -rf /tmp/build',
    reason: 'destructive command',
    created_at: '2026-08-31T05:00:00Z',
    status: 'pending',
    ...overrides,
  }
}

beforeEach(() => {
  mockMutate.mockClear()
  mockRefetch.mockClear()
  pendingItems = []
  isLoading = false
  isError = false
  approvePending = false
})

describe('AttentionQueue', () => {
  it('renders nothing when there are no pending approvals and no error', () => {
    const { container } = render(<AttentionQueue />)
    expect(container).toBeEmptyDOMElement()
  })

  it('renders first-class rows with action, resource, risk, and age', () => {
    pendingItems = [
      approval(),
      approval({
        id: 'apr_2',
        action: 'fs.write',
        resource: '/etc/hosts',
        reason: 'outside sandbox',
      }),
    ]
    render(<AttentionQueue />)
    expect(screen.getByText('bash.exec')).toBeInTheDocument()
    expect(screen.getByText('rm -rf /tmp/build')).toBeInTheDocument()
    expect(screen.getByText(/destructive command/)).toBeInTheDocument()
    expect(screen.getByText('fs.write')).toBeInTheDocument()
    // Approve/Deny per row: 2 rows × 2 buttons, all explicitly labeled.
    expect(screen.getAllByRole('button', { name: 'approvals.approve' })).toHaveLength(2)
    expect(screen.getAllByRole('button', { name: 'approvals.deny' })).toHaveLength(2)
  })

  it('dispatches the approve mutation for the clicked row', () => {
    pendingItems = [approval(), approval({ id: 'apr_2' })]
    render(<AttentionQueue />)
    fireEvent.click(screen.getAllByRole('button', { name: 'approvals.approve' })[1]!)
    expect(mockMutate).toHaveBeenCalledWith('apr_2', expect.anything())
  })

  it('dispatches the deny mutation for the clicked row', () => {
    pendingItems = [approval()]
    render(<AttentionQueue />)
    fireEvent.click(screen.getByRole('button', { name: 'approvals.deny' }))
    expect(mockMutate).toHaveBeenCalledWith('apr_1', expect.anything())
  })
  it('disables duplicate submissions while a decision is in flight', () => {
    pendingItems = [approval()]
    render(<AttentionQueue />)
    const approveBtn = screen.getByRole('button', { name: 'approvals.approve' })
    fireEvent.click(approveBtn)
    // busyId is set synchronously on click and cleared onSettled.
    expect(approveBtn).toBeDisabled()
    expect(screen.getByRole('button', { name: 'approvals.deny' })).toBeDisabled()
  })

  it('localizes a failed approvals query to this panel with retry', () => {
    isError = true
    render(<AttentionQueue />)
    const retry = screen.getByRole('button', { name: 'commandCenter.picture.retry' })
    fireEvent.click(retry)
    expect(mockRefetch).toHaveBeenCalled()
  })
})
