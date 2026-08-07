//! E2E Kernel integration test.
//!
//! Tests System Call methods without a real LLM. Verifies core kernel
//! subsystems (state_store, git_layer, audit_trail, budget, resource_monitor)
//! work together correctly.
#![allow(clippy::unwrap_used)] // `.unwrap()` on setup ops is idiomatic in tests (workspace convention)

use std::path::PathBuf;
use tempfile::TempDir;

use oxicode_sdk::{AuditAction, AuditTrail};
use oxios_kernel::{
    budget::{BudgetLimit, BudgetManager},
    git_layer::GitLayer,
    resource_monitor::ResourceMonitor,
    state_store::{Session, StateStore},
};

fn setup() -> TempDir {
    tempfile::tempdir().unwrap()
}

// ═══════════════════════════════════════════════════════════════════════════════
// State Store System Calls
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_state_save_load_json() {
    let dir = setup();
    let store = StateStore::new(PathBuf::from(dir.path())).unwrap();

    let data = serde_json::json!({
        "key": "value",
        "numbers": [1, 2, 3],
        "nested": { "a": true }
    });

    // Save
    store.save_json("test", "item", &data).await.unwrap();

    // Load
    let loaded: Option<serde_json::Value> = store.load_json("test", "item").await.unwrap();
    assert!(loaded.is_some());

    let loaded = loaded.unwrap();
    assert_eq!(loaded["key"], "value");
    assert_eq!(loaded["numbers"], serde_json::json!([1, 2, 3]));
    assert_eq!(loaded["nested"]["a"], true);
}

#[tokio::test]
async fn test_state_save_load_session() {
    let dir = setup();
    let store = StateStore::new(PathBuf::from(dir.path())).unwrap();

    // Create and save session
    let mut session = Session::new("user-123");
    session.add_user_message("Hello, world!");

    store.save_session(&session).await.unwrap();

    // Load by ID
    let loaded: Option<Session> = store.load_session(&session.id).await.unwrap();
    assert!(loaded.is_some());

    let loaded = loaded.unwrap();
    assert_eq!(loaded.user_id, "user-123");
    assert_eq!(loaded.user_messages.len(), 1);
    assert_eq!(loaded.user_messages[0].content, "Hello, world!");
}

#[tokio::test]
async fn test_state_list_category() {
    let dir = setup();
    let store = StateStore::new(PathBuf::from(dir.path())).unwrap();

    // Save multiple items
    store
        .save_json("test", "item1", &serde_json::json!({}))
        .await
        .unwrap();
    store
        .save_json("test", "item2", &serde_json::json!({}))
        .await
        .unwrap();
    store
        .save_json("test", "item3", &serde_json::json!({}))
        .await
        .unwrap();

    // List
    let items = store.list_category("test").await.unwrap();
    assert_eq!(items.len(), 3);
    assert!(items.contains(&"item1".to_string()));
    assert!(items.contains(&"item2".to_string()));
    assert!(items.contains(&"item3".to_string()));
}

#[tokio::test]
async fn test_state_delete_file() {
    let dir = setup();
    let store = StateStore::new(PathBuf::from(dir.path())).unwrap();

    // Save then delete
    store
        .save_json("test", "item", &serde_json::json!({"key": "value"}))
        .await
        .unwrap();
    let deleted = store.delete_file("test", "item").await.unwrap();
    assert!(deleted);

    // Verify deleted
    let gone: Option<serde_json::Value> = store.load_json("test", "item").await.unwrap();
    assert!(gone.is_none());

    // Deleting non-existent should return false
    let deleted_again = store.delete_file("test", "item").await.unwrap();
    assert!(!deleted_again);
}

#[tokio::test]
async fn test_state_markdown() {
    let dir = setup();
    let store = StateStore::new(PathBuf::from(dir.path())).unwrap();

    let markdown = "# Hello\n\nThis is **markdown** content.";

    // Save markdown
    store
        .save_markdown("docs", "intro", markdown)
        .await
        .unwrap();

    // Load markdown
    let loaded = store.load_markdown("docs", "intro").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap(), markdown);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Git Layer System Calls
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_git_commit_and_log() {
    let dir = setup();
    let git = GitLayer::new(dir.path().to_path_buf(), true).unwrap();

    // Write a file and commit
    std::fs::write(dir.path().join("test.txt"), "hello").unwrap();
    let info = git.commit_file("test.txt", "initial commit").unwrap();

    assert!(!info.hash.is_empty());
    assert_eq!(info.short_hash.len(), 7);
    assert_eq!(info.message, "initial commit");

    // Log
    let log = git.log(10).unwrap();
    assert!(!log.is_empty());
    assert_eq!(log[0].message, "initial commit");
    assert!(!log[0].hash.is_empty());
}

