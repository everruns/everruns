#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use everruns_core::{DEFAULT_ORG_ID, McpServerActsAs, tool_types::ToolCall};
use everruns_host::{HostComposition, RuntimeHostAdapter};
use everruns_provider::typed_id::{
    AgentId, AgentIdentityId, HarnessId, PrincipalId, SessionId,
};
use everruns_server::grpc_service::WorkerServiceImpl;
use everruns_server::storage::models::{
    CreateAgentIdentityConnectionRow, CreateAgentIdentityRow, CreateAgentRow,
    CreateMcpServerRow, CreatePrincipalRow, CreateSessionRow, CreateUserConnectionRow,
    CreateUserRow,
};
use everruns_server::storage::{EncryptionService, StorageBackend};
use everruns_server::{EventDelivery, seed};
use everruns_test_support::{MockMcpOAuthServer, MockMcpProtocolEra};
use everruns_worker::{GrpcWorkerAdapters, WorkerRuntimeHost};
use serde_json::json;
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

struct ActsAsArrangement {
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    mock: MockMcpOAuthServer,
    catalog_name: String,
    provider: String,
    harness_id: HarnessId,
    agent_id: AgentId,
    identity_id: AgentIdentityId,
    user_id: Uuid,
    user_principal_id: PrincipalId,
    identity_principal_id: PrincipalId,
}

