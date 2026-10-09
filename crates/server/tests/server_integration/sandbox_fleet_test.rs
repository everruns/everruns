//! PostgreSQL contracts for the org-wide Sandbox fleet (migration 175): the
//! lifecycle log, history that outlives its Session, attention reasons, and
//! retention.

use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_capabilities::sandbox_state::SandboxStateStore;
use everruns_contracts::session_sandbox::{
    SessionSandboxInstance, SessionSandboxState, SessionSandboxStatus,
};
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::background::sandbox_history_retention::purge_deleted_sandboxes;
use everruns_server::records::{
    ResolvedSandboxSpec, SandboxBootstrap, SandboxContainmentSpec, SandboxDurability,
    SandboxLifecycle, SandboxNetworkPolicy, SandboxTargetSpec,
};
use everruns_server::storage::backend::sandbox_fleet::SandboxFleetFilter;
use everruns_server::storage::{Database, PgSandboxCheckpointStore, StorageBackend};

use crate::repository_conformance_test::{create_test_principal, session_input};
use crate::test_harness::get_database_url;

fn managed_spec() -> ResolvedSandboxSpec {
    ResolvedSandboxSpec {
        template_revision_id: None,
        target: SandboxTargetSpec::managed("daytona"),
        containment: SandboxContainmentSpec {
            network: SandboxNetworkPolicy::Allow,
            ..SandboxContainmentSpec::isolated()
        },
        durability: SandboxDurability::Checkpointed,
        lifecycle: SandboxLifecycle::default(),
        bootstrap: SandboxBootstrap::default(),
    }
}

fn state(
    external_id: &str,
    status: SessionSandboxStatus,
    init_error: Option<&str>,
) -> SessionSandboxState {
    let now = Utc::now().to_rfc3339();
    SessionSandboxState {
        sandbox: None,
        provider: "daytona".to_string(),
        status,
        instance: SessionSandboxInstance {
            external_id: external_id.to_string(),
            display_name: None,
            workspace_path: Some("/home/daytona/workspace".to_string()),
            provider_state: json!({}),
            metadata: json!({}),
        },
        init_completed_at: None,
        last_init_error: init_error.map(str::to_string),
        created_at: now.clone(),
        updated_at: now,
    }
}

fn only(ids: &[Uuid]) -> SandboxFleetFilter {
    SandboxFleetFilter {
        ids: Some(ids.to_vec()),
        ..Default::default()
    }
}

