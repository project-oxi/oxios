//! RAII guard that ensures a browser tab is properly closed.
//!
//! Prevents tab leaks by tracking lifecycle and warning on implicit drops.
//! Use `TabGuard::close().await` for explicit async close, or `into_inner()`
//! to transfer ownership.

use super::engine::OxiosTab;

/// RAII wrapper around [`OxiosTab`].
///
/// If dropped without calling [`close`](TabGuard::close) or
/// [`into_inner`](TabGuard::into_inner), a `tracing::warn` is emitted.
/// Since Rust's `Drop` cannot be async, the tab itself cannot be closed
/// synchronously — always prefer `guard.close().await` before the guard
/// goes out of scope.
pub struct TabGuard {
    tab: Option<OxiosTab>,
    explicitly_consumed: bool,
}

impl TabGuard {
    /// Create a new guard wrapping an opened tab.
    pub fn new(tab: OxiosTab) -> Self {
        Self {
            tab: Some(tab),
            explicitly_consumed: false,
        }
    }

    /// Access the underlying tab reference.
    ///
    /// # Panics
    ///
    /// Panics if the guard has already been consumed.
    #[allow(clippy::expect_used)]
    pub fn tab(&self) -> &OxiosTab {
        self.tab.as_ref().expect("TabGuard: tab already consumed")
    }

    /// Explicitly close the tab and consume the guard.
    ///
    /// If `close()` fails on the underlying tab, a warning is logged but
    /// no error is propagated — the guard is still consumed.
    pub async fn close(mut self) {
        self.explicitly_consumed = true;
        if let Some(tab) = self.tab.take() {
            tab.clear_progress_callback();
            if let Err(e) = tab.close().await {
                tracing::warn!("TabGuard: tab close failed: {}", e);
            }
        }
    }

    /// Take ownership of the tab without closing it.
    #[allow(clippy::expect_used)]
    pub fn into_inner(mut self) -> OxiosTab {
        self.explicitly_consumed = true;
        self.tab.take().expect("TabGuard: tab already consumed")
    }
}

impl Drop for TabGuard {
    fn drop(&mut self) {
        if !self.explicitly_consumed {
            tracing::warn!(
                "TabGuard dropped without explicit close — tab may leak. \
                 Call .close().await or .into_inner() to prevent this."
            );
        }
    }
}
