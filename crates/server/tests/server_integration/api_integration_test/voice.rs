//! API integration tests: voice calls, voice channels and voice tenancy (EVE-1171).
//!
//! Every voice route takes a caller-supplied session id. The org-scoped session
//! lookup returning `None` must be a rejection, not a pass: otherwise a caller
//! that knows another organization's session id can write voice leased
//! resources and `voice.session.*` events into that session.

use crate::session_row_fixture::base_session_row;
use crate::test_harness;
use axum::http::StatusCode;
use everruns_contracts::typed_id::{PrincipalId, SessionId};
use everruns_server::storage::UpsertLeasedResourceRow;
use everruns_server::storage::models::{
    CreateOrganizationRow, CreatePrincipalRow, CreateSessionRow,
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
            public_id: everruns_server::records::generate_org_public_id(),
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
            connection_id: None,
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

/// The session voice routes, each with a body that passes request validation
/// so the only thing standing between the caller and a write is the session
/// check.
fn voice_requests(session_id: &str) -> Vec<(String, Value)> {
    vec![
        (
            format!("/v1/sessions/{session_id}/voice/calls"),
            json!({ "sdp": "v=0" }),
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
            connection_id: None,
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

/// An agent on the simulated provider, which also serves simulated voice.
async fn create_voice_agent(server: &TestServer) -> Value {
    let provider: Value = server
        .post(
            "/v1/providers",
            json!({ "name": "voice-sim", "provider_type": "llmsim", "api_key": "sim-key" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let model: Value = server
        .post(
            &format!("/v1/providers/{}/models", provider["id"].as_str().unwrap()),
            json!({ "model_id": "voice-sim-model", "display_name": "Voice sim", "enabled": true }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    server
        .post(
            "/v1/agents",
            json!({
                "name": "voice-agent",
                "system_prompt": "You are a concise test agent.",
                "default_model_id": model["id"],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

#[tokio::test]
async fn test_voice_channel_config_is_validated_and_normalized() {
    let server = TestServer::in_memory().await;
    let agent = create_voice_agent(&server).await;
    let channels = format!("/v1/agents/{}/channels", agent["id"].as_str().unwrap());

    let bad = server
        .post(
            &channels,
            json!({ "channel_type": "voice", "channel_config": { "voice": " " } }),
        )
        .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST, "{}", bad.text());
    let with_auth = server
        .post(
            &channels,
            json!({ "channel_type": "voice", "channel_config": { "auth": { "mode": "oidc" } } }),
        )
        .await;
    assert_eq!(with_auth.status(), StatusCode::BAD_REQUEST);

    let channel: Value = server
        .post(
            &channels,
            json!({ "channel_type": "voice", "channel_config": {} }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(channel["channel_type"], "voice");
    assert_eq!(channel["channel_config"]["mode"], "delegated");
    assert_eq!(channel["channel_config"]["model"], "gpt-realtime-2");
    assert_eq!(channel["channel_config"]["interruption"], "steer");
}

#[tokio::test]
async fn test_voice_channel_call_speaks_greeting_sends_utterances_and_speaks_answers() {
    use everruns_core::events::{EventContext, EventRequest, OutputMessageDeltaData};
    use everruns_llmsim::realtime::{SIMULATED_ANSWER_SDP, SimulatedCall};
    use std::time::Duration;

    let server = TestServer::in_memory().await;
    let agent = create_voice_agent(&server).await;
    let agent_id = agent["id"].as_str().unwrap();
    let channel: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({
                "channel_type": "voice",
                "channel_config": {
                    "greeting": "Hi, you are talking to an AI assistant.",
                    "filler_after_ms": 0,
                    "speaking_style": "Warm and brief."
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let channel_id = channel["id"].as_str().unwrap();

    let started: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels/{channel_id}/voice/calls"),
            json!({ "sdp": "v=0" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(started["voice"]["answer_sdp"], SIMULATED_ANSWER_SDP);
    assert_eq!(started["voice"]["provider"], "llmsim");
    assert_eq!(started["voice"]["channel_id"], channel_id);
    assert_eq!(started["session"]["agent_id"], agent_id);
    let session_id: SessionId = started["session"]["id"].as_str().unwrap().parse().unwrap();
    let call_id = started["voice"]["provider_call_id"].as_str().unwrap();
    let call = SimulatedCall::find(call_id).expect("simulated call placed");
    let speech = call.session().expect("session settings");
    assert!(speech.instructions.contains("Warm and brief."));
    assert!(
        speech
            .safety_identifier
            .as_deref()
            .is_some_and(|id| id.starts_with("evr_"))
    );

    // The greeting is spoken as soon as the call is attached.
    let spoken = call.wait_spoken(1, Duration::from_secs(10)).await;
    assert_eq!(
        spoken,
        vec!["Hi, you are talking to an AI assistant.".to_string()]
    );

    // A caller utterance becomes a user message on the session.
    call.say("What is the weather in Kyiv?");
    let mut found = false;
    for _ in 0..100 {
        let messages: Value = server
            .get(&format!("/v1/sessions/{session_id}/messages"))
            .await
            .assert_status(StatusCode::OK)
            .json();
        found = messages["data"].as_array().into_iter().flatten().any(|m| {
            m["role"] == "user"
                && m["content"][0]["text"] == "What is the weather in Kyiv?"
                && m["metadata"]["source"] == "voice"
        });
        if found {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(found, "utterance stored as a user message from voice");

    // The agent's streamed answer is spoken sentence by sentence.
    server
        .event_service
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            OutputMessageDeltaData {
                turn_id: everruns_contracts::typed_id::TurnId::new(),
                message_id: everruns_contracts::typed_id::MessageId::new(),
                delta: "It is sunny. Highs of twenty today. ".into(),
                accumulated: "It is sunny. Highs of twenty today. ".into(),
                phase: None,
            },
        ))
        .await
        .expect("emit answer delta");
    let spoken = call.wait_spoken(3, Duration::from_secs(10)).await;
    assert_eq!(&spoken[1..], ["It is sunny.", "Highs of twenty today."]);

    // Ending the call releases its lease and records the end.
    let voice_connection_id = started["voice"]["voice_connection_id"].as_str().unwrap();
    server
        .post(
            &format!("/v1/sessions/{session_id}/voice/{voice_connection_id}/end"),
            json!({ "reason": "done" }),
        )
        .await
        .assert_status(StatusCode::OK);
    let (statuses, _) = foreign_state(&server, session_id).await;
    assert!(
        !statuses.contains(&"active".to_string()),
        "lease released: {statuses:?}"
    );
    let events = server
        .db
        .list_events(session_id, None, None, &[], &[], None, None)
        .await
        .unwrap();
    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    for expected in [
        "voice.session.started",
        "voice.input_transcript.completed",
        "voice.session.ended",
    ] {
        assert!(
            types.contains(&expected),
            "{expected} missing from {types:?}"
        );
    }
}

#[tokio::test]
async fn test_session_voice_call_uses_the_agents_voice_channel() {
    use everruns_llmsim::realtime::SimulatedCall;

    let server = TestServer::in_memory().await;
    let agent = create_voice_agent(&server).await;
    let agent_id = agent["id"].as_str().unwrap();
    let channel: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({ "channel_type": "voice", "channel_config": { "voice": "cedar" } }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session: Value = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id, "agent_id": agent_id }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let started: Value = server
        .post(
            &format!(
                "/v1/sessions/{}/voice/calls",
                session["id"].as_str().unwrap()
            ),
            json!({ "sdp": "v=0" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(started["channel_id"], channel["id"]);
    assert_eq!(started["voice"], "cedar");
    let call = SimulatedCall::find(started["provider_call_id"].as_str().unwrap()).unwrap();
    assert_eq!(call.session().unwrap().voice, "cedar");
    call.hang_up();

    // A channel of another type cannot be used for a voice call.
    let webhook: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({ "channel_type": "webhook", "channel_config": { "token": "webhook-token", "message": "{{payload}}" } }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let wrong = server
        .post(
            &format!(
                "/v1/agents/{agent_id}/channels/{}/voice/calls",
                webhook["id"].as_str().unwrap()
            ),
            json!({ "sdp": "v=0" }),
        )
        .await;
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
}
