//! Service API keys resolve only from the agent identity.
use super::*;

async fn agentmail_grant(fixture: &McpFixture, service: bool, key: &str) {
    let token = Some(fixture.encryption.encrypt_string(key).unwrap());
    let metadata = Some(serde_json::json!({ "inbox_id": format!("{key}@agentmail.to") }));
    if service {
        fixture
            .db
            .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
                virtual_user_id: fixture.identity_id,
                provider: "agentmail".into(),
                connection_type: "api_key".into(),
                provider_user_id: None,
                provider_username: None,
                access_token_encrypted: token,
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: None,
                provider_metadata: metadata,
            })
            .await
            .unwrap();
    } else {
        fixture
            .db
            .upsert_user_connection(CreateUserConnectionRow {
                user_id: fixture.user_id,
                provider: "agentmail".into(),
                connection_type: "api_key".into(),
                provider_user_id: None,
                provider_username: None,
                access_token_encrypted: token,
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: None,
                provider_metadata: metadata,
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_service_api_key_comes_from_the_agent_and_never_from_the_invoking_user() {
    let fixture = mcp_setup(ATTENDED, false, false).await;
    agentmail_grant(&fixture, false, "user-key").await;
    let resolver = resolver_for(&fixture);
    assert!(
        resolver
            .get_service_api_key_connection(fixture.session_id, "agentmail")
            .await
            .unwrap()
            .is_none(),
        "the invoking user's key must never stand in for the agent's"
    );

    agentmail_grant(&fixture, true, "agent-key").await;
    let connection = resolver
        .get_service_api_key_connection(fixture.session_id, "agentmail")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(connection.api_key, "agent-key");
    assert_eq!(
        connection.metadata,
        Some(serde_json::json!({ "inbox_id": "agent-key@agentmail.to" }))
    );
}
