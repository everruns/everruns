//! PostgreSQL integration coverage for service-owned MCP OAuth grants.
//!
//! Run with:
//! `cargo test -p everruns-server --test service_mcp_oauth_lifecycle_test -- --test-threads=1`

mod test_harness;

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::extract::{Path, Query, State};
use axum_extra::extract::cookie::CookieJar;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::{
    Caller, EgressRequest, EgressResponse, EgressService, McpServerActsAs, OrgRole,
};
use everruns_platform::connector::ConnectorRegistry;
use everruns_provider::typed_id::{AgentIdentityId, HarnessId, PrincipalId, SessionId};
use everruns_server::api::user_connections::{
    AppState, OAuthAuthorizeQuery, OAuthCallbackQuery, authorize_connection,
    connection_oauth_callback,
};
use everruns_server::auth::{AuthConfig, AuthState, ResolvedOrg};
use everruns_server::domains::mcp_servers::McpServerService;
use everruns_server::storage::{
    CreateAgentRow, CreateMcpServerRow, CreatePrincipalRow, CreateSessionRow, CreateUserRow,
    DbConnectionResolver, EncryptionService, StorageBackend,
};
use serde_json::{Value, json};
use uuid::Uuid;

use test_harness::{TestServer, get_database_url};

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const TOKEN_URL: &str = "https://8.8.8.8/oauth/token";
const MCP_URL: &str = "https://8.8.4.4/mcp";

#[derive(Default)]
struct MockOAuthAndMcpServers {
    oauth_bodies: Mutex<Vec<String>>,
    mcp_authorization_headers: Mutex<Vec<String>>,
}

#[async_trait]
impl EgressService for MockOAuthAndMcpServers {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        if request.url == TOKEN_URL {
            let body = String::from_utf8(request.body).expect("OAuth request must be UTF-8");
            self.oauth_bodies.lock().unwrap().push(body.clone());
            let token = if body.contains("grant_type=authorization_code") {
                json!({
                    "access_token": "initial-access",
                    "refresh_token": "refresh-1",
                    "expires_in": 0,
                    "scope": "issues.write"
                })
            } else {
                assert!(body.contains("grant_type=refresh_token"));
                assert!(body.contains("refresh_token=refresh-1"));
                json!({
                    "access_token": "refreshed-access",
                    "refresh_token": "refresh-2",
                    "expires_in": 3600,
                    "scope": "issues.write"
                })
            };
            return Ok(EgressResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: serde_json::to_vec(&token).unwrap(),
            });
        }

        assert_eq!(request.url, MCP_URL);
        let authorization = request
            .headers
            .get("Authorization")
            .expect("MCP call must carry authorization")
            .clone();
        self.mcp_authorization_headers
            .lock()
            .unwrap()
            .push(authorization);
        let request_body: Value = serde_json::from_slice(&request.body).unwrap();
        Ok(EgressResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": request_body["id"],
                "result": {
                    "tools": [{
                        "name": "create_issue",
                        "description": "Create an issue",
                        "inputSchema": {"type": "object"}
                    }]
                }
            }))
            .unwrap(),
        })
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("streaming egress is not used by this test")
    }
}

fn resolved_org(user_id: Uuid) -> ResolvedOrg {
    ResolvedOrg {
        org_id: everruns_core::DEFAULT_ORG_ID,
        public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        name: "Default Organization".to_string(),
        user_id: Some(user_id),
        role: OrgRole::Owner,
        is_platform_user: false,
        feature_flags: everruns_platform::FeatureFlags::current(),
    }
}

async fn create_user(db: &StorageBackend, label: &str) -> Uuid {
    db.create_user(CreateUserRow {
        email: format!("{label}-{}@example.com", Uuid::now_v7()),
        name: label.to_string(),
        avatar_url: None,
        roles: vec!["user".to_string()],
        password_hash: None,
        email_verified: true,
        auth_provider: None,
        auth_provider_id: None,
        external_id: None,
    })
    .await
    .unwrap()
    .id
}

async fn create_user_principal(db: &StorageBackend, user_id: Uuid) -> PrincipalId {
    db.create_principal(CreatePrincipalRow {
        id: PrincipalId::new(),
        org_id: everruns_core::DEFAULT_ORG_ID,
        kind: "user".to_string(),
        subject_id: Some(user_id),
        parent_principal_id: None,
        resolved_user_id: Some(user_id),
        metadata: json!({}),
    })
    .await
    .unwrap()
    .id
}

