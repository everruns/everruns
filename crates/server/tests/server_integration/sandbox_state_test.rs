//! PostgreSQL contracts for durable logical environment incarnations.

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;

use everruns_capabilities::sandbox_state::SandboxStateStore;
use everruns_contracts::session_sandbox::{
    SessionSandboxInstance, SessionSandboxState, SessionSandboxStatus,
};
use everruns_server::domains::sandbox_templates::record::{
    ResolvedSandboxSpec, SandboxBootstrap, SandboxContainmentSpec, SandboxDurability,
    SandboxLifecycle, SandboxNetworkPolicy, SandboxTargetSpec,
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
    let backend = StorageBackend::from_database(Database::new(pool.clone()));
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
        everruns_capabilities::sandbox_state::SandboxStateError::StaleGeneration {
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

#[tokio::test]
async fn postgres_pins_profile_and_keeps_logical_environment_after_instance_deletion() {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let backend = StorageBackend::from_database(Database::new(pool.clone()));
    let owner = create_test_principal(&backend, "profiled-environment").await;
    let session = backend
        .create_session(session_input(owner, "profiled-environment"))
        .await
        .expect("create session");
    let profile = ResolvedSandboxSpec {
        template_revision_id: None,
        target: SandboxTargetSpec::managed("daytona"),
        containment: SandboxContainmentSpec {
            network: SandboxNetworkPolicy::Allow,
            ..SandboxContainmentSpec::isolated()
        },
        durability: SandboxDurability::Checkpointed,
        lifecycle: SandboxLifecycle::default(),
        bootstrap: SandboxBootstrap::default(),
    };

    let pinned = backend
        .pin_primary_sandbox(session.id, "build", &profile)
        .await
        .expect("pin environment");
    assert_eq!(pinned.binding_name, "build");
    assert_eq!(pinned.spec, profile);
    assert_eq!(pinned.observed_state, "absent");

    let store = PgSandboxCheckpointStore::new(pool);
    let now = Utc::now().to_rfc3339();
    let state = SessionSandboxState {
        sandbox: None,
        provider: "daytona".to_string(),
        status: SessionSandboxStatus::Running,
        instance: SessionSandboxInstance {
            external_id: "sb-profiled".to_string(),
            display_name: None,
            workspace_path: Some("/home/daytona/workspace".to_string()),
            provider_state: json!({}),
            metadata: json!({}),
        },
        init_completed_at: None,
        last_init_error: None,
        created_at: now.clone(),
        updated_at: now,
    };
    let sandbox_ref = store
        .save_state(session.id, &state, None)
        .await
        .expect("attach physical instance");
    assert_eq!(
        backend
            .get_primary_sandbox(session.id)
            .await
            .expect("load environment")
            .expect("environment exists")
            .observed_state,
        "ready"
    );

    assert!(
        store
            .delete_state(session.id, "daytona", Some(&sandbox_ref))
            .await
            .expect("delete physical instance")
    );
    let logical = backend
        .get_primary_sandbox(session.id)
        .await
        .expect("load retained environment")
        .expect("logical environment survives");
    assert_eq!(logical.id, pinned.id);
    assert_eq!(logical.observed_state, "deleted");
    assert_eq!(logical.desired_state, "deleted");
}