#[test]
fn test_git_tag_operations() {
    let dir = setup();
    let git = GitLayer::new(dir.path().to_path_buf(), true).unwrap();

    // Create initial commit
    std::fs::write(dir.path().join("file.txt"), "content").unwrap();
    git.commit_file("file.txt", "first commit").unwrap();

    // Tag
    git.tag("v0.1", "first tag").unwrap();

    // List tags
    let tags = git.list_tags().unwrap();
    assert!(tags.iter().any(|t| t.contains("v0.1")));
}

#[test]
fn test_git_verify() {
    let dir = setup();
    let git = GitLayer::new(dir.path().to_path_buf(), true).unwrap();

    // Verify clean repo
    assert!(git.verify().unwrap());
}

#[test]
fn test_git_restore_file() {
    let dir = setup();
    let git = GitLayer::new(dir.path().to_path_buf(), true).unwrap();

    // Create file and commit v1
    std::fs::write(dir.path().join("state.json"), "v1").unwrap();
    let first = git.commit_file("state.json", "version 1").unwrap();

    // Update to v2 and commit
    std::fs::write(dir.path().join("state.json"), "v2").unwrap();
    git.commit_file("state.json", "version 2").unwrap();

    // Restore to v1
    git.restore_file("state.json", &first.short_hash).unwrap();

    let content = std::fs::read_to_string(dir.path().join("state.json")).unwrap();
    assert_eq!(content, "v1");
}

#[test]
fn test_git_disabled_noop() {
    let dir = setup();
    let git = GitLayer::new(dir.path().to_path_buf(), false).unwrap();

    std::fs::write(dir.path().join("test.txt"), "content").unwrap();
    let info = git.commit_file("test.txt", "noop commit").unwrap();

    // Disabled git returns placeholder hash
    assert_eq!(info.hash, "(disabled)");
    assert_eq!(info.short_hash, "(dis)");
}

#[test]
fn test_git_batch_commit() {
    let dir = setup();
    let git = GitLayer::new(dir.path().to_path_buf(), true).unwrap();

    std::fs::write(dir.path().join("a.txt"), "a").unwrap();
    std::fs::write(dir.path().join("b.txt"), "b").unwrap();

    let info = git
        .commit_files(&["a.txt", "b.txt"], "batch commit")
        .unwrap();

    assert!(!info.hash.is_empty());
    assert_eq!(info.message, "batch commit");
}

#[test]
fn test_git_remove_file() {
    let dir = setup();
    let git = GitLayer::new(dir.path().to_path_buf(), true).unwrap();

    // Add and commit file
    std::fs::write(dir.path().join("todelete.txt"), "delete me").unwrap();
    git.commit_file("todelete.txt", "add file").unwrap();

    // Remove from filesystem
    std::fs::remove_file(dir.path().join("todelete.txt")).unwrap();

    // Remove from git
    let info = git.remove_file("todelete.txt", "remove file").unwrap();
    assert!(!info.hash.is_empty());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Audit Trail System Calls
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_audit_append_generates_hash() {
    let audit = AuditTrail::new(1000);

    let hash = audit.append(
        "agent-001".to_string(),
        AuditAction::AgentSpawn {
            task_type: "test".to_string(),
        },
        "/test/resource".to_string(),
    );

    assert!(!hash.is_empty());
    assert_eq!(hash.len(), 64); // blake3 hex is 64 chars
}

#[test]
fn test_audit_hash_chain() {
    let audit = AuditTrail::new(1000);

    let hash1 = audit.append(
        "agent-001".to_string(),
        AuditAction::AgentSpawn {
            task_type: "test".to_string(),
        },
        "/test/resource".to_string(),
    );

    let hash2 = audit.append(
        "agent-001".to_string(),
        AuditAction::ToolCall {
            tool: "bash".to_string(),
            args_json: "{}".to_string(),
        },
        "/test/resource".to_string(),
    );

    // Different hashes
    assert_ne!(hash1, hash2);

    // Entries linked
    let entries = audit.all_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].prev_hash, "genesis");
    assert_eq!(entries[1].prev_hash, entries[0].hash);
}

