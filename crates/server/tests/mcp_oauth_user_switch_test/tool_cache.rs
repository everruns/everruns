use super::test_harness::TestServer;
use async_trait::async_trait;
use axum::http::StatusCode;
use everruns_capability::CapabilityRef;
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::{
    Caller, DEFAULT_ORG_ID, EgressRequest, EgressResponse, EgressResult, EgressService,
    EgressStreamResponse, McpServerActsAs, McpServerAuthMode, ScopedMcpServer, ScopedMcpServers,
    mcp_oauth_provider_id_for_uuid,
};
use everruns_provider::typed_id::{AgentId, AgentIdentityId, PrincipalId, SessionId};
use everruns_server::CapabilityService;
use everruns_server::domains::mcp_servers::scoped_mcp::build_materialized_scoped_mcp_tool_definitions;
use everruns_server::domains::mcp_servers::{McpServerService, McpServerSettings};
use everruns_server::storage::models::{
    CreateAgentIdentityConnectionRow, CreateMcpServerRow, CreatePrincipalRow, CreateSessionRow,
    CreateUserConnectionRow, UpdateMcpServerTools,
};
use everruns_server::storage::{DbConnectionResolver, EncryptionService, StorageBackend};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use uuid::Uuid;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

struct CountingMcpServer {
    calls: AtomicUsize,
    scope: &'static str,
    ttl_ms: i64,
}

impl CountingMcpServer {
    fn new(scope: &'static str, ttl_ms: i64) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            scope,
            ttl_ms,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl EgressService for CountingMcpServer {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["method"], "tools/list");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let token = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .and_then(|(_, value)| value.strip_prefix("Bearer "))
            .unwrap_or("anonymous")
            .replace('-', "_");
        Ok(EgressResponse {
            status: 200,
            headers: Default::default(),
            body: serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "tools": [{
                        "name": format!("tool_{token}"),
                        "description": "identity-scoped tool",
                        "inputSchema": {"type": "object"}
                    }],
                    "ttlMs": self.ttl_ms,
                    "cacheScope": self.scope
                }
            }))
            .unwrap(),
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        panic!("MCP tool discovery must use a bounded non-streaming response")
    }
}

struct CacheFixture {
    db: Arc<StorageBackend>,
    encryption: EncryptionService,
    egress: Arc<CountingMcpServer>,
    resolver: Arc<dyn UserConnectionResolver>,
    server_id: Uuid,
    provider: String,
    agent_id: Uuid,
    identity_id: AgentIdentityId,
    user_a: Uuid,
    user_b: Uuid,
    session_a: SessionId,
    session_b: SessionId,
}

impl CacheFixture {
    async fn new(scope: &'static str, ttl_ms: i64) -> Self {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = EncryptionService::new(TEST_KEY, &[]).unwrap();
        let egress = Arc::new(CountingMcpServer::new(scope, ttl_ms));
        let row = db
            .create_mcp_server(
                DEFAULT_ORG_ID,
                CreateMcpServerRow {
                    name: format!("cache-{}", Uuid::new_v4()),
                    description: None,
                    url: "http://8.8.8.8/mcp".to_string(),
                    transport_type: "streamable_http".to_string(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: Some(
                        serde_json::to_value(McpServerSettings {
                            auth_mode: McpServerAuthMode::OAuth,
                            ..Default::default()
                        })
                        .unwrap(),
                    ),
                },
            )
            .await
            .unwrap();
        let server_id = row.id.uuid();
        let provider = mcp_oauth_provider_id_for_uuid(server_id);
        let agent_id = Uuid::new_v4();
        let identity_id = AgentIdentityId::new();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();
        let session_a = create_persisted_session(&db, agent_id, identity_id, user_a).await;
        let session_b = create_persisted_session(&db, agent_id, identity_id, user_b).await;
        let resolver: Arc<dyn UserConnectionResolver> = Arc::new(DbConnectionResolver::new(
            db.as_ref().clone(),
            encryption.clone(),
            None,
            egress.clone(),
        ));
        Self {
            db,
            encryption,
            egress,
            resolver,
            server_id,
            provider,
            agent_id,
            identity_id,
            user_a,
            user_b,
            session_a,
            session_b,
        }
    }

    async fn discover(
        &self,
        session_id: SessionId,
        acts_as: McpServerActsAs,
    ) -> Vec<everruns_provider::tool_types::ToolDefinition> {
        let name = self
            .db
            .get_mcp_server(DEFAULT_ORG_ID, self.server_id)
            .await
            .unwrap()
            .unwrap()
            .name;
        let servers = ScopedMcpServers::from([(
            "linear".to_string(),
            ScopedMcpServer {
                preset: Some(format!("catalog:{name}").parse().unwrap()),
                acts_as,
                ..Default::default()
            },
        )]);
        build_materialized_scoped_mcp_tool_definitions(
            self.db.as_ref(),
            DEFAULT_ORG_ID,
            &servers,
            Some(session_id),
            Some(&self.resolver),
            self.egress.as_ref(),
        )
        .await
        .unwrap()
    }

    async fn set_identity_token(&self, token: &str) {
        self.db
            .upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
                agent_identity_id: self.identity_id,
                provider: self.provider.clone(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: None,
                access_token_encrypted: Some(self.encryption.encrypt_string(token).unwrap()),
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: None,
                provider_metadata: None,
            })
            .await
            .unwrap();
    }