impl ActsAsArrangement {
    async fn new(authenticated: bool) -> Self {
        let db = Arc::new(StorageBackend::in_memory());
        seed::seed_all(
            &db,
            everruns_core::DeploymentGrade::Dev,
            &seed::SeedAuthContext::default(),
        )
        .await
        .unwrap();
        let encryption = Arc::new(EncryptionService::new(TEST_KEY, &[]).unwrap());
        let mock = MockMcpOAuthServer::new(MockMcpProtocolEra::V2026July);
        let catalog_name = format!("linear-{}", Uuid::now_v7().simple());
        let settings = if authenticated {
            json!({
                "auth_mode": "oauth",
                "protocol_mode": "2026-07-28",
                "oauth": {}
            })
        } else {
            json!({
                "auth_mode": "none",
                "protocol_mode": "2026-07-28"
            })
        };
        let server = db
            .create_mcp_server(
                DEFAULT_ORG_ID,
                CreateMcpServerRow {
                    name: catalog_name.clone(),
                    description: None,
                    url: mock.mcp_url(),
                    transport_type: "http".to_string(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: Some(settings),
                },
            )
            .await
            .unwrap();
        let provider = everruns_core::mcp_oauth_provider_id_for_uuid(server.id.uuid());
        let harness_id = db
            .get_harness_by_name(DEFAULT_ORG_ID, "generic")
            .await
            .unwrap()
            .expect("seeded generic harness")
            .id;
        let user_id = db
            .create_user(CreateUserRow {
                email: format!("acts-as-{}@example.com", Uuid::now_v7()),
                name: "Acts As User".to_string(),
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
            .id;
        let user_principal_id = PrincipalId::new();
        db.create_principal(CreatePrincipalRow {
            id: user_principal_id,
            org_id: DEFAULT_ORG_ID,
            kind: "user".to_string(),
            subject_id: Some(user_id),
            parent_principal_id: None,
            resolved_user_id: Some(user_id),
            metadata: json!({}),
        })
        .await
        .unwrap();
        let identity_id = AgentIdentityId::new();
        db.create_agent_identity(CreateAgentIdentityRow {
            org_id: DEFAULT_ORG_ID,
            id: identity_id,
            name: "Acts As Identity".to_string(),
            description: None,
            avatar_url: None,
            locale: None,
            timezone: None,
        })
        .await
        .unwrap();
        let identity_principal_id = PrincipalId::new();
        db.create_principal(CreatePrincipalRow {
            id: identity_principal_id,
            org_id: DEFAULT_ORG_ID,
            kind: "agent_identity".to_string(),
            subject_id: Some(identity_id.uuid()),
            parent_principal_id: Some(user_principal_id),
            resolved_user_id: Some(user_id),
            metadata: json!({}),
        })
        .await
        .unwrap();
        let public_id = AgentId::new();
        let agent = db
            .create_agent(
                DEFAULT_ORG_ID,
                CreateAgentRow {
                    public_id: public_id.to_string(),
                    name: "acts-as-agent".to_string(),
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
        assert!(
            db.set_agent_identity_id(DEFAULT_ORG_ID, agent.id, identity_id)
                .await
                .unwrap()
        );

        Self {
            db,
            encryption,
            mock,
            catalog_name,
            provider,
            harness_id,
            agent_id: agent.id,
            identity_id,
            user_id,
            user_principal_id,
            identity_principal_id,
        }
    }

    fn attachment(&self, acts_as: McpServerActsAs) -> serde_json::Value {
        json!({
            "linear": {
                "use": format!("catalog:{}", self.catalog_name),
                "actsAs": acts_as.to_string()
            }
        })
    }

    async fn user_grant(&self, token: &str) {
        self.db
            .upsert_user_connection(CreateUserConnectionRow {
                user_id: self.user_id,
                provider: self.provider.clone(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: Some("acts-as-user".to_string()),
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

    async fn identity_grant(&self, token: &str) {
        self.db
            .upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
                agent_identity_id: self.identity_id,
                provider: self.provider.clone(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: Some("acts-as-identity".to_string()),
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

    async fn attended_session(&self, acts_as: McpServerActsAs) -> SessionId {
        self.session(self.user_principal_id, acts_as).await
    }

    async fn unattended_session(&self, acts_as: McpServerActsAs) -> SessionId {
        self.session(self.identity_principal_id, acts_as).await
    }

    async fn session(
        &self,
        owner_principal_id: PrincipalId,
        acts_as: McpServerActsAs,
    ) -> SessionId {
        self.db
            .create_session(CreateSessionRow {
                source: everruns_platform::SessionSource::Api,
                workspace_id: None,
                org_id: DEFAULT_ORG_ID,
                app_id: None,
                endpoint_id: None,
                harness_id: Some(self.harness_id),
                agent_id: Some(self.agent_id),
                agent_version_id: None,
                agent_config_hash: None,
                agent_identity_id: Some(self.identity_id),
                owner_principal_id,
                resolved_owner_user_id: Some(self.user_id),
                title: None,
                locale: None,
                tags: vec![],
                model_id: None,
                capabilities: json!([]),
                tools: json!([]),
                mcp_servers: self.attachment(acts_as),
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

    fn worker_service(&self) -> WorkerServiceImpl {
        let event_service = everruns_server::services::EventService::with_listeners(
            self.db.clone(),
            EventDelivery::in_memory(),
            vec![],
        );
        WorkerServiceImpl::new(
            event_service,
            self.db.clone(),
            Some(self.encryption.clone()),
            None,
            everruns_server::oss_host_composition_for_grade(
                everruns_core::DeploymentGrade::Dev,
            ),
        )
    }
}

async fn start_grpc_server(
    service: WorkerServiceImpl,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let (incoming_tx, incoming_rx) = tokio::sync::mpsc::channel(8);
    let server = tokio::spawn(async move {
        let accept_task = tokio::spawn(async move {
            let mut shutdown_rx = shutdown_rx;
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                if incoming_tx.send(Ok(stream)).await.is_err() {
                                    break;
                                }
                            }
                            Err(error) => {
                                let _ = incoming_tx.send(Err(error)).await;
                                break;
                            }
                        }
                    }
                }
            }
        });
        tonic::transport::Server::builder()
            .add_service(service.into_server())
            .serve_with_incoming(ReceiverStream::new(incoming_rx))
            .await
            .unwrap();
        accept_task.await.unwrap();
    });
    (addr.to_string(), shutdown_tx, server)
}

async fn invoke(
    fixture: &ActsAsArrangement,
    session_id: SessionId,
) -> everruns_core::tool_types::ToolResult {
    let (addr, shutdown, server) = start_grpc_server(fixture.worker_service()).await;
    let composition = HostComposition::builder()
        .egress_service(Arc::new(fixture.mock.clone()))
        .build();
    let adapters = GrpcWorkerAdapters::connect_with_host_composition(&addr, composition)
        .await
        .unwrap();
    let host = WorkerRuntimeHost::new(adapters);
    let executor = host
        .mcp_executor(DEFAULT_ORG_ID, session_id, Some(fixture.agent_id))
        .await
        .expect("worker host must expose MCP execution");
    let result = executor
        .invoke(&ToolCall {
            id: "call-1".to_string(),
            name: everruns_core::mcp_tool_name("linear", "echo"),
            arguments: json!({"value": "hello"}),
        })
        .await
        .unwrap();
    let _ = shutdown.send(());
    server.await.unwrap();
    result
}

#[tokio::test]
async fn user_attachment_reaches_mcp_with_the_invoking_users_grant_over_grpc() {
    let fixture = ActsAsArrangement::new(true).await;
    fixture.user_grant("user-token").await;
    fixture.identity_grant("identity-token").await;
    let session_id = fixture.attended_session(McpServerActsAs::User).await;

    let result = invoke(&fixture, session_id).await;

    assert!(result.error.is_none());
    fixture.mock.assert_called_as_user("user-token");
}

#[tokio::test]
async fn service_attachment_reaches_mcp_with_the_identity_grant_over_grpc() {
    let fixture = ActsAsArrangement::new(true).await;
    fixture.user_grant("user-token").await;
    fixture.identity_grant("identity-token").await;
    let session_id = fixture.attended_session(McpServerActsAs::Service).await;

    let result = invoke(&fixture, session_id).await;

    assert!(result.error.is_none());
    fixture.mock.assert_called_as_identity("identity-token");
}

#[tokio::test]
async fn unattended_user_attachment_fails_closed_before_mcp_over_grpc() {
    let fixture = ActsAsArrangement::new(true).await;
    fixture.user_grant("user-token").await;
    let session_id = fixture.unattended_session(McpServerActsAs::User).await;

    let result = invoke(&fixture, session_id).await;

    assert!(result.connection_required.is_some());
    assert!(fixture.mock.mcp_requests().is_empty());
}

#[tokio::test]
async fn unattended_no_auth_attachment_reaches_mcp_without_a_header_over_grpc() {
    let fixture = ActsAsArrangement::new(false).await;
    let session_id = fixture.unattended_session(McpServerActsAs::None).await;

    let result = invoke(&fixture, session_id).await;

    assert!(result.error.is_none());
    fixture.mock.assert_no_authorization_header();
}
