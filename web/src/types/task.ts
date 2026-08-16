// Task types (ported from LobeHub builtin-tool-task)
// Matches the Rust Task struct in crates/oxios-kernel/src/task/model.rs

export type TaskStatus =
  | 'backlog'
  | 'scheduled'
  | 'running'
  | 'paused'
  | 'completed'
  | 'failed'
  | 'canceled'

export type TaskAutomationMode = 'schedule' | 'heartbeat'

export type TaskRunTrigger = 'manual' | 'schedule' | 'heartbeat'

export interface Task {
  id: string
  identifier: string
  name: string
  description?: string
  instruction: string
  status: TaskStatus
  priority: number
  sortOrder?: number
  parentTaskId?: string
  assigneeAgentId?: string
  createdByAgentId?: string
  createdBySessionId?: string
  automationMode?: TaskAutomationMode | null
  schedulePattern?: string | null
  scheduleTimezone?: string | null
  heartbeatIntervalSecs?: number | null
  maxExecutions?: number | null
  executionCount: number
  verifyEnabled: boolean
  verifyRequirement?: string
  verifyMaxIterations: number
  verifyVerifierAgentId?: string
  createdAt: string
  updatedAt: string
  startedAt?: string
  completedAt?: string
  lastRunAt?: string
  nextRunAt?: string
  lastError?: string
  consecutiveFailures: number
  dependencies: string[]
}
// ── Update (partial edit) ──

export interface UpdateTaskParams {
  name?: string
  description?: string
  instruction?: string
  priority?: number
  sortOrder?: number
  parentTaskId?: string
  assigneeAgentId?: string
}

// ── Migration report ──

export interface CronMigrationSkipped {
  name: string
  reason: string
}

export interface CronMigrationReport {
  created: Task[]
  skipped: CronMigrationSkipped[]
}

// ── Comment author + mutations ──

export interface AddCommentParams {
  content: string
  authorAgentId?: string
}

export interface AddDependencyParams {
  dependsOnTaskId: string
}

export interface MigrateCronParams {
  dryRun?: boolean
}

export interface TaskComment {
  id: string
  taskId: string
  content: string
  authorAgentId?: string
  createdAt: string
  updatedAt?: string
}

export interface TaskRun {
  id: string
  taskId: string
  sessionId?: string
  trigger: TaskRunTrigger
  status: string
  summary?: string
  resultContent?: string
  startedAt: string
  completedAt?: string
  error?: string
  costUsd?: number
  tokensUsed?: number
}

// ── Params ──

export interface CreateTaskParams {
  name: string
  instruction: string
  identifier?: string
  description?: string
  priority?: number
  parentTaskId?: string
  assigneeAgentId?: string
}

export interface ListTasksParams {
  statuses?: TaskStatus[]
  assigneeAgentId?: string
  parentTaskId?: string
  limit?: number
  offset?: number
}

export interface SetScheduleParams {
  automationMode?: TaskAutomationMode | null
  schedulePattern?: string | null
  scheduleTimezone?: string | null
  heartbeatIntervalSecs?: number
  maxExecutions?: number | null
}

export interface SetVerifyParams {
  enabled?: boolean | null
  requirement?: string | null
  maxIterations?: number | null
  verifierAgentId?: string | null
}

// ── Status metadata ──

export const TASK_STATUS_META: Record<
  TaskStatus,
  { label: string; color: string; bgColor: string }
> = {
  backlog: { label: 'Backlog', color: 'text-muted-foreground', bgColor: 'bg-muted' },
  scheduled: {
    label: 'Scheduled',
    color: 'text-status-info-on-surface',
    bgColor: 'bg-status-info/10',
  },
  running: {
    label: 'Running',
    color: 'text-status-warning-on-surface',
    bgColor: 'bg-status-warning/10',
  },
  paused: { label: 'Paused', color: 'text-hue-purple', bgColor: 'bg-hue-purple/10' },
  completed: {
    label: 'Completed',
    color: 'text-status-success-on-surface',
    bgColor: 'bg-status-success/10',
  },
  failed: { label: 'Failed', color: 'text-status-error-on-surface', bgColor: 'bg-status-error/10' },
  canceled: { label: 'Canceled', color: 'text-muted-foreground', bgColor: 'bg-muted' },
}

export const TASK_STATUSES: TaskStatus[] = [
  'backlog',
  'scheduled',
  'running',
  'paused',
  'completed',
  'failed',
  'canceled',
]
