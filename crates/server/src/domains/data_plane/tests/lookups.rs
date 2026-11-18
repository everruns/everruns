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
    assert!(lookups::allowed("worker_knowledge"));
    assert!(lookups::allowed("worker_poppy_call"));
    assert!(lookups::allowed("worker_get_org_egress_allowlist"));
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

/// THREAT[TM-AGENT-017]: a platform call runs as the person the forwarding
/// cell names, only while they are a member, and only for a session that
/// resolves `platform`; without a person it runs as no one.
#[tokio::test]
async fn a_platform_call_runs_as_the_named_member_and_never_as_the_org() {
    use everruns_contracts::dataplane::PLATFORM_COMMAND_SURFACE;

    let (cp, _, _) = control_plane().await;
    // The default org, where the built-in harness the v1 worker's test uses
    // lives; the answer is still sealed for ORG's data plane.
    let org = everruns_core::DEFAULT_ORG_ID;
    // A session without a harness reads as the org's base harness.
    crate::setup::org_init::initialize_org_harnesses(&cp.db, org)
        .await
        .unwrap();
    let member = cp.db.create_test_user(uuid::Uuid::now_v7()).await;
    cp.db
        .ensure_membership(member, org, "member")
        .await
        .unwrap();
    let outsider = cp.db.create_test_user(uuid::Uuid::now_v7()).await;
    let platform = cp
        .db
        .create_session(crate::storage::CreateSessionRow {
            capabilities: json!([{ "ref": "platform" }]),
            ..crate::storage::test_database::test_session_row(org)
        })
        .await
        .unwrap()
        .id;
    let plain = cp
        .db
        .create_session(crate::storage::test_database::test_session_row(org))
        .await
        .unwrap()
        .id;
    let call = |session: &dyn std::fmt::Display, acting_user: Value| {
        json!({
            "session_id": session.to_string(),
            "input_message_id": uuid::Uuid::now_v7().to_string(),
            "operation": "discover",
            "arguments": { "query": "models" },
            "acting_user": acting_user,
        })
    };

    let answer = ask(
        &cp,
        org,
        PLATFORM_COMMAND_SURFACE,
        call(&platform, json!(member.to_string())),
    )
    .await;
    let LookupAnswer::Value(Value::String(output)) = answer else {
        panic!("a member's call runs: {answer:?}");
    };
    assert!(output.contains("list_models"), "{output}");

    // KIND_FORBIDDEN in the internal protocol.
    for (session, acting_user, why) in [
        (platform, Value::Null, "no person: runs as no one"),
        (platform, json!("someone"), "not a user id"),
        (
            platform,
            json!(outsider.to_string()),
            "not a member of the org",
        ),
        (
            plain,
            json!(member.to_string()),
            "the session lacks platform",
        ),
    ] {
        let answer = ask(
            &cp,
            org,
            PLATFORM_COMMAND_SURFACE,
            call(&session, acting_user),
        )
        .await;
        assert!(
            matches!(answer, LookupAnswer::Error { kind: 2, .. }),
            "{why}: {answer:?}"
        );
    }
}
