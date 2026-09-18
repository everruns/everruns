use super::*;
use crate::kernel_imports::{HarnessId, ScopedMcpServer, SessionId};
use crate::storage::models::{CreateMcpServerRow, UpdateMcpServer};
use chrono::Utc;
use everruns_core::{CapabilityMcpServer, CapabilityMcpServers};
use everruns_platform::{Agent, AgentStatus, generate_agent_public_id};
use std::sync::atomic::{AtomicUsize, Ordering};

struct MutableConnectionResolver {
    token: tokio::sync::RwLock<Option<String>>,
}

#[async_trait::async_trait]
impl UserConnectionResolver for MutableConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> everruns_provider::error::Result<Option<String>> {
        Ok(self.token.read().await.clone())
    }
}

#[derive(Default)]
struct CountingConnectionResolver {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl UserConnectionResolver for CountingConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> everruns_provider::error::Result<Option<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Some("legacy-token".to_string()))
    }
}

#[derive(Default)]
struct ActingIdentityResolver {
    calls: std::sync::Mutex<Vec<McpServerActsAs>>,
}

#[async_trait::async_trait]
impl UserConnectionResolver for ActingIdentityResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> everruns_provider::error::Result<Option<String>> {
        Ok(Some("legacy-token".to_string()))
    }

    async fn get_mcp_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
        acts_as: McpServerActsAs,
    ) -> everruns_provider::error::Result<Option<String>> {
        self.calls.lock().unwrap().push(acts_as);
        Ok(Some(format!("{acts_as}-token")))
    }
}

#[derive(Default)]
struct CatalogPreviewEgress {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl EgressService for CatalogPreviewEgress {
    async fn send(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        assert!(
            request
                .headers
                .keys()
                .all(|name| !name.eq_ignore_ascii_case("Authorization"))
        );
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        match body["method"].as_str().unwrap_or_default() {
            "initialize" => Ok(everruns_core::EgressResponse {
                status: 200,
                headers: std::collections::BTreeMap::from([(
                    "Mcp-Session-Id".to_string(),
                    "preview-session".to_string(),
                )]),
                body: serde_json::to_vec(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 0,
                    "result": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {}
                    }
                }))
                .unwrap(),
            }),
            "notifications/initialized" => Ok(everruns_core::EgressResponse {
                status: 202,
                headers: Default::default(),
                body: Vec::new(),
            }),
            "tools/list" => {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(everruns_core::EgressResponse {
                    status: 200,
                    headers: Default::default(),
                    body: serde_json::to_vec(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "result": {
                            "tools": [{
                                "name": "echo",
                                "description": "Echo a message",
                                "inputSchema": {"type": "object"}
                            }]
                        }
                    }))
                    .unwrap(),
                })
            }
            method => panic!("unexpected MCP preview method: {method}"),
        }
    }

    async fn send_stream(
        &self,
        _request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("MCP discovery should not use streaming egress")
    }
}

fn scoped_server(url: &str) -> ScopedMcpServer {
    ScopedMcpServer {
        url: url.to_string(),
        ..Default::default()
    }
}
fn oauth_scoped_server(url: &str, provider: &str) -> ScopedMcpServer {
    ScopedMcpServer {
        url: url.to_string(),
        auth_mode: McpServerAuthMode::OAuth,
        oauth_provider_id: Some(provider.to_string()),
        ..Default::default()
    }
}
fn catalog_server(preset: &str, acts_as: McpServerActsAs) -> ScopedMcpServer {
    ScopedMcpServer {
        preset: Some(format!("catalog:{preset}").parse().unwrap()),
        acts_as,
        ..Default::default()
    }
}

async fn seed_catalog_server(
    db: &StorageBackend,
    name: &str,
    oauth: bool,
) -> everruns_provider::typed_id::McpServerId {
    let settings = crate::domains::mcp_servers::service::McpServerSettings {
        auth_mode: if oauth {
            McpServerAuthMode::OAuth
        } else {
            McpServerAuthMode::None
        },
        protocol_mode: McpProtocolMode::V2025June,
        oauth: oauth
            .then_some(crate::domains::mcp_servers::service::McpServerOAuthSettings::default()),
    };
    db.create_mcp_server(
        everruns_core::DEFAULT_ORG_ID,
        CreateMcpServerRow {
            name: name.to_string(),
            description: None,
            url: "http://8.8.8.8/mcp".to_string(),
            transport_type: "http".to_string(),
            api_key_encrypted: None,
            headers: Some(serde_json::json!({"X-Catalog":"value"})),
            settings: Some(serde_json::to_value(settings).unwrap()),
        },
    )
    .await
    .unwrap()
    .id
}

