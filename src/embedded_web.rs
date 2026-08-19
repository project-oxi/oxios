//! Embedded web UI assets.
//!
//! When the binary is compiled with a built `web/dist/` present (local builds,
//! CI, the release binary job — see `build.rs`), the React SPA is baked into
//! the binary via [`include_dir!`]. The daemon then serves it directly with
//! **no first-run download** and no GitHub dependency.
//!
//! When `web/dist/` is absent (`cargo install` from crates.io — it is
//! gitignored and not shipped in the crate tarball), the `web_embedded` cfg is
//! not set, asset lookups return `None`, and the daemon falls back to the
//! runtime download path in [`crate::web_dist`].
//!
//! # Authoritative and exclusive
//!
//! When embedded, [`crate::web_dist::ensure_web_dist`] returns
//! [`WebDistResult::Embedded`](crate::web_dist::WebDistResult::Embedded),
//! serving goes straight to the baked-in assets (see
//! `crate::api::plugin::serve_file`), and every sync path
//! ([`crate::web_dist::sync`], `sync_to_disk`, the runtime update API) is a
//! no-op — no competing on-disk dist is ever created or honored (RFC-024 C3:
//! never mix two build hashes). The former `~/.oxios/web/dist/` manual
//! override was removed: it silently shadowed binary deploys with stale UIs
//! (2026-08-19).
//!
//! Both `web_embedded` (set by `build.rs`) and the `web` feature (the only
//! config that actually serves assets) must hold for embedding to activate —
//! without `web`, there is no server and embedded bytes would be dead weight.

#[cfg(all(web_embedded, feature = "web"))]
use include_dir::{Dir, include_dir};

/// The compiled web UI tree, baked in at compile time.
#[cfg(all(web_embedded, feature = "web"))]
static EMBEDDED_WEB: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/web/dist");

/// Whether this binary has the web UI embedded and is serving it.
///
/// `const` so call sites ([`crate::web_dist::ensure_web_dist`], the sync gate)
/// branch at compile time — the not-embedded build pays zero runtime cost.
#[cfg(all(web_embedded, feature = "web"))]
pub const fn is_embedded() -> bool {
    true
}

#[cfg(not(all(web_embedded, feature = "web")))]
pub const fn is_embedded() -> bool {
    false
}

/// Fetch an embedded asset by web path (e.g. `"index.html"`,
/// `"assets/index-AbC123.js"`). Returns the raw bytes, or `None` if this
/// build has no embedded assets or the path is absent.
#[cfg(all(web_embedded, feature = "web"))]
pub fn get(path: &str) -> Option<&'static [u8]> {
    let clean = path.trim_start_matches('/');
    EMBEDDED_WEB.get_file(clean).map(|f| f.contents())
}

#[cfg(not(all(web_embedded, feature = "web")))]
pub fn get(_path: &str) -> Option<&'static [u8]> {
    None
}

/// Read the `version.json` stamp from the embedded tree, for the
/// `X-Web-Version` header. Falls back to `"embedded"` (never `None`) so the
/// dashboard always renders a sane badge.
#[cfg(all(web_embedded, feature = "web"))]
pub fn version() -> String {
    #[derive(serde::Deserialize)]
    struct VersionFile {
        version: Option<String>,
    }
    get("version.json")
        .and_then(|b| serde_json::from_slice::<VersionFile>(b).ok())
        .and_then(|v| v.version)
        .unwrap_or_else(|| "embedded".to_string())
}

#[cfg(not(all(web_embedded, feature = "web")))]
pub fn version() -> String {
    "dev".to_string()
}