    async fn set_user_token(&self, user_id: Uuid, token: &str) {
        self.db
            .upsert_user_connection(CreateUserConnectionRow {
                user_id,
                provider: self.provider.clone(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: None,
                access_token_encrypted: Some(self.encryption.encrypt_string(token).unwrap()),
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: None,
                provider_metadata: None,
            })
            .await
            .unwrap();
    }
}

async fn create_persisted_session(
    db: &StorageBackend,
    agent_id: Uuid,
    identity_id: AgentIdentityId,
    user_id: Uuid,
) -> SessionId {
    let principal_id = PrincipalId::new();
    db.create_principal(CreatePrincipalRow {
        id: principal_id,
        org_id: DEFAULT_ORG_ID,
        kind: "user".to_string(),
        subject_id: Some(user_id),
        parent_principal_id: None,
        resolved_user_id: Some(user_id),
        metadata: json!({}),
    })
    .await
    .unwrap();
    db.create_session(CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        org_id: DEFAULT_ORG_ID,
        workspace_id: None,
        app_id: None,
        endpoint_id: None,
        harness_id: None,
        agent_id: Some(AgentId::from_uuid(agent_id)),
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: Some(identity_id),
        owner_principal_id: principal_id,
        resolved_owner_user_id: Some(user_id),
        title: None,
        locale: None,
        tags: Vec::new(),
        model_id: None,
        capabilities: json!([]),
        tools: json!([]),
        mcp_servers: json!({}),
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

fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[tokio::test]
async fn persisted_sessions_isolate_user_cache_and_share_service_cache_per_agent() {
    let fixture = CacheFixture::new("public", 60_000).await;
    fixture.set_identity_token("service-token").await;
    fixture.set_user_token(fixture.user_a, "user-a").await;
    fixture.set_user_token(fixture.user_b, "user-b").await;

    let service_a = fixture
        .discover(fixture.session_a, McpServerActsAs::Service)
        .await;
    let service_b = fixture
        .discover(fixture.session_b, McpServerActsAs::Service)
        .await;
    assert_eq!(service_a[0].name(), service_b[0].name());
    assert!(service_a[0].name().ends_with("tool_service_token"));
    assert_eq!(fixture.egress.calls(), 1);

    let user_a = fixture
        .discover(fixture.session_a, McpServerActsAs::User)
        .await;
    let user_b = fixture
        .discover(fixture.session_b, McpServerActsAs::User)
        .await;
    assert!(user_a[0].name().ends_with("tool_user_a"));
    assert!(user_b[0].name().ends_with("tool_user_b"));
    assert_ne!(user_a[0].name(), user_b[0].name());
    assert_eq!(fixture.egress.calls(), 3);

    fixture
        .discover(fixture.session_a, McpServerActsAs::Service)
        .await;
    fixture
        .discover(fixture.session_a, McpServerActsAs::User)
        .await;
    assert_eq!(fixture.egress.calls(), 3);
}

#[tokio::test]
async fn persisted_grant_revoke_and_acts_as_transition_do_not_reuse_cache_entries() {
    let fixture = CacheFixture::new("public", 60_000).await;
    fixture.set_identity_token("service-one").await;
    fixture.set_user_token(fixture.user_a, "user-one").await;

    fixture
        .discover(fixture.session_a, McpServerActsAs::Service)
        .await;
    assert_eq!(fixture.egress.calls(), 1);
    fixture
        .db
        .delete_agent_identity_connection(fixture.identity_id, &fixture.provider)
        .await
        .unwrap();
    assert!(
        fixture
            .discover(fixture.session_a, McpServerActsAs::Service)
            .await
            .is_empty()
    );
    assert!(
        fixture
            .db
            .get_mcp_service_tool_cache(
                DEFAULT_ORG_ID,
                fixture.server_id,
                fixture.agent_id,
                "public",
                "",
            )
            .await
            .unwrap()
            .is_none()
    );

    let user = fixture
        .discover(fixture.session_a, McpServerActsAs::User)
        .await;
    assert!(user[0].name().ends_with("tool_user_one"));
    fixture.set_identity_token("service-two").await;
    let service = fixture
        .discover(fixture.session_a, McpServerActsAs::Service)
        .await;
    assert!(service[0].name().ends_with("tool_service_two"));
    assert_eq!(fixture.egress.calls(), 3);
}

#[tokio::test]
async fn private_rotation_reaps_old_service_rows_and_ttl_expiry_refreshes() {
    let fixture = CacheFixture::new("private", 60_000).await;
    fixture.set_identity_token("private-one").await;
    fixture
        .discover(fixture.session_a, McpServerActsAs::Service)
        .await;
    fixture.set_identity_token("private-two").await;
    fixture
        .discover(fixture.session_a, McpServerActsAs::Service)
        .await;

    assert_eq!(fixture.egress.calls(), 2);
    assert!(
        fixture
            .db
            .get_mcp_service_tool_cache(
                DEFAULT_ORG_ID,
                fixture.server_id,
                fixture.agent_id,
                "private",
                &hash("private-one"),
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .db
            .get_mcp_service_tool_cache(
                DEFAULT_ORG_ID,
                fixture.server_id,
                fixture.agent_id,
                "private",
                &hash("private-two"),
            )
            .await
            .unwrap()
            .is_some()
    );

    let expiring = CacheFixture::new("public", 1).await;
    expiring.set_identity_token("expiring").await;
    expiring
        .discover(expiring.session_a, McpServerActsAs::Service)
        .await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    expiring
        .discover(expiring.session_a, McpServerActsAs::Service)
        .await;
    assert_eq!(expiring.egress.calls(), 2);
}

#[tokio::test]
async fn oauth_callback_discovery_and_capability_reads_never_write_or_leak_shared_tools() {
    let server = TestServer::in_memory().await;
    let row = server
        .db
        .create_mcp_server(
            DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: format!("legacy-oauth-{}", Uuid::new_v4()),
                description: None,
                url: "http://8.8.8.8/mcp".to_string(),
                transport_type: "streamable_http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(
                    serde_json::to_value(McpServerSettings {
                        auth_mode: McpServerAuthMode::OAuth,
                        ..Default::default()
                    })
                    .unwrap(),
                ),
            },
        )
        .await
        .unwrap();
    let id = row.id.uuid();
    let capability_id = everruns_mcp::mcp_capability_id(id);
    let egress = Arc::new(CountingMcpServer::new("private", 60_000));
    let mcp_service = McpServerService::with_egress_service(
        server.db.clone(),
        server.encryption.clone(),
        egress.clone(),
    );

    let discovered = mcp_service
        .cache_tools_for_bearer_token(&Caller::internal(DEFAULT_ORG_ID), id, "callback-token")
        .await
        .unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(
        server
            .db
            .get_mcp_server(DEFAULT_ORG_ID, id)
            .await
            .unwrap()
            .unwrap()
            .cached_tools,
        json!([])
    );

    async fn poison(db: &StorageBackend, id: Uuid) {
        db.update_mcp_server_tools(
            DEFAULT_ORG_ID,
            id,
            UpdateMcpServerTools {
                cached_tools: json!([{
                    "name": "user_a_private_tool",
                    "description": "Only visible to user A",
                    "inputSchema": {"type": "object"}
                }]),
            },
        )
        .await
        .unwrap();
    }

    poison(server.db.as_ref(), id).await;
    let listed: Value = server
        .get("/v1/capabilities?limit=100")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let listed_mcp = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == capability_id)
        .unwrap();
    assert!(
        listed_mcp["tool_definitions"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );

    poison(server.db.as_ref(), id).await;
    let detailed: Value = server
        .get(&format!("/v1/capabilities/{capability_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(
        detailed["tool_definitions"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );

    poison(server.db.as_ref(), id).await;
    let capability_service = CapabilityService::new(server.db.clone(), server.encryption.clone());
    let (_, preview) = capability_service
        .preview(1, "base", &[CapabilityRef::new(capability_id)])
        .await
        .unwrap();
    assert!(preview.is_empty());
    let persisted = server
        .db
        .get_mcp_server(DEFAULT_ORG_ID, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.cached_tools, json!([]));
    assert!(persisted.tools_cached_at.is_none());
}