#[test]
fn detects_authorization_header_case_insensitively() {
    let mut headers = HashMap::new();
    assert!(!has_authorization_header(&headers));

    headers.insert("authorization".to_string(), "Bearer token".to_string());
    assert!(has_authorization_header(&headers));
}
#[tokio::test]
async fn inline_identity_attachment_discards_authorization_headers_after_resolution() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = McpServerService::new(db, None);

    for acts_as in [McpServerActsAs::User, McpServerActsAs::Service] {
        let server = ScopedMcpServer {
            url: "https://mcp.example.com/mcp".to_string(),
            acts_as,
            headers: HashMap::from([
                ("aUtHoRiZaTiOn".to_string(), "Bearer injected".to_string()),
                ("X-Tenant".to_string(), "tenant-1".to_string()),
            ]),
            ..Default::default()
        };

        let resolved = resolve_matched_scoped_mcp_server(
            &service,
            everruns_core::DEFAULT_ORG_ID,
            Uuid::now_v7(),
            Some(("contributed".to_string(), server)),
        )
        .await
        .unwrap()
        .expect("inline server should resolve");

        assert_eq!(resolved.acts_as, acts_as);
        assert_eq!(
            resolved.headers,
            HashMap::from([("X-Tenant".to_string(), "tenant-1".to_string())])
        );
    }
}

#[tokio::test]
async fn newly_connected_token_is_visible_on_next_turn_in_same_session() {
    let session_id = SessionId::new();
    let server = oauth_scoped_server("https://mcp.resend.com/mcp", "mcp_oauth_resend");
    let resolver = Arc::new(MutableConnectionResolver {
        token: tokio::sync::RwLock::new(None),
    });
    let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();

    assert!(
        resolve_scoped_mcp_discovery_token(
            "resend",
            &server,
            Some(session_id),
            Some(&resolver_trait),
        )
        .await
        .unwrap()
        .is_none()
    );

    *resolver.token.write().await = Some("fresh-oauth-token".to_string());

    assert_eq!(
        resolve_scoped_mcp_discovery_token(
            "resend",
            &server,
            Some(session_id),
            Some(&resolver_trait),
        )
        .await
        .unwrap()
        .as_deref(),
        Some("fresh-oauth-token")
    );
}

#[tokio::test]
async fn contributed_identity_discovery_uses_only_the_declared_grant_path() {
    let session_id = SessionId::new();
    let resolver = Arc::new(ActingIdentityResolver::default());
    let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();

    for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
        let contribution = CapabilityMcpServer::new(
            oauth_scoped_server("https://mcp.example.com/mcp", "mcp_oauth_contribution"),
            acts_as,
        );
        assert_eq!(
            resolve_scoped_mcp_discovery_token(
                "contributed",
                contribution.as_scoped(),
                Some(session_id),
                Some(&resolver_trait),
            )
            .await
            .unwrap(),
            Some(format!("{acts_as}-token"))
        );
    }

    assert_eq!(
        *resolver.calls.lock().unwrap(),
        vec![McpServerActsAs::Service, McpServerActsAs::User]
    );
}

