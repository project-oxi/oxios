# RFC-046: Browser Independence — Direct oxibrowser-core

> **Status**: In Progress (2026-08-11)
> **Supersedes**: `rfc-browser-migration.md` (which migrated TO the SDK)

## Summary

Remove oxios's dependency on `oxicode-sdk`'s browser port (`BrowserEngine`
trait, `OxicodeBrowserEngine`, browse tools) and implement a fully
self-contained browse stack that depends on `oxibrowser-core` 0.20 directly.

## Motivation

1. **oxicode is deprecating the browser port.** SDK 0.69+ marks the entire
   browser surface with `#[oxicode_unstable(feature = "browser")]`. When
   removed, oxios's browse tools break overnight.
2. **Dual-version problem.** oxios currently compiles oxibrowser-core **0.16**
   (via SDK for browsing) and **0.20** (direct, for screenshots) in the same
   binary.
3. **Unnecessary indirection.** The SDK's `BrowserEngine` trait + 867-line
   adapter exists to paper over a version difference that doesn't exist —
   0.16 and 0.20 have byte-identical `Browser`/`Tab` public APIs.

## Architecture

### Before

```
Agent loop
  → oxicode_sdk::BrowseTool / BrowseExtractTool / BrowseSessionTool / BrowseScriptTool
  → oxicode_sdk::BrowserEngine (trait)          ← to be removed by oxicode
  → oxicode_sdk::OxicodeBrowserEngine (adapter) ← 867 lines of pass-through
  → oxibrowser-core 0.16

ScreenshotTool → ScreenshotEngine → oxibrowser-core 0.20 (separate instance)
```

### After

```
Agent loop
  → oxios::browse::BrowseTool / BrowseExtractTool / BrowseSessionTool / BrowseScriptTool
  → oxios::browse::OxiosBrowser (concrete, wraps oxibrowser_core::Browser)
  → oxibrowser-core 0.20

ScreenshotTool → ScreenshotEngine → shared oxibrowser_core::Browser 0.20
```

### Key design decisions

1. **No trait abstraction.** `OxiosBrowser` and `OxiosTab` are concrete
   structs wrapping `oxibrowser_core::Browser`/`Tab`. There is only one
   backend; a trait would be YAGNI.

2. **Single Browser instance.** Both browse tools and screenshot share the
   same `oxibrowser_core::Browser` — shared cookies, HTTP client, session
   pool.

3. **All types oxios-owned.** `PageContent`, `BrowserError`, `Observation`,
   `BrowseProgress`, `BrowseConfig`, etc. live in
   `oxios-kernel::tools::browse`. No dependency on `oxicode-agent`'s browse
   module for types.

4. **AgentTool trait compatibility.** The tools still implement
   `oxicode_agent::AgentTool` — that's the tool-calling protocol, not the
   browser port. `on_browse_progress` uses `oxicode_agent::BrowseProgressCallback`
   (always compiled, not feature-gated).

## New module structure

```
crates/oxios-kernel/src/tools/browse/
├── mod.rs               # module structure + re-exports
├── types.rs             # PageContent, BrowserError, Observation, etc.
├── config.rs            # BrowseConfig
├── callback.rs          # TabCallbackRegistry, BrowseCallbacks
├── engine.rs            # OxiosBrowser, OxiosTab (wraps oxibrowser_core)
├── helpers.rs           # JS helpers + parsing
├── tab_guard.rs         # RAII guard for OxiosTab
├── browse_tool.rs       # BrowseTool
├── browse_extract_tool.rs  # BrowseExtractTool
├── browse_session_tool.rs  # BrowseSessionTool
└── browse_script_tool.rs   # BrowseScriptTool
```

## Feature gate change

```toml
# Before:
native-browser = ["oxicode-sdk/native-browser"]
screenshot = ["dep:oxibrowser-core"]

# After:
browser = ["dep:oxibrowser-core"]  # unified: browse + screenshot
```

The `native-browser` and `screenshot` features merge into a single `browser`
feature. Both capabilities use the same `oxibrowser-core` 0.20 dependency.

## Dependency changes

| Crate | Before | After |
|---|---|---|
| `oxicode-sdk` | `features = ["browser", ...]` | `features = [...]` (no browser) |
| `oxibrowser-core` | `optional, version = "0.20"` (screenshot only) | `version = "0.20"` (shared) |
| `serde_yaml` | not in kernel | added (for BrowseScriptTool) |
| `oxibrowser` (top-level) | `"0.20"` in root | removed (replaced by oxibrowser-core) |

## Migration scope

- ~2,700 lines of tool implementations ported (4 tools)
- ~1,700 lines of adapter/trait code eliminated
- `BrowserApi` rewritten to use `OxiosBrowser` directly
- `ScreenshotEngine` unified with shared Browser instance
