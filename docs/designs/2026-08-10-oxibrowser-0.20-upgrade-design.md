# oxibrowser 0.20 Upgrade + Screenshot Capture — Design

**Date:** 2026-08-10
**Status:** Proposal
**Depends on:** oxibrowser 0.20.0, oxibrowser-render 0.20.0 (both on crates.io)

## TL;DR

oxibrowser 0.18–0.20 shipped a Blitz-backed CSS rendering pipeline
(`oxibrowser-render`: Stylo CSS + Taffy layout + vello\_cpu paint) producing
real pixel-accurate screenshots — replacing the legacy 8×16 bitmap-font
renderer. It also added `Page.printToPDF`, full Shadow DOM, per-frame JS
contexts, multi-tab, async fetch/XHR/WebSocket, and CORS/preflight.

The SDK (`oxicode-sdk` 0.69.0, released today) still pins `oxibrowser ^0.16`
internally, so SDK-path browse tools cannot use 0.20's rendering yet. But
oxios can **add `oxibrowser-render` as a direct dependency** behind a feature
gate, enabling a new `browse_screenshot` tool + `POST /api/screenshot`
endpoint that renders any URL to a CSS-laid-out PNG right now — independent
of SDK version.

---

## 1. What changed in oxibrowser 0.18–0.20

### 0.18.0 — "real headless browser" milestone

| Area | Change |
|---|---|
| **Rendering** | Blitz-backed `oxibrowser-render` crate: Stylo CSS cascade → Taffy layout → vello\_cpu paint → PNG. Replaces the text-based bitmap renderer. |
| **Screenshot** | `Page.captureScreenshot` / `capture_png` now produce pixel-accurate CSS-laid-out images (was 8×16 bitmap-font text approximation). |
| **PDF** | `Page.printToPDF` returns a real single-page PDF (was empty stub). |
| **Shadow DOM** | Full support: `attachShadow`, slot composition (flattened tree), closed mode, declarative `<template shadowrootmode>`, shadow-aware screenshot rasterization. |
| **IFrames** | Per-frame isolated JS execution contexts (`boa_engine::Context` per frame). `Runtime.evaluate` honors `contextId`. |
| **Multi-tab** | `Target.createTarget` creates a real session (was stub). Flat-protocol `sessionId` multiplex. |
| **Network** | Async (non-blocking) `fetch`/`XMLHttpRequest`. `WebSocket` (ws+wss). CORS preflight. Cookie security (Public Suffix List, `__Host-`/`__Secure-` prefixes, `Max-Age`/`Expires`). |
| **CDP** | Tracing domain (Playwright `page.tracing`). Emulation domain (`setDeviceMetricsOverride`). DOM layout-geometry (`getBoxModel`, `getContentQuads`, `getNodeForLocation`). Concurrent command dispatch (long-running commands no longer stall event forwarding). |
| **Page lifecycle** | `<script>` tags execute on navigation in document order. `DOMContentLoaded`/`load` fire. `@font-face` webfont loading (0.19). |
| **Dialogs** | Event-driven `alert`/`confirm`/`prompt` with `Page.handleJavaScriptDialog`. |
| **Canvas/WebGL** | Canvas 2D context shim (full surface) + best-effort WebGL/WebGL2. |

### 0.19.0

- `@font-face` webfont loading into Parley `FontContext` (no Blitz fork required).
- `srcdoc` / `about:blank` iframe contexts.
- Acceptance harness (8/8 PASS e2e).
- Fixed `Fetch.fulfillRequest` body decoding (base64 unconditional).

### 0.20.0

- Nested-iframe contexts (recursive frame tree BFS).
- External `<link rel=stylesheet>` fetched and inlined before render.
- `window.addEventListener` mirror on `window` object.
- Relative fetch URL resolution (`fetch('/api/x')`).
- `hashchange` event on `location.hash` assignment.

---

## 2. Current oxios dependency landscape