async fn create_session(
    db: &StorageBackend,
    agent_id: everruns_provider::typed_id::AgentId,
    identity_id: AgentIdentityId,
    harness_id: HarnessId,
    owner_principal_id: PrincipalId,
    user_id: Uuid,
) -> SessionId {
    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: everruns_core::DEFAULT_ORG_ID,
        app_id: None,
        endpoint_id: None,
        harness_id: Some(harness_id),
        agent_id: Some(agent_id),
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: Some(identity_id),
        owner_principal_id,
        resolved_owner_user_id: Some(user_id),
        title: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: json!([]),
        tools: json!([]),
        mcp_servers: json!({
            "linear": {
                "use": "catalog:linear",
                "actsAs": "service"
            }
        }),
        system_prompt: None,
        initial_files: json!([]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
    })
    .await
    .unwrap()
    .id
}

fn pending_state_value(jar: &CookieJar, provider: &str) -> String {
    let cookie_name = format!("oauth_connection_state_{provider}");
    let cookie = jar.get(&cookie_name).expect("OAuth state cookie");
    let pending: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cookie.value()).unwrap()).unwrap();
    pending["state"].as_str().unwrap().to_string()
}

fn verify_grant_in_separate_process(identity_id: AgentIdentityId, provider: &str) {
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("identity_grant_decrypt_helper")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env("EVE_IDENTITY_GRANT_DECRYPT_HELPER", "1")
        .env("EVE_IDENTITY_ID", identity_id.to_string())
        .env("EVE_IDENTITY_PROVIDER", provider)
        .env("EVE_EXPECTED_ACCESS_TOKEN", "refreshed-access")
        .env("DATABASE_URL", get_database_url())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "separate-process decryption failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "decryption helper did not run:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn identity_grant_decrypt_helper() {
    if std::env::var("EVE_IDENTITY_GRANT_DECRYPT_HELPER").as_deref() != Ok("1") {
        return;
    }
    let identity_id: AgentIdentityId = std::env::var("EVE_IDENTITY_ID").unwrap().parse().unwrap();
    let provider = std::env::var("EVE_IDENTITY_PROVIDER").unwrap();
    let expected = std::env::var("EVE_EXPECTED_ACCESS_TOKEN").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let db = StorageBackend::postgres(&get_database_url()).await.unwrap();
        let row = db
            .get_agent_identity_connection(identity_id, &provider)
            .await
            .unwrap()
            .expect("identity grant must be visible in the shared database");
        let encrypted = row
            .access_token_encrypted
            .expect("identity access token must be encrypted");
        let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
        assert_eq!(encryption.decrypt_to_string(&encrypted).unwrap(), expected);
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn service_grant_authorize_call_refresh_and_revoke_uses_shared_postgres() {
    let server = TestServer::new().await;
    let db = server.db.clone();
    let encryption = server.encryption.clone().unwrap();
    let remote = Arc::new(MockOAuthAndMcpServers::default());
    let auth_config = AuthConfig {
        base_url: "https://everruns.example/api".to_string(),
        frontend_url: "https://everruns.example".to_string(),
        ..Default::default()
    };
    let mcp_service = Arc::new(McpServerService::with_egress_service(
        db.clone(),
        Some(encryption.clone()),
        remote.clone(),
    ));
    let state = AppState::new(
        db.clone(),
        Some(encryption.clone()),
        AuthState::builtin(auth_config.clone(), db.clone()),
        auth_config,
        ConnectorRegistry::new(),
        mcp_service.clone(),
    );

    let server_id = Uuid::now_v7();
    db.create_mcp_server_with_id(
        everruns_core::DEFAULT_ORG_ID,
        server_id,
        CreateMcpServerRow {
            name: format!("linear-{}", &server_id.to_string()[..8]),
            description: None,
            url: MCP_URL.to_string(),
            transport_type: "http".to_string(),
            api_key_encrypted: None,
            headers: None,
            settings: Some(json!({
                "auth_mode": "oauth",
                "protocol_mode": "2026-07-28",
                "oauth": {
                    "authorization_endpoint": "https://8.8.8.8/oauth/authorize",
                    "token_endpoint": TOKEN_URL,
                    "client_id": "test-client"
                }
            })),
        },
    )
    .await
    .unwrap();
    let harness_id: HarnessId = server.seed_generic_harness_id.parse().unwrap();
    let agent = db
        .create_agent(
            everruns_core::DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: everruns_provider::typed_id::AgentId::new().to_string(),
                name: format!("service-oauth-{}", &server_id.to_string()[..8]),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: json!([]),
                system_prompt: String::new(),
                default_model_id: None,
                harness_id,
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    let alice_id = create_user(&db, "Service OAuth Alice").await;
    let bob_id = create_user(&db, "Service OAuth Bob").await;
    assert_ne!(alice_id, bob_id);
    let org = resolved_org(alice_id);
    let provider = everruns_core::mcp_oauth_provider_id_for_uuid(server_id);

    let (jar, _) = authorize_connection(
        State(state.clone()),
        org.clone(),
        CookieJar::new(),
        Path(provider.clone()),
        Query(OAuthAuthorizeQuery {
            return_to: None,
            mode: Some("identity".to_string()),
            session_id: None,
            agent_id: Some(agent.public_id.clone()),
            popup: None,
        }),
    )
    .await
    .unwrap();
    let oauth_state = pending_state_value(&jar, &provider);
    let (_jar, _redirect) = connection_oauth_callback(
        State(state),
        org.clone(),
        jar,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: "authorization-code".to_string(),
            state: Some(oauth_state),
        }),
    )
    .await
    .unwrap();
    assert!(
        remote.mcp_authorization_headers.lock().unwrap().is_empty(),
        "OAuth callback must not perform shared tool discovery"
    );
    let catalog_row = db
        .get_mcp_server(everruns_core::DEFAULT_ORG_ID, server_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(catalog_row.cached_tools, json!([]));
    assert!(catalog_row.tools_cached_at.is_none());

    let agent = db
        .get_agent(everruns_core::DEFAULT_ORG_ID, agent.id)
        .await
        .unwrap()
        .unwrap();
    let identity_id = agent.agent_identity_id.unwrap();
    assert!(
        db.get_user_connection(alice_id, &provider)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_user_connection(bob_id, &provider)
            .await
            .unwrap()
            .is_none()
    );

    let alice_session = create_session(
        &db,
        agent.id,
        identity_id,
        harness_id,
        create_user_principal(&db, alice_id).await,
        alice_id,
    )
    .await;
    let bob_session = create_session(
        &db,
        agent.id,
        identity_id,
        harness_id,
        create_user_principal(&db, bob_id).await,
        bob_id,
    )
    .await;
    let resolver = DbConnectionResolver::new(
        db.as_ref().clone(),
        encryption.as_ref().clone(),
        None,
        remote.clone(),
    );
    let alice_token = resolver
        .get_mcp_connection_token(alice_session, &provider, McpServerActsAs::Service)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alice_token, "refreshed-access");
    mcp_service
        .cache_tools_for_bearer_token(&Caller::from(&org), server_id, &alice_token)
        .await
        .unwrap();
    let bob_token = resolver
        .get_mcp_connection_token(bob_session, &provider, McpServerActsAs::Service)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bob_token, alice_token);
    mcp_service
        .cache_tools_for_bearer_token(&Caller::from(&org), server_id, &bob_token)
        .await
        .unwrap();

    let oauth_bodies = remote.oauth_bodies.lock().unwrap().clone();
    assert_eq!(oauth_bodies.len(), 2);
    assert!(oauth_bodies[0].contains("grant_type=authorization_code"));
    assert!(oauth_bodies[1].contains("grant_type=refresh_token"));
    let authorization_headers = remote.mcp_authorization_headers.lock().unwrap().clone();
    assert_eq!(
        authorization_headers,
        vec!["Bearer refreshed-access", "Bearer refreshed-access"]
    );
    verify_grant_in_separate_process(identity_id, &provider);

    assert!(
        db.delete_agent_identity_connection(identity_id, &provider)
            .await
            .unwrap()
    );
    for session_id in [alice_session, bob_session] {
        assert!(
            resolver
                .get_mcp_connection_token(session_id, &provider, McpServerActsAs::Service)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(
        db.get_agent_identity_connection(identity_id, &provider)
            .await
            .unwrap()
            .is_none()
    );
}
