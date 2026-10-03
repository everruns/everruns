//! PostgreSQL contracts for durable logical environment incarnations.

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;

use everruns_platform::sandbox_state::SandboxStateStore;
use everruns_platform::session_sandbox::{
    SessionSandboxInstance, SessionSandboxState, SessionSandboxStatus,
};
use everruns_server::storage::{Database, PgSandboxCheckpointStore, StorageBackend};

use crate::repository_conformance_test::{create_test_principal, session_input};
use crate::test_harness::get_database_url;

/// A logical environment survives physical-instance replacement while its
/// generation fence advances and the former incarnation becomes history.
#[tokio::test]
async fn postgres_replaces_and_fences_physical_incarnations() {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let backend = StorageBackend::Postgres(Database::new(pool.clone()));
    let owner = create_test_principal(&backend, "sandbox-state").await;
    let session = backend
        .create_session(session_input(owner, "sandbox-state"))
        .await
        .expect("create session");
    let store = PgSandboxCheckpointStore::new(pool.clone());

    let now = Utc::now().to_rfc3339();
    let state = |external_id: &str, metadata: serde_json::Value| SessionSandboxState {
        sandbox: None,
        provider: "daytona".to_string(),
        status: SessionSandboxStatus::Running,
        instance: SessionSandboxInstance {
            external_id: external_id.to_string(),
            display_name: Some("Managed workspace".to_string()),
            workspace_path: Some("/workspace".to_string()),
            provider_state: json!({"recovery": {"head_revision": "rev-1"}}),
            metadata,
        },
        init_completed_at: Some(now.clone()),
        last_init_error: None,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let first = store
        .save_state(
            session.id,
            &state("sb-1", json!({"remote": "started"})),
            None,
        )
        .await
        .expect("save first incarnation");
    assert_eq!(first.generation, 1);

    let updated = store
        .save_state(
            session.id,
            &state("sb-1", json!({"heartbeat": 2})),
            Some(&first),
        )
        .await
        .expect("update current incarnation");
    assert_eq!(
        updated, first,
        "an in-place update must not advance fencing"
    );
    assert_eq!(
        store
            .load_current_state(session.id)
            .await
            .expect("load current state")
            .expect("state exists")
            .instance
            .metadata,
        json!({"heartbeat": 2})
    );

    let replacement = store
        .save_state(
            session.id,
            &state("sb-2", json!({"recovered": true})),
            Some(&first),
        )
        .await
        .expect("save replacement incarnation");
    assert_eq!(replacement.id, first.id);
    assert_eq!(replacement.generation, 2);

    let stale = store
        .save_state(
            session.id,
            &state("sb-1", json!({"late": true})),
            Some(&first),
        )
        .await
        .expect_err("a late response from the retired incarnation is fenced");
    assert!(matches!(
        stale,
        everruns_platform::sandbox_state::SandboxStateError::StaleGeneration {
            current: 2,
            carried: 1,
            ..
        }
    ));

    let rows: Vec<(String, i64, Option<chrono::DateTime<Utc>>)> = sqlx::query_as(
        "SELECT external_id, generation, retired_at FROM sandbox_instances WHERE sandbox_id = $1 ORDER BY generation",
    )
    .bind(first.id)
    .fetch_all(&pool)
    .await
    .expect("load incarnation history");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "sb-1");
    assert!(rows[0].2.is_some());
    assert_eq!(rows[1].0, "sb-2");
    assert!(rows[1].2.is_none());

    store
        .delete_state(session.id, "daytona", Some(&first))
        .await
        .expect_err("a stale delete cannot remove the replacement incarnation");
    assert_eq!(
        store
            .load_current_state(session.id)
            .await
            .expect("load after stale delete")
            .expect("replacement survives")
            .instance
            .external_id,
        "sb-2"
    );

    assert!(
        store
            .delete_state(session.id, "daytona", Some(&replacement))
            .await
            .expect("delete logical sandbox")
    );
    assert!(
        store
            .load_current_state(session.id)
            .await
            .expect("load after delete")
            .is_none()
    );
}