fn test_harness() -> Harness {
    Harness {
        id: HarnessId::new(),
        name: "test-harness".to_string(),
        display_name: None,
        icon: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("harness".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        network_access: None,
        parallel_tool_calls: None,
        mcp_servers: Default::default(),
        embedder_metadata: Default::default(),
        is_built_in: false,
        status: everruns_platform::HarnessStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
    }
}

fn test_agent() -> Agent {
    let public_id = generate_agent_public_id();
    Agent {
        public_id,
        internal_id: public_id.uuid(),
        name: "test-agent".to_string(),
        display_name: None,
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "agent".to_string(),
        default_model_id: None,
        harness_id: everruns_provider::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
        default_version_id: None,
        forked_from_agent_id: None,
        forked_from_version_id: None,
        root_agent_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        tools: vec![],
        mcp_servers: Default::default(),
        status: AgentStatus::Active,
        exposures_suspended: false,
        exposed: false,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        archived_at: None,
        deleted_at: None,
        usage: None,
    }
}

fn test_session(harness_id: HarnessId, agent_id: everruns_provider::typed_id::AgentId) -> Session {
    let session_id = SessionId::new();
    Session {
        source: Default::default(),
        activity: Default::default(),
        run_summary: None,
        id: session_id,
        // Default 1:1 session<->workspace: workspace.id mirrors the session id.
        workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid(session_id.uuid()),
        organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        harness_id,
        agent_id: Some(agent_id),
        agent_version_id: None,
        agent_identity_id: None,
        owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        owner: None,
        effective_owner: None,
        title: None,
        goal: None,
        locale: None,
        preview: None,
        output_preview: None,
        tags: vec![],
        model_id: None,
        capabilities: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        system_prompt: None,
        initial_files: vec![],
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        status: everruns_platform::SessionStatus::Started,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        started_at: None,
        finished_at: None,
        usage: None,
        is_pinned: None,
        archived_at: None,
        active_schedule_count: None,
        event_count: None,
        task_count: None,
        file_count: None,
        features: vec![],
        parent_session_id: None,
        forked_from_session_id: None,
        forked_from_sequence: None,
        blueprint_id: None,
        blueprint_config: None,
    }
}

#[test]
fn merge_effective_scoped_mcp_servers_strips_explicit_oauth_fields() {
    let mut harness = test_harness();
    harness.mcp_servers.insert(
        "docs".to_string(),
        oauth_scoped_server("https://harness.example.com/mcp", "github"),
    );

    let agent = test_agent();
    let session = test_session(harness.id, agent.public_id);

    let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);
    let docs = merged.get("docs").expect("server exists");

    assert_eq!(docs.auth_mode, McpServerAuthMode::None);
    assert!(docs.oauth_provider_id.is_none());
    assert!(
        !docs.tool_discovery,
        "tool discovery must be disabled on stripped explicit OAuth servers — without a token, an authenticated tools/list would 401",
    );
}

#[test]
fn merge_effective_scoped_mcp_servers_strips_explicit_only_when_oauth() {
    // Non-OAuth explicit entries (e.g. `auth_mode = None`) must pass through
    // unchanged. The sanitizer is narrow on purpose — see the doc comment on
    // strip_untrusted_oauth_from_scoped_mcp_servers.
    let mut harness = test_harness();
    harness.mcp_servers.insert(
        "docs".to_string(),
        scoped_server("https://harness.example.com/mcp"),
    );
    let agent = test_agent();
    let session = test_session(harness.id, agent.public_id);

    let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);
    let docs = merged.get("docs").expect("server exists");

    assert_eq!(docs.auth_mode, McpServerAuthMode::None);
    assert!(docs.oauth_provider_id.is_none());
    assert!(docs.tool_discovery);
}

