use async_trait::async_trait;
use everruns_core::capabilities::{Capability, CapabilityRegistry, RiskLevel};
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::{
    CapabilityMcpServer, CapabilityMcpServers, EgressRequest, EgressResponse, EgressService,
    McpProtocolMode, McpServerActsAs, McpServerAuthMode, ScopedMcpServer,
};
use everruns_provider::error::Result as ProviderResult;
use everruns_provider::typed_id::{AgentId, HarnessId, PrincipalId, SessionId};
use everruns_server::domains::mcp_servers::McpServerService;
use everruns_server::services::{EventService, ProviderResolverService};
use everruns_server::storage::{
    CreateAgentRow, CreateDeclarativeCapabilityRow, CreateHarnessRow, CreateMcpServerRow,
    CreateSessionRow, StorageBackend,
};
use everruns_server::{DirectWorkerAdapters, EventDelivery};
use everruns_worker::WorkerAdapters;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const MCP_URL: &str = "https://8.8.4.4/mcp";

struct IdentityContribution {
    id: String,
    acts_as: McpServerActsAs,
}

impl Capability for IdentityContribution {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        "Identity contribution"
    }

    fn description(&self) -> &str {
        "Contributes an identity-scoped MCP server"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Low
    }

    fn mcp_servers(&self) -> CapabilityMcpServers {
        [(
            "shared".to_string(),
            CapabilityMcpServer::new(
                ScopedMcpServer {
                    url: "https://contributed.example.com/mcp".to_string(),
                    auth_mode: McpServerAuthMode::OAuth,
                    oauth_provider_id: Some("contributed-provider".to_string()),
                    protocol_mode: McpProtocolMode::V2026July,
                    headers: HashMap::from([(
                        "Authorization".to_string(),
                        "Bearer contributed-header".to_string(),
                    )]),
                    ..Default::default()
                },
                self.acts_as,
            ),
        )]
        .into_iter()
        .collect()
    }
}

#[derive(Default)]
struct RecordingMcp {
    requests: Mutex<Vec<(String, Option<String>)>>,
}

#[async_trait]
impl EgressService for RecordingMcp {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.clone());
        self.requests
            .lock()
            .unwrap()
            .push((request.url.clone(), authorization));
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body["method"], "tools/list");
        Ok(EgressResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": body["id"],
                "result": {
                    "tools": [{
                        "name": "echo",
                        "description": "Echo",
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
        panic!("MCP discovery should not stream")
    }
}

struct ActingResolver;

#[async_trait]
impl UserConnectionResolver for ActingResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> ProviderResult<Option<String>> {
        panic!("identity attachments must not use the legacy lookup")
    }

    async fn get_mcp_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
        acts_as: McpServerActsAs,
    ) -> ProviderResult<Option<String>> {
        Ok(Some(format!("{acts_as}-token")))
    }
}

fn adapters(
    db: Arc<StorageBackend>,
    registry: CapabilityRegistry,
    egress: Arc<RecordingMcp>,
) -> DirectWorkerAdapters {
    let event_service = Arc::new(EventService::new(db.clone(), EventDelivery::in_memory()));
    let provider_resolver = Arc::new(ProviderResolverService::new(db.clone(), None));
    let mcp_service = Arc::new(McpServerService::with_egress_service(
        db.clone(),
        None,
        egress.clone(),
    ));
    let sqldb_backend = Arc::new(everruns_server::session_sqldb::InMemorySqlDbBackend::new());
    let sqldb_store: Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore> = Arc::new(
        everruns_server::session_sqldb::InMemorySqlDbStore::new(sqldb_backend),
    );

    DirectWorkerAdapters::new(
        db,
        event_service,
        provider_resolver,
        mcp_service,
        registry,
        everruns_worker::create_driver_registry(),
        sqldb_store,
    )
    .with_connection_resolver(Arc::new(ActingResolver))
    .with_egress_service(egress)
}

