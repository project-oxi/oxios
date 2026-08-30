//! Unified error type for the Oxios web API.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// Application-level error with HTTP status mapping.
#[derive(Debug)]
pub enum AppError {
    /// Resource not found.
    NotFound(String),
    /// Bad request from the client.
    BadRequest(String),
    /// Internal server error.
    Internal(String),
    /// Permission denied.
    Forbidden(String),
    /// Service unavailable (e.g. optional subsystem not initialized).
    ServiceUnavailable(String),
    /// Payload too large (超过限制).
    PayloadTooLarge {
        /// Actual size in bytes.
        size: usize,
        /// Maximum allowed size.
        limit: usize,
    },
    /// Gateway did not respond within the configured deadline (RFC-024 C1).
    GatewayTimeout(String),
    /// Precondition failed — ETag mismatch on optimistic concurrency check (S-2).
    Conflict(String),
    /// Semantically invalid payload (e.g. project root path validation, design §9).
    UnprocessableEntity(String),
    /// Not implemented on this platform/build (e.g. native folder picker
    /// outside macOS). Only constructed on non-macos targets, hence the
    /// targeted allow.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    NotImplemented(String),
    /// The resource is claimed by a live owner (issue assignment).
    ///
    /// Distinct from [`Self::Conflict`]: a conflict is resolved by reloading
    /// and reapplying, whereas a lock is resolved by waiting or releasing the
    /// claim. The client shows different remedies, so the status differs.
    Locked(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::NotFound(m) => write!(f, "Not Found: {m}"),
            AppError::BadRequest(m) => write!(f, "Bad Request: {m}"),
            AppError::Internal(m) => write!(f, "Internal Error: {m}"),
            AppError::Forbidden(m) => write!(f, "Forbidden: {m}"),
            AppError::ServiceUnavailable(m) => write!(f, "Service Unavailable: {m}"),
            AppError::PayloadTooLarge { size, limit } => write!(
                f,
                "Payload too large: {size} bytes exceeds limit of {limit} bytes"
            ),
            AppError::GatewayTimeout(m) => write!(f, "Gateway Timeout: {m}"),
            AppError::Conflict(m) => write!(f, "Conflict: {m}"),
            AppError::UnprocessableEntity(m) => write!(f, "Unprocessable Entity: {m}"),
            AppError::NotImplemented(m) => write!(f, "Not Implemented: {m}"),
            AppError::Locked(m) => write!(f, "Locked: {m}"),
        }
    }
}

impl std::error::Error for AppError {}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m.clone()),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m.clone()),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m.clone()),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m.clone()),
            AppError::ServiceUnavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, m.clone()),
            AppError::PayloadTooLarge { size, limit } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                format!("{size} bytes exceeds limit of {limit} bytes"),
            ),
            AppError::GatewayTimeout(m) => (StatusCode::GATEWAY_TIMEOUT, m.clone()),
            AppError::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            AppError::UnprocessableEntity(m) => (StatusCode::UNPROCESSABLE_ENTITY, m.clone()),
            AppError::NotImplemented(m) => (StatusCode::NOT_IMPLEMENTED, m.clone()),
            AppError::Locked(m) => (StatusCode::LOCKED, m.clone()),
        };
        let body = json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}

// Convenience conversions

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        // Walk the error chain to find a typed KernelError and preserve its
        // intended HTTP status. Without this, agent/session/program not-found
        // errors that propagate through anyhow surface as 500 instead of 404.
        for source in err.chain() {
            if let Some(ke) = source.downcast_ref::<oxios_kernel::error::KernelError>() {
                match ke.http_status() {
                    oxios_kernel::error::HttpStatus::NotFound => {
                        return AppError::NotFound(err.to_string());
                    }
                    oxios_kernel::error::HttpStatus::BadRequest => {
                        return AppError::BadRequest(err.to_string());
                    }
                    oxios_kernel::error::HttpStatus::Forbidden => {
                        return AppError::Forbidden(err.to_string());
                    }
                    _ => {}
                }
            }
        }
        AppError::Internal(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        assert_eq!(
            AppError::NotFound("test".into()).to_string(),
            "Not Found: test"
        );
        assert_eq!(
            AppError::BadRequest("bad".into()).to_string(),
            "Bad Request: bad"
        );
    }

    #[test]
    fn test_anyhow_conversion() {
        let err: AppError = anyhow::anyhow!("something failed").into();
        match err {
            AppError::Internal(msg) => assert_eq!(msg, "something failed"),
            _ => panic!("Expected Internal variant"),
        }
    }

    #[test]
    fn test_kernel_not_found_preserved() {
        // A KernelError::SessionNotFound that propagates through anyhow should
        // map to AppError::NotFound (404), not Internal (500).
        let kernel_err = oxios_kernel::error::KernelError::SessionNotFound { id: "s1".into() };
        let anyhow_err: anyhow::Error = kernel_err.into();
        let app_err: AppError = anyhow_err.into();
        assert!(
            matches!(app_err, AppError::NotFound(_)),
            "SessionNotFound should map to NotFound, not Internal"
        );
    }

    #[test]
    fn test_kernel_not_found_through_context() {
        // KernelError wrapped in anyhow context should still be detected via
        // the error chain walk.
        let kernel_err = oxios_kernel::error::KernelError::SessionNotFound { id: "s2".into() };
        let anyhow_err = anyhow::Error::new(kernel_err).context("while fetching session");
        let app_err: AppError = anyhow_err.into();
        assert!(
            matches!(app_err, AppError::NotFound(_)),
            "context-wrapped SessionNotFound should still map to NotFound"
        );
    }
}
