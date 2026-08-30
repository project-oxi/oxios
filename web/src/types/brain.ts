/**
 * Brain types (oxibrain 0.8 — daemonless session model).
 *
 * Status and search contracts mirror the wire shapes emitted by the
 * oxios-kernel handlers in `src/api/routes/workspace.rs`. The supervisor
 * lifecycle is gone: `available` flips on a real brain session, while
 * `binary.installed` reports the bundled oxibrain CLI on disk. See
 * docs/designs/2026-08-27-daemonless-brain-design.md.
 */

export interface BrainBinaryStatus {
  installed: boolean
  path: string | null
  version: string | null
}

/** GET /api/brain/status */
export interface BrainStatus {
  available: boolean
  pending_extraction: number | null
  binary: BrainBinaryStatus
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
 * GET /api/brain/search — memory-plane entity hit (engine shape, deserialized
 * straight from the two-plane envelope's `memory` field).
 */
export interface BrainSearchHit {
  entity_id: string
  entity_surface: string
  entity_type: string
  score: number
  snippet: string
}

/** Documents-plane hit in the two-plane search envelope. */
export interface DocumentHit {
  document_id: string
  root: string
  locator: string
  revision: string
  ordinal: number
  text: string
  /** Millis since epoch; wire key is `modified_at`. */
  modified_at: number
  score: number
}

export interface DocumentFreshness {
  reconciled_roots: string[]
  skipped_roots: [string, string][]
  skipped_files: number
  stale_after_retry: string[]
  dense_coverage: number | null
}

/** GET /api/brain/search — two-plane envelope (oxibrain 0.8). */
export interface SearchResponse {
  memory: BrainSearchHit[]
  documents: DocumentHit[]
  freshness: DocumentFreshness
}

/** GET /api/brain/document-history — gix-backed revisions, oldest first. */
export interface DocumentRevision {
  revision: string
  committed_at_ms: number
  content: string
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
