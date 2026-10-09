//! The in-process worker's stores over internal commands, per org.

use super::*;

/// The in-process schedule store runs the same internal commands a gRPC worker
/// does, scoped to the org it was built for (EVE-56: it once ignored org_id).
#[tokio::test]
async fn schedule_store_runs_the_internal_commands_scoped_to_its_org() {
    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let harness = seed_harness_for_platform_store(&adapters.db, org_id, "sched", false).await;
    let session = seed_platform_session(&adapters.db, org_id, harness, None).await;

    let store = adapters.schedule_store(org_id);
    let created = store
        .create_schedule_enforcing_limits(
            session,
            "nightly".into(),
            Some("0 0 3 * * *".into()),
            None,
            "UTC".into(),
        )
        .await
        .ok()
        .expect("create through the command");
    assert_eq!(store.list_schedules(session).await.unwrap().len(), 1);
    assert_eq!(store.count_active_schedules(session).await.unwrap(), 1);
    let rejected = store
        .create_schedule_enforcing_limits(
            session,
            "too often".into(),
            Some("* * * * *".into()),
            None,
            "UTC".into(),
        )
        .await;
    assert!(matches!(
        rejected,
        Err(everruns_core::session_schedule::ScheduleLimitError::Rejected(_))
    ));

    let other_org = adapters.schedule_store(org_id + 1);
    assert!(other_org.list_schedules(session).await.is_err());
    assert_eq!(other_org.count_active_org_schedules().await.unwrap(), 0);

    let cancelled = store.cancel_schedule(session, created.id).await.unwrap();
    assert!(!cancelled.enabled);
}

/// The in-process session resource registry runs the same internal commands a
/// gRPC worker does, and refuses a session outside the org it was built for.
#[tokio::test]
async fn session_resource_registry_runs_the_internal_commands_scoped_to_its_org() {
    use everruns_core::{RegisterSessionResource, SessionResourceStatus};

    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let harness = seed_harness_for_platform_store(&adapters.db, org_id, "res", false).await;
    let session = seed_platform_session(&adapters.db, org_id, harness, None).await;
    let register = || RegisterSessionResource {
        session_id: session,
        resource_id: "sbx-1".into(),
        kind: "sandbox".into(),
        display_name: "Sandbox".into(),
        status: SessionResourceStatus::Active,
        metadata: serde_json::json!({ "region": "eu" }),
    };

    let registry = adapters.session_resource_registry(org_id).unwrap();
    let entry = registry.register(register()).await.unwrap();
    assert_eq!(entry.metadata["region"], "eu");
    let found = registry.get(session, "sbx-1").await.unwrap().unwrap();
    assert_eq!(found.status, SessionResourceStatus::Active);
    let updated = registry
        .update_status(session, "sbx-1", SessionResourceStatus::Released)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.status, SessionResourceStatus::Released);

    let other_org = adapters.session_resource_registry(org_id + 1).unwrap();
    assert!(other_org.register(register()).await.is_err());
    assert!(other_org.list(session, None).await.is_err());
    assert!(other_org.deregister(session, "sbx-1").await.is_err());

    assert!(registry.deregister(session, "sbx-1").await.unwrap());
    assert!(registry.list(session, None).await.unwrap().is_empty());
}

