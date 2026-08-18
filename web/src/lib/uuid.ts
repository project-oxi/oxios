/**
 * UUID v4 generation that works on insecure origins.
 *
 * `crypto.randomUUID()` is gated to secure contexts (HTTPS or localhost).
 * When the dashboard is served over plain HTTP from a remote host — a LAN IP
 * or a `tailscale serve` tailnet URL — the method is `undefined` and calling
 * it throws `TypeError: crypto.randomUUID is not a function`, killing the
 * chat send path on mobile. `crypto.getRandomValues` carries no such gate,
 * so derive an RFC 4122 v4 from it and keep `Math.random` as a terminal
 * fallback for exotic embedders.
 */
export function uuid(): string {
  const c: Crypto | undefined = typeof crypto === 'undefined' ? undefined : crypto
  const randomUUID = c?.randomUUID?.bind(c)
  if (randomUUID) return randomUUID()

  const bytes = new Uint8Array(16)
  if (c?.getRandomValues) {
    c.getRandomValues(bytes)
  } else {
    for (let i = 0; i < 16; i += 1) {
      bytes[i] = Math.floor(Math.random() * 256)
    }
  }
  // RFC 4122 version 4: set version and variant bits.
  bytes[6] = ((bytes[6] ?? 0) & 0x0f) | 0x40
  bytes[8] = ((bytes[8] ?? 0) & 0x3f) | 0x80
  const hex = Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}
