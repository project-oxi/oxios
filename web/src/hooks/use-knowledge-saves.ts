import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { api } from '@/lib/api-client'

export interface KnowledgeSaveRecord {
  message_index: number
  knowledge_path: string
  saved_at: string
  source: 'hook' | 'user' | 'tool'
}

export interface KnowledgeSavesResponse {
  saves: KnowledgeSaveRecord[]
}

export interface SaveResult {
  path: string
}

export interface SaveError {
  error: string
  path: string
}

export interface DeleteResult {
  deleted_path: string
}

/** Load all knowledge save records for a session. */
export function useKnowledgeSaves(sessionId: string | null) {
  return useQuery({
    queryKey: ['chat', 'knowledge-saves', sessionId],
    queryFn: () =>
      api.get<KnowledgeSavesResponse>(
        `/api/chat/${encodeURIComponent(sessionId!)}/knowledge-saves`,
      ),
    enabled: !!sessionId,
  })
}

/** Save an assistant message to the knowledge vault. */
export function useSaveToKnowledge(sessionId: string | null) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ messageIndex, path }: { messageIndex: number; path?: string }) =>
      api.post<SaveResult | SaveError>(
        `/api/chat/${encodeURIComponent(sessionId!)}/messages/${messageIndex}/save-to-knowledge`,
        // Always a JSON object: the axum `Json<SaveToKnowledgeRequest>` extractor
        // answers a bodyless POST (headers-only) with 400.
        path ? { path } : {},
      ),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['chat', 'knowledge-saves', sessionId] })
      qc.invalidateQueries({ queryKey: ['knowledge', 'tree'] })
    },
  })
}

/** Remove a knowledge save (delete the note). */
export function useRemoveKnowledgeSave(sessionId: string | null) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (messageIndex: number) =>
      api.delete<DeleteResult>(
        `/api/chat/${encodeURIComponent(sessionId!)}/messages/${messageIndex}/knowledge-save`,
      ),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['chat', 'knowledge-saves', sessionId] })
      qc.invalidateQueries({ queryKey: ['knowledge', 'tree'] })
    },
  })
}
