//! Tests: mounts_tests.

use super::*;
use crate::domains::common::Command;
use crate::domains::memory::CreateMemory;
use crate::domains::memory::types::{CreateMemorySourceRequest, GitMemorySourceRequest};
use crate::domains::{agents::types::CreateAgentRequest, harnesses::types::CreateHarnessRequest};
use crate::kernel_imports::{Caller, DEFAULT_ORG_ID, OrgRole};
use crate::storage::{CreateHarnessRow, CreateMemoryFileRow, StorageBackend};

use super::tests_support::*;

#[tokio::test]
async fn create_rejects_harness_from_another_org() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let other_org_id = create_second_org(&db).await;
    let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone()).await;

    let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "other-harness".to_string(),
        display_name: Some("Other Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Other".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&other_ctx)
    .await
    .unwrap();

    let err = session_service
        .create(
            &caller,
            other_harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(other_harness.id, None, None),
        )
        .await
        .unwrap_err();

    let not_found = err.downcast_ref::<ResourceNotFoundError>().unwrap();
    assert_eq!(not_found.resource(), "Harness");
}

#[tokio::test]
async fn create_rejects_model_from_another_org() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;
    let other_org_id = create_second_org(&db).await;
    let other_model_id = create_model(&db, other_org_id, "cross-org-model").await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "harness".to_string(),
        display_name: Some("Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let err = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, Some(other_model_id)),
        )
        .await
        .unwrap_err();

    let not_found = err.downcast_ref::<ResourceNotFoundError>().unwrap();
    assert_eq!(not_found.resource(), "Model");
}

#[tokio::test]
async fn get_skips_foreign_harness_and_agent_capability_features() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let other_org_id = create_second_org(&db).await;
    let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone()).await;

    let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "other-harness".to_string(),
        display_name: Some("Other Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Other".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::new("session_file_system")],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&other_ctx)
    .await
    .unwrap();

    let other_agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        id: None,
        name: "other-agent".to_string(),
        display_name: Some("Other Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "Other".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::new("session_schedule")],
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    })
    .execute(&other_ctx)
    .await
    .unwrap();

    let session_row = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: caller.org_id,
            app_id: None,
            endpoint_id: None,
            harness_id: Some(other_harness.id),
            agent_id: Some(AgentId::from_uuid(other_agent.internal_id)),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("Corrupt Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::to_value(vec![AgentCapabilityConfig::new(
                "session_sql_database",
            )])
            .unwrap(),
            tools: serde_json::json!([]),
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
        .unwrap();

    let session = session_service
        .get(&caller, session_row.id.uuid(), None)
        .await
        .unwrap()
        .unwrap();

    assert!(
        session.features.contains(&"sql_database".to_string()),
        "session capability should still apply: {:?}",
        session.features
    );
    assert!(
        !session.features.contains(&"file_system".to_string()),
        "foreign harness capability should not contribute features: {:?}",
        session.features
    );
    assert!(
        !session.features.contains(&"schedules".to_string()),
        "foreign agent capability should not contribute features: {:?}",
        session.features
    );
}

