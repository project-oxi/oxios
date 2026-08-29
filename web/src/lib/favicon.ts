// Shared favicon resolver — Google favicon service (same source as the
// backend's `favicon_url` in chat.rs, so chat cards and portal panels agree).
export function faviconUrl(url: string): string {
  try {
    const host = new URL(url).hostname
    return `https://www.google.com/s2/favicons?domain=${host}&sz=32`
  } catch {
    return ''
  }
}

/** Hostname without the leading `www.` — safe on malformed URLs. */
export function domainOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}
