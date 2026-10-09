// `user_or_service` resolution and connection-backed presets.
//
// Spec: knowledge/integrations/user-mcp-servers.md (step 5). The person's own
// grant wins when the run is attended and they have one; otherwise the agent's
// service credential answers; unattended runs never read a person's grant.

use super::*;

async fn credential(
    fixture: &McpFixture,
    provider: &str,
    acts_as: McpServerActsAs,
) -> Option<everruns_core::connection_services::McpResolvedCredential> {
    resolver_for(fixture)
        .get_mcp_connection_credential(fixture.session_id, provider, acts_as)
        .await
        .unwrap()
}

#[tokio::test]
async fn user_or_service_prefers_the_persons_grant_when_attended() {
    let fixture = mcp_setup(ATTENDED, true, true).await;
    let resolved = credential(&fixture, &fixture.provider, McpServerActsAs::UserOrService)
        .await
        .expect("a grant should resolve");
    assert_eq!(resolved.token, "user-token");
    assert_eq!(resolved.acted_as, McpServerActsAs::User);

    let token = resolver_for(&fixture)
        .get_mcp_connection_token(
            fixture.session_id,
            &fixture.provider,
            McpServerActsAs::UserOrService,
        )
        .await
        .unwrap();
    assert_eq!(token.as_deref(), Some("user-token"));
}

#[tokio::test]
async fn user_or_service_falls_back_to_the_agent_when_the_person_has_not_connected() {
    let fixture = mcp_setup(ATTENDED, false, true).await;
    let resolved = credential(&fixture, &fixture.provider, McpServerActsAs::UserOrService)
        .await
        .expect("the agent's grant should answer");
    assert_eq!(resolved.token, "identity-token");
    assert_eq!(resolved.acted_as, McpServerActsAs::Service);
}

#[tokio::test]
async fn user_or_service_unattended_uses_the_agent_even_when_the_person_has_a_grant() {
    let fixture = mcp_setup(UNATTENDED, true, true).await;
    let resolved = credential(&fixture, &fixture.provider, McpServerActsAs::UserOrService)
        .await
        .expect("the agent's grant should answer");
    assert_eq!(resolved.token, "identity-token");
    assert_eq!(resolved.acted_as, McpServerActsAs::Service);
}

#[tokio::test]
async fn user_or_service_without_any_grant_resolves_nothing() {
    let attended = mcp_setup(ATTENDED, false, false).await;
    assert!(
        credential(
            &attended,
            &attended.provider,
            McpServerActsAs::UserOrService
        )
        .await
        .is_none()
    );
    // Unattended with only the person's grant: still nothing, never the person.
    let unattended = mcp_setup(UNATTENDED, true, false).await;
    assert!(
        credential(
            &unattended,
            &unattended.provider,
            McpServerActsAs::UserOrService
        )
        .await
        .is_none()
    );
}

/// A preset backed by the agent's `github` connection, next to the fixture's
/// own preset, with the agent's GitHub connection in place. Returns its MCP
/// provider key.
async fn github_backed_preset(fixture: &McpFixture, url: &str) -> String {
    let provider = github_backed_preset_row(fixture, url).await;
    agent_github_connection(fixture).await;
    provider
}

/// The connection-backed preset row alone, without the agent's connection.
async fn github_backed_preset_row(fixture: &McpFixture, url: &str) -> String {
    let server_id = Uuid::now_v7();
    fixture
        .db
        .create_mcp_server_with_id(
            DEFAULT_ORG_ID,
            server_id,
            CreateMcpServerRow {
                name: "github".to_string(),
                description: None,
                url: url.to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(serde_json::json!({
                    "auth_mode": "oauth",
                    "service_connection_provider": "github"
                })),
            },
        )
        .await
        .unwrap();
    format!("mcp_oauth_{server_id}")
}

