//! Error classification for user-facing error messages.
//!
//! Converts `anyhow::Error` into structured `UserFacingError` using a hybrid
//! of type-based classification and message-pattern heuristics.
//! When the kernel migrates to `thiserror`, `downcast` can replace heuristics.

use crate::message::{ErrorKind, UserFacingError};

/// Classifies an `anyhow::Error` into a user-friendly error.
///
/// Uses type checking, cause-chain traversal, and message-pattern matching
/// (in that priority). Falls back to `ErrorKind::Internal` when nothing matches.
pub fn classify_error(e: &anyhow::Error) -> UserFacingError {
    let kind = infer_kind(e);
    let (message, suggestion) = user_message_and_suggestion(&kind, &e.to_string());

    UserFacingError {
        message,
        kind,
        suggestion,
    }
}

fn infer_kind(e: &anyhow::Error) -> ErrorKind {
    // 1. Type-based classification (exact).
    if e.is::<tokio::time::error::Elapsed>() {
        return ErrorKind::Timeout;
    }

    // 2. Cause-chain traversal.
    let mut source = e.source();
    while let Some(err) = source {
        if err.is::<tokio::time::error::Elapsed>() {
            return ErrorKind::Timeout;
        }
        source = err.source();
    }

    // 3. Message-pattern matching (heuristic).
    let msg = e.to_string().to_lowercase();
    if msg.contains("missing api key") || msg.contains("no api key") {
        return ErrorKind::ApiKeyMissing;
    }
    if msg.contains("rate limit") || msg.contains("api key") || msg.contains("provider") {
        return ErrorKind::ProviderError;
    }
    if msg.contains("permission") || msg.contains("unauthorized") || msg.contains("access denied") {
        return ErrorKind::PermissionDenied;
    }
    if msg.contains("timeout") || msg.contains("deadline exceeded") {
        return ErrorKind::Timeout;
    }
    if msg.contains("validation") || msg.contains("invalid") || msg.contains("empty") {
        return ErrorKind::ValidationError;
    }

    ErrorKind::Internal
}

fn user_message_and_suggestion(kind: &ErrorKind, _raw_msg: &str) -> (String, Option<String>) {
    match kind {
        ErrorKind::ApiKeyMissing => (
            "No API key configured.".to_string(),
            Some("Register an API key in settings, or switch to an available model.".to_string()),
        ),
        ErrorKind::ExecutionFailed => (
            "An error occurred while processing your request.".to_string(),
            None,
        ),
        ErrorKind::ProviderError => (
            "The AI service is temporarily unavailable. Please try again shortly.".to_string(),
            Some("Try again in 1-2 minutes, or select a different model.".to_string()),
        ),
        ErrorKind::Timeout => (
            "The request timed out.".to_string(),
            Some("Try a simpler request, or increase the timeout.".to_string()),
        ),
        ErrorKind::PermissionDenied => (
            "You don't have permission to perform this action.".to_string(),
            Some("Request the required permission from your administrator.".to_string()),
        ),
        ErrorKind::ValidationError => ("Invalid input.".to_string(), None),
        ErrorKind::Internal => ("An internal error occurred.".to_string(), None),
        // RFC-049: never produced by the classify path — the gateway emits
        // the cancelled terminal directly with its own message. Included
        // only to satisfy the exhaustive match.
        ErrorKind::Cancelled => ("Turn cancelled by user.".to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_from_message_heuristic() {
        let e = anyhow::anyhow!("connection timeout");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::Timeout);
    }

    #[test]
    fn provider_from_message() {
        let e = anyhow::anyhow!("rate limit exceeded");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::ProviderError);
    }

    #[test]
    fn permission_from_message() {
        let e = anyhow::anyhow!("permission denied for resource");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::PermissionDenied);
    }

    #[test]
    fn validation_from_message() {
        let e = anyhow::anyhow!("invalid input provided");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::ValidationError);
    }

    #[test]
    fn internal_fallback() {
        let e = anyhow::anyhow!("something went wrong in the system");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::Internal);
    }

    #[test]
    fn api_key_missing_classified() {
        let e = anyhow::anyhow!("Missing API key");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::ApiKeyMissing);
    }

    #[test]
    fn no_api_key_classified() {
        let e = anyhow::anyhow!("No API key configured");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::ApiKeyMissing);
    }

    #[test]
    fn suggestion_for_provider() {
        let e = anyhow::anyhow!("api key invalid");
        let ufe = classify_error(&e);
        assert!(ufe.suggestion.is_some());
        assert_eq!(ufe.kind, ErrorKind::ProviderError);
    }

    #[test]
    fn no_suggestion_for_internal() {
        let e = anyhow::anyhow!("unknown");
        let ufe = classify_error(&e);
        assert!(ufe.suggestion.is_none());
    }

    #[test]
    fn timeout_from_elapsed_type() {
        // Use tokio::time::timeout to create a real Elapsed error
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(async {
            tokio::time::timeout(std::time::Duration::ZERO, std::future::pending::<()>()).await
        });
        let e = anyhow::anyhow!(result.unwrap_err());
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::Timeout);
    }

    #[test]
    fn provider_from_api_key_message() {
        let e = anyhow::anyhow!("API key is invalid");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::ProviderError);
    }

    #[test]
    fn permission_from_unauthorized() {
        let e = anyhow::anyhow!("unauthorized access");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::PermissionDenied);
    }

    #[test]
    fn validation_from_empty_message() {
        let e = anyhow::anyhow!("empty input");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::ValidationError);
    }

    #[test]
    fn deadline_exceeded_is_timeout() {
        let e = anyhow::anyhow!("deadline exceeded");
        let ufe = classify_error(&e);
        assert_eq!(ufe.kind, ErrorKind::Timeout);
    }

    #[test]
    fn user_messages_are_korean() {
        for kind in &[
            ErrorKind::ExecutionFailed,
            ErrorKind::ApiKeyMissing,
            ErrorKind::ProviderError,
            ErrorKind::Timeout,
            ErrorKind::PermissionDenied,
            ErrorKind::ValidationError,
            ErrorKind::Internal,
            ErrorKind::Cancelled,
        ] {
            let (msg, _) = user_message_and_suggestion(kind, "");
            assert!(
                !msg.is_empty(),
                "user_message should not be empty for {:?}",
                kind
            );
        }
    }

    #[test]
    fn suggestion_for_timeout() {
        let e = anyhow::anyhow!("timeout");
        let ufe = classify_error(&e);
        assert!(ufe.suggestion.is_some());
    }

    #[test]
    fn suggestion_for_permission() {
        let e = anyhow::anyhow!("access denied");
        let ufe = classify_error(&e);
        assert!(ufe.suggestion.is_some());
    }
}
