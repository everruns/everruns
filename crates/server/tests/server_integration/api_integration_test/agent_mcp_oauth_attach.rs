//! Agent MCP attachments to OAuth catalog servers that are not registered yet.

use crate::test_harness;
use axum::http::StatusCode;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::storage::CreateMcpServerRow;
use serde_json::{Value, json};
use test_harness::TestServer;

/// An OAuth catalog server nobody has signed in to yet (no discovery or client
/// registration, so no stored OAuth settings) can be attached as the agent
/// before anyone authorizes it. The attachment reports the missing login and
/// offers the agent's Authorize, which performs the registration.
#[tokio::test]
async fn test_agent_attaches_unregistered_oauth_preset_and_offers_authorize() {
    let server = TestServer::in_memory().await;
    for (name, settings) in [
        ("visti", json!({ "auth_mode": "oauth" })),
        ("plain-catalog", json!({ "auth_mode": "none" })),
    ] {
        server
            .db
            .create_mcp_server(
                DEFAULT_ORG_ID,
                CreateMcpServerRow {
                    name: name.to_string(),
                    description: None,
                    url: "https://8.8.8.8/mcp".to_string(),
                    transport_type: "http".to_string(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: Some(settings),
                },
            )
            .await
            .unwrap();
    }

    let created: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "visti-agent",
                "system_prompt": "Post updates",
                "mcpServers": {
                    "visti": { "use": "catalog:visti", "actsAs": "service" }
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = created["id"].as_str().unwrap().to_string();

    // Updating keeps working, including switching the identity it acts as.
    server
        .patch(
            &format!("/v1/agents/{agent_id}"),
            json!({
                "mcpServers": {
                    "visti": { "use": "catalog:visti", "actsAs": "user_or_service" }
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);
    server
        .patch(
            &format!("/v1/agents/{agent_id}"),
            json!({
                "mcpServers": {
                    "visti": { "use": "catalog:visti", "actsAs": "service" }
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    let attachments: Value = server
        .get(&format!("/v1/agents/{agent_id}/mcp-attachments"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let visti = attachments
        .as_array()
        .unwrap()
        .iter()
        .find(|attachment| attachment["name"] == "visti")
        .expect("visti attachment");
    assert_eq!(visti["state"], "connection_missing", "{visti}");
    assert_eq!(visti["action"], "authorize", "{visti}");
    assert_eq!(visti["preset_name"], "visti", "{visti}");
    assert!(
        visti["connection_provider"]
            .as_str()
            .is_some_and(|provider| provider.starts_with("mcp_oauth_")),
        "{visti}"
    );

    // A preset that does not use OAuth still cannot carry an identity.
    let error: Value = server
        .patch(
            &format!("/v1/agents/{agent_id}"),
            json!({
                "mcpServers": {
                    "plain": { "use": "catalog:plain-catalog", "actsAs": "service" }
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert!(error.to_string().contains("to use OAuth"), "{error}");
}