async fn create_harness(db: &StorageBackend) -> HarnessId {
    db.create_harness(
        everruns_core::DEFAULT_ORG_ID,
        CreateHarnessRow {
            name: format!("acts-as-test-{}", Uuid::now_v7()),
            display_name: None,
            icon: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: json!([]),
            system_prompt: Some(String::new()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            initial_files: json!([]),
            mcp_servers: json!({}),
            network_access: None,
            embedder_metadata: json!({}),
            is_built_in: false,
        },
    )
    .await
    .unwrap()
    .id
}

async fn create_agent_and_session(
    db: &StorageBackend,
    harness_id: HarnessId,
    capability_id: &str,
    catalog_name: Option<&str>,
    explicit_acts_as: McpServerActsAs,
) -> (AgentId, SessionId) {
    let mcp_servers = catalog_name.map_or_else(
        || json!({}),
        |catalog_name| {
            json!({
                "shared": {
                    "use": format!("catalog:{catalog_name}"),
                    "actsAs": explicit_acts_as.to_string()
                }
            })
        },
    );
    let agent = db
        .create_agent(
            everruns_core::DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: format!("acts-as-agent-{}", Uuid::now_v7()),
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
                mcp_servers,
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    db.set_agent_capabilities(
        agent.id.uuid(),
        vec![(capability_id.to_string(), 0, json!({}))],
    )
    .await
    .unwrap();
    let session = db
        .create_session(CreateSessionRow {
            workspace_id: None,
            org_id: everruns_core::DEFAULT_ORG_ID,
            source: everruns_platform::SessionSource::Api,
            app_id: None,
            endpoint_id: None,
            harness_id: Some(harness_id),
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: Some(Uuid::now_v7()),
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: json!([{
                "ref": capability_id,
                "config": {}
            }]),
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
        .unwrap();
    (agent.id, session.id)
}

#[tokio::test]
async fn explicit_attachment_replaces_contributed_entry_in_both_identity_directions() {
    let db = Arc::new(StorageBackend::in_memory());
    let egress = Arc::new(RecordingMcp::default());
    let mut registry = CapabilityRegistry::new();
    for (id, acts_as) in [
        ("contributed_user", McpServerActsAs::User),
        ("contributed_service", McpServerActsAs::Service),
    ] {
        registry.register(IdentityContribution {
            id: id.to_string(),
            acts_as,
        });
    }
    let harness_id = create_harness(&db).await;

    for (capability_id, explicit_acts_as) in [
        ("contributed_user", McpServerActsAs::Service),
        ("contributed_service", McpServerActsAs::User),
    ] {
        let catalog_name = format!("override-{}", explicit_acts_as);
        db.create_mcp_server(
            everruns_core::DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: catalog_name.clone(),
                description: None,
                url: MCP_URL.to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(json!({
                    "auth_mode": "oauth",
                    "protocol_mode": "2026-07-28",
                    "oauth": {}
                })),
            },
        )
        .await
        .unwrap();
        let (_, session_id) = create_agent_and_session(
            &db,
            harness_id,
            capability_id,
            Some(&catalog_name),
            explicit_acts_as,
        )
        .await;

        let context = adapters(db.clone(), registry.clone(), egress.clone())
            .load_turn_context(everruns_core::DEFAULT_ORG_ID, session_id.uuid())
            .await
            .unwrap();
        assert!(
            context
                .mcp_tool_definitions
                .iter()
                .any(|tool| tool.name() == "mcp_shared__echo"),
            "resolved tools: {:?}; requests: {:?}",
            context
                .mcp_tool_definitions
                .iter()
                .map(|tool| tool.name())
                .collect::<Vec<_>>(),
            *egress.requests.lock().unwrap()
        );
    }

    assert_eq!(
        *egress.requests.lock().unwrap(),
        vec![
            (
                MCP_URL.to_string(),
                Some("Bearer service-token".to_string())
            ),
            (MCP_URL.to_string(), Some("Bearer user-token".to_string()))
        ]
    );
}

#[tokio::test]
async fn contributed_inline_identity_server_cannot_retain_authorization_header() {
    let db = Arc::new(StorageBackend::in_memory());
    let egress = Arc::new(RecordingMcp::default());
    let mut registry = CapabilityRegistry::new();
    registry.register(IdentityContribution {
        id: "contributed_user".to_string(),
        acts_as: McpServerActsAs::User,
    });
    let harness_id = create_harness(&db).await;
    let (_, session_id) = create_agent_and_session(
        &db,
        harness_id,
        "contributed_user",
        None,
        McpServerActsAs::None,
    )
    .await;

    let server = adapters(db, registry, egress)
        .get_mcp_server_by_prefix(
            everruns_core::DEFAULT_ORG_ID,
            Some(session_id.uuid()),
            "shared",
        )
        .await
        .unwrap();

    assert_eq!(server.acts_as, McpServerActsAs::User);
    assert_eq!(server.url, "https://contributed.example.com/mcp");
    assert!(
        server
            .headers
            .keys()
            .all(|name| !name.eq_ignore_ascii_case("authorization"))
    );
}

#[tokio::test]
async fn persisted_legacy_unauthenticated_contribution_remains_active() {
    let db = Arc::new(StorageBackend::in_memory());
    let egress = Arc::new(RecordingMcp::default());
    let capability_name = format!("legacy_mcp_{}", &Uuid::now_v7().to_string()[..8]);
    db.create_declarative_capability(
        everruns_core::DEFAULT_ORG_ID,
        CreateDeclarativeCapabilityRow {
            public_id: everruns_provider::typed_id::DeclarativeCapabilityId::new().to_string(),
            name: capability_name.clone(),
            display_name: None,
            description: "Legacy MCP contribution".to_string(),
            definition: json!({
                "name": capability_name,
                "description": "Legacy MCP contribution",
                "mcp_servers": {
                    "legacy_docs": {
                        "url": MCP_URL,
                        "protocol_mode": "2026-07-28"
                    }
                }
            }),
        },
    )
    .await
    .unwrap();
    let harness_id = create_harness(&db).await;
    let capability_id = format!("declarative:{capability_name}");
    let (_, session_id) =
        create_agent_and_session(&db, harness_id, &capability_id, None, McpServerActsAs::None)
            .await;

    let context = adapters(db, CapabilityRegistry::new(), egress.clone())
        .load_turn_context(everruns_core::DEFAULT_ORG_ID, session_id.uuid())
        .await
        .unwrap();

    assert!(
        context
            .mcp_tool_definitions
            .iter()
            .any(|tool| tool.name() == "mcp_legacy_docs__echo")
    );
    assert_eq!(
        *egress.requests.lock().unwrap(),
        vec![(MCP_URL.to_string(), None)]
    );
}
