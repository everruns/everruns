use super::super::tests::{run, session, worker_ctx};
use crate::domains::common::{Ctx, *};
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::SessionId;
use everruns_core::organization::OrgRole;
use everruns_core::{Caller, DEFAULT_ORG_ID};
use serde_json::{Value, json};
use std::sync::Arc;

pub(in crate::domains::session_resources::commands) const COMMANDS: [&str; 3] = [
    "worker_upsert_leased_resource",
    "worker_release_leased_resource",
    "worker_list_session_leased_resources",
];

fn lease(session_id: SessionId, external_id: &str) -> Value {
    json!({
        "session_id": session_id.to_string(),
        "provider": "daytona",
        "resource_type": "sandbox",
        "external_id": external_id,
        "display_name": "Sandbox",
        "lease_duration_seconds": 900,
        "metadata": { "region": "eu" },
    })
}

#[tokio::test]
async fn the_worker_lease_store_round_trips_through_dispatch() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let ctx = worker_ctx(db, DEFAULT_ORG_ID);

    let created = run(
        &ctx,
        "worker_upsert_leased_resource",
        lease(session_id, "sbx-1"),
    )
    .await
    .expect("upsert");
    assert_eq!(created["status"], "active");
    assert_eq!(created["metadata"]["region"], "eu");
    let touched = run(
        &ctx,
        "worker_upsert_leased_resource",
        lease(session_id, "sbx-1"),
    )
    .await
    .expect("upsert again");
    assert_eq!(
        touched["id"], created["id"],
        "an upsert refreshes the lease"
    );

    let listed = run(
        &ctx,
        "worker_list_session_leased_resources",
        json!({ "session_id": session_id.to_string() }),
    )
    .await
    .expect("list");
    assert_eq!(listed.as_array().map(Vec::len), Some(1));
    let registry = run(
        &ctx,
        "worker_list_session_resources",
        json!({ "session_id": session_id.to_string() }),
    )
    .await
    .expect("registry");
    assert_eq!(
        registry[0]["resource_id"], created["id"],
        "an upsert mirrors into the session resource registry"
    );

    let mut release = lease(session_id, "sbx-1");
    release["display_name"] = Value::Null;
    let released = run(&ctx, "worker_release_leased_resource", release.clone())
        .await
        .expect("release");
    assert_eq!(released["status"], "released");
    release["external_id"] = json!("absent");
    let absent = run(&ctx, "worker_release_leased_resource", release)
        .await
        .expect("release of an unknown lease");
    assert!(absent.is_null());
}

/// THREAT[TM-TENANT-001]: a worker acting for one org cannot lease, release, or
/// read another org's session resources by naming the session. The RPCs these
/// replace took the session alone.
#[tokio::test]
async fn another_orgs_session_is_not_found() {
    let db = Arc::new(StorageBackend::test_database());
    let foreign = session(&db, DEFAULT_ORG_ID).await;
    let owner_ctx = worker_ctx(db.clone(), DEFAULT_ORG_ID);
    run(
        &owner_ctx,
        "worker_upsert_leased_resource",
        lease(foreign, "sbx-1"),
    )
    .await
    .expect("upsert in the owning org");
    let ctx = worker_ctx(db, DEFAULT_ORG_ID + 1);

    for name in COMMANDS {
        let error = run(&ctx, name, lease(foreign, "sbx-1"))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::NotFound(_)),
            "{name}: {error:?}"
        );
    }

    let still = run(
        &owner_ctx,
        "worker_list_session_leased_resources",
        json!({ "session_id": foreign.to_string() }),
    )
    .await
    .unwrap();
    assert_eq!(
        still[0]["status"], "active",
        "the foreign worker changed nothing"
    );
}

/// THREAT[TM-AUTHZ-002]: the worker lease store is not a person's API. An
/// owner, who passes every session policy, is still refused.
#[tokio::test]
async fn a_person_cannot_run_internal_commands() {
    let db = Arc::new(StorageBackend::test_database());
    let session_id = session(&db, DEFAULT_ORG_ID).await;
    let owner = Caller {
        is_internal: false,
        user_id: Some(uuid::Uuid::nil()),
        role: OrgRole::Owner,
        ..Caller::internal(DEFAULT_ORG_ID)
    };
    let ctx = Ctx::minimal_for_test(owner, db, None);

    for name in COMMANDS {
        let error = run(&ctx, name, lease(session_id, "sbx-1"))
            .await
            .expect_err(name);
        assert!(
            matches!(error.kind, CommandErrorKind::Forbidden(_)),
            "{name}: {error:?}"
        );
    }
}