#[test]
fn explicit_entries_replace_capability_entries_including_acts_as() {
    use everruns_capability::CapabilityRef as AgentCapabilityConfig;
    use everruns_core::capabilities::{Capability, CapabilityRegistry, RiskLevel};

    struct OAuthMcpCapability {
        acts_as: McpServerActsAs,
    }

    impl Capability for OAuthMcpCapability {
        fn id(&self) -> &str {
            "oauth_mcp_test"
        }

        fn name(&self) -> &str {
            "OAuth MCP Test"
        }

        fn description(&self) -> &str {
            "Capability that contributes scoped MCP servers with OAuth"
        }

        fn risk_level(&self) -> RiskLevel {
            RiskLevel::Low
        }

        fn mcp_servers(&self) -> CapabilityMcpServers {
            ["shared", "capability_only"]
                .into_iter()
                .map(|name| {
                    (
                        name.to_string(),
                        CapabilityMcpServer::new(
                            oauth_scoped_server("https://capability.example.com/mcp", "github"),
                            self.acts_as,
                        ),
                    )
                })
                .collect()
        }
    }

    for (contributed_acts_as, explicit_acts_as) in [
        (McpServerActsAs::User, McpServerActsAs::Service),
        (McpServerActsAs::Service, McpServerActsAs::User),
    ] {
        let mut registry = CapabilityRegistry::new();
        registry.register(OAuthMcpCapability {
            acts_as: contributed_acts_as,
        });
        let mut harness = test_harness();
        harness
            .capabilities
            .push(AgentCapabilityConfig::new("oauth_mcp_test"));
        harness.mcp_servers.insert(
            "shared".to_string(),
            catalog_server("linear", explicit_acts_as),
        );
        let agent = test_agent();
        let session = test_session(harness.id, agent.public_id);

        let merged = merge_effective_scoped_mcp_servers_with_capabilities(
            &harness,
            Some(&agent),
            &session,
            &registry,
        );

        assert_eq!(merged.len(), 2);
        let contributed = merged.get("capability_only").expect("contributed server");
        assert_eq!(contributed.auth_mode, McpServerAuthMode::OAuth);
        assert_eq!(contributed.oauth_provider_id.as_deref(), Some("github"));
        assert_eq!(contributed.acts_as, contributed_acts_as);

        let shared = merged.get("shared").expect("one effective shared server");
        assert_eq!(
            shared.preset.as_ref().map(|preset| preset.catalog_name()),
            Some("linear")
        );
        assert_eq!(shared.acts_as, explicit_acts_as);
        assert_eq!(shared.auth_mode, McpServerAuthMode::None);
        assert!(shared.oauth_provider_id.is_none());
    }
}

#[test]
fn merge_effective_scoped_mcp_servers_prefers_more_specific_layers() {
    let mut harness = test_harness();
    harness.mcp_servers.insert(
        "docs".to_string(),
        scoped_server("https://harness.example.com/mcp"),
    );

    let mut agent = test_agent();
    agent.mcp_servers.insert(
        "docs".to_string(),
        scoped_server("https://agent.example.com/mcp"),
    );
    agent.mcp_servers.insert(
        "search".to_string(),
        scoped_server("https://agent-search.example.com/mcp"),
    );

    let mut session = test_session(harness.id, agent.public_id);
    session.mcp_servers.insert(
        "docs".to_string(),
        scoped_server("https://session.example.com/mcp"),
    );

    let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);

    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged.get("docs").map(|server| server.url.as_str()),
        Some("https://session.example.com/mcp")
    );
    assert_eq!(
        merged.get("search").map(|server| server.url.as_str()),
        Some("https://agent-search.example.com/mcp")
    );
}

#[test]
fn validate_scoped_mcp_servers_rejects_stdio_transport() {
    let mut servers = ScopedMcpServers::default();
    servers.insert(
        "fs".to_string(),
        ScopedMcpServer {
            transport_type: everruns_core::McpServerTransportType::Stdio,
            command: Some("mcp-server-filesystem".to_string()),
            ..Default::default()
        },
    );

    let error = validate_scoped_mcp_servers(&servers).unwrap_err();
    assert!(
        error.to_string().contains("stdio"),
        "expected stdio rejection, got: {error}"
    );
}

#[test]
fn validate_scoped_mcp_servers_rejects_inline_identity_and_preset_fields() {
    for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
        let servers = ScopedMcpServers::from([(
            "docs".into(),
            ScopedMcpServer {
                url: "https://docs.example.com/mcp".into(),
                acts_as,
                ..Default::default()
            },
        )]);
        let error = validate_scoped_mcp_servers(&servers).unwrap_err();
        assert!(error.to_string().contains("requires a catalog preset"));
    }

    for (field, server) in [
        (
            "url",
            ScopedMcpServer {
                url: "https://docs.example.com/mcp".into(),
                ..catalog_server("linear", McpServerActsAs::None)
            },
        ),
        (
            "headers",
            ScopedMcpServer {
                headers: HashMap::from([("X-Test".into(), "value".into())]),
                ..catalog_server("linear", McpServerActsAs::None)
            },
        ),
    ] {
        let servers = ScopedMcpServers::from([("docs".into(), server)]);
        let error = validate_scoped_mcp_servers(&servers).unwrap_err();
        assert!(error.to_string().contains(field), "{error}");
    }
}