```
oxios binary crate
├── oxibrowser = "0.17"          ← direct dep (search::dispatch in search.rs)
├── oxicode-sdk = "0.66.0"       ← with features: browser, delegation, circuit-breaker, router
│   ├── oxicode-agent 0.66.0
│   │   └── oxibrowser ^0.16     ← browse tools backend (BrowseTool, BrowseSessionTool, …)
│   │   └── oxibrowser-core ^0.16 (optional, native-browser)
│   └── oxibrowser-core ^0.17    (optional, native-browser feature in SDK crate)
└── feature: browser = ["oxios-kernel/native-browser"]
```

**Two oxibrowser versions already coexist** in `Cargo.lock` today (0.16 via
SDK + 0.17 via direct dep). This is by design — the SDK pins its own
version, and the direct dep is used only for `search::dispatch()`.

### SDK version gap

| Crate | oxios uses | Latest on crates.io | oxibrowser pin |
|---|---|---|---|
| `oxicode-sdk` | 0.66.0 | **0.69.0** (2026-08-10) | `oxibrowser-core ^0.17` |
| `oxicode-agent` | 0.66.0 | 0.69.0 | `oxibrowser ^0.16` |
| `oxibrowser` (direct) | 0.17 | **0.20.0** | — |
| `oxibrowser-render` | — | **0.20.0** (new crate) | — |

**No SDK version supports oxibrowser 0.20 yet.** The SDK's browse tools
(`BrowseTool`, `BrowseSessionTool`, etc.) use oxibrowser 0.16 internally.
The screenshot they produce still uses the old bitmap renderer.

---

## 3. SDK BrowserTab trait — screenshot already exists

The `BrowserTab` trait in `oxicode-agent::tools::browse::engine` already
defines:

```rust
#[async_trait]
pub trait BrowserTab: Send + Sync {
    async fn screenshot(&self, width: u32) -> Result<Vec<u8>, BrowserError>;
    async fn goto(&self, url: &str) -> Result<PageContent, BrowserError>;
    async fn content(&self) -> Result<PageContent, BrowserError>;
    async fn evaluate(&self, js: &str) -> Result<Value, BrowserError>;
    async fn observe(&self) -> Result<Observation, BrowserError>;
    // … 30+ methods (click, type, fill, press, wait_for, select, check, …)
}
```

The screenshot method **exists but is never called** by any SDK tool. No
SDK browse tool exposes it. And oxios has **no screenshot tool or API
endpoint** today — confirmed by full grep of `src/`.

**Implication:** The SDK already has the plumbing; it just lacks a tool that
calls `tab.screenshot()`. When the SDK eventually upgrades to oxibrowser
0.20+, screenshots through the SDK path improve automatically (CSS rendering).

---

## 4. Upgrade strategy

### Phase 1: Direct dep upgrade (low risk, immediate)

```toml
# Cargo.toml — root
oxibrowser = "0.20"               # was "0.17"
```

This upgrades the `search::dispatch()` path. The `dispatch()` function
signature is unchanged between 0.17 and 0.20 (verified from source). The
search engines (DDG, Wiki, Bing, GitHub) benefit from improved stealth,
SSRF filtering, and HTTP client robustness.

**Risk:** The oxibrowser 0.20 crate depends on `oxibrowser-core 0.20` and
`oxibrowser-cdp 0.20` (both published). This adds a third version of
oxibrowser-core to the dependency tree alongside the SDK's 0.16. No API
conflict — they're separate major semver ranges compiled independently.

### Phase 2: Add `oxibrowser-core` (default feature, full browser)

```toml
# crates/oxios-kernel/Cargo.toml [dependencies]
oxibrowser-core = { version = "0.20", optional = true }

# crates/oxios-kernel/Cargo.toml [features]
screenshot = ["dep:oxibrowser-core"]

# Cargo.toml (root) [features]
screenshot = ["oxios-kernel/screenshot"]
default = ["web", "cli", "browser", "screenshot", "sqlite-memory"]
```

**Key decision: `oxibrowser-core` (full browser), NOT `oxibrowser-render` standalone.**

`oxibrowser-render` alone takes a static HTML string → PNG with ZERO network
I/O — no external CSS, images, fonts, or JS. It produces unstyled output for
real web pages. Instead, `oxibrowser-core::Browser` performs full navigation
(HTTP fetch + external stylesheet loading + JS execution) and its
`Session::capture_screenshot_png` calls `self.js_runtime.capture_png()` —
the integrated Blitz renderer capturing the **live post-JS DOM**.

