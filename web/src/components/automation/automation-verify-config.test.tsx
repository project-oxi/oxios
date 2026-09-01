// AutomationVerifyConfig — verifies the toggle + save round-trip commits a merged
// SetVerifyParams payload.
//
// The component stages edits in local state; on Save it calls `onSave` with
// every field from the staged state, so the store receives a complete
// snapshot. The test pins that contract: flipping the enabled toggle and
// editing the requirement/maxIterations must produce a single merged payload.

import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { AutomationVerifyConfig } from './automation-verify-config'

describe('AutomationVerifyConfig', () => {
  it('renders the verify title and current requirement', () => {
    render(
      <AutomationVerifyConfig
        enabled
        requirement="Must include 3 pairings"
        maxIterations={3}
        onSave={vi.fn()}
      />,
    )
    expect(screen.getByText('automations.verifyTitle')).toBeInTheDocument()
    expect(screen.getByDisplayValue('Must include 3 pairings')).toBeInTheDocument()
    expect(screen.getByDisplayValue('3')).toBeInTheDocument()
  })

  it('toggles enabled and saves with the merged payload', () => {
    const onSave = vi.fn()
    render(
      <AutomationVerifyConfig enabled={false} requirement="" maxIterations={2} onSave={onSave} />,
    )
    // Flip the switch on.
    const toggle = screen.getByRole('switch')
    fireEvent.click(toggle)
    // Fill in the criterion.
    const textarea = screen.getByPlaceholderText('automations.verifyRequirementPlaceholder')
    fireEvent.change(textarea, { target: { value: 'returns a JSON object' } })
    // Bump max iterations.
    const iterations = screen.getByLabelText('automations.verifyMaxIterations')
    fireEvent.change(iterations, { target: { value: '5' } })
    // Save.
    fireEvent.click(screen.getByRole('button', { name: 'automations.verifySave' }))
    expect(onSave).toHaveBeenCalledTimes(1)
    expect(onSave).toHaveBeenCalledWith({
      enabled: true,
      requirement: 'returns a JSON object',
      maxIterations: 5,
    })
  })

  it('trims the requirement and sends null when empty', () => {
    const onSave = vi.fn()
    render(<AutomationVerifyConfig enabled requirement="old" maxIterations={3} onSave={onSave} />)
    const textarea = screen.getByPlaceholderText('automations.verifyRequirementPlaceholder')
    fireEvent.change(textarea, { target: { value: '   ' } })
    fireEvent.click(screen.getByRole('button', { name: 'automations.verifySave' }))
    expect(onSave).toHaveBeenCalledWith({
      enabled: true,
      requirement: null,
      maxIterations: 3,
    })
  })
})
