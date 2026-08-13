/**
 * Brain daemon types (RFC-047) — the oxibrain-powered memory surface.
 *
 * The daemon returns rich nested JSON; these types model only the shapes the
 * panel consumes and stay permissive where the payload is deep.
 */

/** GET /api/brain/status */
export interface BrainStatus {
  /** Daemon reachable via Unix socket. */
  available: boolean
  /** Configured space name (null when unavailable). */
  space: string | null
  /** Episode count in the space (null when unavailable). */
  episodes: number | null
}

/** GET /api/brain/stats */
export interface BrainStats {
  episodes: number
  entities: number
  statements: number
  contradictions: number
}

/** GET /api/brain/recall — assembled context text. */
export interface BrainRecallResponse {
  context: string | null
}

/** A ranked search hit (shape from the daemon's RankingResult). */
export interface BrainSearchHit {
  target: { kind: 'episode' | 'statement' | 'entity'; id: string }
  fused_score?: number
  rank?: number
  salience?: number
}

/** GET /api/brain/search */
export interface BrainSearchResponse {
  items: BrainSearchHit[]
  total_found?: number
  dropped?: unknown[]
}

/** Permissive detail payloads — the daemon's beliefs/timeline/why JSON. */
export type EntityDetail = Record<string, unknown>
export type TimelineEntry = Record<string, unknown>
export type WhyDetail = Record<string, unknown>
export type Contradiction = Record<string, unknown>
