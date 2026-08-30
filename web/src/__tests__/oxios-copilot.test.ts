import { describe, expect, it } from 'vitest'
import {
  buildCopilotMessagePayload,
  buildSeedRequestBody,
  COPILOT_PERSONA_ID,
} from '@/stores/oxios-copilot'
import type { ChatBlock } from '@/types'

describe('COPILOT_PERSONA_ID', () => {
  it('is locked to oxios', () => {
    expect(COPILOT_PERSONA_ID).toBe('oxios')
  })
})

describe('buildCopilotMessagePayload', () => {
  it('locks persona to oxios and stays ephemeral', () => {
    expect(buildCopilotMessagePayload('hi', 'openai/gpt-x')).toEqual({
      type: 'message',
      content: 'hi',
      ephemeral: true,
      persona_id: 'oxios',
      model: 'openai/gpt-x',
    })
  })

  it('sends empty model string when none selected', () => {
    expect(buildCopilotMessagePayload('hi', null).model).toBe('')
  })
})

describe('buildSeedRequestBody', () => {
  it('carries persona_id oxios with the captured exchange', () => {
    const blocks = [
      { type: 'reasoning', text: 'why' },
      {
        type: 'tool',
        apiName: 'mcp_manage',
        arguments: { action: 'list' },
        result: 'ok',
        durationMs: 5,
      },
    ] as unknown as ChatBlock[]
    const body = buildSeedRequestBody({
      prompt: 'q',
      reply: 'a',
      blocks,
      model: 'openai/gpt-x',
      sessionId: undefined,
    })
    expect(body).toMatchObject({
      user_message: 'q',
      agent_response: 'a',
      persona_id: 'oxios',
      reasoning_text: 'why',
    })
  })

  it('maps tool blocks into trajectory_steps with duration_ms', () => {
    const blocks = [
      {
        type: 'tool',
        apiName: 'mcp_manage',
        arguments: { action: 'list' },
        result: 'ok',
        durationMs: 5,
      },
    ] as unknown as ChatBlock[]
    const body = buildSeedRequestBody({
      prompt: 'q',
      reply: 'a',
      blocks,
      model: 'openai/gpt-x',
      sessionId: undefined,
    })
    expect(body.trajectory_steps).toEqual([
      {
        tool: 'mcp_manage',
        input: { action: 'list' },
        output: 'ok',
        duration_ms: 5,
      },
    ])
  })
})
