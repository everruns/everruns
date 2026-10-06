// SubagentSpawnStore tests (EVE-535).
//
// Tests the CAS semantics of PgSubagentSpawnStore on a private test database.

use everruns_contracts::typed_id::SessionId;
use everruns_core::{
    delegation_services::SpawnClaimResult, delegation_services::SubagentSpawnStore,
};
use everruns_server::storage::{CreateSessionRow, PgSubagentSpawnStore, StorageBackend};
use uuid::Uuid;

/// A spawn store plus the database its sessions live in.
struct Fixture {
    db: StorageBackend,
    store: PgSubagentSpawnStore,
}

impl std::ops::Deref for Fixture {
    type Target = PgSubagentSpawnStore;
    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl Fixture {
    /// A real session: spawn handles reference parent and child by foreign key.
    async fn session(&self) -> SessionId {
        self.db
            .create_session(CreateSessionRow {
                playground_user_id: None,
                source: everruns_server::records::SessionSource::Api,
                workspace_id: None,
                org_id: 1,
                app_id: None,
                channel_id: None,
                trigger_id: None,
                harness_id: None,
                agent_id: None,
                agent_version_id: None,
                agent_config_hash: None,
                virtual_user_id: None,
                owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
                resolved_owner_user_id: None,
                title: Some("Test Session".to_string()),
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                system_prompt: None,
                initial_files: serde_json::Value::Array(vec![]),
                hints: None,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                blueprint_id: None,
                blueprint_config: None,
                parent_session_id: None,
                budget_root_session_id: None,
            })
            .await
            .expect("create session")
            .id
    }
}

fn make_store() -> Fixture {
    let db = StorageBackend::test_database();
    let store = PgSubagentSpawnStore::new(db.database().pool().clone());
    Fixture { db, store }
}

/// First claim for a new (parent, tool_call_id) pair returns Claimed.
#[tokio::test]
async fn test_spawn_handle_claimed_on_first_call() {
    let store = make_store();
    let parent = store.session().await;
    let token = Uuid::new_v4();

    let result = store
        .try_claim_spawn(parent, "call-1", token)
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

/// Before register_child_session, replay returns ClaimedPendingChild with stored token.
#[tokio::test]
async fn test_spawn_handle_reattach_pending() {
    let store = make_store();
    let parent = store.session().await;
    let token = Uuid::new_v4();

    // First call: claim (no register_child_session — simulates crash before register)
    store
        .try_claim_spawn(parent, "call-pending", token)
        .await
        .expect("first claim");

    // Replay: row exists but child not registered → ClaimedPendingChild
    let result = store
        .try_claim_spawn(parent, "call-pending", Uuid::new_v4())
        .await
        .expect("replay should not error");

    match result {
        SpawnClaimResult::ClaimedPendingChild {
            claim_token,
            spawn_handle_id,
        } => {
            assert_eq!(claim_token, token, "should return the stored token");
            assert!(!spawn_handle_id.is_nil());
        }
        other => panic!("Expected ClaimedPendingChild, got {other:?}"),
    }
}

/// After register_child_session, replay returns AlreadyRunning with the stored claim_token.
#[tokio::test]
async fn test_spawn_handle_reattach_running() {
    let store = make_store();
    let parent = store.session().await;
    let child = store.session().await;
    let token = Uuid::new_v4();

    // Claim
    let claim = store
        .try_claim_spawn(parent, "call-2", token)
        .await
        .expect("first claim");

    let (handle_id, stored_token) = match claim {
        SpawnClaimResult::Claimed {
            spawn_handle_id,
            claim_token,
        } => (spawn_handle_id, claim_token),
        other => panic!("Expected Claimed, got {other:?}"),
    };

    // Register child
    store
        .register_child_session(handle_id, stored_token, child)
        .await
        .expect("register should not error");

    // Replay: should return AlreadyRunning with the stored (not fresh) token
    let result = store
        .try_claim_spawn(parent, "call-2", Uuid::new_v4())
        .await
        .expect("second call should not error");

    match result {
        SpawnClaimResult::AlreadyRunning {
            child_session_id,
            claim_token,
        } => {
            assert_eq!(
                child_session_id, child,
                "should return the registered child"
            );
            assert_eq!(
                claim_token, token,
                "must return the stored claim_token for settle"
            );
        }
        other => panic!("Expected AlreadyRunning, got {other:?}"),
    }
}

/// After settle, replay returns AlreadySettled with stored status and result.
#[tokio::test]
async fn test_spawn_handle_reattach_settled() {
    let store = make_store();
    let parent = store.session().await;
    let child = store.session().await;
    let token = Uuid::new_v4();

    // Claim + register
    let claim = store
        .try_claim_spawn(parent, "call-3", token)
        .await
        .expect("claim");
    let (handle_id, _) = match claim {
        SpawnClaimResult::Claimed {
            spawn_handle_id,
            claim_token,
        } => (spawn_handle_id, claim_token),
        other => panic!("Expected Claimed, got {other:?}"),
    };
    store
        .register_child_session(handle_id, token, child)
        .await
        .expect("register");

    // Settle
    store
        .settle_spawn(parent, "call-3", token, "idle", "Final answer: 42")
        .await
        .expect("settle");

    // Replay — should return AlreadySettled with correct status and result
    let result = store
        .try_claim_spawn(parent, "call-3", Uuid::new_v4())
        .await
        .expect("replay");

    match result {
        SpawnClaimResult::AlreadySettled {
            child_session_id,
            terminal_status,
            terminal_result,
        } => {
            assert_eq!(child_session_id, child);
            assert_eq!(terminal_status, "idle");
            assert_eq!(terminal_result, "Final answer: 42");
        }
        other => panic!("Expected AlreadySettled, got {other:?}"),
    }
}

/// Settle with a wrong claim token is silently ignored (no error, no state change).
#[tokio::test]
async fn test_settle_wrong_token_is_ignored() {
    let store = make_store();
    let parent = store.session().await;
    let child = store.session().await;
    let token = Uuid::new_v4();
    let wrong_token = Uuid::new_v4();

    let claim = store
        .try_claim_spawn(parent, "call-4", token)
        .await
        .expect("claim");
    let (handle_id, _) = match claim {
        SpawnClaimResult::Claimed {
            spawn_handle_id,
            claim_token,
        } => (spawn_handle_id, claim_token),
        other => panic!("Expected Claimed, got {other:?}"),
    };
    store
        .register_child_session(handle_id, token, child)
        .await
        .expect("register");

    // Settle with wrong token — should not error, but should not settle
    store
        .settle_spawn(parent, "call-4", wrong_token, "idle", "wrong result")
        .await
        .expect("wrong-token settle should not propagate error");

    // State should still be running
    let result = store
        .try_claim_spawn(parent, "call-4", Uuid::new_v4())
        .await
        .expect("replay after bad settle");

    match result {
        SpawnClaimResult::AlreadyRunning { claim_token, .. } => {
            assert_eq!(claim_token, token, "stored token must be unchanged");
        }
        other => panic!("Expected AlreadyRunning, got {other:?}"),
    }
}

/// Independent (parent, tool_call_id) pairs do not interfere with each other.
#[tokio::test]
async fn test_different_tool_call_ids_are_independent() {
    let store = make_store();
    let parent = store.session().await;
    let child_a = store.session().await;
    let child_b = store.session().await;
    let token_a = Uuid::new_v4();
    let token_b = Uuid::new_v4();

    let claim_a = store
        .try_claim_spawn(parent, "call-a", token_a)
        .await
        .expect("claim a");
    let (handle_a, _) = match claim_a {
        SpawnClaimResult::Claimed {
            spawn_handle_id,
            claim_token,
        } => (spawn_handle_id, claim_token),
        other => panic!("Expected Claimed, got {other:?}"),
    };

    let claim_b = store
        .try_claim_spawn(parent, "call-b", token_b)
        .await
        .expect("claim b");
    let (handle_b, _) = match claim_b {
        SpawnClaimResult::Claimed {
            spawn_handle_id,
            claim_token,
        } => (spawn_handle_id, claim_token),
        other => panic!("Expected Claimed, got {other:?}"),
    };

    store
        .register_child_session(handle_a, token_a, child_a)
        .await
        .expect("register a");
    store
        .register_child_session(handle_b, token_b, child_b)
        .await
        .expect("register b");

    store
        .settle_spawn(parent, "call-a", token_a, "idle", "result a")
        .await
        .expect("settle a");

    // call-b should still be running
    let result_b = store
        .try_claim_spawn(parent, "call-b", Uuid::new_v4())
        .await
        .expect("replay b");

    match result_b {
        SpawnClaimResult::AlreadyRunning { claim_token, .. } => {
            assert_eq!(claim_token, token_b, "call-b should still be running");
        }
        other => panic!("Expected AlreadyRunning, got {other:?}"),
    }
}
