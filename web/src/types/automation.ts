// Automation types — mirror the Rust wire shape (oxios-kernel Automation
// struct, see crates/oxios-kernel/src/automation/model.rs).
//
// Wire shape rules:
// - All field names are camelCase (the Rust struct uses #[serde(rename_all =
//   "camelCase")]). Nested fields follow the same rule; the AutomationContextSnapshot
//   is captured and frozen when a run opens and is exposed verbatim to the UI.
// - Timestamps are RFC 3339 strings (`2026-08-31T00:00:00+00:00`).
// - Optional bindings (project_id, persona_id, brain_space) are `string | null`
//   with NO empty-string sentinel: projectless means `null`, never `""`.
// - Trigger-specific fields are validated by the server (cron pattern required
//   for cron; heartbeat_interval_secs required for heartbeat; neither for
//   manual). The wire shape carries them as `Option<…>` / `string | null` so
//   the client can render pre-flight errors instead of round-tripping.
//
// The legacy Task* names from the prior task-management domain are gone:
// identifier, priority, sortOrder, parentTaskId, assignee/createdBy, the
// `dependencies[]` array, comments, and the batch-create hook were dropped
// at the IA cutover. There are intentionally no aliases for the old names.

/** Lifecycle state of an automation definition (spec §8.7). */
export type AutomationStatus = 'active' | 'paused' | 'exhausted' | 'failed'

/** What starts an automation. */
export type AutomationTrigger = 'manual' | 'cron' | 'heartbeat'

/** What started a specific run (same value set as AutomationTrigger; kept
 * distinct so a run's trigger survives later edits to the definition). */
export type AutomationRunTrigger = 'manual' | 'cron' | 'heartbeat'

/** Terminal state of a single run. */
export type AutomationRunStatus = 'running' | 'succeeded' | 'failed' | 'canceled'

/**
 * Verify-gate configuration. Mirrors AutomationVerifyConfig in the kernel.
 * `enabled: false` short-circuits the gate; when true the runner asks a
 * verifier pass to confirm the result satisfies `requirement` (defaulting
 * to the instruction) up to `max_iterations` times.
 */
export interface AutomationVerifyConfig {
  enabled: boolean
  requirement?: string | null
  maxIterations: number
}

/**
 * Immutable execution context captured when a run opens. The runner reads
 * this snapshot for the whole execution; it MUST NOT reload a changed
 * Automation mid-run. Rendered verbatim to the UI as the run's "context"
 * so operators can see exactly which persona/project/brain the run was
 * bound to when it started.
 */
export interface AutomationContextSnapshot {
  personaId?: string | null
  projectId?: string | null
  brainSpace?: string | null
  trigger: AutomationRunTrigger
  verify: AutomationVerifyConfig
  instruction: string
}

/** An automation definition. */
export interface Automation {
  id: string
  name: string
  instruction: string
  description?: string | null
  trigger: AutomationTrigger
  cronPattern?: string | null
  timezone?: string | null
  heartbeatIntervalSecs?: number | null
  maxExecutions?: number | null
  executionCount: number
  personaId?: string | null
  projectId?: string | null
  brainSpace?: string | null
  verify: AutomationVerifyConfig
  status: AutomationStatus
  nextRunAt?: string | null
  lastRunAt?: string | null
  lastError?: string | null
  createdAt: string
  updatedAt: string
}

/** One recorded execution of an automation. */
export interface AutomationRun {
  id: string
  automationId: string
  sessionId?: string | null
  trigger: AutomationRunTrigger
  status: AutomationRunStatus
  /** Captured at run start; immutable for the lifetime of the run. */
  contextSnapshot: AutomationContextSnapshot
  summary?: string | null
  resultContent?: string | null
  error?: string | null
  costUsd?: number | null
  tokensUsed?: number | null
  startedAt: string
  completedAt?: string | null
}

// ── Params ──

/** Create an automation. */
export interface CreateAutomationParams {
  name: string
  instruction: string
  description?: string | null
  trigger?: AutomationTrigger
  cronPattern?: string | null
  timezone?: string | null
  heartbeatIntervalSecs?: number | null
  maxExecutions?: number | null
  personaId?: string | null
  projectId?: string | null
  brainSpace?: string | null
  verify?: AutomationVerifyConfig
}

/** List filter — undefined fields are unfiltered. */
export interface ListAutomationsParams {
  status?: AutomationStatus
  limit?: number
  offset?: number
}

/** Partial definition update — undefined fields are left unchanged. */
export interface UpdateAutomationParams {
  name?: string
  description?: string | null
  instruction?: string
  personaId?: string | null
  projectId?: string | null
  brainSpace?: string | null
}

/** Replace the trigger configuration and (re)arm scheduling. */
export interface SetTriggerParams {
  trigger: AutomationTrigger
  cronPattern?: string | null
  timezone?: string | null
  heartbeatIntervalSecs?: number | null
  maxExecutions?: number | null
}

/** Partial verify-gate update — undefined fields keep their current value. */
export interface SetVerifyParams {
  enabled?: boolean | null
  requirement?: string | null
  maxIterations?: number | null
}

// ── Status metadata ──
// `label` holds an i18n KEY resolved via t() at render sites (the page and
// dialogs translate it) so status copy is bilingual. relativeTime strings
// stay hardcoded for now — same convention as the old TASK_STATUS_META.
export const AUTOMATION_STATUS_META: Record<
  AutomationStatus,
  { label: string; color: string; bgColor: string }
> = {
  active: {
    label: 'automations.status.active',
    color: 'text-status-success-on-surface',
    bgColor: 'bg-status-success/10',
  },
  paused: {
    label: 'automations.status.paused',
    color: 'text-hue-purple',
    bgColor: 'bg-hue-purple/10',
  },
  exhausted: {
    label: 'automations.status.exhausted',
    color: 'text-muted-foreground',
    bgColor: 'bg-muted',
  },
  failed: {
    label: 'automations.status.failed',
    color: 'text-status-error-on-surface',
    bgColor: 'bg-status-error/10',
  },
}

export const AUTOMATION_STATUSES: AutomationStatus[] = ['active', 'paused', 'exhausted', 'failed']

export const AUTOMATION_TRIGGERS: AutomationTrigger[] = ['manual', 'cron', 'heartbeat']