`oxibrowser-core 0.20` depends on `oxibrowser-render 0.20` (Cargo.toml line 44),
so the Blitz stack is included transitively. No separate render dep needed.

The SDK's `oxibrowser-core 0.16` (via `oxicode-agent`) coexists — different
semver range, compiled independently. The screenshot tool creates its own
`Browser` instance (0.20), separate from the SDK's browse-tool engine (0.16).

### Phase 3: SDK upgrade (separate effort, not blocking)

Upgrading `oxicode-sdk` 0.66.0 → 0.69.0 is a separate migration with its own
breaking-change analysis (3 minor versions). It does **not** bring
oxibrowser 0.20 to the SDK path — the SDK still pins 0.16. Tracked
separately.

---

## 5. Feature design: Screenshot capture

### 5.1 Pipeline

```
Agent / Web UI
     │
     ▼
┌──────────────────────────────────────────────────┐
│  ScreenshotEngine (kernel_handle/screenshot_api.rs)  │
│  ┌──────────────────────────────────────────────┐ │
│  │  1. Browser::new(config)     [lazy, once]    │ │
│  │  2. browser.new_tab()                         │ │  ← oxibrowser-core 0.20
│  │  3. tab.goto(url)  [fetch + CSS + JS + DOM]   │ │
│  │  4. tab.screenshot(width) [Blitz CSS render]  │ │
│  │  5. tab.close()                               │ │
│  │  6. Return PNG bytes                          │ │
│  └──────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────┘
     │                           │
     ▼                           ▼
  Agent tool               GET /api/screenshot
  (browse_screenshot)      (web UI search panel <img>)
```

### 5.2 Threading model

No special threading needed. `oxibrowser_core::Browser` is fully async/tokio-native.
The internal JS runtime (boa `Context`, which is `!Send`) runs on a dedicated
`std::thread` inside oxibrowser-core — that's an implementation detail hidden
behind the async `Tab` API. From oxios's perspective, it's regular `async fn`.

### 5.3 Agent tool: `browse_screenshot`

**File:** `crates/oxios-kernel/src/tools/builtin/screenshot_tool.rs`

| Parameter | Type | Default | Description |
|---|---|---|---|
| `url` | string | required | URL to screenshot |
| `width` | number | 1280 | Viewport width (CSS px, clamped 320–4096) |
| `height` | number | 800 | Viewport height (CSS px, clamped 240–4096) |

Saves PNG to `~/Library/Caches/oxios/screenshots/<blake3_hash>.png`,
returns the file path + metadata. Registered in `builtin/mod.rs` behind
`#[cfg(feature = "screenshot")]`.

### 5.4 API endpoint: `GET /api/screenshot`

**File:** `src/api/routes/search.rs`

```
GET /api/screenshot?url=https://example.com&w=1280&h=800
→ 200 image/png (binary body, cache-control: public max-age=86400)
```

The frontend uses it as a native `<img src>` — no JS round-trip needed.
A process-wide `LazyLock<ScreenshotEngine>` is shared across requests.

### 5.5 Frontend integration

**SearchView (`web/src/components/portal/views/search-view.tsx`):**

- **Card header**: 16×12 (w-16 h-12) thumbnail, lazy-loaded,
  `onError` hides the `<img>` if rendering fails.
- **Expanded body**: full-width preview (`w-full max-h-64`) above the
  "Read page" button.

### 5.6 Caching

The screenshot endpoint sets `Cache-Control: public, max-age=86400` —
the browser/CDN caches PNGs for 24h. The agent tool saves each capture
to `~/Library/Caches/oxios/screenshots/` with a content-hash filename,
so identical URL+size requests reuse the file.

---

## 6. Additional improvements enabled by 0.20

### 6.1 Stealth-mode search

oxibrowser 0.16+ added `ChallengeDetector` (Cloudflare/Turnstile/reCAPTCHA
detection) and `wreq` for Chrome JA4+ fingerprint emulation. Upgrading the
direct dep to 0.20 brings these improvements to `search::dispatch()` —
better success rate against bot-protected sites.

