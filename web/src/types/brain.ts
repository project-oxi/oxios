/**
 * Brain daemon types (RFC-047) — the oxibrain-powered memory surface.
 *
 * These mirror the daemon's wire contracts (verified against a live v0.6.x
 * daemon; see docs/designs/2026-08-24-brain-tab-redesign.md). The daemon
 * serves entity/timeline/space data as MCP resources and pages/graph/console
 * data as MCP tools.
 */

export type SupervisorState =
  | 'disabled'
  | 'not_installed'
  | 'installing'
  | 'starting'
  | 'online'
  | 'failed'

export type ManagedBy = 'none' | 'launchd' | 'spawn' | 'external'

export interface BrainSupervisorStatus {
  state: SupervisorState
  installed_version: string | null
  daemon_version: string | null
  managed_by: ManagedBy
  last_error: string | null
}

/** GET /api/brain/status */
export interface BrainStatus {
  /** Daemon reachable via Unix socket. */
  available: boolean
  /** Configured space name (null when unavailable). */
  space: string | null
  /** Episode count in the space (null when unavailable). */
  episodes: number | null
  /** Optional first-party supervisor lifecycle (2026-08-19 spec). */
  supervisor?: BrainSupervisorStatus | null
}

/** GET /api/brain/stats — fields can be null when the daemon is degraded. */
export interface BrainStats {
  episodes: number | null
  entities: number | null
  statements: number | null
  contradictions: number | null
}

/** GET /api/brain/recall — assembled context text. */
export interface BrainRecallResponse {
  context: string | null
}

/**
 * GET /api/brain/search — a bare array of entity hits (daemon contract).
 * The search tool targets entities only; as_of/known_at/min_confidence
 * filter beliefs server-side.
 */
export interface BrainSearchHit {
  entity_id: string
  entity_surface: string
  entity_type: string
  score: number
  snippet: string
}

/** Mirrors `oxibrain_core::knowledge::Belief` (entity:// resource). */
export interface Belief {
  statement: string
  valid_from: number
  valid_to: number
  support: BeliefSupport
  confidence: number
  status: 'active' | 'superseded' | 'contradicted' | 'retracted'
}

export interface BeliefSupport {
  affirm_count: number
  deny_count: number
  distinct_episodes: number
  /** Serde tuples as arrays: ["trusted", 2]. */
  trust_weights: Array<[string, number]>
}

/** Mirrors `oxibrain_store::timeline::TimelineEntry` (timeline:// resource). */
export interface TimelineEntry {
  statement_id: string
  predicate: string
  object_repr: string
  object_entity: string | null
  valid_from: number
  valid_to: number
  status: string
  recorded_at: number
}

/** Mirrors `oxibrain_store::query::ContradictionDetail` (contradictions tool). */
export interface ContradictionDetail {
  statement_id: string
  subject_id: string
  subject_surface: string
  subject_type: string
  predicate: string
  object_kind: 'entity' | 'literal'
  object_value: string
  affirm_episodes: string[]
  deny_episodes: string[]
}

export interface AssertionDetail {
  assertion_id: string
  episode_id: string
  extractor: string | null
  polarity: string
  confidence: number
  recorded_at: number
  /** Verbatim subject-mention surface — absent on legacy rows. */
  mention?: string
}

/** Mirrors the `why` tool's statement projection. */
export interface ExplainBlock {
  statement: {
    id: string
    space: string
    subject: string
    predicate: string
    object: { id?: string; kind?: string; surface?: string } | unknown
  }
  status: string
  assertions: AssertionDetail[]
  confidence_breakdown: {
    raw_confidence: number
    support_count: number
    contradiction_count: number
  }
}

/** Mirrors `oxibrain_core::knowledge::EntityMerge` (review_merges section=merges). */
export interface MergeRecord {
  id: string
  loser: string
  winner: string
  /** Tagged enum: {kind: rule|user|import, data?}. */
  decided_by: { kind: string; data?: unknown }
  provenance: string
  evidence: string[]
  decided_at: number
  undone_at: number | null
}

/** Mirrors `oxibrain_store::quarantine::ExtractionFailure` (section=failures). */
export interface ExtractionFailure {
  id: number
  episode_id: string
  extractor_id: string
  raw_response: string
  errors_json: string
  created_at: number
}

/** Mirrors `oxibrain_store::ledger::SourceRow` (section=sources). */
export interface SourceRow {
  id: string
  space: string
  name: string
  kind: string
  mode: string
  claims_json: string
  created_at: number
}

/** Mirrors `oxibrain_core::retrieval::TraversalNode` (traverse/graph). */
export interface GraphNode {
  entity: string
  depth: number
  salience: number
}

/** Mirrors `oxibrain_core::retrieval::TraversalEdge`. */
export interface GraphEdge {
  from: string
  to: string
  predicate: string
  statement_id: string
  depth: number
}

export interface TraversalResult {
  nodes: GraphNode[]
  edges: GraphEdge[]
  truncated: boolean
}

/** One space from `spaces/list`. */
export interface SpaceSummary {
  id: string
  name: string
  created_at: number
  entity_count: number
  episode_count: number
}

export interface EntityCard {
  id: string
  surface: string
  type: string
}

/** `space://{name}` resource — configured-space overview. */
export interface SpaceOverview {
  space: string
  space_id: string
  entity_count: number
  episode_count: number
  contradiction_count: number
  recent_entities: EntityCard[]
}

/** GET /api/brain/brief — rendered page markdown (null when degraded). */
export interface BrainBriefResponse {
  markdown: string | null
}

export type OperationsSection = 'merges' | 'failures' | 'sources'

export type RetrievalMode = 'hybrid' | 'lexical' | 'lexical-vector' | 'graph' | 'community'

/**
 * Valid-time sentinels: the daemon uses ±(i64::MIN+1 / i64::MAX-1)-scale
 * values to mean "always". Anything beyond 100,000 days in milliseconds is
 * a sentinel, not a real timestamp.
 */
export function isSentinelTime(ms: number): boolean {
  return Math.abs(ms) > 8.64e15
}
