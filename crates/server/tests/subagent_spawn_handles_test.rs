// In-memory SubagentSpawnStore unit tests (EVE-535).
//
// Tests the CAS semantics of InMemorySubagentSpawnStore without a database.

use everruns_core::traits::{SpawnClaimResult, SubagentSpawnStore};
use everruns_core::typed_id::SessionId;
use everruns_server::storage::memory::subagent_spawn_handles::InMemorySubagentSpawnStore;
use uuid::Uuid;

fn make_store() -> InMemorySubagentSpawnStore {
    InMemorySubagentSpawnStore::new()
}

/// First claim for a new (parent, tool_call_id) pair returns Claimed.
#[tokio::test]
async fn test_spawn_handle_claimed_on_first_call() {
    let store = make_store();
    let parent = SessionId::new();
    let child = SessionId::new();
    let token = Uuid::new_v4();

    let result = store
        .try_claim_spawn(parent, "call-1", child, "Worker", "do work", token)
        .await
        .expect("claim should not error");

    match result {
        SpawnClaimResult::Claimed {
            spawn_handle_id,
            claim_token,
        } => {
            assert!(!spawn_handle_id.is_nil());
            assert_eq!(claim_token, token);
        }
        other => panic!("Expected Claimed, got {other:?}"),
    }
}

/// Second call for the same (parent, tool_call_id) returns AlreadySpawned with running child.
#[tokio::test]
async fn test_spawn_handle_reattach_running() {
    let store = make_store();
    let parent = SessionId::new();
    let child = SessionId::new();
    let token = Uuid::new_v4();

    // First call: claim
    store
        .try_claim_spawn(parent, "call-2", child, "Worker", "task", token)
        .await
        .expect("first claim");

    // Second call: same parent + tool_call_id — should return AlreadySpawned(running)
    let child2 = SessionId::new(); // different prospective child; should be ignored
    let token2 = Uuid::new_v4();
    let result = store
        .try_claim_spawn(parent, "call-2", child2, "Worker", "task", token2)
        .await
        .expect("second call should not error");

    match result {
        SpawnClaimResult::AlreadySpawned {
            child_session_id,
            terminal_result,
        } => {
            assert_eq!(child_session_id, child, "should return the original child");
            assert!(
                terminal_result.is_none(),
                "not yet settled — terminal_result should be None"
            );
        }
        other => panic!("Expected AlreadySpawned, got {other:?}"),
    }
}

/// After settle, replay returns AlreadySpawned with the stored result.
#[tokio::test]
async fn test_spawn_handle_reattach_settled() {
    let store = make_store();
    let parent = SessionId::new();
    let child = SessionId::new();
    let token = Uuid::new_v4();

    // Claim
    store
        .try_claim_spawn(parent, "call-3", child, "Worker", "task", token)
        .await
        .expect("claim");

    // Settle
    store
        .settle_spawn(parent, "call-3", token, "Final answer: 42")
        .await
        .expect("settle");

    // Replay — should return AlreadySpawned with the terminal result
    let result = store
        .try_claim_spawn(parent, "call-3", child, "Worker", "task", Uuid::new_v4())
        .await
        .expect("replay");

    match result {
        SpawnClaimResult::AlreadySpawned {
            child_session_id,
            terminal_result,
        } => {
            assert_eq!(child_session_id, child);
            assert_eq!(
                terminal_result.as_deref(),
                Some("Final answer: 42"),
                "settled result should be returned"
            );
        }
        other => panic!("Expected AlreadySpawned(settled), got {other:?}"),
    }
}

/// Settle with a wrong claim token is silently ignored (no error, no state change).
#[tokio::test]
async fn test_settle_wrong_token_is_ignored() {
    let store = make_store();
    let parent = SessionId::new();
    let child = SessionId::new();
    let token = Uuid::new_v4();
    let wrong_token = Uuid::new_v4();

    store
        .try_claim_spawn(parent, "call-4", child, "Worker", "task", token)
        .await
        .expect("claim");

    // Settle with wrong token — should not error, but should not settle
    store
        .settle_spawn(parent, "call-4", wrong_token, "wrong result")
        .await
        .expect("wrong-token settle should not propagate error");

    // State should still be running
    let result = store
        .try_claim_spawn(parent, "call-4", child, "Worker", "task", Uuid::new_v4())
        .await
        .expect("replay after bad settle");

    match result {
        SpawnClaimResult::AlreadySpawned {
            terminal_result, ..
        } => {
            assert!(
                terminal_result.is_none(),
                "wrong-token settle should not have set terminal_result"
            );
        }
        other => panic!("Expected AlreadySpawned, got {other:?}"),
    }
}

/// Independent (parent, tool_call_id) pairs do not interfere with each other.
#[tokio::test]
async fn test_different_tool_call_ids_are_independent() {
    let store = make_store();
    let parent = SessionId::new();
    let child_a = SessionId::new();
    let child_b = SessionId::new();
    let token_a = Uuid::new_v4();
    let token_b = Uuid::new_v4();

    store
        .try_claim_spawn(parent, "call-a", child_a, "WorkerA", "task a", token_a)
        .await
        .expect("claim a");
    store
        .try_claim_spawn(parent, "call-b", child_b, "WorkerB", "task b", token_b)
        .await
        .expect("claim b");

    store
        .settle_spawn(parent, "call-a", token_a, "result a")
        .await
        .expect("settle a");

    // call-b should still be running
    let result_b = store
        .try_claim_spawn(
            parent,
            "call-b",
            child_b,
            "WorkerB",
            "task b",
            Uuid::new_v4(),
        )
        .await
        .expect("replay b");

    match result_b {
        SpawnClaimResult::AlreadySpawned {
            terminal_result, ..
        } => {
            assert!(
                terminal_result.is_none(),
                "call-b should still be running after call-a was settled"
            );
        }
        other => panic!("Expected AlreadySpawned running, got {other:?}"),
    }
}
