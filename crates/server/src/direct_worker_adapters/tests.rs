use super::*;
use crate::direct_worker_adapters_mcp::resolved_mcp_server_to_worker_info;
use crate::domains::mcp_servers::McpServerResolved;
use crate::storage::models::CreateHarnessRow;

#[test]
fn string_to_provider_type_maps_gemini() {
    assert_eq!(string_to_provider_type("gemini").to_string(), "gemini");
}

#[test]
fn string_to_provider_type_maps_unknown_to_external() {
    // Unknown ids resolve to External, preserving the id.
    assert_eq!(
        string_to_provider_type("custom-provider").to_string(),
        "custom-provider"
    );
}

#[test]
fn direct_mcp_adapter_preserves_neutral_catalog_descriptors() {
    for acts_as in [
        everruns_core::McpServerActsAs::None,
        everruns_core::McpServerActsAs::Service,
        everruns_core::McpServerActsAs::User,
    ] {
        let resolved = McpServerResolved {
            id: Uuid::new_v4(),
            name: "linear".to_string(),
            url: "https://mcp.linear.app/mcp".to_string(),
            auth_mode: everruns_core::McpServerAuthMode::None,
            protocol_mode: everruns_core::McpProtocolMode::Auto,
            oauth_provider_id: None,
            acts_as,
            elicitation_policy: Default::default(),
            api_key: None,
            headers: HashMap::new(),
        };

        let info = resolved_mcp_server_to_worker_info(resolved, HashMap::new());

        assert_eq!(info.acts_as, acts_as);
        assert_eq!(info.auth_mode, everruns_core::McpServerAuthMode::None);
        assert!(info.oauth_provider_id.is_none());
        assert!(info.api_key.is_none());
    }
}

// =========================================================================
// Capability deduplication helpers (EVE-47)
// =========================================================================

/// Build a DirectWorkerAdapters with in-memory backends for unit tests.
fn test_adapters() -> DirectWorkerAdapters {
    let db = Arc::new(crate::storage::StorageBackend::in_memory());
    let event_service = Arc::new(crate::services::EventService::new(
        db.clone(),
        crate::event_delivery::EventDelivery::in_memory(),
    ));
    let provider_resolver = Arc::new(crate::services::ProviderResolverService::new(
        db.clone(),
        None,
    ));
    let mcp_server_service = Arc::new(crate::domains::mcp_servers::McpServerService::new(
        db.clone(),
        None,
    ));
    let cap_registry = CapabilityRegistry::new();
    let driver_registry = everruns_worker::create_driver_registry();
    let sqldb_backend = Arc::new(crate::session_sqldb::InMemorySqlDbBackend::new());
    let sqldb_store: std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore> =
        Arc::new(crate::session_sqldb::InMemorySqlDbStore::new(sqldb_backend));

    DirectWorkerAdapters::new(
        db,
        event_service,
        provider_resolver,
        mcp_server_service,
        cap_registry,
        driver_registry,
        sqldb_store,
    )
}

// Missing-agent lookup is covered by the `direct_adapter_contract` macro
// suite's `get_agent_nonexistent_returns_none` (same production path and
// assertion), so it is not duplicated here.

#[tokio::test]
async fn get_agent_resolves_by_public_id() {
    let adapters = test_adapters();
    let agent_id = seed_agent(&adapters.db).await;

    let agent = adapters
        .get_agent(everruns_core::DEFAULT_ORG_ID, agent_id)
        .await
        .unwrap()
        .expect("agent should exist");

    assert_eq!(agent.name, "test-agent");
}

#[path = "phase_read_tests.rs"]
mod phase_read_tests;

#[tokio::test]
async fn build_mcp_tool_definitions_with_empty_capabilities() {
    let adapters = test_adapters();
    let tools = adapters
        .build_mcp_tool_definitions_with_capabilities(everruns_core::DEFAULT_ORG_ID, &[])
        .await
        .unwrap();
    assert!(tools.is_empty());
}

#[tokio::test]
async fn build_mcp_tool_definitions_skips_non_mcp_capabilities() {
    let adapters = test_adapters();
    let agent_id = Uuid::new_v4();
    let rows = vec![
        fake_capability_row(agent_id, "web_search"),
        fake_capability_row(agent_id, "code_execution"),
    ];
    let tools = adapters
        .build_mcp_tool_definitions_with_capabilities(everruns_core::DEFAULT_ORG_ID, &rows)
        .await
        .unwrap();
    // None of these are MCP capabilities, so no tool definitions produced
    assert!(tools.is_empty());
}