/// The agent's own `github` connection on its service identity.
async fn agent_github_connection(fixture: &McpFixture) {
    fixture
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: fixture.identity_id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("agent-app".to_string()),
            access_token_encrypted: Some(
                fixture
                    .encryption
                    .encrypt_string("agent-github-token")
                    .unwrap(),
            ),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn connection_backed_preset_uses_the_agents_provider_connection_as_service() {
    let fixture = mcp_setup(ATTENDED, false, false).await;
    let provider = github_backed_preset(&fixture, "https://api.githubcopilot.com/mcp/").await;

    let service = resolver_for(&fixture)
        .get_mcp_connection_token(fixture.session_id, &provider, McpServerActsAs::Service)
        .await
        .unwrap();
    assert_eq!(service.as_deref(), Some("agent-github-token"));

    let fallback = credential(&fixture, &provider, McpServerActsAs::UserOrService)
        .await
        .expect("the agent's GitHub connection should answer");
    assert_eq!(fallback.token, "agent-github-token");
    assert_eq!(fallback.acted_as, McpServerActsAs::Service);

    // The person's side is never served from the agent's connection.
    let user = resolver_for(&fixture)
        .get_mcp_connection_token(fixture.session_id, &provider, McpServerActsAs::User)
        .await
        .unwrap();
    assert_eq!(user, None);
}

#[tokio::test]
async fn connection_backed_preset_on_a_foreign_host_never_receives_the_token() {
    // A row edited outside the API to point elsewhere: the resolver re-checks
    // the host and fails closed instead of forwarding the GitHub token.
    let fixture = mcp_setup(ATTENDED, false, false).await;
    let provider = github_backed_preset(&fixture, "https://mcp.example.com/mcp").await;

    let token = resolver_for(&fixture)
        .get_mcp_connection_token(fixture.session_id, &provider, McpServerActsAs::Service)
        .await
        .unwrap();
    assert_eq!(token, None);
}

// Session-less resolution, as an `mcp_event` trigger subscribes and polls
// before any session exists. It must reach the same credential a `service`
// tool call does.

async fn agent_service_token(fixture: &McpFixture, provider: &str) -> Option<String> {
    resolver_for(fixture)
        .agent_service_mcp_token(DEFAULT_ORG_ID, fixture.agent_id, provider)
        .await
        .unwrap()
}

#[tokio::test]
async fn trigger_on_a_connection_backed_preset_uses_the_agents_provider_connection() {
    let fixture = mcp_setup(UNATTENDED, false, false).await;
    let provider = github_backed_preset(&fixture, "https://api.githubcopilot.com/mcp/").await;

    assert_eq!(
        agent_service_token(&fixture, &provider).await.as_deref(),
        Some("agent-github-token")
    );
}

#[tokio::test]
async fn trigger_on_a_connection_backed_preset_without_the_connection_resolves_nothing() {
    // An MCP OAuth grant keyed by the preset is not a substitute: the preset
    // names its credential source, so only that connection may answer.
    let fixture = mcp_setup(UNATTENDED, false, false).await;
    let provider = github_backed_preset_row(&fixture, "https://api.githubcopilot.com/mcp/").await;
    fixture
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: fixture.identity_id,
            provider: provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("the-agent".to_string()),
            access_token_encrypted: Some(fixture.encryption.encrypt_string("stray-grant").unwrap()),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();

    assert_eq!(agent_service_token(&fixture, &provider).await, None);
}

#[tokio::test]
async fn trigger_on_a_connection_backed_preset_on_a_foreign_host_never_receives_the_token() {
    let fixture = mcp_setup(UNATTENDED, false, false).await;
    let provider = github_backed_preset(&fixture, "https://mcp.example.com/mcp").await;

    assert_eq!(agent_service_token(&fixture, &provider).await, None);
}

#[tokio::test]
async fn trigger_on_an_ordinary_preset_keeps_using_the_agents_mcp_grant() {
    let fixture = mcp_setup(UNATTENDED, true, true).await;
    // The agent's GitHub connection exists but the preset does not name it.
    agent_github_connection(&fixture).await;
    assert_eq!(
        agent_service_token(&fixture, &fixture.provider)
            .await
            .as_deref(),
        Some("identity-token")
    );

    let ungranted = mcp_setup(UNATTENDED, true, false).await;
    agent_github_connection(&ungranted).await;
    assert_eq!(
        agent_service_token(&ungranted, &ungranted.provider).await,
        None
    );
}
