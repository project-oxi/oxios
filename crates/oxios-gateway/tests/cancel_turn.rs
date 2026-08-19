//! RFC-049: cancelling an in-flight turn must wake the dispatch task and
//! surface a terminal `Cancelled` error rather than letting the turn run on.
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

use oxios_gateway::message::ErrorKind;

#[test]
fn error_kind_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&ErrorKind::ProviderError).unwrap(),
        "\"provider_error\""
    );
    assert_eq!(
        serde_json::to_string(&ErrorKind::Cancelled).unwrap(),
        "\"cancelled\""
    );
    assert_eq!(
        serde_json::to_string(&ErrorKind::ApiKeyMissing).unwrap(),
        "\"api_key_missing\""
    );
}

#[tokio::test]
async fn cancelled_token_resolves_and_reports_agent() {
    let reg = oxios_kernel::turn_registry::TurnRegistry::new();
    let token = reg.open("sess-cancel");
    let agent = uuid::Uuid::new_v4();
    reg.bind_agent("sess-cancel", agent);

    let waiter = tokio::spawn(async move {
        token.cancelled().await;
        true
    });

    assert_eq!(reg.cancel("sess-cancel"), Some(agent));
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("must not hang")
            .unwrap()
    );
}