async fn setup(
    label: &str,
) -> (
    PgPool,
    StorageBackend,
    everruns_contracts::typed_id::SessionId,
) {
    let pool = PgPool::connect(&get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let backend = StorageBackend::from_database(Database::new(pool.clone()));
    let owner = create_test_principal(&backend, label).await;
    let session = backend
        .create_session(session_input(owner, label))
        .await
        .expect("create session");
    (pool, backend, session.id)
}

/// Every state change is logged, a replaced provider resource counts as a
/// recovery, and deleting the Session keeps the Sandbox as readable history.
#[tokio::test]
async fn sandbox_history_outlives_its_session() {
    let (pool, backend, session_id) = setup("fleet-history").await;
    let pinned = backend
        .pin_primary_sandbox(session_id, "build", &managed_spec())
        .await
        .expect("pin");
    let store = PgSandboxCheckpointStore::new(pool.clone());

    let first = store
        .save_state(
            session_id,
            &state("fleet-1", SessionSandboxStatus::Running, None),
            None,
        )
        .await
        .expect("start");
    let paused = store
        .save_state(
            session_id,
            &state("fleet-1", SessionSandboxStatus::Paused, None),
            Some(&first),
        )
        .await
        .expect("pause");
    store
        .save_state(
            session_id,
            &state("fleet-2", SessionSandboxStatus::Running, None),
            Some(&paused),
        )
        .await
        .expect("replace");

    let (rows, total) = backend
        .list_sandbox_fleet(DEFAULT_ORG_ID, &only(&[pinned.id]), 10, 0)
        .await
        .expect("list");
    assert_eq!(total, 1);
    let row = &rows[0];
    assert_eq!(row.fleet_state, "running");
    assert_eq!(row.generation, 2);
    assert_eq!(row.external_id.as_deref(), Some("fleet-2"));
    assert_eq!(row.target_kind.as_deref(), Some("managed"));
    assert_eq!(
        row.session_title.as_deref(),
        Some("conformance-fleet-history")
    );
    assert!(row.attention.is_empty(), "{:?}", row.attention);

    let history = backend
        .list_sandbox_transitions(
            DEFAULT_ORG_ID,
            Some(pinned.id),
            Utc::now() - Duration::hours(1),
            Utc::now() + Duration::seconds(1),
        )
        .await
        .expect("history");
    let states: Vec<(&str, i64)> = history
        .iter()
        .map(|t| (t.state.as_str(), t.generation))
        .collect();
    assert_eq!(
        states,
        vec![("absent", 1), ("ready", 1), ("paused", 1), ("ready", 2)]
    );
    assert!(history.last().unwrap().current);

    let stats = backend
        .sandbox_fleet_aggregates(DEFAULT_ORG_ID, &only(&[pinned.id]))
        .await
        .expect("stats");
    assert_eq!(stats.by_state, vec![("running".to_string(), 1)]);
    assert_eq!(stats.live_by_provider, vec![("daytona".to_string(), 1)]);
    assert_eq!(stats.recoveries_in_window, 1);
    assert_eq!(stats.created_in_window, 1);

    assert!(
        backend
            .delete_session(DEFAULT_ORG_ID, session_id)
            .await
            .expect("delete session")
    );
    let row = backend
        .get_sandbox_fleet_row(DEFAULT_ORG_ID, pinned.id)
        .await
        .expect("get")
        .expect("history survives the Session");
    assert_eq!(row.fleet_state, "deleted");
    assert_eq!(row.session_id, None);
    assert_eq!(
        row.session_title.as_deref(),
        Some("conformance-fleet-history")
    );
    assert!(row.deleted_at.is_some());
    let retired: Option<chrono::DateTime<Utc>> = sqlx::query_scalar(
        "SELECT retired_at FROM sandbox_instances WHERE sandbox_id = $1 AND generation = 2",
    )
    .bind(pinned.id)
    .fetch_one(&pool)
    .await
    .expect("instance");
    assert!(retired.is_some());

    let live = SandboxFleetFilter {
        states: Some(vec!["running".into(), "paused".into()]),
        ..only(&[pinned.id])
    };
    let (rows, _) = backend
        .list_sandbox_fleet(DEFAULT_ORG_ID, &live, 10, 0)
        .await
        .expect("live");
    assert!(rows.is_empty(), "deleted Sandboxes are not live");

    // Retention keeps recent history and purges what is past the window.
    assert_eq!(purge_deleted_sandboxes(&pool, 30).await.expect("purge"), 0);
    sqlx::query("UPDATE sandboxes SET deleted_at = now() - interval '40 days' WHERE id = $1")
        .bind(pinned.id)
        .execute(&pool)
        .await
        .expect("age");
    assert!(purge_deleted_sandboxes(&pool, 30).await.expect("purge") >= 1);
    assert!(
        backend
            .get_sandbox_fleet_row(DEFAULT_ORG_ID, pinned.id)
            .await
            .expect("get")
            .is_none()
    );
}

/// Attention reasons come from the stored state, and in-process targets stay
/// out of the fleet unless asked for.
#[tokio::test]
async fn attention_reasons_and_in_process_targets() {
    let (pool, backend, session_id) = setup("fleet-attention").await;
    let pinned = backend
        .pin_primary_sandbox(session_id, "build", &managed_spec())
        .await
        .expect("pin");
    let store = PgSandboxCheckpointStore::new(pool.clone());
    let started = store
        .save_state(
            session_id,
            &state(
                "fleet-attn",
                SessionSandboxStatus::Running,
                Some("pip exited 1"),
            ),
            None,
        )
        .await
        .expect("start");
    store
        .save_state(
            session_id,
            &state(
                "fleet-attn",
                SessionSandboxStatus::Lost,
                Some("pip exited 1"),
            ),
            Some(&started),
        )
        .await
        .expect("lose");

    let attention = SandboxFleetFilter {
        needs_attention: true,
        ..only(&[pinned.id])
    };
    let (rows, _) = backend
        .list_sandbox_fleet(DEFAULT_ORG_ID, &attention, 10, 0)
        .await
        .expect("attention");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].attention, vec!["lost", "init_failed"]);

    // A running Sandbox with no activity for over an hour is flagged idle.
    sqlx::query(
        "UPDATE sandboxes SET observed_state = 'ready', last_activity_at = now() - interval '2 hours' WHERE id = $1",
    )
    .bind(pinned.id)
    .execute(&pool)
    .await
    .expect("age activity");
    let row = backend
        .get_sandbox_fleet_row(DEFAULT_ORG_ID, pinned.id)
        .await
        .expect("get")
        .expect("row");
    assert!(row.attention.contains(&"idle_running".to_string()));

    // A virtual-filesystem Sandbox has no provider resource.
    let owner = create_test_principal(&backend, "fleet-vfs").await;
    let vfs_session = backend
        .create_session(session_input(owner, "fleet-vfs"))
        .await
        .expect("session");
    let vfs = backend
        .pin_primary_sandbox(
            vfs_session.id,
            "default",
            &ResolvedSandboxSpec {
                target: SandboxTargetSpec::vfs("bashkit"),
                ..managed_spec()
            },
        )
        .await
        .expect("pin vfs");
    let (rows, _) = backend
        .list_sandbox_fleet(DEFAULT_ORG_ID, &only(&[vfs.id]), 10, 0)
        .await
        .expect("list");
    assert!(rows.is_empty(), "in-process targets are hidden by default");
    let with_in_process = SandboxFleetFilter {
        include_in_process: true,
        ..only(&[vfs.id])
    };
    let (rows, _) = backend
        .list_sandbox_fleet(DEFAULT_ORG_ID, &with_in_process, 10, 0)
        .await
        .expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].fleet_state, "not_started");
}