#[tokio::test]
async fn catalog_validation_requires_live_existing_oauth_presets() {
    let db = StorageBackend::in_memory();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let missing = ScopedMcpServers::from([(
        "docs".into(),
        catalog_server("missing", McpServerActsAs::None),
    )]);
    let error = validate_scoped_mcp_servers_for_org(&db, org_id, &missing)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing"), "{error}");

    let plain_id = seed_catalog_server(&db, "plain", false).await;
    let needs_oauth = ScopedMcpServers::from([(
        "docs".into(),
        catalog_server("plain", McpServerActsAs::Service),
    )]);
    let error = validate_scoped_mcp_servers_for_org(&db, org_id, &needs_oauth)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("OAuth"), "{error}");
    db.update_mcp_server(
        org_id,
        plain_id.uuid(),
        UpdateMcpServer {
            status: Some("disabled".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let disabled = ScopedMcpServers::from([(
        "docs".into(),
        catalog_server("plain", McpServerActsAs::None),
    )]);
    let error = validate_scoped_mcp_servers_for_org(&db, org_id, &disabled)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("disabled"), "{error}");

    db.update_mcp_server(
        org_id,
        plain_id.uuid(),
        UpdateMcpServer {
            status: Some("archived".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let archived = ScopedMcpServers::from([(
        "docs".into(),
        catalog_server("plain", McpServerActsAs::None),
    )]);
    let error = validate_scoped_mcp_servers_for_org(&db, org_id, &archived)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("plain"), "{error}");

    let deleted_id = seed_catalog_server(&db, "deleted", false).await;
    db.update_mcp_server(
        org_id,
        deleted_id.uuid(),
        UpdateMcpServer {
            status: Some("deleted".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let deleted = ScopedMcpServers::from([(
        "docs".into(),
        catalog_server("deleted", McpServerActsAs::None),
    )]);
    let error = validate_scoped_mcp_servers_for_org(&db, org_id, &deleted)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("deleted"), "{error}");
}

#[tokio::test]
async fn two_logical_names_can_resolve_the_same_catalog_preset() {
    let db = Arc::new(StorageBackend::in_memory());
    let preset_id = seed_catalog_server(&db, "linear", true).await;
    let service = McpServerService::new(db, None);
    let harness = test_harness();
    let agent = test_agent();
    let mut session = test_session(harness.id, agent.public_id);
    session.mcp_servers = ScopedMcpServers::from([
        (
            "issues".into(),
            catalog_server("linear", McpServerActsAs::Service),
        ),
        (
            "projects".into(),
            catalog_server("linear", McpServerActsAs::User),
        ),
    ]);

    let mut descriptor_ids = Vec::new();
    for (prefix, acts_as) in [
        ("issues", McpServerActsAs::Service),
        ("projects", McpServerActsAs::User),
    ] {
        let resolved = resolve_scoped_mcp_server(
            &service,
            everruns_core::DEFAULT_ORG_ID,
            &harness,
            Some(&agent),
            &session,
            prefix,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(resolved.name, prefix);
        assert_eq!(resolved.url, "http://8.8.8.8/mcp");
        assert_eq!(resolved.protocol_mode, McpProtocolMode::V2025June);
        assert_eq!(resolved.acts_as, acts_as);
        assert_eq!(resolved.headers.get("X-Catalog"), Some(&"value".into()));
        // Both attachments declare an acting identity, so both point at the
        // connection store keyed by the shared preset (EVE-1029). The
        // descriptor still carries no credential of its own.
        assert_eq!(resolved.auth_mode, McpServerAuthMode::OAuth);
        assert_eq!(
            resolved.oauth_provider_id.as_deref(),
            Some(everruns_core::mcp_oauth_provider_id_for_uuid(preset_id.uuid()).as_str())
        );
        assert!(resolved.api_key.is_none());
        descriptor_ids.push(resolved.id);
    }

    // Same preset, same provider key, but distinct session-scoped
    // descriptor ids — that is what makes two logical names independent.
    assert_ne!(descriptor_ids[0], descriptor_ids[1]);
}

#[tokio::test]
async fn user_attachment_discards_preset_api_key_and_authorization_header() {
    // A preset carrying service auth, of the shape a config written before
    // validation existed could still have.
    let db = Arc::new(StorageBackend::in_memory());
    let settings = crate::domains::mcp_servers::service::McpServerSettings {
        auth_mode: McpServerAuthMode::OAuth,
        protocol_mode: McpProtocolMode::V2025June,
        oauth: Some(crate::domains::mcp_servers::service::McpServerOAuthSettings::default()),
    };
    db.create_mcp_server(
        everruns_core::DEFAULT_ORG_ID,
        CreateMcpServerRow {
            name: "linear".to_string(),
            description: None,
            url: "http://8.8.8.8/mcp".to_string(),
            transport_type: "http".to_string(),
            api_key_encrypted: None,
            headers: Some(serde_json::json!({
                "X-Catalog": "value",
                "Authorization": "Bearer org-service-token",
            })),
            settings: Some(serde_json::to_value(settings).unwrap()),
        },
    )
    .await
    .unwrap();
    let service = McpServerService::new(db, None);
    let harness = test_harness();
    let agent = test_agent();
    let mut session = test_session(harness.id, agent.public_id);
    session.mcp_servers = ScopedMcpServers::from([(
        "projects".into(),
        catalog_server("linear", McpServerActsAs::User),
    )]);

    let resolved = resolve_scoped_mcp_server(
        &service,
        everruns_core::DEFAULT_ORG_ID,
        &harness,
        Some(&agent),
        &session,
        "projects",
    )
    .await
    .unwrap()
    .unwrap();

    // THREAT[TM-TOOL-041]: a `user` attachment can never carry service auth.
    assert!(
        !has_authorization_header(&resolved.headers),
        "org-held Authorization must not survive onto a user attachment"
    );
    assert!(resolved.api_key.is_none());
    // Non-credential headers are untouched, so this is a scrub, not a wipe.
    assert_eq!(resolved.headers.get("X-Catalog"), Some(&"value".into()));
}

#[tokio::test]
async fn none_attachment_keeps_preset_transport_and_literal_headers() {
    let db = Arc::new(StorageBackend::in_memory());
    seed_catalog_server(&db, "linear", false).await;
    let service = McpServerService::new(db, None);
    let harness = test_harness();
    let agent = test_agent();
    let mut session = test_session(harness.id, agent.public_id);
    session.mcp_servers = ScopedMcpServers::from([(
        "docs".into(),
        catalog_server("linear", McpServerActsAs::None),
    )]);

    let resolved = resolve_scoped_mcp_server(
        &service,
        everruns_core::DEFAULT_ORG_ID,
        &harness,
        Some(&agent),
        &session,
        "docs",
    )
    .await
    .unwrap()
    .unwrap();

    // `none` reads no connection store, so it must not be wired to one.
    assert_eq!(resolved.acts_as, McpServerActsAs::None);
    assert_eq!(resolved.auth_mode, McpServerAuthMode::None);
    assert!(resolved.oauth_provider_id.is_none());
    assert!(resolved.api_key.is_none());
    assert_eq!(resolved.headers.get("X-Catalog"), Some(&"value".into()));
}
#[tokio::test]
async fn catalog_preview_discovery_never_uses_legacy_connection_tokens() {
    let db = StorageBackend::in_memory();
    seed_catalog_server(&db, "linear", true).await;
    let resolver = Arc::new(CountingConnectionResolver::default());
    let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();
    let egress = CatalogPreviewEgress::default();

    for acts_as in [
        McpServerActsAs::None,
        McpServerActsAs::Service,
        McpServerActsAs::User,
    ] {
        let servers =
            ScopedMcpServers::from([("docs".to_string(), catalog_server("linear", acts_as))]);
        let materialized =
            materialize_scoped_mcp_servers(&db, everruns_core::DEFAULT_ORG_ID, &servers)
                .await
                .unwrap();
        let docs = materialized.get("docs").unwrap();
        assert_eq!(docs.auth_mode, McpServerAuthMode::None);
        assert!(docs.oauth_provider_id.is_none());
        assert_eq!(docs.acts_as, acts_as);
        let discovered = fetch_mcp_tools(&egress, &docs.url, None, &docs.headers)
            .await
            .unwrap();
        assert_eq!(discovered.len(), 1);

        let tools = build_materialized_scoped_mcp_tool_definitions(
            &db,
            everruns_core::DEFAULT_ORG_ID,
            &servers,
            None,
            Some(&resolver_trait),
            &egress,
        )
        .await
        .unwrap();
        assert_eq!(tools.len(), 1, "catalog preview must expose one MCP tool");
        assert!(tools[0].name().contains("echo"));
    }

    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(egress.calls.load(Ordering::SeqCst), 6);
}
#[tokio::test]
async fn runtime_catalog_discovery_without_cache_identity_fails_closed() {
    let db = StorageBackend::in_memory();
    seed_catalog_server(&db, "linear", true).await;
    let resolver = Arc::new(CountingConnectionResolver::default());
    let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();
    let egress = CatalogPreviewEgress::default();
    let servers = ScopedMcpServers::from([(
        "docs".to_string(),
        catalog_server("linear", McpServerActsAs::User),
    )]);

    let tools = build_materialized_scoped_mcp_tool_definitions(
        &db,
        everruns_core::DEFAULT_ORG_ID,
        &servers,
        Some(SessionId::new()),
        Some(&resolver_trait),
        &egress,
    )
    .await
    .unwrap();

    assert!(tools.is_empty());
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(egress.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn capability_mcp_validation_rejects_presets_and_allows_declared_identity() {
    let preset = CapabilityMcpServers::from([(
        "docs".to_string(),
        CapabilityMcpServer::new(
            catalog_server("linear", McpServerActsAs::None),
            McpServerActsAs::None,
        ),
    )]);
    let error = validate_capability_mcp_servers(&preset).unwrap_err();
    assert!(error.to_string().contains("cannot use a catalog preset"));

    for acts_as in [
        McpServerActsAs::None,
        McpServerActsAs::Service,
        McpServerActsAs::User,
    ] {
        let contribution = CapabilityMcpServers::from([(
            "docs".to_string(),
            CapabilityMcpServer::new(
                ScopedMcpServer {
                    url: "https://docs.example.com/mcp".to_string(),
                    auth_mode: if acts_as.is_none() {
                        McpServerAuthMode::None
                    } else {
                        McpServerAuthMode::OAuth
                    },
                    ..Default::default()
                },
                acts_as,
            ),
        )]);
        validate_capability_mcp_servers(&contribution).unwrap();
    }
}

#[test]
fn validate_scoped_mcp_servers_rejects_duplicate_sanitized_names() {
    let mut servers = ScopedMcpServers::default();
    servers.insert(
        "Docs API".to_string(),
        scoped_server("https://one.example.com/mcp"),
    );
    servers.insert(
        "docs-api".to_string(),
        scoped_server("https://two.example.com/mcp"),
    );

    let error = validate_scoped_mcp_servers(&servers).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must be unique after sanitization")
    );
}

#[test]
fn validate_scoped_mcp_servers_rejects_reserved_delimiter_after_sanitization() {
    for name in ["admin__foo", "admin..foo", "admin_", "admin-", "_"] {
        let servers =
            ScopedMcpServers::from([(name.into(), scoped_server("https://one.example.com/mcp"))]);
        let error = validate_scoped_mcp_servers(&servers).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("reserved for MCP tool prefix delimiters"),
            "{name}: {error}"
        );
    }
    let servers = ScopedMcpServers::from([(
        "admin_api".into(),
        scoped_server("https://one.example.com/mcp"),
    )]);
    validate_scoped_mcp_servers(&servers).unwrap();
}

#[test]
fn validate_merged_scoped_mcp_servers_rejects_cross_layer_duplicates() {
    let mut harness_servers = ScopedMcpServers::default();
    harness_servers.insert(
        "Docs API".to_string(),
        scoped_server("https://one.example.com/mcp"),
    );

    let mut session_servers = ScopedMcpServers::default();
    session_servers.insert(
        "docs-api".to_string(),
        scoped_server("https://two.example.com/mcp"),
    );

    let error =
        validate_merged_scoped_mcp_servers([&harness_servers, &session_servers]).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("must be unique after sanitization")
    );
}

#[test]
fn scoped_mcp_server_uuid_is_stable_and_namespaced_by_session() {
    let session_a = SessionId::new().uuid();
    let session_b = SessionId::new().uuid();

    assert_eq!(
        scoped_mcp_server_uuid(session_a, "docs"),
        scoped_mcp_server_uuid(session_a, "docs")
    );
    assert_ne!(
        scoped_mcp_server_uuid(session_a, "docs"),
        scoped_mcp_server_uuid(session_b, "docs")
    );
}