#[test]
fn test_audit_verify_chain() {
    let audit = AuditTrail::new(1000);

    audit.append(
        "agent-001".to_string(),
        AuditAction::AgentSpawn {
            task_type: "test".to_string(),
        },
        "/test/resource".to_string(),
    );

    audit.append(
        "agent-001".to_string(),
        AuditAction::AgentExit {
            reason: "done".to_string(),
        },
        "/test/resource".to_string(),
    );

    // Verify should pass
    assert!(audit.verify().is_ok());
}

#[test]
fn test_audit_verify_multiple_entries() {
    let audit = AuditTrail::new(1000);

    audit.append(
        "agent-001".to_string(),
        AuditAction::AgentSpawn {
            task_type: "test".to_string(),
        },
        "/test/resource".to_string(),
    );

    // Add more entries
    audit.append(
        "agent-002".to_string(),
        AuditAction::ToolCall {
            tool: "bash".to_string(),
            args_json: "{}".to_string(),
        },
        "/test/resource".to_string(),
    );

    // Verify chain with multiple entries
    assert!(audit.verify().is_ok());
}

#[test]
fn test_audit_entries_range() {
    let audit = AuditTrail::new(1000);

    for i in 0..5 {
        audit.append(
            "agent-001".to_string(),
            AuditAction::Other {
                detail: format!("action-{i}"),
            },
            "/test/resource".to_string(),
        );
    }

    let range = audit.entries(2, 4);
    assert_eq!(range.len(), 3);
    assert_eq!(range[0].seq, 2);
    assert_eq!(range[2].seq, 4);
}

#[test]
fn test_audit_query_by_agent() {
    let audit = AuditTrail::new(1000);

    audit.append(
        "agent-001".to_string(),
        AuditAction::AgentSpawn {
            task_type: "test".to_string(),
        },
        "/test/resource".to_string(),
    );

    audit.append(
        "agent-002".to_string(),
        AuditAction::AgentSpawn {
            task_type: "test".to_string(),
        },
        "/test/resource".to_string(),
    );

    let agent1_entries = audit.by_agent("agent-001");
    assert_eq!(agent1_entries.len(), 1);

    let agent2_entries = audit.by_agent("agent-002");
    assert_eq!(agent2_entries.len(), 1);

    let empty_entries = audit.by_agent("agent-999");
    assert_eq!(empty_entries.len(), 0);
}

#[test]
fn test_audit_export_json() {
    let audit = AuditTrail::new(1000);

    audit.append(
        "agent-001".to_string(),
        AuditAction::AgentSpawn {
            task_type: "test".to_string(),
        },
        "/test/resource".to_string(),
    );

    let json = audit.export_json(0).unwrap();
    assert!(json.contains("agent-001"));
    assert!(json.contains("AgentSpawn"));

    // Should be valid JSON
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.len(), 1);
}

#[test]
fn test_audit_len() {
    let audit = AuditTrail::new(1000);

    assert_eq!(audit.len(), 0);

    audit.append(
        "agent-001".to_string(),
        AuditAction::Other {
            detail: "test".to_string(),
        },
        "/test/resource".to_string(),
    );

    assert_eq!(audit.len(), 1);
}

#[test]
fn test_audit_all_entries() {
    let audit = AuditTrail::new(1000);

    audit.append(
        "agent-001".to_string(),
        AuditAction::Other {
            detail: "first".to_string(),
        },
        "/test/resource".to_string(),
    );

    audit.append(
        "agent-002".to_string(),
        AuditAction::Other {
            detail: "second".to_string(),
        },
        "/test/resource".to_string(),
    );

    let all = audit.all_entries();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].actor, "agent-001");
    assert_eq!(all[1].actor, "agent-002");
}

#[test]
fn test_audit_auto_prune() {
    let audit = AuditTrail::new(3); // Small limit

    for i in 0..5 {
        audit.append(
            "agent-001".to_string(),
            AuditAction::Other {
                detail: format!("action-{i}"),
            },
            "/test/resource".to_string(),
        );
    }

    // Should only have 3 entries (oldest pruned)
    assert_eq!(audit.len(), 3);

    let entries = audit.all_entries();
    // Should be entries 3, 4, 5 (after pruning 1, 2)
    assert_eq!(entries[0].seq, 3);
    assert_eq!(entries[2].seq, 5);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Budget Manager System Calls
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_budget_set_and_remaining() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 1000,
        calls_budget: 10,
        window_secs: 3600,
    });

    let info = budget.remaining(&agent_id);
    assert_eq!(info.tokens_remaining, 1000);
    assert_eq!(info.calls_remaining, 10);
    assert!(!info.is_exhausted);
}