/// The in-process leased resource store runs the same internal commands a gRPC
/// worker does, mirrors into the session resource registry, and refuses a
/// session outside the org it was built for.
#[tokio::test]
async fn leased_resource_store_runs_the_internal_commands_scoped_to_its_org() {
    use everruns_core::{LeasedResourceStatus, UpsertLeasedResource};

    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let harness = seed_harness_for_platform_store(&adapters.db, org_id, "lease", false).await;
    let session = seed_platform_session(&adapters.db, org_id, harness, None).await;
    let upsert = || UpsertLeasedResource {
        session_id: session,
        provider: "daytona".into(),
        resource_type: "sandbox".into(),
        external_id: "sbx-1".into(),
        display_name: Some("Sandbox".into()),
        owner_user_id: None,
        connection_id: None,
        lease_duration_seconds: 900,
        metadata: serde_json::json!({ "region": "eu" }),
    };

    let store = adapters.leased_resource_store(org_id);
    let lease = store.upsert_resource(upsert()).await.unwrap();
    assert_eq!(lease.status, LeasedResourceStatus::Active);
    let registry = adapters.session_resource_registry(org_id).unwrap();
    let mirrored = registry.get(session, &lease.id.to_string()).await.unwrap();
    assert!(mirrored.is_some(), "the upsert registered the lease");

    let other_org = adapters.leased_resource_store(org_id + 1);
    assert!(other_org.upsert_resource(upsert()).await.is_err());
    assert!(other_org.list_resources(session).await.is_err());
    assert!(
        other_org
            .release_resource(session, "daytona", "sandbox", "sbx-1")
            .await
            .is_err()
    );

    let released = store
        .release_resource(session, "daytona", "sandbox", "sbx-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(released.status, LeasedResourceStatus::Released);
    let listed = store.list_resources(session).await.unwrap();
    assert_eq!(listed.len(), 1);
}

/// The in-process session storage runs the value commands a gRPC worker does,
/// keeps secrets on the database store it was given, and refuses a session
/// outside the org it was built for.
#[tokio::test]
async fn storage_store_runs_the_value_commands_scoped_to_its_org() {
    use everruns_core::session_services::SessionStorageStore;

    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let harness = seed_harness_for_platform_store(&adapters.db, org_id, "kv", false).await;
    let session = seed_platform_session(&adapters.db, org_id, harness, None).await;
    let secrets: std::sync::Arc<dyn SessionStorageStore> = std::sync::Arc::new(
        crate::storage::create_db_session_storage_store_without_encryption(
            crate::storage::Database::new(adapters.db.pool().clone()),
        ),
    );
    let adapters = adapters.with_storage_store(secrets.clone());

    let store = adapters.storage_store(org_id);
    store.set_value(session, "state", "v1").await.unwrap();
    assert_eq!(
        secrets
            .get_value(session, "state")
            .await
            .unwrap()
            .as_deref(),
        Some("v1"),
        "the command wrote the session's row"
    );
    assert_eq!(
        store.get_value(session, "state").await.unwrap().as_deref(),
        Some("v1")
    );
    let keys = store.list_keys(session).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key, "state");

    let other_org = adapters.storage_store(org_id + 1);
    assert!(other_org.set_value(session, "state", "x").await.is_err());
    assert!(other_org.get_value(session, "state").await.is_err());
    assert!(other_org.take_value(session, "state").await.is_err());
    assert!(other_org.delete_value(session, "state").await.is_err());
    assert!(other_org.list_keys(session).await.is_err());

    assert_eq!(
        store.take_value(session, "state").await.unwrap().as_deref(),
        Some("v1")
    );
    assert!(store.take_value(session, "state").await.unwrap().is_none());
    store.set_value(session, "other", "v2").await.unwrap();
    assert!(store.delete_value(session, "other").await.unwrap());
    assert!(store.list_keys(session).await.unwrap().is_empty());

    // Secrets still reach the database store directly; without encryption it
    // refuses them, which shows they did not go through a value command.
    assert!(store.set_secret(session, "TOKEN", "s").await.is_err());
}

/// The in-process task registry runs the same task commands a gRPC worker
/// does, hands back the unredacted spec, and refuses a session outside the
/// org it was built for.
#[tokio::test]
async fn session_task_registry_runs_the_task_commands_scoped_to_its_org() {
    use everruns_core::session_task::{
        CreateSessionTask, NewTaskMessage, SessionTaskState, SessionTaskUpdate,
    };

    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let harness = seed_harness_for_platform_store(&adapters.db, org_id, "tasks", false).await;
    let session = seed_platform_session(&adapters.db, org_id, harness, None).await;
    let registry = adapters.session_task_registry(org_id).expect("registry");

    let create = CreateSessionTask {
        session_id: session,
        id: Some("task_direct".into()),
        kind: "background_tool".into(),
        display_name: "Build".into(),
        spec: serde_json::json!({
            "push_configs": [{ "url": "https://hooks.example.com", "secret": "s3cret" }],
        }),
        state: SessionTaskState::Queued,
        links: Default::default(),
        wake_policy: Default::default(),
    };
    let task = registry.create(create.clone()).await.unwrap();
    assert_eq!(task.spec["push_configs"][0]["secret"], "s3cret");
    let running = registry
        .update(
            session,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Running),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .expect("updated");
    assert_eq!(running.state, SessionTaskState::Running);
    assert_eq!(registry.list(session, None).await.unwrap().len(), 1);
    registry
        .record_message(session, &task.id, NewTaskMessage::outbound_text("progress"))
        .await
        .unwrap();
    assert_eq!(
        registry
            .list_messages(session, &task.id, None, None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        registry
            .request_cancel(session, &task.id)
            .await
            .unwrap()
            .expect("cancel")
            .cancel_requested_at
            .is_some()
    );

    let other_org = adapters
        .session_task_registry(org_id + 1)
        .expect("registry");
    assert!(other_org.create(create).await.is_err());
    assert!(other_org.get(session, &task.id).await.is_err());
    assert!(other_org.list(session, None).await.is_err());
    assert!(
        other_org
            .update(session, &task.id, SessionTaskUpdate::default())
            .await
            .is_err()
    );
    assert!(other_org.request_cancel(session, &task.id).await.is_err());
    assert!(
        other_org
            .record_message(session, &task.id, NewTaskMessage::inbound_text("x"))
            .await
            .is_err()
    );
    assert!(
        other_org
            .list_messages(session, &task.id, None, None)
            .await
            .is_err()
    );
}