#[tokio::test]
async fn scoped_mcp_lookup_uses_pinned_agent_version_in_direct_and_grpc_paths() {
    use crate::storage::models::{
        CreateAgentRow, CreateAgentVersionRow, CreateMcpServerRow, CreateSessionRow,
    };
    use everruns_contracts::typed_id::{AgentVersionId, PrincipalId};
    use everruns_internal_protocol::proto::{
        GetMcpServerByPrefixRequest, Uuid as ProtoUuid,
        worker_service_server::WorkerService as GrpcWorkerService,
    };

    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let harness_id =
        seed_harness_for_platform_store(&adapters.db, org_id, "pinned-mcp-harness", false).await;
    adapters
        .db
        .create_mcp_server(
            org_id,
            CreateMcpServerRow {
                name: "pinned-catalog".to_string(),
                description: None,
                url: "https://pinned.example.com/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(serde_json::json!({
                    "auth_mode": "oauth",
                    "oauth": {}
                })),
            },
        )
        .await
        .expect("create pinned catalog server");

    let agent_id = AgentId::new();
    adapters
        .db
        .create_agent_with_id(
            org_id,
            agent_id,
            CreateAgentRow {
                public_id: agent_id.to_string(),
                name: "pinned-mcp-agent".to_string(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: String::new(),
                default_model_id: None,
                harness_id,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({
                    "docs": {
                        "type": "http",
                        "url": "https://current.example.com/mcp"
                    }
                }),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .expect("create current agent")
        .expect("agent should be created");

    let version_id = AgentVersionId::new();
    let pinned_config = serde_json::json!({
        "mcp_servers": {
            "docs": {
                "use": "catalog:pinned-catalog",
                "actsAs": "service"
            }
        }
    });
    adapters
        .db
        .create_agent_version(CreateAgentVersionRow {
            id: version_id,
            public_id: version_id.to_string(),
            org_id,
            agent_id,
            version_number: 1,
            semver_major: 0,
            semver_minor: 1,
            semver_patch: 0,
            version: "0.1.0".to_string(),
            is_published: true,
            parent_version_id: None,
            source_version_id: None,
            created_by_principal_id: None,
            change_kind: "minor".to_string(),
            summary: None,
            config_hash: "pinned-mcp-config".to_string(),
            authored_config: pinned_config.clone(),
            resolved_config: pinned_config,
        })
        .await
        .expect("create pinned agent version");

    let session = adapters
        .db
        .create_session(CreateSessionRow {
            playground_user_id: None,
            trigger_id: None,
            source: crate::records::SessionSource::Api,
            workspace_id: None,
            org_id,
            app_id: None,
            channel_id: None,
            harness_id: Some(harness_id),
            agent_id: Some(agent_id),
            agent_version_id: Some(version_id),
            agent_config_hash: None,
            virtual_user_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
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
        .expect("create pinned session");

    let direct = adapters
        .get_mcp_server_by_prefix(org_id, Some(session.id.uuid()), "docs")
        .await
        .expect("direct pinned MCP lookup");
    let grpc_service = crate::grpc_service::WorkerServiceImpl::new(
        adapters.event_service.as_ref().clone(),
        adapters.db.clone(),
        None,
        None,
        crate::oss_host_composition_for_grade(everruns_core::DeploymentGrade::Dev),
    );
    let grpc = grpc_service
        .get_mcp_server_by_prefix(tonic::Request::new(GetMcpServerByPrefixRequest {
            input_message_id: None,
            org_id,
            session_id: Some(ProtoUuid {
                value: session.id.uuid().to_string(),
            }),
            server_prefix: "docs".to_string(),
        }))
        .await
        .expect("gRPC pinned MCP lookup")
        .into_inner()
        .server
        .expect("gRPC MCP descriptor");

    assert_eq!(direct.url, "https://pinned.example.com/mcp");
    assert_eq!(direct.acts_as, everruns_core::McpServerActsAs::Service);
    assert_eq!(grpc.url, direct.url);
    assert_eq!(grpc.acts_as, direct.acts_as.to_string());
    // A `service` attachment is wired to the connection store keyed by its
    // preset, so both paths report OAuth rather than a neutral descriptor
    // (EVE-1029). The point of this test is that the two paths agree.
    assert_eq!(grpc.auth_mode, "oauth");
    assert_eq!(grpc.auth_mode, direct.auth_mode.to_string());
    assert_eq!(grpc.oauth_provider_id, direct.oauth_provider_id);
    assert!(
        direct.oauth_provider_id.is_some(),
        "a service attachment must name the store it resolves from"
    );
    // Neither path hands the worker a credential of its own.
    assert!(direct.api_key.is_none());
    assert!(grpc.api_key.is_none());
}
// =========================================================================
// grep_files parity tests (EVE-58)
// =========================================================================

/// Seed a file into the in-memory store for grep tests.
async fn seed_file(db: &StorageBackend, session_id: Uuid, path: &str, content: &str) {
    use crate::storage::models::CreateSessionFileRow;
    let create = CreateSessionFileRow {
        session_id: SessionId::from_uuid(session_id),
        path: path.to_string(),
        content: Some(content.as_bytes().to_vec()),
        is_directory: false,
        is_readonly: false,
    };
    db.create_session_file(create).await.expect("seed file");
}

// Basic grep_files match/no-match/multi-file/regex/invalid-regex behavior
// is covered by the `direct_adapter_contract` macro suite below (EVE-61),
// which exercises the same production path (`grep_files`) so it is not
// duplicated here. This file keeps only the context-merging case, which
// the contract suite does not cover.
#[tokio::test]
async fn grep_files_returns_bounded_merged_context() {
    let adapters = test_adapters();
    let session_id = Uuid::new_v4();
    seed_file(
        &adapters.db,
        session_id,
        "/events.log",
        "before\nError one\nbetween\nError two\nafter\n",
    )
    .await;

    let result = adapters
        .grep_files_with_options(
            everruns_core::DEFAULT_ORG_ID,
            session_id,
            "Error",
            &GrepOptions {
                path_pattern: None,
                before_context: 1,
                after_context: 1,
                offset: 0,
                limit: 2,
                max_bytes: everruns_core::GREP_MAX_RETURN_BYTES,
            },
        )
        .await
        .unwrap();

    assert_eq!(result.total_matches, 2);
    assert_eq!(result.returned_matches, 2);
    assert_eq!(result.blocks.len(), 1);
    assert_eq!(result.blocks[0].match_line_numbers, vec![2, 4]);
    assert_eq!(result.blocks[0].lines.len(), 5);
}

async fn seed_harness_for_platform_store(
    db: &StorageBackend,
    org_id: i64,
    name: &str,
    is_built_in: bool,
) -> HarnessId {
    db.create_harness(
        org_id,
        CreateHarnessRow {
            name: name.to_string(),
            display_name: None,
            icon: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: serde_json::json!([]),
            system_prompt: Some("test prompt".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            initial_files: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            embedder_metadata: serde_json::json!({}),
            is_built_in,
        },
    )
    .await
    .expect("seed harness")
    .id
}

async fn seed_platform_session(
    db: &StorageBackend,
    org_id: i64,
    harness_id: HarnessId,
    resolved_owner_user_id: Option<Uuid>,
) -> SessionId {
    use crate::storage::models::CreateSessionRow;

    let session = db
        .create_session(CreateSessionRow {
            playground_user_id: None,
            trigger_id: None,
            source: crate::records::SessionSource::Api,
            workspace_id: None,
            org_id,
            app_id: None,
            channel_id: None,
            harness_id: Some(harness_id),
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            virtual_user_id: None,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id,
            title: Some("platform-store-test".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
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
        .expect("seed session")
        .id;
    if let Some(user) = resolved_owner_user_id {
        let subject = db.default_virtual_user(org_id, user).await.unwrap();
        db.record_runtime_invocation(
            org_id,
            session,
            session.uuid(),
            Some(subject.id),
            Some(user),
            None,
        )
        .await
        .unwrap();
    }
    session
}

async fn seed_platform_owner(db: &StorageBackend, org_id: i64, email: &str) -> Uuid {
    use crate::storage::models::CreateUserRow;

    let user = db
        .create_user(CreateUserRow {
            email: email.to_string(),
            name: "Platform Owner".to_string(),
            avatar_url: None,
            external_id: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("test".to_string()),
            auth_provider_id: None,
        })
        .await
        .expect("create owner user");
    db.ensure_membership(user.id, org_id, "owner")
        .await
        .expect("ensure owner membership");
    user.id
}

#[tokio::test]
async fn platform_store_uses_session_owner_permissions() {
    use crate::storage::models::{CreateOrganizationRow, CreateUserRow};

    let adapters = test_adapters();
    let org = adapters
        .db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000042".to_string(),
            name: "Platform Store Auth".to_string(),
            created_by: None,
        })
        .await
        .expect("create org");
    let user = adapters
        .db
        .create_user(CreateUserRow {
            email: "member@example.com".to_string(),
            name: "Member".to_string(),
            avatar_url: None,
            external_id: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("test".to_string()),
            auth_provider_id: None,
        })
        .await
        .expect("create user");
    adapters
        .db
        .ensure_membership(user.id, org.org_id, "member")
        .await
        .expect("ensure membership");

    let harness_id =
        seed_harness_for_platform_store(&adapters.db, org.org_id, "session-harness", false).await;
    let session_id =
        seed_platform_session(&adapters.db, org.org_id, harness_id, Some(user.id)).await;
    let store = adapters
        .platform_store(org.org_id, session_id)
        .for_execution(session_id.uuid())
        .unwrap();

    let discovered = store
        .platform_discover(serde_json::json!({ "query": "models" }))
        .await
        .expect("members may inspect the command catalog");
    assert!(discovered.contains("list_models"));

    store
        .platform_query(serde_json::json!({ "commands": "list_models" }))
        .await
        .expect("members may run read-only platform queries");

    let script_error = store
        .platform_execute(serde_json::json!({
            "commands": "create_harness --name forbidden-script"
        }))
        .await
        .expect_err("member should not mutate through the command surface");
    assert!(
        script_error.to_string().contains("forbidden")
            || script_error.to_string().contains("Access denied"),
        "unexpected authorization error: {script_error}"
    );
}

struct DenySessionManageResolver;

impl everruns_core::PermissionResolver for DenySessionManageResolver {
    fn has_permission(
        &self,
        caller: &everruns_core::Caller,
        permission: &everruns_core::Permission,
    ) -> bool {
        *permission != everruns_core::Permission::OrgSessionsManage
            && everruns_core::DefaultPermissionResolver.has_permission(caller, permission)
    }

    fn caller_permissions(&self, caller: &everruns_core::Caller) -> Vec<everruns_core::Permission> {
        everruns_core::DefaultPermissionResolver
            .caller_permissions(caller)
            .into_iter()
            .filter(|permission| *permission != everruns_core::Permission::OrgSessionsManage)
            .collect()
    }
}

#[tokio::test]
async fn session_creation_authority_uses_owner_permission_and_returns_root() {
    let adapters = test_adapters();
    let org_id = everruns_core::DEFAULT_ORG_ID;
    let owner_id =
        seed_platform_owner(&adapters.db, org_id, "detached-authority-owner@example.com").await;
    let harness_id =
        seed_harness_for_platform_store(&adapters.db, org_id, "authority-harness", false).await;
    let root_id = seed_platform_session(&adapters.db, org_id, harness_id, Some(owner_id)).await;
    let authority = adapters
        .session_creation_authority(org_id, root_id)
        .expect("direct authority")
        .for_execution(root_id.uuid())
        .unwrap();
    assert_eq!(
        authority
            .authorize_session_creation(root_id)
            .await
            .expect("owner may create sessions"),
        root_id
    );

    let denied_adapters =
        test_adapters().with_permission_resolver(Arc::new(DenySessionManageResolver));
    let denied_owner = seed_platform_owner(
        &denied_adapters.db,
        org_id,
        "detached-authority-denied@example.com",
    )
    .await;
    let denied_harness = seed_harness_for_platform_store(
        &denied_adapters.db,
        org_id,
        "denied-authority-harness",
        false,
    )
    .await;
    let denied_session = seed_platform_session(
        &denied_adapters.db,
        org_id,
        denied_harness,
        Some(denied_owner),
    )
    .await;
    let denied = denied_adapters
        .session_creation_authority(org_id, denied_session)
        .expect("direct authority")
        .for_execution(denied_session.uuid())
        .unwrap()
        .authorize_session_creation(denied_session)
        .await
        .expect_err("custom resolver denial must be honored");
    assert!(denied.to_string().contains("Access denied"));
}

/// Regression: the direct platform store's command ctx must carry the
/// event service — wait_for_idle probes terminal turn events through the
/// list_events command, and without it every subagent wait failed with
/// "Event service not configured".
#[tokio::test]
async fn platform_store_wait_for_idle_reaches_event_service() {
    use everruns_core::DEFAULT_ORG_ID;

    let adapters = test_adapters();
    let user_id = seed_platform_owner(&adapters.db, DEFAULT_ORG_ID, "waiter@example.com").await;
    let harness_id =
        seed_harness_for_platform_store(&adapters.db, DEFAULT_ORG_ID, "wait-harness", false).await;
    let session_id =
        seed_platform_session(&adapters.db, DEFAULT_ORG_ID, harness_id, Some(user_id)).await;
    let store = adapters
        .platform_store(DEFAULT_ORG_ID, session_id)
        .for_execution(session_id.uuid())
        .unwrap();

    // No terminal turn events exist, so a zero-timeout wait reports a
    // timeout — the point is it must NOT fail on a missing event service.
    let status = store
        .wait_for_idle(session_id, Some(0))
        .await
        .expect("wait_for_idle should reach the event service");
    assert!(status.starts_with("timeout"), "unexpected status: {status}");
}

// ---- helpers ----

/// Seed a test agent, returning its UUID.
///
/// Uses `create_agent_with_id` so public_id matches the internal UUID,
/// consistent with normal agent creation via the API.
async fn seed_agent(db: &StorageBackend) -> Uuid {
    use crate::storage::models::CreateAgentRow;
    let id = AgentId::new();
    let public_id = id.to_string();
    let create = CreateAgentRow {
        public_id,
        name: "test-agent".to_string(),
        display_name: Some("Test Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: serde_json::json!([]),
        system_prompt: String::new(),
        default_model_id: None,
        harness_id: everruns_contracts::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
        tags: vec![],
        initial_files: serde_json::Value::Array(vec![]),
        tools: serde_json::Value::Array(vec![]),
        mcp_servers: serde_json::json!({}),
        max_iterations: None,
        network_access: None,
        parallel_tool_calls: None,
        environments: None,
        is_built_in: false,
    };
    db.create_agent_with_id(everruns_core::DEFAULT_ORG_ID, id, create)
        .await
        .expect("seed agent")
        .expect("seed agent should create");
    id.uuid()
}

// =========================================================================
// Cross-org isolation regression tests (EVE-56)
// =========================================================================

/// Regression test: schedule_store must be scoped to the provided org_id,
/// not a hardcoded default. Before EVE-56, schedule_store() had no org_id
/// parameter and used DEFAULT_ORG_ID for all calls.
#[test]
fn schedule_store_is_scoped_to_org_id() {
    let adapters = test_adapters();
    let store_org1 = adapters.schedule_store(1);
    let store_org2 = adapters.schedule_store(2);
    // Verify these are distinct instances (different Arc pointers)
    assert!(!Arc::ptr_eq(&store_org1, &store_org2));
}

/// Regression test: platform_store must use the provided org_id, not
/// DEFAULT_ORG_ID. Agents created in org 1 must not be visible through
/// org 2's platform store.
#[tokio::test]
async fn platform_store_cross_org_isolation() {
    use crate::storage::models::CreateOrganizationRow;

    let adapters = test_adapters();
    let agent_id = seed_agent(&adapters.db).await;
    let org2 = adapters
        .db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000999".to_string(),
            name: "Platform Store Org 2".to_string(),
            created_by: None,
        })
        .await
        .expect("create org 2");
    let harness_org1 = seed_harness_for_platform_store(
        &adapters.db,
        everruns_core::DEFAULT_ORG_ID,
        "org1-session-harness",
        false,
    )
    .await;
    let harness_org2 =
        seed_harness_for_platform_store(&adapters.db, org2.org_id, "org2-session-harness", false)
            .await;
    let owner_org1 = seed_platform_owner(
        &adapters.db,
        everruns_core::DEFAULT_ORG_ID,
        "org1-owner@example.com",
    )
    .await;
    let owner_org2 = seed_platform_owner(&adapters.db, org2.org_id, "org2-owner@example.com").await;
    let session_org1 = seed_platform_session(
        &adapters.db,
        everruns_core::DEFAULT_ORG_ID,
        harness_org1,
        Some(owner_org1),
    )
    .await;
    let session_org2 =
        seed_platform_session(&adapters.db, org2.org_id, harness_org2, Some(owner_org2)).await;

    // Agent seeded in org 1 should be visible via org 1's platform store
    let store_org1 = adapters
        .platform_store(everruns_core::DEFAULT_ORG_ID, session_org1)
        .for_execution(session_org1.uuid())
        .unwrap();
    let agent = store_org1
        .get_agent_by_id(everruns_contracts::typed_id::AgentId::from_uuid(agent_id))
        .await
        .unwrap();
    assert!(agent.is_some(), "agent should be visible in org 1");

    // Same agent must NOT be visible via org 2's platform store
    let store_org2 = adapters
        .platform_store(org2.org_id, session_org2)
        .for_execution(session_org2.uuid())
        .unwrap();
    let agent = store_org2
        .get_agent_by_id(everruns_contracts::typed_id::AgentId::from_uuid(agent_id))
        .await
        .unwrap();
    assert!(agent.is_none(), "agent must NOT be visible in org 2");
}

// EVE-56: resolve_image must accept org_id so the gRPC service scopes the
// lookup by org — enforced at compile time by the signature, and the
// missing-image runtime behavior is covered by the `direct_adapter_contract`
// macro suite's `resolve_image_missing_returns_none`.

/// Build a fake `AgentCapabilityRow` for testing.
fn fake_capability_row(agent_id: Uuid, capability_id: &str) -> AgentCapabilityRow {
    AgentCapabilityRow {
        id: Uuid::new_v4(),
        agent_id: AgentId::from_uuid(agent_id),
        capability_id: capability_id.to_string(),
        position: 0,
        config: serde_json::Value::Object(Default::default()),
        created_at: chrono::Utc::now(),
    }
}

// =========================================================================
// MCP server API key decryption tests (EVE-55)
// =========================================================================

fn test_encryption() -> Arc<crate::storage::EncryptionService> {
    Arc::new(
        crate::storage::EncryptionService::new(
            "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            &[],
        )
        .unwrap(),
    )
}

fn test_adapters_with_encryption() -> DirectWorkerAdapters {
    let encryption = test_encryption();
    let db = Arc::new(crate::storage::StorageBackend::in_memory());
    let event_service = Arc::new(crate::services::EventService::new(
        db.clone(),
        crate::event_delivery::EventDelivery::in_memory(),
    ));
    let provider_resolver = Arc::new(crate::services::ProviderResolverService::new(
        db.clone(),
        None,
    ));
    let mcp_server_service = Arc::new(crate::domains::mcp_servers::McpServerService::new(
        db.clone(),
        Some(encryption),
    ));
    let cap_registry = CapabilityRegistry::new();
    let driver_registry = everruns_worker::create_driver_registry();
    let sqldb_backend = Arc::new(crate::session_sqldb::InMemorySqlDbBackend::new());
    let sqldb_store: std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore> =
        Arc::new(crate::session_sqldb::InMemorySqlDbStore::new(sqldb_backend));

    DirectWorkerAdapters::new(
        db,
        event_service,
        provider_resolver,
        mcp_server_service,
        cap_registry,
        driver_registry,
        sqldb_store,
    )
}

fn test_adapters_with_budget_service() -> DirectWorkerAdapters {
    let adapters = test_adapters();
    let budget_service = Arc::new(crate::domains::budgets::BudgetService::new(
        adapters.db.clone(),
    ));
    adapters.with_budget_service(budget_service)
}

#[tokio::test]
async fn budget_checker_is_absent_without_service() {
    let adapters = test_adapters();
    assert!(
        adapters
            .budget_checker(everruns_core::DEFAULT_ORG_ID, None)
            .is_none()
    );
}

#[tokio::test]
async fn budget_checker_returns_no_budgets_without_rows() {
    let adapters = test_adapters_with_budget_service();
    let checker = adapters
        .budget_checker(everruns_core::DEFAULT_ORG_ID, None)
        .expect("budget checker should be wired");

    let response = checker
        .check_budgets("session_test_budget")
        .await
        .expect("budget check should succeed");

    assert_eq!(response.status, "no_budgets");
    assert!(response.budgets.is_empty());
    assert!(response.hint.is_some());
}

/// Seed an MCP server with an optional encrypted API key. Returns server UUID.
async fn seed_mcp_server(
    db: &crate::storage::StorageBackend,
    name: &str,
    api_key_encrypted: Option<Vec<u8>>,
) -> Uuid {
    use crate::storage::models::CreateMcpServerRow;
    let row = db
        .create_mcp_server(
            everruns_core::DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: name.to_string(),
                description: None,
                url: "https://example.com/mcp".to_string(),
                transport_type: "streamable_http".to_string(),
                api_key_encrypted,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("seed mcp server");
    row.id.uuid()
}

// No-api-key and decrypted-api-key lookups are covered by the
// `direct_adapter_contract` macro suite's `mcp_server_no_api_key` and
// `mcp_server_with_encrypted_api_key` (same production path and
// assertions), so they are not duplicated here.

#[tokio::test]
async fn get_mcp_server_by_prefix_errors_when_encryption_not_configured() {
    // Adapters without encryption
    let adapters = test_adapters();
    let encrypted = test_encryption().encrypt_string("sk-secret").unwrap();
    seed_mcp_server(&adapters.db, "No Enc Server", Some(encrypted)).await;

    let result = adapters
        .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "no_enc_server")
        .await;
    assert!(result.is_err());
}

// Server-not-found lookup is covered by the `direct_adapter_contract`
// macro suite's `mcp_server_not_found_is_error` (same production path and
// assertion), so it is not duplicated here.

// =========================================================================
// Adapter contract test harness (EVE-61)
//
// Reusable macro-driven suite that defines behavioral contracts for
// WorkerAdapters. Each test is parameterised by an adapter constructor,
// so it runs against DirectWorkerAdapters today and can be wired to
// GrpcWorkerAdapters once an in-process gRPC loopback is available.
// =========================================================================

macro_rules! adapter_contract_tests {
    ($mod_name:ident, $make_adapters:expr) => {
        mod $mod_name {
            use super::*;

            #[tokio::test]
            async fn grep_single_match() {
                let (adapters, db) = $make_adapters;
                let sid = Uuid::new_v4();
                seed_file(&db, sid, "/hello.rs", "fn main() {\n    hello();\n}\n").await;
                let results = adapters
                    .grep_files(everruns_core::DEFAULT_ORG_ID, sid, "hello", None)
                    .await
                    .unwrap();
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].path, "/hello.rs");
                assert_eq!(results[0].line_number, 2);
                assert!(results[0].line.contains("hello"));
            }

            #[tokio::test]
            async fn grep_no_match_returns_empty() {
                let (adapters, db) = $make_adapters;
                let sid = Uuid::new_v4();
                seed_file(&db, sid, "/code.rs", "let x = 1;\n").await;
                let results = adapters
                    .grep_files(everruns_core::DEFAULT_ORG_ID, sid, "no_such_pattern", None)
                    .await
                    .unwrap();
                assert!(results.is_empty());
            }

            #[tokio::test]
            async fn grep_multiple_files_and_lines() {
                let (adapters, db) = $make_adapters;
                let sid = Uuid::new_v4();
                seed_file(&db, sid, "/a.txt", "ERR line1\nok\nERR line3\n").await;
                seed_file(&db, sid, "/b.txt", "ok\nERR line2\n").await;
                let results = adapters
                    .grep_files(everruns_core::DEFAULT_ORG_ID, sid, "ERR", None)
                    .await
                    .unwrap();
                assert_eq!(results.len(), 3);
                let a: Vec<_> = results.iter().filter(|m| m.path == "/a.txt").collect();
                assert_eq!(a.len(), 2);
                assert_eq!(a[0].line_number, 1);
                assert_eq!(a[1].line_number, 3);
                let b: Vec<_> = results.iter().filter(|m| m.path == "/b.txt").collect();
                assert_eq!(b.len(), 1);
                assert_eq!(b[0].line_number, 2);
            }

            #[tokio::test]
            async fn grep_regex_pattern() {
                let (adapters, db) = $make_adapters;
                let sid = Uuid::new_v4();
                seed_file(&db, sid, "/nums.txt", "val 1\nval 22\nval 333\n").await;
                let results = adapters
                    .grep_files(everruns_core::DEFAULT_ORG_ID, sid, r"\d{2,}", None)
                    .await
                    .unwrap();
                assert_eq!(results.len(), 2);
            }

            #[tokio::test]
            async fn grep_invalid_regex_is_error() {
                let (adapters, _db) = $make_adapters;
                let sid = Uuid::new_v4();
                assert!(
                    adapters
                        .grep_files(everruns_core::DEFAULT_ORG_ID, sid, "[bad", None)
                        .await
                        .is_err()
                );
            }

            #[tokio::test]
            async fn grep_empty_session_returns_empty() {
                let (adapters, _db) = $make_adapters;
                let sid = Uuid::new_v4();
                let results = adapters
                    .grep_files(everruns_core::DEFAULT_ORG_ID, sid, "anything", None)
                    .await
                    .unwrap();
                assert!(results.is_empty());
            }

            #[tokio::test]
            async fn write_then_read_file() {
                let (adapters, _db) = $make_adapters;
                let sid = Uuid::new_v4();
                let written = adapters
                    .write_file(
                        everruns_core::DEFAULT_ORG_ID,
                        sid,
                        "/test.txt",
                        "content",
                        "text",
                    )
                    .await
                    .unwrap();
                assert_eq!(written.path, "/test.txt");
                let read = adapters
                    .read_file(everruns_core::DEFAULT_ORG_ID, sid, "/test.txt")
                    .await
                    .unwrap();
                assert!(read.is_some());
                assert_eq!(read.unwrap().path, "/test.txt");
            }

            #[tokio::test]
            async fn read_nonexistent_file_returns_none() {
                let (adapters, _db) = $make_adapters;
                let sid = Uuid::new_v4();
                assert!(
                    adapters
                        .read_file(everruns_core::DEFAULT_ORG_ID, sid, "/nope.txt")
                        .await
                        .unwrap()
                        .is_none()
                );
            }

            #[tokio::test]
            async fn delete_file_returns_true() {
                let (adapters, _db) = $make_adapters;
                let sid = Uuid::new_v4();
                adapters
                    .write_file(
                        everruns_core::DEFAULT_ORG_ID,
                        sid,
                        "/del.txt",
                        "bye",
                        "text",
                    )
                    .await
                    .unwrap();
                assert!(
                    adapters
                        .delete_file(everruns_core::DEFAULT_ORG_ID, sid, "/del.txt", false)
                        .await
                        .unwrap()
                );
                assert!(
                    adapters
                        .read_file(everruns_core::DEFAULT_ORG_ID, sid, "/del.txt")
                        .await
                        .unwrap()
                        .is_none()
                );
            }

            #[tokio::test]
            async fn resolve_image_missing_returns_none() {
                let (adapters, _db) = $make_adapters;
                let result = adapters
                    .resolve_image(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
                    .await
                    .unwrap();
                assert!(result.is_none());
            }

            #[tokio::test]
            async fn resolve_images_batch_empty_ids() {
                let (adapters, _db) = $make_adapters;
                let result = adapters
                    .resolve_images_batch(everruns_core::DEFAULT_ORG_ID, &[])
                    .await
                    .unwrap();
                assert!(result.is_empty());
            }

            #[tokio::test]
            async fn get_session_nonexistent_returns_none() {
                let (adapters, _db) = $make_adapters;
                let result = adapters
                    .get_session(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
                    .await
                    .unwrap();
                assert!(result.is_none());
            }

            #[tokio::test]
            async fn get_agent_nonexistent_returns_none() {
                let (adapters, _db) = $make_adapters;
                let result = adapters
                    .get_agent(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
                    .await
                    .unwrap();
                assert!(result.is_none());
            }

            #[tokio::test]
            async fn mcp_server_not_found_is_error() {
                let (adapters, _db) = $make_adapters;
                assert!(
                    adapters
                        .get_mcp_server_by_prefix(
                            everruns_core::DEFAULT_ORG_ID,
                            None,
                            "no_such_prefix"
                        )
                        .await
                        .is_err()
                );
            }

            #[tokio::test]
            async fn mcp_server_no_api_key() {
                let (adapters, db) = $make_adapters;
                seed_mcp_server(&db, "Plain Server", None).await;
                let info = adapters
                    .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "plain_server")
                    .await
                    .unwrap();
                assert!(info.api_key.is_none());
                assert_eq!(info.name, "Plain Server");
            }

            #[tokio::test]
            async fn mcp_server_with_encrypted_api_key() {
                let adapters = test_adapters_with_encryption();
                let encrypted = test_encryption().encrypt_string("sk-contract").unwrap();
                seed_mcp_server(&adapters.db, "Enc Server", Some(encrypted)).await;
                let info = adapters
                    .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "enc_server")
                    .await
                    .unwrap();
                assert_eq!(info.api_key.as_deref(), Some("sk-contract"));
            }
        }
    };
}

adapter_contract_tests!(direct_adapter_contract, {
    let a = test_adapters();
    let db = a.db.clone();
    (a, db)
});

// =========================================================================
// Cross-org image resolution regression (EVE-56 / EVE-61)
// =========================================================================

async fn seed_image(db: &StorageBackend, org_id: i64) -> Uuid {
    use crate::storage::models::CreateImageRow;
    let row = db
        .create_image(
            org_id,
            CreateImageRow {
                org_id,
                filename: "test.png".to_string(),
                content_type: "image/png".to_string(),
                size_bytes: 4,
                data: vec![0x89, 0x50, 0x4E, 0x47],
                thumbnail_data: None,
                thumbnail_content_type: None,
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("seed image");
    row.id.into()
}

#[tokio::test]
async fn resolve_image_cross_org_isolation() {
    let adapters = test_adapters();
    let image_id = seed_image(&adapters.db, everruns_core::DEFAULT_ORG_ID).await;
    let found = adapters
        .resolve_image(everruns_core::DEFAULT_ORG_ID, image_id)
        .await
        .unwrap();
    assert!(found.is_some(), "image should be visible in owning org");
    let cross = adapters.resolve_image(999, image_id).await.unwrap();
    assert!(
        cross.is_none(),
        "image must NOT be visible in different org"
    );
}

#[tokio::test]
async fn resolve_images_batch_cross_org_isolation() {
    let adapters = test_adapters();
    let img1 = seed_image(&adapters.db, everruns_core::DEFAULT_ORG_ID).await;
    let img2 = seed_image(&adapters.db, everruns_core::DEFAULT_ORG_ID).await;
    let batch = adapters
        .resolve_images_batch(everruns_core::DEFAULT_ORG_ID, &[img1, img2])
        .await
        .unwrap();
    assert_eq!(batch.len(), 2);
    let cross = adapters
        .resolve_images_batch(999, &[img1, img2])
        .await
        .unwrap();
    assert!(cross.is_empty(), "images must not leak across orgs");
}

// =========================================================================
// MCP auth-required execution coverage (EVE-55 / EVE-61)
// =========================================================================

// Decrypted-key retrieval is covered by the `direct_adapter_contract`
// macro suite's `mcp_server_with_encrypted_api_key`, and the
// no-encryption-service failure by
// `get_mcp_server_by_prefix_errors_when_encryption_not_configured` above
// (same production path and assertions in both cases), so neither is
// duplicated here.

#[tokio::test]
async fn mcp_server_wrong_org_not_found() {
    let adapters = test_adapters();
    seed_mcp_server(&adapters.db, "Org Scoped MCP", None).await;
    assert!(
        adapters
            .get_mcp_server_by_prefix(everruns_core::DEFAULT_ORG_ID, None, "org_scoped_mcp")
            .await
            .is_ok()
    );
    assert!(
        adapters
            .get_mcp_server_by_prefix(999, None, "org_scoped_mcp")
            .await
            .is_err(),
        "MCP server must not be visible in wrong org"
    );
}

// =========================================================================
// Cross-org isolation regression tests (EVE-59)
// =========================================================================

/// Regression: get_session must populate organization_id from org_id,
/// not hardcode DEFAULT_ORG_PUBLIC_ID.
#[tokio::test]
async fn get_session_carries_org_public_id() {
    use crate::storage::models::CreateSessionRow;

    let adapters = test_adapters();
    let agent_id = seed_agent(&adapters.db).await;

    let row = adapters
        .db
        .create_session(CreateSessionRow {
            playground_user_id: None,
            trigger_id: None,
            source: crate::records::SessionSource::Api,
            workspace_id: None,
            org_id: everruns_core::DEFAULT_ORG_ID,
            app_id: None,
            channel_id: None,
            agent_id: Some(AgentId::from_uuid(agent_id)),
            agent_version_id: None,
            agent_config_hash: None,
            virtual_user_id: None,
            harness_id: Some(HarnessId::from_seed(1)),
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::Value::Array(vec![]),
            tools: serde_json::Value::Array(vec![]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
            hints: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            network_access: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .expect("create session");

    let session = adapters
        .get_session(everruns_core::DEFAULT_ORG_ID, row.id.uuid())
        .await
        .unwrap()
        .expect("session should exist");

    assert_eq!(
        session.organization_id,
        everruns_core::DEFAULT_ORG_PUBLIC_ID,
        "session must carry the correct org public_id"
    );
}

// =========================================================================
// Encrypted DB key model sync regression (EVE-57 / EVE-61)
// =========================================================================

#[tokio::test]
async fn get_model_spec_returns_none_for_missing() {
    let adapters = test_adapters();
    let result = adapters
        .get_model_spec(everruns_core::DEFAULT_ORG_ID, Uuid::new_v4())
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn get_default_model_spec_returns_none_when_no_providers() {
    let adapters = test_adapters();
    let result = adapters
        .get_default_model_spec(everruns_core::DEFAULT_ORG_ID)
        .await
        .unwrap();
    assert!(result.is_none());
}

// =========================================================================
// Usage attribution org scoping (EVE-56 / EVE-61)
// =========================================================================

#[tokio::test]
async fn platform_store_agent_count_isolated_per_org() {
    use crate::storage::models::CreateOrganizationRow;

    let adapters = test_adapters();
    seed_agent(&adapters.db).await;
    let org2 = adapters
        .db
        .create_organization(CreateOrganizationRow {
            public_id: "org_00000000000000000000000000000998".to_string(),
            name: "Platform Store Agent Count Org 2".to_string(),
            created_by: None,
        })
        .await
        .expect("create org 2");
    let harness_org1 = seed_harness_for_platform_store(
        &adapters.db,
        everruns_core::DEFAULT_ORG_ID,
        "agent-count-org1",
        false,
    )
    .await;
    let harness_org2 =
        seed_harness_for_platform_store(&adapters.db, org2.org_id, "agent-count-org2", false).await;
    let owner_org1 = seed_platform_owner(
        &adapters.db,
        everruns_core::DEFAULT_ORG_ID,
        "agent-count-org1-owner@example.com",
    )
    .await;
    let owner_org2 = seed_platform_owner(
        &adapters.db,
        org2.org_id,
        "agent-count-org2-owner@example.com",
    )
    .await;
    let session_org1 = seed_platform_session(
        &adapters.db,
        everruns_core::DEFAULT_ORG_ID,
        harness_org1,
        Some(owner_org1),
    )
    .await;
    let session_org2 =
        seed_platform_session(&adapters.db, org2.org_id, harness_org2, Some(owner_org2)).await;

    // `list_agents` moved off PlatformStore with the legacy CRUD (EVE-953);
    // the org boundary it guarded is now enforced on the command surface,
    // so assert it there instead of dropping the coverage.
    let store_org1 = adapters
        .platform_store(everruns_core::DEFAULT_ORG_ID, session_org1)
        .for_execution(session_org1.uuid())
        .unwrap();
    let agents_org1 = store_org1
        .platform_query(serde_json::json!({ "commands": "list_agents" }))
        .await
        .expect("default org may list its agents");
    assert!(
        agents_org1.contains("test-agent"),
        "default org should see its seeded agent: {agents_org1}"
    );

    let store_org2 = adapters
        .platform_store(org2.org_id, session_org2)
        .for_execution(session_org2.uuid())
        .unwrap();
    let agents_org2 = store_org2
        .platform_query(serde_json::json!({ "commands": "list_agents" }))
        .await
        .expect("second org may list its agents");
    assert!(
        !agents_org2.contains("test-agent"),
        "second org must not see the default org's agent: {agents_org2}"
    );
}
