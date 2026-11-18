//! A data plane's call-time lookups (`lookups.rs`), on a real database: the
//! org's internal command runs as on a v1 worker, and its answer comes back
//! sealed for the org under the request's own secret.

use everruns_contracts::dataplane::{LookupAnswer, LookupRequest, lookup_secret};

use super::*;
use crate::domains::data_plane::lookups;

/// The org's internal caller, as the route builds it.
fn internal_ctx(cp: &ControlPlane, org_id: i64) -> crate::domains::common::Ctx {
    crate::domains::common::Ctx::new(
        everruns_core::Caller::internal(org_id),
        cp.db.clone(),
        Arc::new(crate::services::CapabilityService::new(cp.db.clone(), None)),
        cp.encryption.clone(),
        Arc::new(everruns_core::DefaultPermissionResolver),
    )
}

async fn ask(cp: &ControlPlane, org_id: i64, command: &str, params: Value) -> LookupAnswer {
    let row = cp.db.dp_org(ORG).await.unwrap().unwrap();
    let request = uuid::Uuid::now_v7().to_string();
    let sealed = lookups::answer(
        cp,
        &row,
        &internal_ctx(cp, org_id),
        LookupRequest {
            request: request.clone(),
            command: command.into(),
            params,
        },
    )
    .await
    .unwrap();
    assert_eq!(sealed.secret_id, lookup_secret(&request));
    assert_eq!(sealed.revision, 0);
    let key = cp.data_key(&row).unwrap();
    let plain = seal::unseal(&key, &org_public(ORG), &sealed).unwrap();
    // Sealed for this request only: under another request's id it does not open.
    let mut other = sealed.clone();
    other.secret_id = lookup_secret("another");
    assert!(seal::unseal(&key, &org_public(ORG), &other).is_err());
    serde_json::from_slice(&plain).unwrap()
}

#[tokio::test]
async fn a_lookup_runs_the_workers_command_and_comes_back_sealed() {
    let (cp, _, _) = control_plane().await;
    let session = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(ORG))
        .await
        .unwrap()
        .id;
    let answer = ask(
        &cp,
        ORG,
        "worker_get_web_search_settings",
        json!({ "session_id": session.to_string() }),
    )
    .await;
    let LookupAnswer::Value(settings) = answer else {
        panic!("settings, not {answer:?}");
    };
    assert!(settings.is_object(), "{settings}");
}

#[tokio::test]
async fn another_orgs_session_is_not_found_and_only_lookups_are_allowed() {
    let (cp, _, _) = control_plane().await;
    // A session of the default org, asked as ORG.
    let stranger = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(
            everruns_core::DEFAULT_ORG_ID,
        ))
        .await
        .unwrap()
        .id;
    let answer = ask(
        &cp,
        ORG,
        "worker_get_web_search_settings",
        json!({ "session_id": stranger.to_string() }),
    )
    .await;
    assert!(
        matches!(answer, LookupAnswer::Error { kind: 3, .. }),
        "not found: {answer:?}"
    );

    assert!(lookups::allowed("worker_get_connection_token"));
    for command in [
        "worker_get_sandbox_connection_token",
        "worker_get_mcp_connection_token",
        "worker_get_connection_token_for_connection",
        "session_storage_get",
        "agents_delete",
    ] {
        assert!(!lookups::allowed(command), "{command} is not a lookup");
    }
}
