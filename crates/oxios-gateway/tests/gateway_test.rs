//! Gateway unit tests — channel registration and message type validation.
#![allow(clippy::unwrap_used)] // `.unwrap()` in tests is idiomatic (workspace convention)

use oxios_gateway::message::{IncomingMessage, OutgoingMessage};

// ---------------------------------------------------------------------------
// Message type tests (no gateway needed — pure data validation)
// ---------------------------------------------------------------------------

#[test]
fn test_incoming_message_new() {
    let msg = IncomingMessage::new("web", "user1", "Hello");
    assert_eq!(msg.channel, "web");
    assert_eq!(msg.user_id, "user1");
    assert_eq!(msg.content, "Hello");
    assert!(msg.metadata.is_empty());
}

#[test]
fn test_outgoing_message_new() {
    let msg = OutgoingMessage::new("web", "user1", "Response");
    assert_eq!(msg.channel, "web");
    assert_eq!(msg.user_id, "user1");
    assert_eq!(msg.content, "Response");
    assert!(msg.metadata.is_empty());
}

#[test]
fn test_outgoing_message_with_metadata() {
    use std::collections::HashMap;
    let mut meta = HashMap::new();
    meta.insert("phase".to_string(), "execute".to_string());
    meta.insert("evaluation_passed".to_string(), "true".to_string());

    let msg = OutgoingMessage::with_metadata("web", "user1", "Done", meta.clone());
    assert_eq!(msg.metadata["phase"], "execute");
    assert_eq!(msg.metadata["evaluation_passed"], "true");
}

#[test]
fn test_incoming_message_has_unique_ids() {
    let msg1 = IncomingMessage::new("web", "user1", "A");
    let msg2 = IncomingMessage::new("web", "user1", "B");
    assert_ne!(msg1.id, msg2.id);
}

#[test]
fn test_outgoing_message_has_unique_ids() {
    let msg1 = OutgoingMessage::new("web", "user1", "A");
    let msg2 = OutgoingMessage::new("web", "user1", "B");
    assert_ne!(msg1.id, msg2.id);
}

#[test]
fn test_message_serialization_roundtrip() {
    let msg = IncomingMessage::new("web", "user1", "Test");
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.channel, msg.channel);
    assert_eq!(parsed.user_id, msg.user_id);
    assert_eq!(parsed.content, msg.content);
    assert_eq!(parsed.id, msg.id);
}

#[test]
fn test_outgoing_serialization_roundtrip() {
    use std::collections::HashMap;
    let mut meta = HashMap::new();
    meta.insert("key".to_string(), "value".to_string());
    let msg = OutgoingMessage::with_metadata("cli", "user2", "OK", meta);
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: OutgoingMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.metadata["key"], "value");
}
