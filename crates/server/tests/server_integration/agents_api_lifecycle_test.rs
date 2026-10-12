//! OpenAI Agents API provider-session lifecycle against PostgreSQL (EVE-1126):
//! tombstones, retention release, and the deletion queue.

use crate::repository_conformance_test::{create_test_principal, session_input};
use crate::test_harness::get_database_url;
use everruns_contracts::typed_id::PrincipalId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::storage::{Database, StorageBackend};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// EVE-1126: every path that drops a provider session id queues it for remote
/// deletion in the same transaction (replacement, retention release, session
/// delete), and the deletion queue retries, converges, and gives up durably.
#[tokio::test]
async fn postgres_agents_api_provider_sessions_are_tombstoned_retained_and_deleted() {
    use everruns_contracts::typed_id::{MessageId, TurnId};
    use everruns_core::agents_api_store::{
        AgentsApiCheckpoint, AgentsApiLease, AgentsApiStore, ParkReason, ToolResultOutbox,
        ToolResultState,
    };
    use everruns_server::background::agents_api_lifecycle::{
        DeletionAttempt, MAX_DELETE_ATTEMPTS, ProviderDeletionRow, ProviderSessionDeleter,
        drain_deletions,
    };
    use everruns_server::storage::{EncryptionService, PgAgentsApiStore};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct ScriptedDeleter {
        script: HashMap<String, DeletionAttempt>,
        calls: Mutex<Vec<(String, Option<String>)>>,
    }
    #[async_trait::async_trait]
    impl ProviderSessionDeleter for ScriptedDeleter {
        async fn delete(&self, row: &ProviderDeletionRow) -> DeletionAttempt {
            self.calls
                .lock()
                .unwrap()
                .push((row.provider_session_id.clone(), row.provider_key.clone()));
            // Rows left by earlier runs are cleared.
            *self
                .script
                .get(&row.provider_session_id)
                .unwrap_or(&DeletionAttempt::Deleted)
        }
    }

    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("connect PostgreSQL");
    let backend = StorageBackend::from_database(Database::new(pool.clone()));
    let principal = create_test_principal(&backend, "agents-api-lifecycle").await;
    let encryption = Arc::new(
        EncryptionService::new("test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
    );
    let store = PgAgentsApiStore::new(pool.clone(), encryption);
    let unique = Uuid::new_v4().simple().to_string();
    let id = |name: &str| format!("sess_{name}_{unique}");

    async fn new_session(
        backend: &StorageBackend,
        principal: PrincipalId,
        label: &str,
    ) -> everruns_contracts::typed_id::SessionId {
        backend
            .create_session(session_input(principal, label))
            .await
            .unwrap()
            .id
    }
    let lease_for = |session_id| AgentsApiLease {
        org_id: DEFAULT_ORG_ID,
        session_id,
        owner: Uuid::new_v4(),
    };
    let tombstone = |provider_session_id: String| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (String, Option<String>, String, i32, Option<String>)>(
                "SELECT reason, provider_key, state, attempts, last_error \
                 FROM agents_api_provider_deletions WHERE provider_session_id = $1",
            )
            .bind(provider_session_id)
            .fetch_optional(&pool)
            .await
            .unwrap()
        }
    };

    // Replacement: a new provider session queues the old one, with the
    // provider that owned it; saving the same id again queues nothing.
    let replaced = new_session(&backend, principal, "agents-api-replaced").await;
    let lease = lease_for(replaced);
    let mut checkpoint = store.acquire(lease).await.unwrap();
    checkpoint.provider_session_id = Some(id("a"));
    checkpoint.provider_key = Some("provider-1".into());
    store.save(lease, &checkpoint).await.unwrap();
    store.save(lease, &checkpoint).await.unwrap();
    assert_eq!(tombstone(id("a")).await, None, "an unchanged id is kept");
    checkpoint.provider_session_id = Some(id("b"));
    checkpoint.provider_key = Some("provider-2".into());
    store.save(lease, &checkpoint).await.unwrap();
    let (reason, provider_key, state, _, _) = tombstone(id("a")).await.unwrap();
    assert_eq!(
        (reason.as_str(), provider_key.as_deref(), state.as_str()),
        ("replaced", Some("provider-1"), "pending")
    );
    store.release(lease).await.unwrap();

    // Retention: an idle, unleased checkpoint of an idle session is released
    // and queued; one that holds a parked call, or is leased, is kept.
    let parked = new_session(&backend, principal, "agents-api-parked").await;
    let parked_lease = lease_for(parked);
    let mut parked_checkpoint = AgentsApiCheckpoint {
        provider_session_id: Some(id("parked")),
        provider_key: Some("provider-1".into()),
        ..store.acquire(parked_lease).await.unwrap()
    };
    parked_checkpoint
        .turn_mut(TurnId::new(), MessageId::new())
        .tool_results
        .insert(
            "call_1".into(),
            ToolResultOutbox {
                provider_turn_id: "turn_1".into(),
                call_id: "call_1".into(),
                name: "lookup".into(),
                arguments: json!({}),
                attempt: 0,
                state: ToolResultState::Parked {
                    reason: ParkReason::ClientResult,
                    iteration: 1,
                },
            },
        );
    store.save(parked_lease, &parked_checkpoint).await.unwrap();
    store.release(parked_lease).await.unwrap();
    let leased = new_session(&backend, principal, "agents-api-leased").await;
    let leased_lease = lease_for(leased);
    let leased_checkpoint = AgentsApiCheckpoint {
        provider_session_id: Some(id("leased")),
        ..store.acquire(leased_lease).await.unwrap()
    };
    store.save(leased_lease, &leased_checkpoint).await.unwrap();
    for session in [replaced, parked, leased] {
        sqlx::query("UPDATE sessions SET status = 'idle' WHERE id = $1")
            .bind(session)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE agents_api_sessions SET updated_at = now() - interval '40 days' \
             WHERE session_id = $1",
        )
        .bind(session)
        .execute(&pool)
        .await
        .unwrap();
    }
    store
        .release_idle_provider_sessions(std::time::Duration::from_secs(30 * 86_400), 10_000)
        .await
        .unwrap();
    let (reason, provider_key, _, _, _) = tombstone(id("b")).await.unwrap();
    assert_eq!(
        (reason.as_str(), provider_key.as_deref()),
        ("released", Some("provider-2"))
    );
    let reacquired = store.acquire(lease_for(replaced)).await.unwrap();
    assert_eq!(
        reacquired.provider_session_id, None,
        "the next turn starts anew"
    );
    assert_eq!(reacquired.provider_key.as_deref(), Some("provider-2"));
    assert_eq!(
        tombstone(id("parked")).await,
        None,
        "a parked call keeps it"
    );
    assert_eq!(
        tombstone(id("leased")).await,
        None,
        "a live worker keeps it"
    );

    // Session delete: the cascade queues the provider session; a session
    // without one queues nothing.
    backend
        .delete_session(DEFAULT_ORG_ID, parked)
        .await
        .unwrap();
    assert_eq!(tombstone(id("parked")).await.unwrap().0, "session_deleted");
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM agents_api_provider_deletions")
        .fetch_one(&pool)
        .await
        .unwrap();
    backend
        .delete_session(DEFAULT_ORG_ID, replaced)
        .await
        .unwrap();
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM agents_api_provider_deletions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after, "no provider session, nothing to delete");

    // Deletion: success removes the tombstone, a retryable failure backs off
    // with a stable code, an impossible one is kept as failed.
    let deleter = ScriptedDeleter {
        script: HashMap::from([
            (id("a"), DeletionAttempt::Deleted),
            (id("b"), DeletionAttempt::Retry("provider_unavailable")),
            (id("parked"), DeletionAttempt::Abandon("provider_deleted")),
        ]),
        calls: Mutex::default(),
    };
    drain_deletions(&pool, &deleter).await.unwrap();
    assert!(
        deleter
            .calls
            .lock()
            .unwrap()
            .contains(&(id("a"), Some("provider-1".to_string())))
    );
    assert_eq!(tombstone(id("a")).await, None);
    let (_, _, state, attempts, last_error) = tombstone(id("b")).await.unwrap();
    assert_eq!(
        (state.as_str(), attempts, last_error.as_deref()),
        ("pending", 1, Some("provider_unavailable"))
    );
    let (_, _, state, _, last_error) = tombstone(id("parked")).await.unwrap();
    assert_eq!(
        (state.as_str(), last_error.as_deref()),
        ("failed", Some("provider_deleted"))
    );

    // Backoff: not due yet, so a second pass does not retry it.
    deleter.calls.lock().unwrap().clear();
    drain_deletions(&pool, &deleter).await.unwrap();
    assert!(
        !deleter
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(provider_session_id, _)| *provider_session_id == id("b"))
    );

    // A deletion that keeps failing is eventually kept as failed.
    sqlx::query(
        "UPDATE agents_api_provider_deletions SET attempts = $2, next_attempt_at = now() \
         WHERE provider_session_id = $1",
    )
    .bind(id("b"))
    .bind(MAX_DELETE_ATTEMPTS - 1)
    .execute(&pool)
    .await
    .unwrap();
    drain_deletions(&pool, &deleter).await.unwrap();
    let (_, _, state, attempts, _) = tombstone(id("b")).await.unwrap();
    assert_eq!((state.as_str(), attempts), ("failed", MAX_DELETE_ATTEMPTS));

    // Clean up this run's failed rows and the leased session.
    store.release(leased_lease).await.unwrap();
    backend
        .delete_session(DEFAULT_ORG_ID, leased)
        .await
        .unwrap();
    sqlx::query("DELETE FROM agents_api_provider_deletions WHERE provider_session_id LIKE $1")
        .bind(format!("%_{unique}"))
        .execute(&pool)
        .await
        .unwrap();
}