### 6.2 Extract engine improvements

The extraction engine (`extract.rs`) was significantly enhanced in 0.16:
structured HTML-to-markdown with link collection, metadata parsing
(`og:title`, `og:description`, `twitter:card`, canonical URL), and content
normalization. These flow through to oxios's `/api/browse` endpoint
automatically.

### 6.3 Session-based automation

oxibrowser 0.11+ added the `session` command (stdin/stdout JSON REPL with
22 commands). While oxios uses the SDK's `BrowseSessionTool` (which wraps
the same engine), the underlying session capabilities (multi-tab,
navigation history, wait conditions) are richer in 0.20.

### 6.4 PDF export (future)

`Page.printToPDF` is now real. A future `browse_pdf` tool could render a
page to PDF — useful for archiving articles or generating reports. This
requires the SDK to expose PDF (currently not in the `BrowserTab` trait).
Could be done via the direct `oxibrowser` dep path (CDP server) if needed.

---

## 7. Risks & mitigations

| Risk | Severity | Mitigation |
|---|---|---|
| Blitz pre-alpha API churn (0.3.0-beta.1) | Med | `oxibrowser-render` wraps it — we depend on `oxibrowser-render`'s stable API, not Blitz directly. Version pin isolates us. |
| Compile time (+60–90s) | Med | Feature-gated (`screenshot`). Default build unaffected. CI only compiles it when `--all-features` or `--features screenshot`. |
| Binary size (+8–12 MB) | Low | Feature-gated. Release binary includes it only when built with `--features screenshot`. |
| `!Send` RenderDocument | Low | `spawn_blocking` — no shared state, no locks. Proven pattern. |
| Static-only rendering (no JS) | Med | `oxibrowser-render` Phase 1 takes HTML→PNG without JS execution. For dynamic pages, the SDK's `BrowserTab::screenshot()` path (with old renderer) is the fallback. When SDK upgrades, both paths converge. |
| External stylesheet fetch SSRF | Low | Reuse oxibrowser's SSRF filter (`check_url_ssrf`) — already scheme-aware, CIDR-blocking. The render crate delegates stylesheet fetching through the existing HTTP client. |
| Screenshot of large pages (OOM) | Low | Cap viewport dimensions (max 4096×4096). Cap full-page height (max 16384px). Return error for oversized. |

---

## 8. Implementation phases

### Phase A — Direct dep upgrade (1 hour)

1. Bump `oxibrowser = "0.20"` in root `Cargo.toml`.
2. `cargo update -p oxibrowser`.
3. Verify `search::dispatch()` compiles and works.
4. Run existing browse/search tests.

### Phase B — Screenshot capability (4–6 hours)

1. Add `oxibrowser-render` dep behind `screenshot` feature.
2. Implement `ScreenshotApi` in `kernel_handle/screenshot_api.rs`.
3. Implement `browse_screenshot` tool in `tools/screenshot_tool.rs`.
4. Add `GET/POST /api/screenshot` endpoint.
5. Add screenshot cache.
6. Test: screenshot a known page, verify PNG dimensions and non-blank.

### Phase C — Frontend integration (2–3 hours)

1. Add screenshot thumbnail to SearchView result cards.
2. Add screenshot preview to browse tool activity card.
3. Lazy-load with shimmer placeholder.
4. Test: open browse panel, verify thumbnails render.

### Phase D — SDK upgrade (separate effort)

1. Upgrade `oxicode-sdk` 0.66.0 → 0.69.0.
2. Analyze breaking changes across 3 minor versions.
3. Update feature flags if needed.
4. This is tracked separately — does not block Phases A–C.

---

## 9. Feature flag summary

```toml
[features]
default = ["web", "cli", "browser", "sqlite-memory"]

# Existing
browser = ["oxios-kernel/native-browser"]

# New — opt-in CSS screenshot rendering
screenshot = ["dep:oxibrowser-render"]
```

The `screenshot` feature is **independent** of `browser`. It works even
without the SDK's native-browser backend — it renders HTML directly via
Blitz without going through the SDK's `BrowserEngine` at all. This is a
deliberate architectural choice: decouples screenshot rendering from SDK
version constraints.