// EVE-709: a harness declaring a built-in capability that is not registered in
// this deployment (e.g. feature-gated `container_sandbox`) must fail session
// creation with a clear error rather than silently dropping the capability's
// tools and degrading into a different execution environment.
#[tokio::test]
async fn create_rejects_harness_with_unavailable_builtin_capability() {
    let db = Arc::new(StorageBackend::in_memory());
    // Empty registry stands in for a deployment where `container_sandbox` is
    // feature-gated off, so it is absent from the capability registry.
    let registry = CapabilityRegistry::new();
    let session_service = SessionService::with_registry(db.clone(), registry);
    let owner = Caller::internal(DEFAULT_ORG_ID);

    let harness = db
        .create_harness(
            owner.org_id,
            CreateHarnessRow {
                name: "coding-container".to_string(),
                display_name: Some("Coding (Container)".to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some("coding".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    db.set_harness_capabilities(
        harness.id.uuid(),
        vec![("container_sandbox".to_string(), 0, serde_json::json!({}))],
    )
    .await
    .unwrap();

    // Owner (admin) so the high-risk capability gate does not fire first.
    let err = session_service
        .create(
            &owner,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("capabilities unavailable in this deployment")
            && err.to_string().contains("container_sandbox"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn create_rejects_high_risk_harness_capabilities_for_members() {
    let db = Arc::new(StorageBackend::in_memory());
    let mut registry = CapabilityRegistry::new();
    registry.register(TestHighRiskCapability);
    let session_service = SessionService::with_registry(db.clone(), registry);
    let owner = Caller::internal(DEFAULT_ORG_ID);

    let harness = db
        .create_harness(
            owner.org_id,
            CreateHarnessRow {
                name: "restricted-harness".to_string(),
                display_name: Some("Restricted Harness".to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some("restricted".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    db.set_harness_capabilities(
        harness.id.uuid(),
        vec![("test_high_risk".to_string(), 0, serde_json::json!({}))],
    )
    .await
    .unwrap();

    let member = Caller {
        org_id: owner.org_id,
        org_public_id: owner.org_public_id.clone(),
        user_id: None,
        role: OrgRole::Member,
        is_platform_user: false,
        is_internal: false,
    };

    let err = session_service
        .create(
            &member,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("Admin role required to create sessions with high-risk capabilities"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn create_rejects_declarative_capability_with_high_risk_dependency_for_members() {
    use crate::storage::models::CreateDeclarativeCapabilityRow;

    let db = Arc::new(StorageBackend::in_memory());
    let mut registry = CapabilityRegistry::new();
    registry.register(TestHighRiskCapability);
    let session_service = SessionService::with_registry(db.clone(), registry);
    let owner = Caller::internal(DEFAULT_ORG_ID);

    // Declarative capability that hides a high-risk built-in dependency.
    db.create_declarative_capability(
        owner.org_id,
        CreateDeclarativeCapabilityRow {
            public_id: everruns_provider::typed_id::DeclarativeCapabilityId::new().to_string(),
            name: "hidden_admin_tool".to_string(),
            display_name: Some("Hidden Admin Tool".to_string()),
            description: "wraps a high-risk built-in".to_string(),
            definition: serde_json::json!({
                "name": "hidden_admin_tool",
                "description": "wraps a high-risk built-in",
                "dependencies": ["test_high_risk"],
            }),
        },
    )
    .await
    .unwrap();

    let harness = db
        .create_harness(
            owner.org_id,
            CreateHarnessRow {
                name: "declarative-harness".to_string(),
                display_name: Some("Declarative Harness".to_string()),
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some("declarative".to_string()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    db.set_harness_capabilities(
        harness.id.uuid(),
        vec![(
            "declarative:hidden_admin_tool".to_string(),
            0,
            serde_json::json!({}),
        )],
    )
    .await
    .unwrap();

    let member = Caller {
        org_id: owner.org_id,
        org_public_id: owner.org_public_id.clone(),
        user_id: None,
        role: OrgRole::Member,
        is_platform_user: false,
        is_internal: false,
    };

    let err = session_service
        .create(
            &member,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("Admin role required to create sessions with high-risk capabilities"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn apply_capability_mounts_skips_foreign_harness_and_agent_capabilities() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let file_service = WorkspaceFileService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let other_org_id = create_second_org(&db).await;
    let other_ctx = test_ctx(Caller::internal(other_org_id), db.clone()).await;

    let other_harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "other-harness".to_string(),
        display_name: Some("Other Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Other".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::new("data_knowledge")],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&other_ctx)
    .await
    .unwrap();

    let other_agent = crate::domains::agents::CreateAgent(CreateAgentRequest {
        id: None,
        name: "other-agent".to_string(),
        display_name: Some("Other Agent".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: "Other".to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::new("data_knowledge")],
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    })
    .execute(&other_ctx)
    .await
    .unwrap();

    let session_row = db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: caller.org_id,
            app_id: None,
            endpoint_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("Mount Test".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
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
        .unwrap();

    session_service
        .apply_capability_mounts(
            caller.org_id,
            other_harness.id.uuid(),
            Some(other_agent.internal_id),
            &[],
            session_row.id.uuid(),
            None,
        )
        .await
        .unwrap();

    assert!(
        file_service
            .read_file(session_row.id.uuid(), "/knowledge/index.md")
            .await
            .unwrap()
            .is_none(),
        "foreign harness/agent capabilities should not mount files"
    );
}

#[tokio::test]
async fn workspace_memory_mount_materializes_source_memory_readonly() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let file_service = WorkspaceFileService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let memory = CreateMemory {
        name: "Repo Memory".to_string(),
        description: None,
        source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
            url: "https://example.com/org/repo.git".to_string(),
            branch: None,
            root_folder: None,
            sync_interval_secs: None,
        })),
    }
    .execute(&ctx)
    .await
    .unwrap();
    let claimed = db
        .claim_next_memory_sync()
        .await
        .unwrap()
        .expect("memory should be pending sync");
    db.complete_memory_sync(
        claimed.id,
        claimed.updated_at,
        vec![CreateMemoryFileRow {
            path: "/README.md".to_string(),
            content: Some(b"hello from memory".to_vec()),
            is_directory: false,
            content_hash: None,
        }],
    )
    .await
    .unwrap();

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "memory-harness".to_string(),
        display_name: Some("Memory Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::with_config(
            MEMORY_CAPABILITY_ID,
            serde_json::json!({
                "mounts": [{
                    "memory": memory.id.to_string(),
                    "path": "/workspace/repo",
                    "mode": "readonly"
                }]
            }),
        )],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let session = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();
    let file = file_service
        .read_file(session.id.uuid(), "/workspace/repo/README.md")
        .await
        .unwrap()
        .expect("mounted file should exist");

    let content = SessionFile::decode_content(
        file.content.as_deref().expect("mounted file has content"),
        &file.encoding,
    )
    .unwrap();
    assert_eq!(content, b"hello from memory");
    assert!(file.is_readonly);
}

#[tokio::test]
async fn workspace_memory_mount_rejects_readwrite_source_volume() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let memory = CreateMemory {
        name: "Read-only Repo".to_string(),
        description: None,
        source: Some(CreateMemorySourceRequest::Git(GitMemorySourceRequest {
            url: "https://example.com/org/repo.git".to_string(),
            branch: None,
            root_folder: None,
            sync_interval_secs: None,
        })),
    }
    .execute(&ctx)
    .await
    .unwrap();
    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "readwrite-memory-harness".to_string(),
        display_name: Some("Readwrite Memory Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![AgentCapabilityConfig::with_config(
            MEMORY_CAPABILITY_ID,
            serde_json::json!({
                "mounts": [{
                    "memory": memory.id.to_string(),
                    "path": "/workspace/repo",
                    "mode": "readwrite"
                }]
            }),
        )],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let err = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("is read-only and cannot be mounted readwrite"),
        "unexpected error: {err}",
    );
}

#[tokio::test]
async fn update_rejects_reserved_routing_tags_for_external_callers() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "harness".to_string(),
        display_name: Some("Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let mut create_req = build_create_request(harness.id, None, None);
    create_req.tags = vec!["baseline".to_string()];
    let session = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            create_req,
        )
        .await
        .unwrap();

    for forbidden in [
        vec!["__internal:app_invocation".to_string()],
        vec!["app:app_other".to_string()],
        vec!["app_channel:appchan_other".to_string()],
        vec!["slack:app:app_legacy_other".to_string()],
        vec!["slack:endpoint:appchan_other".to_string()],
        vec!["fcp:endpoint:appchan_other".to_string()],
        vec!["ag_ui:app:app_ag_ui_other".to_string()],
        vec!["agent:agent_other".to_string()],
        vec!["endpoint:endpoint_other".to_string()],
    ] {
        let err = session_service
            .update(
                &external_caller(DEFAULT_ORG_ID),
                session.id.uuid(),
                UpdateSessionRequest {
                    title: None,
                    goal: None,
                    agent_identity_id: UpdateField::Unchanged,
                    locale: None,
                    tags: Some(forbidden.clone()),
                },
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("reserved for internal subsystems"),
            "got: {err} for tags: {forbidden:?}"
        );
        let reloaded = session_service
            .get(&external_caller(DEFAULT_ORG_ID), session.id.uuid(), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.tags, ["baseline"]);
    }
}

#[tokio::test]
async fn create_rejects_reserved_routing_tags_for_external_callers() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "harness".to_string(),
        display_name: Some("Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    for forbidden in [
        "__internal:app_invocation",
        "app:app_someone_else",
        "app_channel:appchan_someone_else",
        "slack:app:app_legacy_someone_else",
        "slack:endpoint:appchan_someone_else",
        "fcp:endpoint:appchan_someone_else",
        "ag_ui:app:app_ag_ui_someone_else",
        "agent:agent_someone_else",
        "endpoint:endpoint_someone_else",
    ] {
        let mut req = build_create_request(harness.id, None, None);
        req.tags = vec![forbidden.to_string()];

        let err = session_service
            .create(
                &external_caller(DEFAULT_ORG_ID),
                harness.id.uuid(),
                None,
                None,
                SessionSource::Api,
                req,
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("reserved for internal subsystems"),
            "got: {err} for tag: {forbidden}"
        );
    }
}

#[tokio::test]
async fn internal_callers_can_set_reserved_routing_tags() {
    let db = Arc::new(StorageBackend::in_memory());
    let session_service = SessionService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "harness".to_string(),
        display_name: Some("Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("Harness prompt".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let mut req = build_create_request(harness.id, None, None);
    req.tags = RESERVED_SESSION_TAG_PREFIXES
        .iter()
        .map(|prefix| format!("{prefix}owned"))
        .collect();
    let session = session_service
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            req,
        )
        .await
        .unwrap();

    let updated_tags = RESERVED_SESSION_TAG_PREFIXES
        .iter()
        .map(|prefix| format!("{prefix}updated"))
        .collect::<Vec<_>>();
    let updated = session_service
        .update(
            &caller,
            session.id.uuid(),
            UpdateSessionRequest {
                title: None,
                goal: None,
                agent_identity_id: UpdateField::Unchanged,
                locale: None,
                tags: Some(updated_tags.clone()),
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.tags, updated_tags);
}

#[tokio::test]
async fn app_session_creation_enforces_total_session_cap() {
    let db = Arc::new(StorageBackend::in_memory());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "app-session-cap-harness".to_string(),
        display_name: Some("App Session Cap Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("test".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let resource_limits = ResourceLimitsConfig {
        max_sessions_per_org: 1,
        ..Default::default()
    };
    let svc = SessionService::new(db.clone()).with_resource_limits(resource_limits);

    let owner_principal = svc
        .principal_service
        .default_owner_principal(&caller, None)
        .await
        .unwrap();
    let app_id = Uuid::new_v4();

    svc.create_from_app(
        &caller,
        harness.id.uuid(),
        None,
        None,
        app_id,
        Some(Uuid::new_v4()),
        owner_principal.id,
        owner_principal.resolved_user_id,
        SessionSource::Api,
        build_create_request(harness.id, None, None),
    )
    .await
    .unwrap();

    let err = svc
        .create_from_app(
            &caller,
            harness.id.uuid(),
            None,
            None,
            app_id,
            Some(Uuid::new_v4()),
            owner_principal.id,
            owner_principal.resolved_user_id,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap_err();

    assert!(
        err.downcast_ref::<ResourceLimitError>().is_some(),
        "expected ResourceLimitError, got: {err}"
    );
    assert!(
        err.to_string().contains("Session limit reached"),
        "got: {err}"
    );
}

#[tokio::test]
async fn concurrent_session_cap_enforced() {
    use crate::domains::sessions::limits::OrgCaps;
    use crate::errors::BadRequestError;
    use crate::storage::models::UpdateSession;

    let db = Arc::new(StorageBackend::in_memory());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let ctx = test_ctx(caller.clone(), db.clone()).await;

    let harness = crate::domains::harnesses::CreateHarness(CreateHarnessRequest {
        name: "cap-test-harness".to_string(),
        display_name: Some("Cap Test Harness".to_string()),
        description: None,
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: Some("test".to_string()),
        parent_harness_id: None,
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    })
    .execute(&ctx)
    .await
    .unwrap();

    let svc = SessionService::new(db.clone()).with_caps(OrgCaps {
        max_concurrent_sessions: 1,
        max_active_turns: 1_000,
    });

    // First session succeeds.
    let session = svc
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap();

    // Ensure the session is in an active status (created as 'started' in in-memory backend).
    let _ = db
        .update_session(
            DEFAULT_ORG_ID,
            session.id,
            UpdateSession {
                status: Some("started".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Second session is rejected because cap = 1 is already reached.
    let err = svc
        .create(
            &caller,
            harness.id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness.id, None, None),
        )
        .await
        .unwrap_err();

    assert!(
        err.downcast_ref::<BadRequestError>().is_some(),
        "expected BadRequestError, got: {err}"
    );
    assert!(
        err.to_string().contains("Too many concurrent sessions"),
        "got: {err}"
    );
}

#[tokio::test]
async fn session_start_hook_fires_on_create() {
    let db = Arc::new(StorageBackend::in_memory());
    let mut registry = CapabilityRegistry::new();
    registry.register(everruns_platform::capabilities::UserHooksCapability);
    let session_service = SessionService::with_registry(db.clone(), registry);
    let caller = Caller::internal(DEFAULT_ORG_ID);

    let harness_id = harness_with_user_hook(
        &db,
        caller.org_id,
        "session-start-harness",
        "session_start",
        "echo started > /workspace/.session_start_ok",
    )
    .await;

    let session = session_service
        .create(
            &caller,
            harness_id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness_id, None, None),
        )
        .await
        .unwrap();

    // The session_start hook ran during create and wrote into the VFS.
    let file = WorkspaceFileService::new(db)
        .read_file(session.id.uuid(), "/.session_start_ok")
        .await
        .unwrap();
    assert!(
        file.as_ref()
            .is_some_and(|f| f.content.as_deref().is_some_and(|c| c.contains("started"))),
        "session_start hook should have written the sentinel, got: {file:?}"
    );
}

#[tokio::test]
async fn session_end_hook_fires_on_delete_without_blocking() {
    let db = Arc::new(StorageBackend::in_memory());
    let mut registry = CapabilityRegistry::new();
    registry.register(everruns_platform::capabilities::UserHooksCapability);
    let session_service = SessionService::with_registry(db.clone(), registry);
    let caller = Caller::internal(DEFAULT_ORG_ID);

    let harness_id = harness_with_user_hook(
        &db,
        caller.org_id,
        "session-end-harness",
        "session_end",
        "echo ending > /workspace/.session_end_ok",
    )
    .await;

    let session = session_service
        .create(
            &caller,
            harness_id.uuid(),
            None,
            None,
            SessionSource::Api,
            build_create_request(harness_id, None, None),
        )
        .await
        .unwrap();

    // session_end is advisory: the hook fires (resolve -> dispatch) but never
    // blocks the delete, which must still succeed.
    let deleted = session_service
        .delete(&caller, session.id.uuid())
        .await
        .unwrap();
    assert!(
        deleted,
        "delete should succeed with a session_end hook present"
    );
}
