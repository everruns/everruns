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