#[test]
fn test_budget_reserve_success() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 1000,
        calls_budget: 10,
        window_secs: 3600,
    });

    // Reserve within budget
    let result = budget.reserve(&agent_id, 500);
    assert!(result.is_ok());

    let info = budget.remaining(&agent_id);
    assert_eq!(info.tokens_remaining, 500);
}

#[test]
fn test_budget_reserve_exceed() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 1000,
        calls_budget: 10,
        window_secs: 3600,
    });

    // Exhaust budget
    budget.reserve(&agent_id, 1000).unwrap();

    // Try to reserve more
    let result = budget.reserve(&agent_id, 1);
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert_eq!(err.agent_id, agent_id);
}

#[test]
fn test_budget_can_schedule() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 1000,
        calls_budget: 10,
        window_secs: 3600,
    });

    // Should be schedulable
    assert!(budget.can_schedule(&agent_id));

    // Exhaust tokens
    budget.reserve(&agent_id, 1000).unwrap();

    // Should not be schedulable
    assert!(!budget.can_schedule(&agent_id));
}

#[test]
fn test_budget_reset_window() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 1000,
        calls_budget: 10,
        window_secs: 3600,
    });

    // Use some budget
    budget.reserve(&agent_id, 500).unwrap();
    let info_before = budget.remaining(&agent_id);
    assert_eq!(info_before.tokens_remaining, 500);

    // Reset
    budget.reset_window(&agent_id);

    let info_after = budget.remaining(&agent_id);
    assert_eq!(info_after.tokens_remaining, 1000);
}

#[test]
fn test_budget_track_call() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 1000,
        calls_budget: 3,
        window_secs: 3600,
    });

    // Track calls
    assert!(budget.track_call(&agent_id).is_ok());
    assert!(budget.track_call(&agent_id).is_ok());
    assert!(budget.track_call(&agent_id).is_ok());

    // Fourth call should fail
    let result = budget.track_call(&agent_id);
    assert!(result.is_err());
}

#[test]
fn test_budget_release_tokens() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 1000,
        calls_budget: 10,
        window_secs: 3600,
    });

    budget.reserve(&agent_id, 500).unwrap();
    budget.release(&agent_id, 200);

    let info = budget.remaining(&agent_id);
    assert_eq!(info.tokens_remaining, 700);
}

#[test]
fn test_budget_remove_budget() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 1000,
        calls_budget: 10,
        window_secs: 3600,
    });

    budget.reserve(&agent_id, 100).unwrap();
    budget.remove_budget(&agent_id);

    // Should fail after removal
    let result = budget.reserve(&agent_id, 100);
    assert!(result.is_err());
}

#[test]
fn test_budget_no_configured() {
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    // Reserve without budget should fail
    let result = budget.reserve(&agent_id, 100);
    assert!(result.is_err());

    // remaining() returns exhausted state
    let info = budget.remaining(&agent_id);
    assert!(info.is_exhausted);
}

// ═══════════════════════════════════════════════════════════════════════════════
// Resource Monitor System Calls
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_resource_snapshot() {
    let monitor = ResourceMonitor::new(60, 10);

    let snap = monitor.snapshot();

    assert!(snap.cpu_percent >= 0.0);
    assert!(snap.memory_total_mb > 0);
    assert!(snap.load_avg_1m >= 0.0);
    assert!(snap.disk_used_gb >= 0.0);
}

#[test]
fn test_resource_set_metrics() {
    let monitor = ResourceMonitor::new(60, 10);

    monitor.set_active_agents(5);
    monitor.set_pending_tasks(3);
    monitor.add_token_usage(1000);

    let snap = monitor.snapshot();
    assert_eq!(snap.active_agents, 5);
    assert_eq!(snap.pending_tasks, 3);
    assert_eq!(snap.total_token_usage, 1000);
}

#[test]
fn test_resource_history() {
    let monitor = ResourceMonitor::new(60, 10);

    // Initially empty
    let history = monitor.history(10);
    assert!(history.is_empty());
}

#[test]
fn test_resource_overload_threshold() {
    let monitor = ResourceMonitor::new(60, 10);

    let threshold = monitor.overload_threshold();
    assert_eq!(threshold.cpu_percent, 90.0);
    assert_eq!(threshold.memory_percent, 90.0);
    assert_eq!(threshold.load_avg, 8.0);
}

