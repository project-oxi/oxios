import { useAuthStore } from '@/stores/auth'

const API_BASE = import.meta.env.VITE_API_BASE || ''

export class ApiError extends Error {
  constructor(
    public status: number,
    public statusText: string,
    public body?: string,
  ) {
    super(`API Error ${status}: ${statusText}`)
    this.name = 'ApiError'
  }
}
/** A 503 "subsystem not available" — calendar/email/skills.sh/etc. is
 * disabled in config (no provider credentials, no API key, etc).
 * It is a permanent failure and must not be retried. We match the status
 * code rather than the body so new gated endpoints slot in without
 * updating this matcher. */
export function isSubsystemUnavailable(error: unknown): boolean {
  return Boolean(error instanceof ApiError && error.status === 503)
}

interface RequestOptions {
  method?: string
  body?: unknown
  params?: Record<string, string>
  headers?: Record<string, string>
  /** Skip JSON encoding — send body as-is (for file PUT with raw markdown) */
  rawBody?: boolean
  /** body is a FormData — let the browser set the multipart Content-Type */
  formData?: boolean
}

export async function apiClient<T>(path: string, options?: RequestOptions): Promise<T> {
  const url = new URL(`${API_BASE}${path}`, window.location.origin)
  if (options?.params) {
    for (const [k, v] of Object.entries(options.params)) {
      url.searchParams.set(k, v)
    }
  }
  const token = useAuthStore.getState().token
  const isRawBody = options?.rawBody === true
  const isFormData = options?.formData === true
  const res = await fetch(url.toString(), {
    method: options?.method ?? 'GET',
    headers: {
      // FormData: the browser must set Content-Type (with boundary) itself.
      ...(isFormData ? {} : { 'Content-Type': isRawBody ? 'text/markdown' : 'application/json' }),
      ...(token ? { Authorization: `Bearer ${token}` } : {}),
      ...options?.headers,
    },
    body: isFormData
      ? ((options.body as FormData) ?? undefined)
      : isRawBody
        ? ((options.body as string) ?? undefined)
        : options?.body
          ? JSON.stringify(options.body)
          : undefined,
  })

  if (!res.ok) {
    // F5: On 401 the token is expired/invalid — log out automatically so the
    // user is routed back to authentication instead of seeing error states on
    // every subsequent request.
    if (res.status === 401) {
      useAuthStore.getState().logout()
    }
    const text = await res.text().catch(() => '')
    throw new ApiError(res.status, res.statusText, text)
  }

  if (res.status === 204) return undefined as T
  const contentType = res.headers.get('content-type') ?? ''
  if (contentType.includes('application/json')) {
    return res.json() as Promise<T>
  }
  if (contentType.includes('text/') || contentType.includes('application/toml')) {
    return res.text() as Promise<T>
  }
  return res.json() as Promise<T>
}

export const api = {
  get: <T>(path: string, params?: Record<string, string>) => apiClient<T>(path, { params }),
  post: <T>(path: string, body?: unknown) => apiClient<T>(path, { method: 'POST', body }),
  put: <T>(path: string, body?: unknown, rawBody?: boolean, headers?: Record<string, string>) =>
    apiClient<T>(path, { method: 'PUT', body, rawBody, headers }),
  /** GET that also returns the ETag response header (S-2 optimistic concurrency). */
  getWithEtag: async <T>(path: string): Promise<{ data: T; etag: string | null }> => {
    const url = new URL(`${API_BASE}${path}`, window.location.origin)
    const token = useAuthStore.getState().token
    const res = await fetch(url.toString(), {
      method: 'GET',
      headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    })
    if (!res.ok) {
      if (res.status === 401) useAuthStore.getState().logout()
      const text = await res.text().catch(() => '')
      throw new ApiError(res.status, res.statusText, text)
    }
    if (res.status === 204) return { data: undefined as T, etag: null }
    const etag = res.headers.get('etag')
    const contentType = res.headers.get('content-type') ?? ''
    const data: T = contentType.includes('application/json')
      ? ((await res.json()) as T)
      : ((await res.text()) as T)
    return { data, etag }
  },
  patch: <T>(path: string, body?: unknown) => apiClient<T>(path, { method: 'PATCH', body }),
  delete: <T>(path: string) => apiClient<T>(path, { method: 'DELETE' }),
  /** POST a FormData body (multipart upload) — browser sets Content-Type. */
  upload: <T>(path: string, formData: FormData) =>
    apiClient<T>(path, { method: 'POST', body: formData, formData: true }),
}
