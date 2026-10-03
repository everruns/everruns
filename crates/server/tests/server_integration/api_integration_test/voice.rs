//! API integration tests: realtime voice session tenancy (EVE-1171).
//!
//! Every voice route takes a caller-supplied session id. The org-scoped session
//! lookup returning `None` must be a rejection, not a pass: otherwise a caller
//! that knows another organization's session id can write voice leased
//! resources and `voice.session.*` events into that session.

use crate::session_row_fixture::base_session_row;
use crate::test_harness;
use axum::http::StatusCode;
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_server::storage::models::{
    CreateOrganizationRow, CreatePrincipalRow, CreateSessionRow, UpsertLeasedResourceRow,
};
use serde_json::{Value, json};
use test_harness::TestServer;
use uuid::Uuid;

const VOICE_CONNECTION_ID: &str = "voice_conn_foreign";

/// Seed a session in a second organization, holding an active voice lease, so
/// a cross-org write would show up as either a released lease or a new event.
async fn seed_foreign_voice_session(server: &TestServer) -> SessionId {
    let org = server
        .db
        .create_organization(CreateOrganizationRow {
            public_id: everruns_platform::generate_org_public_id(),
            name: "Foreign voice org".to_string(),
            created_by: None,
        })
        .await
        .expect("create foreign org");
    let principal = server
        .db
        .create_principal(CreatePrincipalRow {
            id: PrincipalId::new(),
            org_id: org.org_id,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "voice_cross_org_test" }),
        })
        .await
        .expect("create foreign principal");
    let session = server
        .db
        .create_session(CreateSessionRow {
            owner_principal_id: principal.id,
            ..base_session_row(org.org_id)
        })
        .await
        .expect("create foreign session");
    server
        .db
        .upsert_leased_resource(UpsertLeasedResourceRow {
            org_id: org.org_id,
            session_id: session.id,
            provider: "openai".to_string(),
            resource_type: "voice_connection".to_string(),
            external_id: VOICE_CONNECTION_ID.to_string(),
            display_name: Some("Voice Connection".to_string()),
            owner_user_id: None,
            lease_duration_seconds: 900,
            lease_expires_at: chrono::Utc::now() + chrono::Duration::minutes(15),
            metadata: json!({ "status": "active" }),
        })
        .await
        .expect("seed foreign voice lease");
    session.id
}

async fn foreign_state(server: &TestServer, session_id: SessionId) -> (Vec<String>, usize) {
    let leases = server
        .db
        .list_session_leased_resources(session_id)
        .await
        .expect("list foreign leases");
    let statuses = leases.into_iter().map(|lease| lease.status).collect();
    let events = server
        .db
        .list_events(session_id, None, None, &[], &[], None, None)
        .await
        .expect("list foreign events");
    (statuses, events.len())
}

/// The voice routes, each with a body that passes request validation so the
/// only thing standing between the caller and a write is the session check.
fn voice_requests(session_id: &str) -> Vec<(String, Value)> {
    vec![
        (
            format!("/v1/sessions/{session_id}/voice/client-secret"),
            json!({}),
        ),
        (
            format!("/v1/sessions/{session_id}/voice/calls"),
            json!({ "sdp": "v=0" }),
        ),
        (
            format!("/v1/sessions/{session_id}/voice/{VOICE_CONNECTION_ID}/attach"),
            json!({ "provider_call_id": "rtc_foreign" }),
        ),
        (
            format!("/v1/sessions/{session_id}/voice/{VOICE_CONNECTION_ID}/end"),
            json!({ "reason": "hangup" }),
        ),
    ]
}

#[tokio::test]
async fn test_voice_routes_reject_foreign_org_session_without_writes() {
    let server = TestServer::in_memory().await;
    let foreign_session = seed_foreign_voice_session(&server).await;
    let before = foreign_state(&server, foreign_session).await;
    assert_eq!(before, (vec!["active".to_string()], 0));

    let missing_session = SessionId::new().to_string();
    let foreign_session_str = foreign_session.to_string();
    // Collect every violation before asserting, so one run shows which routes
    // accepted the foreign session and what they wrote.
    let mut violations = Vec::new();
    for ((foreign_uri, body), (missing_uri, _)) in voice_requests(&foreign_session_str)
        .into_iter()
        .zip(voice_requests(&missing_session))
    {
        let foreign = server.post(&foreign_uri, body.clone()).await;
        let missing = server.post(&missing_uri, body).await;
        // Non-disclosing: a foreign session must be indistinguishable from one
        // that never existed, and both must be a 404.
        for (uri, resp) in [(&foreign_uri, &foreign), (&missing_uri, &missing)] {
            if resp.status() != StatusCode::NOT_FOUND {
                violations.push(format!("{uri} -> {} {}", resp.status(), resp.text()));
            }
        }
        if foreign.text() != missing.text() {
            violations.push(format!("{foreign_uri} body differs from a missing session"));
        }
    }
    let after = foreign_state(&server, foreign_session).await;
    if after != before {
        violations.push(format!(
            "foreign session state changed: {before:?} -> {after:?} (lease statuses, event count)"
        ));
    }
    assert!(
        violations.is_empty(),
        "cross-org voice requests were not rejected cleanly:\n{}",
        violations.join("\n")
    );
}

#[tokio::test]
async fn test_voice_end_releases_lease_in_owning_org() {
    let server = TestServer::in_memory().await;
    let session: Value = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_generic_harness_id, "title": "Voice owner" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_id: SessionId = session["id"]
        .as_str()
        .expect("session id")
        .parse()
        .expect("parse session id");
    server
        .db
        .upsert_leased_resource(UpsertLeasedResourceRow {
            org_id: everruns_core::DEFAULT_ORG_ID,
            session_id,
            provider: "openai".to_string(),
            resource_type: "voice_connection".to_string(),
            external_id: VOICE_CONNECTION_ID.to_string(),
            display_name: Some("Voice Connection".to_string()),
            owner_user_id: None,
            lease_duration_seconds: 900,
            lease_expires_at: chrono::Utc::now() + chrono::Duration::minutes(15),
            metadata: json!({ "status": "active" }),
        })
        .await
        .expect("seed voice lease");
    let events_before = foreign_state(&server, session_id).await.1;

    let resp: Value = server
        .post(
            &format!("/v1/sessions/{session_id}/voice/{VOICE_CONNECTION_ID}/end"),
            json!({ "reason": "hangup" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(resp["status"], "ended");

    let (statuses, events_after) = foreign_state(&server, session_id).await;
    assert_ne!(statuses, vec!["active".to_string()], "lease released");
    assert_eq!(
        events_after,
        events_before + 1,
        "voice.session.ended emitted"
    );
}