#[test]
fn test_resource_is_overloaded() {
    let monitor = ResourceMonitor::new(60, 10);

    // Test runs without panic
    let _ = monitor.is_overloaded();
}

// ═══════════════════════════════════════════════════════════════════════════════
// Cross-subsystem Integration
// ═══════════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn test_state_and_git_integration() {
    let dir = setup();

    // Create both stores
    let state_store = StateStore::new(dir.path().to_path_buf()).unwrap();
    let git = GitLayer::new(dir.path().to_path_buf(), true).unwrap();

    // Save state
    let data = serde_json::json!({
        "message": "Hello",
        "timestamp": "2024-01-01T00:00:00Z"
    });
    state_store
        .save_json("messages", "msg1", &data)
        .await
        .unwrap();

    // Commit to git
    git.commit_file("messages/msg1.json", "add message")
        .unwrap();

    // Verify in both systems
    let loaded: serde_json::Value = state_store
        .load_json("messages", "msg1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded["message"], "Hello");

    let log = git.log(10).unwrap();
    assert!(log.iter().any(|e| e.message.contains("add message")));
}

#[test]
fn test_audit_and_budget_integration() {
    let audit = AuditTrail::new(1000);
    let budget = BudgetManager::new();
    let agent_id = uuid::Uuid::new_v4();

    // Set up budget
    budget.set_budget(BudgetLimit {
        agent_id,
        token_budget: 500,
        calls_budget: 5,
        window_secs: 3600,
    });

    // Audit budget allocation
    audit.append(
        agent_id.to_string(),
        AuditAction::ConfigChange {
            key: "token_budget".to_string(),
        },
        format!("/agents/{agent_id}"),
    );

    // Reserve tokens
    budget.reserve(&agent_id, 200).unwrap();

    audit.append(
        agent_id.to_string(),
        AuditAction::ToolCall {
            tool: "reserve_tokens".to_string(),
            args_json: "200".to_string(),
        },
        format!("/agents/{agent_id}/budget"),
    );

    // Verify both are consistent
    let info = budget.remaining(&agent_id);
    let audit_entries = audit.by_agent(&agent_id.to_string());

    assert_eq!(info.tokens_remaining, 300);
    assert_eq!(audit_entries.len(), 2);
    assert!(audit.verify().is_ok());
}

// ═══════════════════════════════════════════════════════════════════════════════
// Direct System Call Tests (mirrors KernelHandle API patterns)
// ═══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_git_system_calls_direct() {
    let dir = setup();
    let git = GitLayer::new(dir.path().to_path_buf(), true).unwrap();

    // Test git_tag directly
    std::fs::write(dir.path().join("file.txt"), "content").unwrap();
    git.commit_file("file.txt", "tag target").unwrap();

    git.tag("v1.0", "release 1.0").unwrap();
    let tags = git.list_tags().unwrap();
    assert!(tags.iter().any(|t| t.contains("v1.0")));
    // Test git_verify
    assert!(git.verify().unwrap());

    // Test git_restore
    std::fs::write(dir.path().join("state.txt"), "v1").unwrap();
    let first = git.commit_file("state.txt", "v1").unwrap();
    std::fs::write(dir.path().join("state.txt"), "v2").unwrap();
    git.commit_file("state.txt", "v2").unwrap();
    git.restore_file("state.txt", &first.short_hash).unwrap();
    let content = std::fs::read_to_string(dir.path().join("state.txt")).unwrap();
    assert_eq!(content, "v1");
}

#[test]
#[ignore = "slow test — takes >4s due to system calls; run with --ignored or separate CI job"]
fn test_resource_monitor_system_calls_direct() {
    let monitor = ResourceMonitor::new(60, 10);

    // Test snapshot
    let snap = monitor.snapshot();
    assert!(snap.cpu_percent >= 0.0);
    assert!(snap.memory_total_mb > 0);
    // Test set metrics
    monitor.set_active_agents(5);
    monitor.set_pending_tasks(3);
    monitor.add_token_usage(1000);
    let snap = monitor.snapshot();
    assert_eq!(snap.active_agents, 5);
    assert_eq!(snap.pending_tasks, 3);
    assert_eq!(snap.total_token_usage, 1000);

    // Test overload check (may or may not be overloaded depending on system)
    let _ = monitor.is_overloaded();
}
