//! API integration tests: pinning an Agent version to an exposure (EVE-1139).
//!
//! A pin is written through the Agent channel or trigger API and must be
//! honoured by a real ingress path against PostgreSQL after the agent's draft
//! and default version have moved on: FCP for a native endpoint, the public
//! webhook route for a webhook trigger, and the API-key session route for an
//! endpoint whose pin was carried over from the App era.

use crate::test_harness;
use axum::http::{Method, StatusCode};
use everruns_server::records::Agent;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_harness::TestServer;

/// Session-time version resolution reads the process-level flag in this pilot;
/// enable it before the in-process server builds its state.
fn enable_agent_versions() {
    unsafe {
        std::env::set_var("FEATURE_AGENT_VERSIONS", "prod");
    }
}

async fn create_agent(server: &TestServer, name: &str) -> String {
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("{name}-{suffix}"),
                "display_name": name,
                "system_prompt": "You are version one"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    agent.public_id.to_string()
}

async fn save_version(server: &TestServer, agent_id: &str, summary: &str) -> String {
    let version: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/versions"),
            json!({ "summary": summary, "change_kind": "manual" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    version["id"].as_str().expect("version id").to_string()
}

/// Edit the agent's draft, save it as a new version and make that the default.
async fn move_draft_and_default(server: &TestServer, agent_id: &str) -> String {
    server
        .patch(
            &format!("/v1/agents/{agent_id}"),
            json!({ "system_prompt": "You are version two" }),
        )
        .await
        .assert_status(StatusCode::OK);
    let v2 = save_version(server, agent_id, "Prompt update").await;
    server
        .post(
            &format!("/v1/agents/{agent_id}/versions/default"),
            json!({ "version_id": v2 }),
        )
        .await
        .assert_status(StatusCode::OK);
    v2
}

async fn session_version(server: &TestServer, session_id: &str) -> Option<String> {
    let session: Value = server
        .get(&format!("/v1/sessions/{session_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    session["agent_version_id"].as_str().map(str::to_owned)
}

async fn create_published_fcp_channel(server: &TestServer, agent_id: &str, extra: Value) -> Value {
    let mut body = json!({
        "channel_type": "fcp",
        "channel_config": { "anonymous": true, "response_timeout_seconds": 1 },
    });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let endpoint: Value = server
        .post(&format!("/v1/agents/{agent_id}/channels"), body)
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let channel_id = endpoint["id"].as_str().unwrap();
    server
        .post(
            &format!("/v1/agents/{agent_id}/channels/{channel_id}/publish"),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);
    endpoint
}

/// Send one FCP turn without a session cookie (so a new session starts) and
/// return the Agent version that session captured.
async fn fcp_session_version(server: &TestServer, channel_id: &str) -> Option<String> {
    let response = server
        .request_raw(
            Method::POST,
            &format!("/v1/channels/{channel_id}/fcp"),
            vec![("content-type", "text/plain")],
            b"which version are you?".to_vec(),
        )
        .await;
    // No worker runs in tests, so the turn itself times out (504); the session
    // is created before the handler waits for the reply.
    assert!(
        response.status() == StatusCode::OK || response.status() == StatusCode::GATEWAY_TIMEOUT,
        "FCP ingress rejected the turn: {}",
        response.status()
    );
    let version: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT s.agent_version_id
         FROM sessions s
         JOIN agent_channels ae ON ae.id = s.channel_id
         WHERE ae.public_id = $1
         ORDER BY s.created_at DESC
         LIMIT 1",
    )
    .bind(channel_id)
    .fetch_one(&server.pool)
    .await
    .expect("FCP ingress created a session");
    version.map(|id| everruns_contracts::typed_id::AgentVersionId::from_uuid(id).to_string())
}

#[tokio::test]
async fn test_pinned_channel_keeps_running_pinned_version_after_draft_changes() {
    enable_agent_versions();
    let server = TestServer::new().await;
    let agent_id = create_agent(&server, "pinned-endpoint-agent").await;
    let v1 = save_version(&server, &agent_id, "Initial").await;

    // Pin v1 to a new endpoint through the management API.
    let pinned = create_published_fcp_channel(
        &server,
        &agent_id,
        json!({ "agent_version_policy": "pinned", "agent_version_id": v1 }),
    )
    .await;
    assert_eq!(pinned["agent_version_policy"], "pinned");
    assert_eq!(pinned["agent_version_id"], v1);
    let pinned_id = pinned["id"].as_str().unwrap().to_string();

    // The agent's draft moves on, and the new version becomes the default.
    let v2 = move_draft_and_default(&server, &agent_id).await;

    // An unpinned sibling follows the default, proving the pin is what holds
    // the first endpoint back.
    let unpinned = create_published_fcp_channel(&server, &agent_id, json!({})).await;
    assert_eq!(unpinned["agent_version_policy"], "default");
    assert!(unpinned["agent_version_id"].is_null());

    assert_eq!(
        fcp_session_version(&server, &pinned_id).await.as_deref(),
        Some(v1.as_str()),
        "pinned endpoint must keep running the pinned version"
    );
    assert_eq!(
        fcp_session_version(&server, unpinned["id"].as_str().unwrap())
            .await
            .as_deref(),
        Some(v2.as_str()),
    );

    // The pin is visible on read.
    let listed: Value = server
        .get(&format!("/v1/agents/{agent_id}/channels"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let listed_pinned = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|endpoint| endpoint["id"] == pinned_id.as_str())
        .expect("pinned endpoint listed");
    assert_eq!(listed_pinned["agent_version_policy"], "pinned");
    assert_eq!(listed_pinned["agent_version_id"], v1);

    // Pinning to a version another agent owns is rejected.
    let other_agent = create_agent(&server, "pinned-endpoint-other").await;
    let foreign = save_version(&server, &other_agent, "Other").await;
    server
        .patch(
            &format!("/v1/agents/{agent_id}/channels/{pinned_id}"),
            json!({ "agent_version_policy": "pinned", "agent_version_id": foreign }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // Unpin: the endpoint now follows the default version.
    let unpinned_again: Value = server
        .patch(
            &format!("/v1/agents/{agent_id}/channels/{pinned_id}"),
            json!({ "agent_version_policy": "default" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(unpinned_again["agent_version_policy"], "default");
    assert!(unpinned_again["agent_version_id"].is_null());
    assert_eq!(
        fcp_session_version(&server, &pinned_id).await.as_deref(),
        Some(v2.as_str()),
    );
}

#[tokio::test]
async fn test_pinned_webhook_trigger_keeps_running_pinned_version() {
    enable_agent_versions();
    let server = TestServer::new().await;
    let agent_id = create_agent(&server, "pinned-trigger-agent").await;
    let v1 = save_version(&server, &agent_id, "Initial").await;

    let token = format!("whk_{}", uuid::Uuid::new_v4().simple());
    let trigger: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/triggers"),
            json!({
                "trigger_type": "webhook",
                "token": token,
                "message": "Process {{payload}}",
                "session_mode": "session_per_invocation",
                "agent_version_policy": "pinned",
                "agent_version_id": v1,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(trigger["agent_version_policy"], "pinned");
    assert_eq!(trigger["agent_version_id"], v1);
    let ingress_id = trigger["ingress_id"].as_str().expect("webhook ingress");

    let v2 = move_draft_and_default(&server, &agent_id).await;

    let fire = || async {
        let accepted: Value = server
            .request_raw(
                Method::POST,
                &format!("/v1/channels/{ingress_id}/webhook"),
                vec![
                    ("content-type", "application/json"),
                    ("x-everruns-webhook-token", token.as_str()),
                ],
                br#"{"event":"version-check"}"#.to_vec(),
            )
            .await
            .assert_status(StatusCode::ACCEPTED)
            .json();
        session_version(
            &server,
            accepted["session_id"].as_str().expect("session id"),
        )
        .await
    };
    assert_eq!(
        fire().await.as_deref(),
        Some(v1.as_str()),
        "pinned trigger must keep running the pinned version"
    );

    let trigger_id = trigger["id"].as_str().unwrap();
    let unpinned: Value = server
        .patch(
            &format!("/v1/agents/{agent_id}/triggers/{trigger_id}"),
            json!({ "agent_version_policy": "default" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(unpinned["agent_version_policy"], "default");
    assert!(unpinned["agent_version_id"].is_null());
    assert_eq!(fire().await.as_deref(), Some(v2.as_str()));
}

#[tokio::test]
async fn test_app_era_channel_pin_is_visible_and_honoured() {
    enable_agent_versions();
    let server = TestServer::new().await;
    let agent_id = create_agent(&server, "app-era-pin-agent").await;
    let v1 = save_version(&server, &agent_id, "Initial").await;
    move_draft_and_default(&server, &agent_id).await;

    // An App-era api_endpoint whose pin was carried over by migration 135.
    let api_key = format!("evr_app_{}", uuid::Uuid::new_v4().simple());
    let app = server
        .seed_app_channel(
            &format!("app-era-pin-{}", uuid::Uuid::new_v4().simple()),
            &agent_id,
            "api_endpoint",
            json!({
                "session_mode": "session_per_invocation",
                "api_key_hash": hex::encode(Sha256::digest(api_key.as_bytes())),
                "api_key_prefix": &api_key[..12],
            }),
        )
        .await;
    let channel_id = app["channels"][0]["id"].as_str().unwrap().to_string();
    let v1_uuid = v1
        .parse::<everruns_contracts::typed_id::AgentVersionId>()
        .expect("version id")
        .uuid();
    sqlx::query(
        "UPDATE agent_channels SET agent_version_policy = 'pinned', agent_version_id = $1 WHERE public_id = $2",
    )
    .bind(v1_uuid)
    .bind(&channel_id)
    .execute(&server.pool)
    .await
    .expect("seed App-era pin");
    server
        .set_app_channels_live(app["id"].as_str().unwrap(), true)
        .await;

    let endpoint: Value = server
        .get(&format!("/v1/agents/{agent_id}/channels/{channel_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(endpoint["agent_version_policy"], "pinned");
    assert_eq!(endpoint["agent_version_id"], v1);

    let created: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/channels/{channel_id}/sessions"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            serde_json::to_vec(&json!({ "message": "which version are you?" })).unwrap(),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(
        session_version(&server, created["session_id"].as_str().unwrap())
            .await
            .as_deref(),
        Some(v1.as_str()),
        "an App-era pin keeps running its version"
    );
}
