//! Repository-level integration tests for Everruns storage layer
//!
//! These tests focus on SQL query correctness and type mappings,
//! testing the storage layer directly with PostgreSQL.
//!
//! Run with: cargo test -p everruns-server --test repository_integration_test -- --test-threads=1
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set
//! - Migrations applied (run migrations from crates/server/migrations/)

mod test_harness;

use chrono::Utc;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use everruns_core::message_filter::MessageQuery;
use everruns_durable::UpdateField;
use everruns_server::api::common::Pagination;
use everruns_server::org_init;
use everruns_server::storage::{
    CreateAgentCapabilityRow, CreateAgentHealthCheckRunRow, CreateAgentRow, CreateAppRow,
    CreateDeclarativeCapabilityRow, CreateEventRow, CreateHarnessRow, CreateImageRow,
    CreateMcpServerRow, CreateModelRow, CreateOrganizationRow, CreatePrincipalRow,
    CreateProviderRow, CreateSessionFileRow, CreateSessionRow, CreateSessionScheduleRow,
    CreateUserConnectionRow, CreateUserRow, Database, SessionListFilters, StorageBackend,
    UpdateAgent, UpdateAgentHealthCheckRunRow, UpdateDeclarativeCapability, UpdateModel,
    UpdateOrganization, UpdateOrganizationSettings, UpdateProvider, UpdateSession,
    UpdateSessionFile, UpdateSessionScheduleRow,
};
use test_harness::get_database_url;

/// Create a test pool
async fn create_test_pool() -> PgPool {
    let database_url = get_database_url();
    PgPool::connect(&database_url)
        .await
        .expect("Failed to connect to PostgreSQL")
}

/// Create a test storage backend
async fn create_test_backend() -> StorageBackend {
    let pool = create_test_pool().await;
    StorageBackend::Postgres(Database::new(pool))
}

/// Test organization ID (default org)
const TEST_ORG_ID: i64 = 1;

async fn ensure_test_harness_id(backend: &StorageBackend) -> everruns_core::HarnessId {
    org_init::initialize_org_harnesses(backend, TEST_ORG_ID)
        .await
        .expect("initialize built-in harnesses");
    org_init::generic_harness_id(backend, TEST_ORG_ID)
        .await
        .expect("generic harness id")
}

async fn create_test_principal(
    backend: &StorageBackend,
    org_id: i64,
) -> everruns_core::PrincipalId {
    backend
        .create_principal(CreatePrincipalRow {
            id: everruns_core::PrincipalId::new(),
            org_id,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "repository_integration_test" }),
        })
        .await
        .expect("Failed to create test principal")
        .id
}

async fn create_test_user(
    backend: &StorageBackend,
    label: &str,
) -> everruns_server::storage::UserRow {
    backend
        .create_user(CreateUserRow {
            email: format!("{label}-{}@example.com", Uuid::now_v7()),
            name: format!("Test {label}"),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("Failed to create test user")
}

async fn create_test_user_principal(
    backend: &StorageBackend,
    org_id: i64,
    user_id: Uuid,
) -> everruns_core::PrincipalId {
    backend
        .create_principal(CreatePrincipalRow {
            id: everruns_core::PrincipalId::new(),
            org_id,
            kind: "user".to_string(),
            subject_id: Some(user_id),
            parent_principal_id: None,
            resolved_user_id: Some(user_id),
            metadata: json!({ "source": "repository_integration_test", "kind": "user" }),
        })
        .await
        .expect("Failed to create user principal")
        .id
}

#[tokio::test]
async fn test_oauth_identity_linking_preserves_existing_provider() {
    let backend = create_test_backend().await;
    let github_id = format!("github-{}", Uuid::now_v7());
    let google_id = format!("google-{}", Uuid::now_v7());
    let user = backend
        .create_user(CreateUserRow {
            email: format!("oauth-link-{}@example.com", Uuid::now_v7()),
            name: "OAuth Link".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: Some("github".to_string()),
            auth_provider_id: Some(github_id.clone()),
            external_id: None,
        })
        .await
        .expect("create OAuth user");

    let linked = backend
        .link_oauth_identity(user.id, "google", &google_id)
        .await
        .expect("link second provider")
        .expect("linked user");
    assert_eq!(linked.id, user.id);

    for (provider, provider_id) in [
        ("github", github_id.as_str()),
        ("google", google_id.as_str()),
    ] {
        assert_eq!(
            backend
                .get_user_by_oauth(provider, provider_id)
                .await
                .expect("look up OAuth identity")
                .expect("identity resolves")
                .id,
            user.id
        );
    }

    assert_eq!(
        backend
            .link_oauth_identity(user.id, "google", &google_id)
            .await
            .expect("relink same identity")
            .expect("idempotent link")
            .id,
        user.id
    );

    assert!(
        backend
            .link_oauth_identity(
                user.id,
                "google",
                &format!("replacement-{}", Uuid::now_v7())
            )
            .await
            .expect("reject provider identity replacement")
            .is_none()
    );

    let other_user = create_test_user(&backend, "oauth-link-other").await;
    assert!(
        backend
            .link_oauth_identity(other_user.id, "google", &google_id)
            .await
            .expect("reject cross-user identity link")
            .is_none()
    );
}

// ============================================
// Agent Repository Tests
// ============================================

#[tokio::test]
async fn test_agent_crud() {
    let backend = create_test_backend().await;

    // Create agent
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("repo-test-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Repo Test Agent".to_string()),
                description: Some("Test description".to_string()),
                system_prompt: "Test prompt".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    assert!(agent.name.starts_with("repo-test-agent-"));

    // Get agent
    let fetched = backend
        .get_agent(TEST_ORG_ID, agent.id)
        .await
        .expect("Failed to get agent")
        .expect("Agent not found");
    assert_eq!(fetched.id, agent.id);

    // Update agent
    let updated = backend
        .update_agent(
            TEST_ORG_ID,
            agent.id,
            UpdateAgent {
                name: Some("Updated Agent".to_string()),
                description: Some("Updated desc".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("Failed to update agent")
        .expect("Agent not found");
    assert_eq!(updated.name, "Updated Agent");

    // List agents
    let pagination = everruns_server::api::common::Pagination::new(0, 1000);
    let (agents, _total) = backend
        .list_agents(TEST_ORG_ID, None, false, pagination)
        .await
        .expect("Failed to list agents");
    assert!(agents.iter().any(|a| a.id == agent.id));

    // Delete agent (soft-delete: archives the agent)
    let deleted = backend
        .delete_agent(TEST_ORG_ID, agent.id)
        .await
        .expect("Failed to delete agent");
    assert!(deleted);

    // Verify agent is archived (not hard-deleted)
    // Note: Repository returns row types with status as String
    let fetched = backend
        .get_agent(TEST_ORG_ID, agent.id)
        .await
        .expect("Failed to get agent")
        .expect("Agent should still exist after soft-delete");
    assert_eq!(fetched.status, "archived");
}

#[tokio::test]
async fn test_agent_upsert_initial_files() {
    let backend = create_test_backend().await;
    let public_id = everruns_core::AgentId::new().to_string();

    // First upsert — creates agent with initial_files
    let (agent, was_created) = backend
        .upsert_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: public_id.clone(),
                name: format!("upsert-files-{}", &public_id[..12]),
                display_name: Some("Upsert Files Agent".to_string()),
                description: None,
                system_prompt: "prompt".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([
                    {"path": "/AGENTS.md", "content": "old", "encoding": "utf-8", "is_readonly": false}
                ]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("First upsert failed");
    assert!(was_created);
    assert_eq!(agent.initial_files.as_array().unwrap().len(), 1);
    assert_eq!(agent.initial_files[0]["content"], "old");

    // Second upsert — updates initial_files
    let (agent2, was_created2) = backend
        .upsert_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: public_id.clone(),
                name: format!("upsert-files-{}", &public_id[..12]),
                display_name: Some("Upsert Files Agent".to_string()),
                description: None,
                system_prompt: "prompt".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([
                    {"path": "/AGENTS.md", "content": "new", "encoding": "utf-8", "is_readonly": false}
                ]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Second upsert failed");
    assert!(!was_created2);
    assert_eq!(agent2.initial_files.as_array().unwrap().len(), 1);
    assert_eq!(agent2.initial_files[0]["content"], "new");

    // Verify via get
    let fetched = backend
        .get_agent(TEST_ORG_ID, agent2.id)
        .await
        .expect("Failed to get agent")
        .expect("Agent not found");
    assert_eq!(fetched.initial_files[0]["content"], "new");
}

#[tokio::test]
async fn test_declarative_capability_crud_and_search_postgres() {
    let backend = create_test_backend().await;
    let unique = Uuid::now_v7().to_string();
    let name = format!("repo_cap_{}", &unique[..8]);

    let capability = backend
        .create_declarative_capability(
            TEST_ORG_ID,
            CreateDeclarativeCapabilityRow {
                public_id: everruns_core::DeclarativeCapabilityId::new().to_string(),
                name: name.clone(),
                display_name: Some("Repository Capability".to_string()),
                description: "Searchable declarative capability".to_string(),
                definition: json!({
                    "name": name,
                    "display_name": "Repository Capability",
                    "description": "Searchable declarative capability",
                    "system_prompt": "Use the repository workflow.",
                    "skills": [],
                    "files": [],
                    "mcp_servers": {}
                }),
            },
        )
        .await
        .expect("create declarative capability");

    assert!(capability.public_id.starts_with("cap_"));
    assert_eq!(
        capability.display_name.as_deref(),
        Some("Repository Capability")
    );

    let by_name = backend
        .get_declarative_capability_by_name(TEST_ORG_ID, &capability.name)
        .await
        .expect("get by name")
        .expect("capability by name");
    assert_eq!(by_name.id, capability.id);

    let by_public_id = backend
        .get_declarative_capability_by_public_id(TEST_ORG_ID, &capability.public_id)
        .await
        .expect("get by public id")
        .expect("capability by public id");
    assert_eq!(by_public_id.id, capability.id);

    let by_display_name = backend
        .list_declarative_capabilities(TEST_ORG_ID, Some("repository"), false)
        .await
        .expect("search by display name");
    assert!(by_display_name.iter().any(|row| row.id == capability.id));

    let updated = backend
        .update_declarative_capability(
            TEST_ORG_ID,
            capability.id,
            UpdateDeclarativeCapability {
                display_name: Some("Repository Capability Updated".to_string()),
                description: Some("Updated searchable declarative capability".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update declarative capability")
        .expect("updated capability");
    assert_eq!(
        updated.display_name.as_deref(),
        Some("Repository Capability Updated")
    );

    let cleared_display_name = backend
        .update_declarative_capability(
            TEST_ORG_ID,
            capability.id,
            UpdateDeclarativeCapability {
                display_name: None,
                definition: Some(json!({
                    "name": capability.name,
                    "description": "Updated searchable declarative capability",
                    "system_prompt": "Use the repository workflow.",
                    "skills": [],
                    "files": [],
                    "mcp_servers": {}
                })),
                ..Default::default()
            },
        )
        .await
        .expect("clear declarative capability display name")
        .expect("updated capability");
    assert_eq!(cleared_display_name.display_name, None);

    assert!(
        backend
            .delete_declarative_capability(TEST_ORG_ID, capability.id)
            .await
            .expect("archive declarative capability")
    );
    let archived = backend
        .get_declarative_capability(TEST_ORG_ID, capability.id)
        .await
        .expect("get archived capability")
        .expect("archived capability");
    assert_eq!(archived.status, "archived");
}

#[tokio::test]
async fn test_agent_get_by_name() {
    let backend = create_test_backend().await;

    // Create unique agent
    let unique_name = format!("name-test-{}", &Uuid::now_v7().to_string()[..8]);
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: unique_name.clone(),
                display_name: Some(unique_name.clone()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    // Find by name
    let found = backend
        .get_agent_by_name(TEST_ORG_ID, &unique_name)
        .await
        .expect("Failed to get agent by name")
        .expect("Agent not found");
    assert_eq!(found.id, agent.id);

    // Cleanup
    backend.delete_agent(TEST_ORG_ID, agent.id).await.unwrap();
}

#[tokio::test]
async fn test_session_connection_resolution_uses_resolved_owner_user() {
    let backend = create_test_backend().await;

    let owner = create_test_user(&backend, "owner").await;
    let other = create_test_user(&backend, "other").await;

    backend
        .add_organization_member(TEST_ORG_ID, owner.id, "member")
        .await
        .expect("Failed to add owner to org");
    backend
        .add_organization_member(TEST_ORG_ID, other.id, "member")
        .await
        .expect("Failed to add other user to org");

    let owner_principal_id = create_test_user_principal(&backend, TEST_ORG_ID, owner.id).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: Some(owner.id),
            title: Some(format!("connection-owner-scope-{}", Uuid::now_v7())),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    backend
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: other.id,
            provider: "gitlab".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("other-gitlab".to_string()),
            provider_username: Some("other".to_string()),
            access_token_encrypted: Some(b"other-token".to_vec()),
            refresh_token_encrypted: None,
            scopes: Some("api".to_string()),
            expires_at: None,
            installation_id: None,
            provider_metadata: Some(serde_json::json!({ "user": "other" })),
        })
        .await
        .expect("Failed to insert other user's OAuth connection");
    backend
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: other.id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("other-github".to_string()),
            provider_username: Some("other".to_string()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some("contents:read".to_string()),
            expires_at: None,
            installation_id: Some(222),
            provider_metadata: None,
        })
        .await
        .expect("Failed to insert other user's GitHub installation");

    assert_eq!(
        backend
            .get_connection_token_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve connection token"),
        None
    );
    assert_eq!(
        backend
            .get_connection_metadata_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve connection metadata"),
        None
    );
    assert_eq!(
        backend
            .get_connection_user_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve connection owner"),
        None
    );
    assert_eq!(
        backend
            .get_installation_id_for_session(session.id, "github")
            .await
            .expect("Failed to resolve GitHub installation"),
        None
    );

    backend
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: owner.id,
            provider: "gitlab".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("owner-gitlab".to_string()),
            provider_username: Some("owner".to_string()),
            access_token_encrypted: Some(b"owner-token".to_vec()),
            refresh_token_encrypted: None,
            scopes: Some("api".to_string()),
            expires_at: None,
            installation_id: None,
            provider_metadata: Some(serde_json::json!({ "user": "owner" })),
        })
        .await
        .expect("Failed to insert owner OAuth connection");
    backend
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: owner.id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some("owner-github".to_string()),
            provider_username: Some("owner".to_string()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some("contents:read".to_string()),
            expires_at: None,
            installation_id: Some(111),
            provider_metadata: None,
        })
        .await
        .expect("Failed to insert owner GitHub installation");

    assert_eq!(
        backend
            .get_connection_token_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve owner connection token"),
        Some(b"owner-token".to_vec())
    );
    assert_eq!(
        backend
            .get_connection_metadata_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve owner connection metadata"),
        Some(serde_json::json!({ "user": "owner" }))
    );
    assert_eq!(
        backend
            .get_connection_user_for_session(session.id, "gitlab")
            .await
            .expect("Failed to resolve owner connection user"),
        Some(owner.id)
    );
    assert_eq!(
        backend
            .get_installation_id_for_session(session.id, "github")
            .await
            .expect("Failed to resolve owner GitHub installation"),
        Some(111)
    );
}

#[tokio::test]
async fn test_detached_budget_root_override_canonicalizes_postgres_chain() {
    let backend = create_test_backend().await;
    let owner = create_test_user(&backend, "detached-budget-root").await;
    backend
        .add_organization_member(TEST_ORG_ID, owner.id, "member")
        .await
        .expect("add owner to org");
    let owner_principal_id = create_test_user_principal(&backend, TEST_ORG_ID, owner.id).await;
    let base = CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: TEST_ORG_ID,
        app_id: None,
        harness_id: None,
        agent_id: None,
        agent_identity_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        owner_principal_id,
        resolved_owner_user_id: Some(owner.id),
        title: Some(format!("detached-root-{}", Uuid::now_v7())),
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
    };
    let root = backend.create_session(base.clone()).await.expect("root");
    let mut detached_input = base.clone();
    detached_input.budget_root_session_id = Some(root.id);
    let detached = backend
        .create_session(detached_input)
        .await
        .expect("detached");
    let mut chain_input = base.clone();
    chain_input.budget_root_session_id = Some(detached.id);
    let chained = backend
        .create_session(chain_input)
        .await
        .expect("detached chain");
    assert_eq!(detached.root_session_id, Some(root.id));
    assert_eq!(chained.root_session_id, Some(root.id));

    let ordinary = backend
        .create_session(base)
        .await
        .expect("ordinary fork root");
    assert_eq!(ordinary.root_session_id, Some(ordinary.id));
}

// ============================================
// Session Repository Tests
// ============================================

#[tokio::test]
async fn test_session_crud() {
    let backend = create_test_backend().await;

    // Create agent first
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("session-test-agent-{}", Uuid::now_v7()),
                display_name: Some("Session Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    // Create session
    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let app_harness = backend
        .create_harness(
            TEST_ORG_ID,
            CreateHarnessRow {
                name: format!("repo-test-app-harness-{}", Uuid::now_v7()),
                display_name: Some("Repo Test App Harness".to_string()),
                description: None,
                system_prompt: Some("Test".to_string()),
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
        .expect("Failed to create app harness");
    let app = backend
        .create_app(
            TEST_ORG_ID,
            CreateAppRow {
                public_id: everruns_core::typed_id::AppId::new().to_string(),
                name: "Repository Test App".to_string(),
                description: None,
                harness_id: app_harness.id.uuid(),
                agent_id: None,
                agent_version_policy: "default".to_string(),
                agent_version_id: None,
                agent_identity_id: None,
                owner_principal_id,
                resolved_owner_user_id: None,
                channel_type: Some("webhook".to_string()),
                channel_config: json!({}),
                channel_config_encrypted: None,
            },
        )
        .await
        .expect("Failed to create app");
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: Some(app.id),
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: Some("Test Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    assert_eq!(session.agent_id, Some(agent.id));
    assert_eq!(session.app_id, Some(app.id));

    // Get session
    let fetched = backend
        .get_session(TEST_ORG_ID, session.id)
        .await
        .expect("Failed to get session")
        .expect("Session not found");
    assert_eq!(fetched.id, session.id);
    assert_eq!(fetched.app_id, Some(app.id));

    // Update session
    let updated = backend
        .update_session(
            TEST_ORG_ID,
            session.id,
            UpdateSession {
                title: Some("Updated Title".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("Failed to update session")
        .expect("Session not found");
    assert_eq!(updated.title.as_deref(), Some("Updated Title"));

    // Create a harness inline to exercise the rehoming update path — avoids
    // coupling this test to any seeded/built-in harness UUIDs.
    let harness = backend
        .create_harness(
            TEST_ORG_ID,
            CreateHarnessRow {
                name: format!("repo-test-harness-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Repo Test Harness".to_string()),
                description: None,
                system_prompt: Some("Test".to_string()),
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
        .expect("Failed to create harness");

    let rehomed = backend
        .update_session(
            TEST_ORG_ID,
            session.id,
            UpdateSession {
                harness_id: Some(harness.id),
                ..Default::default()
            },
        )
        .await
        .expect("Failed to update session harness")
        .expect("Session not found");
    assert_eq!(rehomed.harness_id, Some(harness.id));

    // List sessions with pagination
    let (sessions, total) = backend
        .list_sessions(
            TEST_ORG_ID,
            &SessionListFilters {
                agent_id: Some(agent.id),
                ..Default::default()
            },
            Pagination {
                limit: 10,
                offset: 0,
            },
        )
        .await
        .expect("Failed to list sessions");
    assert!(total >= 1);
    assert!(sessions.iter().any(|s| s.id == session.id));

    // Delete session
    let deleted = backend
        .delete_session(TEST_ORG_ID, session.id)
        .await
        .expect("Failed to delete session");
    assert!(deleted);

    // Cleanup
    backend.delete_agent(TEST_ORG_ID, agent.id).await.unwrap();
    backend
        .delete_harness(TEST_ORG_ID, harness.id)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_sessions_list_has_org_created_at_index() {
    let pool = create_test_pool().await;
    let index_definition: Option<String> = sqlx::query_scalar(
        r#"
        SELECT indexdef
        FROM pg_indexes
        WHERE schemaname = current_schema()
          AND tablename = 'sessions'
          AND indexname = 'idx_sessions_org_created_at'
        "#,
    )
    .fetch_optional(&pool)
    .await
    .expect("Failed to inspect sessions indexes");

    let index_definition = index_definition.expect(
        "sessions listing needs an org_id/created_at index to avoid scanning other tenants",
    );
    assert!(
        index_definition.ends_with("USING btree (org_id, created_at DESC)"),
        "unexpected sessions list index definition: {index_definition}"
    );
}

// ============================================
// Event Repository Tests
// ============================================

#[tokio::test]
async fn test_event_crud() {
    let backend = create_test_backend().await;

    // Create agent and session
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("event-test-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Event Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    // Create event
    let event = backend
        .create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: json!({"turn_id": Uuid::now_v7().to_string(), "role": "user"}),
            data: json!({"message": {"role": "user", "content": [{"type": "text", "text": "Hello"}]}}),
            metadata: None,
            tags: None,
        })
        .await
        .expect("Failed to create event");

    assert_eq!(event.event_type, "input.message");

    // List events
    let events = backend
        .list_events(session.id, None, None, &[], &[], None, None)
        .await
        .expect("Failed to list events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, event.id);

    // List with since_id filter
    let events_since = backend
        .list_events(session.id, None, Some(event.id), &[], &[], None, None)
        .await
        .expect("Failed to list events with since_id");
    assert!(
        events_since.is_empty(),
        "Should not include the event itself"
    );

    // Note: No cleanup - events are append-only so sessions with events cannot be deleted
}

#[tokio::test]
async fn test_event_exclude_types() {
    let backend = create_test_backend().await;

    // Create agent and session
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("event-excl-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Event Exclude Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    // Create events of different types
    backend
        .create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: json!({"role": "user"}),
            data: json!({}),
            metadata: None,
            tags: None,
        })
        .await
        .expect("Failed to create input event");

    backend
        .create_event(CreateEventRow {
            session_id: session.id,
            event_type: "output.message.delta".to_string(),
            ts: Utc::now(),
            context: json!({"role": "agent"}),
            data: json!({}),
            metadata: None,
            tags: None,
        })
        .await
        .expect("Failed to create delta event");

    // List with exclude
    let events = backend
        .list_events(
            session.id,
            None,
            None,
            &[],
            &["output.message.delta".to_string()],
            None,
            None,
        )
        .await
        .expect("Failed to list events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "input.message");

    // Note: No cleanup - events are append-only so sessions with events cannot be deleted
}

#[tokio::test]
async fn test_message_events_filtered_offset_and_latest_limit() {
    let backend = create_test_backend().await;

    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("event-window-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Event Window Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    for text in ["m1", "m2", "m3", "m4", "m5"] {
        backend
            .create_event(CreateEventRow {
                session_id: session.id,
                event_type: "input.message".to_string(),
                ts: Utc::now(),
                context: json!({"role": "user"}),
                data: json!({
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": text}]
                    }
                }),
                metadata: None,
                tags: None,
            })
            .await
            .expect("Failed to create event");
    }

    let query = MessageQuery::new(session.id).with_offset(2).with_limit(2);
    let events = backend
        .list_message_events_filtered(&query)
        .await
        .expect("Failed to list filtered message events");

    let texts: Vec<_> = events
        .iter()
        .map(|event| {
            event
                .data
                .pointer("/message/content/0/text")
                .and_then(|value| value.as_str())
                .expect("message text")
        })
        .collect();

    assert_eq!(texts, vec!["m4", "m5"]);
}

#[tokio::test]
async fn test_message_events_filtered_keep_head_loads_head_and_tail() {
    let backend = create_test_backend().await;

    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("event-anchor-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Event Anchor Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    for text in ["m1", "m2", "m3", "m4", "m5", "m6"] {
        backend
            .create_event(CreateEventRow {
                session_id: session.id,
                event_type: "input.message".to_string(),
                ts: Utc::now(),
                context: json!({"role": "user"}),
                data: json!({
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": text}]
                    }
                }),
                metadata: None,
                tags: None,
            })
            .await
            .expect("Failed to create event");
    }

    let texts = |events: &[everruns_server::storage::EventRow]| -> Vec<String> {
        events
            .iter()
            .map(|event| {
                event
                    .data
                    .pointer("/message/content/0/text")
                    .and_then(|value| value.as_str())
                    .expect("message text")
                    .to_string()
            })
            .collect()
    };

    // limit=2 tail + keep_head=1 anchor: the genuine first message survives even
    // though it sits far outside the tail window.
    let query = MessageQuery::new(session.id)
        .with_limit(2)
        .with_keep_head(1);
    let events = backend
        .list_message_events_filtered(&query)
        .await
        .expect("Failed to list filtered message events");
    assert_eq!(texts(&events), vec!["m1", "m5", "m6"]);

    // Overlapping windows must not duplicate rows.
    let overlap = MessageQuery::new(session.id)
        .with_limit(5)
        .with_keep_head(3);
    let events = backend
        .list_message_events_filtered(&overlap)
        .await
        .expect("Failed to list filtered message events");
    assert_eq!(texts(&events), vec!["m1", "m2", "m3", "m4", "m5", "m6"]);
}

#[tokio::test]
async fn test_long_message_history_reads_are_bounded_and_index_supported() {
    let backend = create_test_backend().await;

    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("long-history-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Long History Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    let total = 3_050;
    for index in 1..=total {
        let (event_type, data) = if index == total - 1 {
            (
                "output.message.completed",
                json!({
                    "message": {
                        "role": "agent",
                        "content": [{
                            "type": "tool_call",
                            "id": "call-final",
                            "name": "lookup",
                            "arguments": "{}"
                        }]
                    }
                }),
            )
        } else if index == total {
            (
                "tool.completed",
                json!({
                    "tool_call_id": "call-final",
                    "tool_name": "lookup",
                    "result": "final result"
                }),
            )
        } else {
            (
                "input.message",
                json!({
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": format!("message {index}")}]
                    }
                }),
            )
        };

        backend
            .create_event(CreateEventRow {
                session_id: session.id,
                event_type: event_type.to_string(),
                ts: Utc::now(),
                context: json!({"turn_id": format!("turn-{index}")}),
                data,
                metadata: None,
                tags: None,
            })
            .await
            .expect("Failed to create long-history event");
    }

    let started = std::time::Instant::now();
    let fallback = backend
        .list_message_events_limited(session.id, None)
        .await
        .expect("list bounded fallback history");
    let fallback_elapsed = started.elapsed();
    assert_eq!(
        fallback.len(),
        everruns_server::storage::repository::MESSAGE_SAFETY_LIMIT
    );
    assert_eq!(
        fallback.first().expect("first fallback row").sequence,
        1_051
    );
    assert_eq!(fallback.last().expect("last fallback row").sequence, total);

    let fallback_bytes: usize = fallback
        .iter()
        .map(|event| {
            event.data.to_string().len()
                + event.context.to_string().len()
                + event
                    .metadata
                    .as_ref()
                    .map(|value| value.to_string().len())
                    .unwrap_or(0)
                + event
                    .tags
                    .as_ref()
                    .map(|tags| tags.iter().map(String::len).sum::<usize>())
                    .unwrap_or(0)
        })
        .sum();

    let window = backend
        .list_message_events_filtered(
            &MessageQuery::new(session.id)
                .with_limit(64)
                .with_keep_head(1),
        )
        .await
        .expect("list bounded head+tail history");
    assert_eq!(window.len(), 65);
    assert_eq!(window.first().expect("head anchor").sequence, 1);
    assert_eq!(
        window[window.len() - 2].event_type,
        "output.message.completed"
    );
    assert_eq!(
        window.last().expect("tool result").event_type,
        "tool.completed"
    );
    assert_eq!(
        window.last().unwrap().data["tool_call_id"].as_str(),
        Some("call-final")
    );

    let pool = backend.pool().expect("postgres pool");
    let message_index: (String,) = sqlx::query_as(
        "SELECT indexdef FROM pg_indexes WHERE schemaname = current_schema() AND indexname = 'idx_events_messages'",
    )
    .fetch_one(pool)
    .await
    .expect("message events partial index should exist");
    assert!(
        message_index.0.contains("WHERE"),
        "idx_events_messages should remain a partial index: {}",
        message_index.0
    );

    let plan_rows: Vec<(String,)> = sqlx::query_as(
        r#"
        EXPLAIN (ANALYZE, BUFFERS)
        SELECT * FROM (
            SELECT id, session_id, sequence, event_type, ts, context, data, metadata, tags, created_at
            FROM events
            WHERE session_id = $1
              AND event_type IN ('input.message', 'output.message.completed', 'tool.completed')
            ORDER BY sequence DESC
            LIMIT $2
        ) recent
        ORDER BY sequence ASC
        "#,
    )
    .bind(session.id.uuid())
    .bind(everruns_server::storage::repository::MESSAGE_SAFETY_LIMIT as i64)
    .fetch_all(pool)
    .await
    .expect("explain bounded message history query");
    let plan = plan_rows
        .into_iter()
        .map(|row| row.0)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !plan.contains("Seq Scan on events"),
        "message history query should not seq-scan events:\n{plan}"
    );

    println!(
        "long message-history benchmark: before_rows={total}, after_rows={}, bytes_returned={}, service_time_ms={}, plan=\n{}",
        fallback.len(),
        fallback_bytes,
        fallback_elapsed.as_millis(),
        plan
    );
}

#[tokio::test]
async fn test_event_filter_types() {
    let backend = create_test_backend().await;

    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("event-filter-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Event Filter Types Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    // Create events of different types
    for event_type in [
        "input.message",
        "output.message.delta",
        "output.message.completed",
        "turn.started",
        "turn.completed",
    ] {
        backend
            .create_event(CreateEventRow {
                session_id: session.id,
                event_type: event_type.to_string(),
                ts: Utc::now(),
                context: json!({}),
                data: json!({}),
                metadata: None,
                tags: None,
            })
            .await
            .expect("Failed to create event");
    }

    // Positive filter: only turn events
    let events = backend
        .list_events(
            session.id,
            None,
            None,
            &["turn.started".to_string(), "turn.completed".to_string()],
            &[],
            None,
            None,
        )
        .await
        .expect("Failed to list events with types filter");
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|e| e.event_type.starts_with("turn.")));

    // Empty types = all events
    let events = backend
        .list_events(session.id, None, None, &[], &[], None, None)
        .await
        .expect("Failed to list all events");
    assert_eq!(events.len(), 5);

    // types + exclude combined: types selects 3, exclude removes 1
    let events = backend
        .list_events(
            session.id,
            None,
            None,
            &[
                "input.message".to_string(),
                "turn.started".to_string(),
                "turn.completed".to_string(),
            ],
            &["turn.completed".to_string()],
            None,
            None,
        )
        .await
        .expect("Failed to list events with types+exclude");
    assert_eq!(events.len(), 2);
    let types: Vec<&str> = events.iter().map(|e| e.event_type.as_str()).collect();
    assert!(types.contains(&"input.message"));
    assert!(types.contains(&"turn.started"));
    assert!(!types.contains(&"turn.completed"));

    // types with no match → empty
    let events = backend
        .list_events(
            session.id,
            None,
            None,
            &["nonexistent.type".to_string()],
            &[],
            None,
            None,
        )
        .await
        .expect("Failed to list events with unmatched types");
    assert!(events.is_empty());
}

// ============================================
// LLM Provider Repository Tests
// ============================================

#[tokio::test]
async fn test_provider_crud() {
    let backend = create_test_backend().await;

    // Create provider
    let provider = backend
        .create_provider(
            TEST_ORG_ID,
            CreateProviderRow {
                name: "Test Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: Some("https://api.openai.com/v1".to_string()),
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("Failed to create provider");

    assert_eq!(provider.name, "Test Provider");

    // Get provider
    let fetched = backend
        .get_provider(TEST_ORG_ID, provider.id.into())
        .await
        .expect("Failed to get provider")
        .expect("Provider not found");
    assert_eq!(fetched.id, provider.id);

    // Update provider
    let updated = backend
        .update_provider(
            TEST_ORG_ID,
            provider.id.into(),
            UpdateProvider {
                name: Some("Updated Provider".to_string()),
                provider_type: None,
                base_url: None,
                api_key_encrypted: None,
                status: None,
                settings: None,
            },
        )
        .await
        .expect("Failed to update provider")
        .expect("Provider not found");
    assert_eq!(updated.name, "Updated Provider");

    // List providers
    let providers = backend
        .list_providers(TEST_ORG_ID)
        .await
        .expect("Failed to list providers");
    assert!(providers.iter().any(|p| p.id == provider.id));

    // Delete provider
    let deleted = backend
        .delete_provider(TEST_ORG_ID, provider.id.into())
        .await
        .expect("Failed to delete provider");
    assert!(deleted);
}

// ============================================
// LLM Model Repository Tests
// ============================================

#[tokio::test]
async fn test_model_crud() {
    let backend = create_test_backend().await;

    // Create provider first
    let provider = backend
        .create_provider(
            TEST_ORG_ID,
            CreateProviderRow {
                name: "Model Test Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("Failed to create provider");

    // Create model (enabled — `get_model` filters disabled rows on the
    // resolution path and is exercised below).
    let model = backend
        .create_model(
            TEST_ORG_ID,
            CreateModelRow {
                provider_id: provider.id,
                model_id: "gpt-4-test".to_string(),
                display_name: "GPT-4 Test".to_string(),
                capabilities: vec!["chat".to_string()],
                enabled: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .expect("Failed to create model");

    assert_eq!(model.model_id, "gpt-4-test");

    // Get model
    let fetched = backend
        .get_model(TEST_ORG_ID, model.id.into())
        .await
        .expect("Failed to get model")
        .expect("Model not found");
    assert_eq!(fetched.id, model.id);

    // Get model with provider
    let with_provider = backend
        .get_model_with_provider(TEST_ORG_ID, model.id.into())
        .await
        .expect("Failed to get model with provider")
        .expect("Model not found");
    assert_eq!(with_provider.provider_id, provider.id);

    // Update model
    let updated = backend
        .update_model(
            TEST_ORG_ID,
            model.id.into(),
            UpdateModel {
                display_name: Some("Updated GPT-4".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("Failed to update model")
        .expect("Model not found");
    assert_eq!(updated.display_name, "Updated GPT-4");

    // List models for provider
    let models = backend
        .list_models_for_provider(TEST_ORG_ID, provider.id.into())
        .await
        .expect("Failed to list models");
    assert!(models.iter().any(|m| m.id == model.id));

    // Cleanup
    backend
        .delete_model(TEST_ORG_ID, model.id.into())
        .await
        .unwrap();
    backend
        .delete_provider(TEST_ORG_ID, provider.id.into())
        .await
        .unwrap();
}

// ============================================
// Session File Repository Tests
// ============================================

#[tokio::test]
async fn test_session_file_crud() {
    let backend = create_test_backend().await;

    // Create agent and session
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("file-test-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("File Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    let session_uuid: Uuid = session.id.into();

    // Create file
    let file = backend
        .create_session_file(CreateSessionFileRow {
            session_id: session.id,
            path: "/test.txt".to_string(),
            is_directory: false,
            content: Some("Hello, World!".as_bytes().to_vec()),
            is_readonly: false,
        })
        .await
        .expect("Failed to create file");

    assert_eq!(file.path, "/test.txt");

    // Get file
    let fetched = backend
        .get_session_file(session_uuid, "/test.txt")
        .await
        .expect("Failed to get file")
        .expect("File not found");
    assert_eq!(fetched.path, "/test.txt");
    assert_eq!(fetched.content, Some("Hello, World!".as_bytes().to_vec()));

    // Update file
    let updated = backend
        .update_session_file(
            session_uuid,
            "/test.txt",
            UpdateSessionFile {
                content: Some("Updated content".as_bytes().to_vec()),
                ..Default::default()
            },
        )
        .await
        .expect("Failed to update file")
        .expect("File not found");
    assert_eq!(updated.content, Some("Updated content".as_bytes().to_vec()));

    // Create directory
    let dir = backend
        .create_session_file(CreateSessionFileRow {
            session_id: session.id,
            path: "/docs".to_string(),
            is_directory: true,
            content: None,
            is_readonly: false,
        })
        .await
        .expect("Failed to create directory");
    assert!(dir.is_directory);

    // List files
    let files = backend
        .list_session_files(session_uuid, "/")
        .await
        .expect("Failed to list files");
    assert_eq!(files.len(), 2);

    // Delete file
    let deleted = backend
        .delete_session_file(session_uuid, "/test.txt")
        .await
        .expect("Failed to delete file");
    assert!(deleted);

    // Cleanup
    backend
        .delete_session(TEST_ORG_ID, session.id)
        .await
        .unwrap();
    backend.delete_agent(TEST_ORG_ID, agent.id).await.unwrap();
}

// ============================================
// MCP Server Repository Tests
// ============================================

#[tokio::test]
async fn test_mcp_server_crud() {
    let backend = create_test_backend().await;

    // Create MCP server
    let unique_name = format!("Test MCP Server {}", Uuid::now_v7());
    let server = backend
        .create_mcp_server(
            TEST_ORG_ID,
            CreateMcpServerRow {
                name: unique_name.clone(),
                description: Some("Test MCP server".to_string()),
                url: "http://localhost:3000".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("Failed to create MCP server");

    assert_eq!(server.name, unique_name);

    // Get MCP server
    let fetched = backend
        .get_mcp_server(TEST_ORG_ID, server.id.uuid())
        .await
        .expect("Failed to get MCP server")
        .expect("MCP server not found");
    assert_eq!(fetched.id, server.id);

    // Get by name
    let by_name = backend
        .get_mcp_server_by_name(TEST_ORG_ID, &unique_name)
        .await
        .expect("Failed to get MCP server by name")
        .expect("MCP server not found");
    assert_eq!(by_name.id, server.id);

    // List MCP servers
    let servers = backend
        .list_mcp_servers(TEST_ORG_ID, None, false)
        .await
        .expect("Failed to list MCP servers");
    assert!(servers.iter().any(|s| s.id == server.id));

    // Delete MCP server
    let deleted = backend
        .delete_mcp_server(TEST_ORG_ID, server.id.uuid())
        .await
        .expect("Failed to delete MCP server");
    assert!(deleted);
}

// ============================================
// Agent Capability Repository Tests
// ============================================

#[tokio::test]
async fn test_agent_capabilities() {
    let backend = create_test_backend().await;

    // Create agent
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("cap-test-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Capability Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    // Add capability
    let capability = backend
        .add_agent_capability(CreateAgentCapabilityRow {
            agent_id: agent.id,
            capability_id: "current_time".to_string(),
            position: 0,
            config: json!({}),
        })
        .await
        .expect("Failed to add capability");

    assert_eq!(capability.capability_id, "current_time");

    // Get capabilities
    let capabilities = backend
        .get_agent_capabilities(agent.id.into())
        .await
        .expect("Failed to get capabilities");
    assert!(
        capabilities
            .iter()
            .any(|c| c.capability_id == "current_time")
    );

    // Set capabilities (replace all)
    let new_caps = backend
        .set_agent_capabilities(
            agent.id.into(),
            vec![
                ("session_file_system".to_string(), 0, json!({})),
                ("web_search".to_string(), 1, json!({})),
            ],
        )
        .await
        .expect("Failed to set capabilities");
    assert!(
        new_caps
            .iter()
            .any(|c| c.capability_id == "session_file_system")
    );
    assert!(new_caps.iter().any(|c| c.capability_id == "web_search"));

    // Verify capabilities replaced
    let capabilities = backend
        .get_agent_capabilities(agent.id.into())
        .await
        .expect("Failed to get capabilities");
    assert!(
        capabilities
            .iter()
            .any(|c| c.capability_id == "session_file_system")
    );
    assert!(capabilities.iter().any(|c| c.capability_id == "web_search"));

    // Remove capability
    let removed = backend
        .remove_agent_capability(agent.id.into(), "web_search")
        .await
        .expect("Failed to remove capability");
    assert!(removed);

    // Cleanup
    backend.delete_agent(TEST_ORG_ID, agent.id).await.unwrap();
}

// ============================================
// Organization Repository Tests
// ============================================

#[tokio::test]
async fn test_organization_crud() {
    let backend = create_test_backend().await;

    // Create organization
    // Note: public_id must match format ^org_[0-9a-f]{32}$
    let public_id = format!("org_{}", Uuid::now_v7().simple());
    let org = backend
        .create_organization(CreateOrganizationRow {
            public_id: public_id.clone(),
            name: format!("Test Org {}", Uuid::now_v7()),
            created_by: None,
        })
        .await
        .expect("Failed to create organization");

    assert!(!org.name.is_empty());

    // Get organization
    let fetched = backend
        .get_organization(org.org_id)
        .await
        .expect("Failed to get organization")
        .expect("Organization not found");
    assert_eq!(fetched.org_id, org.org_id);

    // Get by public_id
    let by_public_id = backend
        .get_organization_by_public_id(&public_id)
        .await
        .expect("Failed to get organization by public_id")
        .expect("Organization not found");
    assert_eq!(by_public_id.org_id, org.org_id);

    // Update organization
    let updated = backend
        .update_organization(
            org.org_id,
            UpdateOrganization {
                name: Some("Updated Org Name".to_string()),
            },
        )
        .await
        .expect("Failed to update organization")
        .expect("Organization not found");
    assert_eq!(updated.name, "Updated Org Name");

    // List organizations
    let orgs = backend
        .list_organizations()
        .await
        .expect("Failed to list organizations");
    assert!(orgs.iter().any(|o| o.org_id == org.org_id));

    // Org creation provisions both resources before required initializers run.
    // They must not block rollback if a required initializer fails.
    org_init::initialize_org_harnesses(&backend, org.org_id)
        .await
        .expect("Failed to initialize org harnesses");
    org_init::seed_default_plugin_marketplace(&backend, org.org_id).await;

    // Delete organization, including its settings and provisioned resources.
    let deleted = backend
        .delete_organization(org.org_id)
        .await
        .expect("Failed to delete organization");
    assert!(deleted);
}

#[tokio::test]
async fn test_organization_settings_harness_roundtrip() {
    let backend = create_test_backend().await;

    let public_id = format!("org_{}", Uuid::now_v7().simple());
    let org = backend
        .create_organization(CreateOrganizationRow {
            public_id,
            name: format!("Test Org {}", Uuid::now_v7()),
            created_by: None,
        })
        .await
        .expect("Failed to create organization");

    org_init::initialize_org_harnesses(&backend, org.org_id)
        .await
        .expect("Failed to initialize org harnesses");

    let harnesses = backend
        .list_harnesses(org.org_id, None, false)
        .await
        .expect("Failed to list harnesses");
    let generic_id = harnesses.iter().find(|h| h.name == "generic").unwrap().id;
    let base_id = harnesses.iter().find(|h| h.name == "base").unwrap().id;

    backend
        .patch_organization_settings(
            org.org_id,
            UpdateOrganizationSettings {
                default_harness_id: UpdateField::Set(generic_id),
                base_harness_id: UpdateField::Set(base_id),
                ..Default::default()
            },
        )
        .await
        .expect("Failed to patch organization settings");

    let settings = backend
        .get_organization_settings(org.org_id)
        .await
        .expect("Failed to get organization settings")
        .expect("Organization settings not found");

    assert_eq!(settings.default_harness_id, Some(generic_id));
    assert_eq!(settings.base_harness_id, Some(base_id));
}

// ============================================
// Image Repository Tests
// ============================================

#[tokio::test]
async fn test_image_crud() {
    let backend = create_test_backend().await;

    // Create image
    let image = backend
        .create_image(
            TEST_ORG_ID,
            CreateImageRow {
                org_id: TEST_ORG_ID,
                filename: "test.png".to_string(),
                content_type: "image/png".to_string(),
                size_bytes: 4,
                data: vec![0x89, 0x50, 0x4E, 0x47], // PNG header
                thumbnail_data: None,
                thumbnail_content_type: None,
                metadata: json!({}),
            },
        )
        .await
        .expect("Failed to create image");

    assert_eq!(image.content_type, "image/png");
    assert_eq!(image.size_bytes, 4);

    // Get image
    let fetched = backend
        .get_image(TEST_ORG_ID, image.id.uuid())
        .await
        .expect("Failed to get image")
        .expect("Image not found");
    assert_eq!(fetched.id, image.id);
    assert_eq!(fetched.data, vec![0x89, 0x50, 0x4E, 0x47]);

    // Get image info (without data)
    let info = backend
        .get_image_info(TEST_ORG_ID, image.id.uuid())
        .await
        .expect("Failed to get image info")
        .expect("Image not found");
    assert_eq!(info.id, image.id);
    assert_eq!(info.size_bytes, 4);

    // List images
    let images = backend
        .list_images(TEST_ORG_ID, 10, 0)
        .await
        .expect("Failed to list images");
    assert!(images.iter().any(|i| i.id == image.id));

    // Delete image
    let deleted = backend
        .delete_image(TEST_ORG_ID, image.id.uuid())
        .await
        .expect("Failed to delete image");
    assert!(deleted);
}

// ============================================
// Session Usage Tracking Tests
// ============================================

#[tokio::test]
async fn test_session_usage_tracking() {
    let backend = create_test_backend().await;

    // Create agent and session
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("usage-test-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Usage Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    // Increment session usage (actual, estimated, best-effort cost)
    backend
        .increment_session_usage(session.id.into(), 100, 50, 0, 0, 0.012, 0.010, 0.012)
        .await
        .expect("Failed to increment session usage");

    // Verify usage updated
    let updated = backend
        .get_session(TEST_ORG_ID, session.id)
        .await
        .expect("Failed to get session")
        .expect("Session not found");
    assert_eq!(updated.total_input_tokens, 100);
    assert_eq!(updated.total_output_tokens, 50);

    // Increment again
    backend
        .increment_session_usage(session.id.into(), 200, 100, 50, 10, 0.024, 0.020, 0.024)
        .await
        .expect("Failed to increment session usage");

    let updated = backend
        .get_session(TEST_ORG_ID, session.id)
        .await
        .expect("Failed to get session")
        .expect("Session not found");
    assert_eq!(updated.total_input_tokens, 300);
    assert_eq!(updated.total_output_tokens, 150);
    assert_eq!(updated.total_cache_read_tokens, 50);
    assert_eq!(updated.total_cache_creation_tokens, 10);
    assert!((updated.total_actual_cost_usd - 0.036).abs() < 1e-9);
    assert!((updated.total_estimated_cost_usd - 0.030).abs() < 1e-9);
    assert!((updated.total_cost_usd - 0.036).abs() < 1e-9);

    // Cleanup
    backend
        .delete_session(TEST_ORG_ID, session.id)
        .await
        .unwrap();
    backend.delete_agent(TEST_ORG_ID, agent.id).await.unwrap();
}

// ============================================
// Session Preview Tests
// ============================================

#[tokio::test]
async fn test_session_previews() {
    let backend = create_test_backend().await;

    // Create agent and sessions
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: format!("preview-test-agent-{}", &Uuid::now_v7().to_string()[..8]),
                display_name: Some("Preview Test Agent".to_string()),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,

                harness_id: ensure_test_harness_id(&backend).await,
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
            },
        )
        .await
        .expect("Failed to create agent");

    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: Some(agent.id),
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create session");

    // Create a user message event
    backend
        .create_event(CreateEventRow {
            session_id: session.id,
            event_type: "input.message".to_string(),
            ts: Utc::now(),
            context: json!({"role": "user"}),
            data: json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello, how are you?"}]
                }
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("Failed to create event");

    // Create an agent message event (must be output.message.completed for preview queries)
    backend
        .create_event(CreateEventRow {
            session_id: session.id,
            event_type: "output.message.completed".to_string(),
            ts: Utc::now(),
            context: json!({"role": "agent"}),
            data: json!({
                "message": {
                    "role": "agent",
                    "content": [{"type": "text", "text": "I'm doing well, thank you!"}]
                }
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("Failed to create event");

    // Get session previews
    let session_uuid: Uuid = session.id.into();
    let previews = backend
        .get_session_previews(&[session_uuid])
        .await
        .expect("Failed to get session previews");
    assert!(previews.contains_key(&session_uuid));
    assert!(previews[&session_uuid].contains("Hello"));

    // Get output previews
    let output_previews = backend
        .get_session_output_previews(&[session_uuid])
        .await
        .expect("Failed to get output previews");
    assert!(output_previews.contains_key(&session_uuid));
    assert!(output_previews[&session_uuid].contains("doing well"));

    // Note: No cleanup - events are append-only so sessions with events cannot be deleted
}

// ============================================
// Organization Isolation Tests (PostgreSQL)
// ============================================

/// Helper to create a new org for isolation testing
async fn create_test_org(backend: &StorageBackend, name: &str) -> i64 {
    let org = backend
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: name.to_string(),
            created_by: None,
        })
        .await
        .expect("Failed to create org");
    org.org_id
}

#[tokio::test]
async fn test_mcp_server_org_isolation_postgres() {
    let backend = create_test_backend().await;
    let org2 = create_test_org(&backend, "Isolation Test Org").await;

    let unique_name = format!("MCP-Iso-{}", Uuid::now_v7());
    let server = backend
        .create_mcp_server(
            TEST_ORG_ID,
            CreateMcpServerRow {
                name: unique_name.clone(),
                description: None,
                url: "https://mcp.example.com".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .expect("create server");

    // Positive: own org sees the server
    assert!(
        backend
            .get_mcp_server(TEST_ORG_ID, server.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        backend
            .get_mcp_server_by_name(TEST_ORG_ID, &unique_name)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        backend
            .list_mcp_servers(TEST_ORG_ID, None, false)
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == server.id)
    );

    // Negative: other org cannot see it
    assert!(
        backend
            .get_mcp_server(org2, server.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .get_mcp_server_by_name(org2, &unique_name)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !backend
            .list_mcp_servers(org2, None, false)
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == server.id)
    );

    // Negative: other org cannot delete
    assert!(
        !backend
            .delete_mcp_server(org2, server.id.uuid())
            .await
            .unwrap()
    );

    // Cleanup
    backend
        .delete_mcp_server(TEST_ORG_ID, server.id.uuid())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_provider_org_isolation_postgres() {
    let backend = create_test_backend().await;
    let org2 = create_test_org(&backend, "Provider Isolation Org").await;

    let provider = backend
        .create_provider(
            TEST_ORG_ID,
            CreateProviderRow {
                name: format!("Provider-{}", Uuid::now_v7()),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("create provider");

    // Positive
    assert!(
        backend
            .get_provider(TEST_ORG_ID, provider.id.uuid())
            .await
            .unwrap()
            .is_some()
    );

    // Negative
    assert!(
        backend
            .get_provider(org2, provider.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !backend
            .list_providers(org2)
            .await
            .unwrap()
            .iter()
            .any(|p| p.id == provider.id)
    );
    assert!(
        !backend
            .delete_provider(org2, provider.id.uuid())
            .await
            .unwrap()
    );

    // Cleanup
    backend
        .delete_provider(TEST_ORG_ID, provider.id.uuid())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_model_org_isolation_postgres() {
    let backend = create_test_backend().await;
    let org2 = create_test_org(&backend, "Model Isolation Org").await;

    let provider = backend
        .create_provider(
            TEST_ORG_ID,
            CreateProviderRow {
                name: format!("Prov-{}", Uuid::now_v7()),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("create provider");

    let model = backend
        .create_model(
            TEST_ORG_ID,
            CreateModelRow {
                provider_id: provider.id,
                model_id: format!("model-{}", Uuid::now_v7()),
                display_name: "Test Model".to_string(),
                capabilities: vec![],
                // Enabled — `get_model` enforces enabled = TRUE on the
                // resolution path; this test focuses on org isolation.
                enabled: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .expect("create model");

    // Positive
    assert!(
        backend
            .get_model(TEST_ORG_ID, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        backend
            .get_model_with_provider(TEST_ORG_ID, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );

    // Negative
    assert!(
        backend
            .get_model(org2, model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .get_model_with_provider(org2, model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .list_models_for_provider(org2, provider.id.uuid())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!backend.delete_model(org2, model.id.uuid()).await.unwrap());

    // Cleanup
    backend
        .delete_model(TEST_ORG_ID, model.id.uuid())
        .await
        .unwrap();
    backend
        .delete_provider(TEST_ORG_ID, provider.id.uuid())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_model_provider_reads_fail_closed_on_cross_org_provider_postgres() {
    let backend = create_test_backend().await;
    let org2 = create_test_org(&backend, "Cross-org Provider Isolation Org").await;

    let foreign_provider = backend
        .create_provider(
            org2,
            CreateProviderRow {
                name: format!("Prov-{}", Uuid::now_v7()),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("create foreign provider");

    let corrupt_model = backend
        .create_model(
            TEST_ORG_ID,
            CreateModelRow {
                provider_id: foreign_provider.id,
                model_id: format!("cross-org-model-{}", Uuid::now_v7()),
                display_name: "Corrupt Model".to_string(),
                capabilities: vec![],
                enabled: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .expect("create corrupt model");

    backend
        .upsert_organization_settings(TEST_ORG_ID, Some(corrupt_model.id.uuid()))
        .await
        .expect("set default model");

    assert!(
        backend
            .get_model(TEST_ORG_ID, corrupt_model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        backend
            .get_model_with_provider(TEST_ORG_ID, corrupt_model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .list_all_models(TEST_ORG_ID)
            .await
            .unwrap()
            .into_iter()
            .all(|model| model.id != corrupt_model.id)
    );
    assert!(
        backend
            .get_model_by_model_id(TEST_ORG_ID, &corrupt_model.model_id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .get_default_model(TEST_ORG_ID)
            .await
            .unwrap()
            .is_none()
    );

    backend
        .delete_model(TEST_ORG_ID, corrupt_model.id.uuid())
        .await
        .unwrap();
    backend
        .delete_provider(org2, foreign_provider.id.uuid())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_disabled_model_is_not_resolvable_or_default_postgres() {
    let backend = create_test_backend().await;
    let org_id = create_test_org(&backend, "Disabled Model Resolution Org").await;

    let provider = backend
        .create_provider(
            org_id,
            CreateProviderRow {
                name: format!("Prov-{}", Uuid::now_v7()),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("create provider");

    let disabled_model = backend
        .create_model(
            org_id,
            CreateModelRow {
                provider_id: provider.id,
                model_id: format!("disabled-model-{}", Uuid::now_v7()),
                display_name: "Disabled Model".to_string(),
                capabilities: vec!["chat".to_string()],
                enabled: false,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .expect("create disabled model");

    backend
        .upsert_organization_settings(org_id, Some(disabled_model.id.uuid()))
        .await
        .expect("set default model");

    // Resolution paths must fail closed for disabled models.
    assert!(
        backend.get_default_model(org_id).await.unwrap().is_none(),
        "default resolution must not return a disabled model"
    );
    assert!(
        backend
            .get_model_by_model_id(org_id, &disabled_model.model_id)
            .await
            .unwrap()
            .is_none(),
        "by-model-id resolution must not return a disabled model"
    );
    assert!(
        backend
            .get_model(org_id, disabled_model.id.uuid())
            .await
            .unwrap()
            .is_none(),
        "by-UUID resolution must not return a disabled model (used by agent execution paths)"
    );

    // Admin listing must still include disabled models so administrators can
    // see and re-enable them via the management UI.
    let listed = backend.list_all_models(org_id).await.unwrap();
    assert!(
        listed.iter().any(|model| model.id == disabled_model.id),
        "admin listing must include disabled models"
    );

    // Teardown the isolated org state created above.
    backend
        .upsert_organization_settings(org_id, None)
        .await
        .expect("clear default model");
    backend
        .delete_model(org_id, disabled_model.id.uuid())
        .await
        .expect("delete disabled model");
    backend
        .delete_provider(org_id, provider.id.uuid())
        .await
        .expect("delete provider");
}

#[tokio::test]
async fn test_image_org_isolation_postgres() {
    let backend = create_test_backend().await;
    let org2 = create_test_org(&backend, "Image Isolation Org").await;

    let image = backend
        .create_image(
            TEST_ORG_ID,
            CreateImageRow {
                org_id: TEST_ORG_ID,
                filename: format!("iso-{}.png", Uuid::now_v7()),
                content_type: "image/png".to_string(),
                size_bytes: 4,
                data: vec![0x89, 0x50, 0x4E, 0x47],
                thumbnail_data: None,
                thumbnail_content_type: None,
                metadata: json!({}),
            },
        )
        .await
        .expect("create image");

    // Positive
    assert!(
        backend
            .get_image(TEST_ORG_ID, image.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        backend
            .get_image_info(TEST_ORG_ID, image.id.uuid())
            .await
            .unwrap()
            .is_some()
    );

    // Negative
    assert!(
        backend
            .get_image(org2, image.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .get_image_info(org2, image.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !backend
            .list_images(org2, 50, 0)
            .await
            .unwrap()
            .iter()
            .any(|i| i.id == image.id)
    );
    assert!(!backend.delete_image(org2, image.id.uuid()).await.unwrap());

    // Cleanup
    backend
        .delete_image(TEST_ORG_ID, image.id.uuid())
        .await
        .unwrap();
}

// ============================================
// Session Task / Schedule Integration Tests
// ============================================

/// PG integration test for `list_monitor_tasks_with_inactive_schedules`.
///
/// Verifies the SQL join/filter logic against real PostgreSQL:
/// - Active schedule  → task is NOT returned.
/// - Disabled schedule → task IS returned with the correct (session_id, task_id, schedule_id) triple.
/// - Malformed `spec.schedule_id` (not matching `^sched_[0-9a-f]{32}$`) → task is NEVER returned.
///
/// Assertions are scoped to IDs created in this test so other pre-existing
/// rows (from parallel test runs or seeded data) do not cause false failures.
#[tokio::test]
async fn list_monitor_tasks_with_inactive_schedules_pg() {
    use everruns_core::ScheduleId;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskState, TASK_KIND_MONITOR, TaskLinks, TaskWakePolicy,
        new_session_task,
    };

    let backend = create_test_backend().await;

    // ------------------------------------------------------------------
    // Fixtures: principal + session (minimal, no agent required)
    // ------------------------------------------------------------------
    let owner_principal_id = create_test_principal(&backend, TEST_ORG_ID).await;
    let session = backend
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            workspace_id: None,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id,
            resolved_owner_user_id: None,
            title: Some(format!("monitor-inactive-sched-test-{}", Uuid::now_v7())),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::Value::Array(vec![]),
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
        .expect("Failed to create test session");

    let session_id = session.id;

    // ------------------------------------------------------------------
    // Fixture: a recurring session schedule (enabled = true by default)
    // ------------------------------------------------------------------
    let schedule = backend
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: TEST_ORG_ID,
            session_id,
            owner_principal_id,
            resolved_owner_user_id: None,
            description: "monitor-inactive-sched-test".to_string(),
            cron_expression: Some("0 * * * *".to_string()),
            scheduled_at: None,
            timezone: "UTC".to_string(),
            next_trigger_at: None,
        })
        .await
        .expect("Failed to create test schedule");

    let schedule_id: ScheduleId = schedule.id;

    // Fixture: a directly canceled one-shot schedule. It should be considered
    // inactive because it has no trigger metadata.
    let one_shot_schedule = backend
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: TEST_ORG_ID,
            session_id,
            owner_principal_id,
            resolved_owner_user_id: None,
            description: "monitor-canceled-one-shot-test".to_string(),
            cron_expression: None,
            scheduled_at: Some(Utc::now()),
            timezone: "UTC".to_string(),
            next_trigger_at: None,
        })
        .await
        .expect("Failed to create one-shot test schedule");

    let one_shot_schedule_id: ScheduleId = one_shot_schedule.id;

    // Fixture: a fired one-shot schedule. It is disabled before its linked
    // monitor is marked Succeeded, so it should not be an orphan candidate.
    let fired_one_shot_schedule = backend
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: TEST_ORG_ID,
            session_id,
            owner_principal_id,
            resolved_owner_user_id: None,
            description: "monitor-fired-one-shot-test".to_string(),
            cron_expression: None,
            scheduled_at: Some(Utc::now()),
            timezone: "UTC".to_string(),
            next_trigger_at: None,
        })
        .await
        .expect("Failed to create fired one-shot test schedule");

    let fired_one_shot_schedule_id: ScheduleId = fired_one_shot_schedule.id;

    // ------------------------------------------------------------------
    // Fixture: running monitor task with a valid prefixed schedule_id
    // ------------------------------------------------------------------
    let monitor_task = new_session_task(
        CreateSessionTask {
            session_id,
            id: None,
            kind: TASK_KIND_MONITOR.to_string(),
            display_name: "Test monitor (inactive-sched pg)".to_string(),
            spec: json!({ "schedule_id": schedule_id.to_string() }),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        },
        Utc::now(),
    );
    backend
        .create_session_task(&monitor_task)
        .await
        .expect("Failed to create monitor task");

    let task_id = monitor_task.id.clone();

    let one_shot_monitor_task = new_session_task(
        CreateSessionTask {
            session_id,
            id: None,
            kind: TASK_KIND_MONITOR.to_string(),
            display_name: "Test monitor (canceled one-shot pg)".to_string(),
            spec: json!({ "schedule_id": one_shot_schedule_id.to_string() }),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        },
        Utc::now(),
    );
    backend
        .create_session_task(&one_shot_monitor_task)
        .await
        .expect("Failed to create one-shot monitor task");

    let one_shot_task_id = one_shot_monitor_task.id.clone();

    let fired_one_shot_monitor_task = new_session_task(
        CreateSessionTask {
            session_id,
            id: None,
            kind: TASK_KIND_MONITOR.to_string(),
            display_name: "Test monitor (fired one-shot pg)".to_string(),
            spec: json!({ "schedule_id": fired_one_shot_schedule_id.to_string() }),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        },
        Utc::now(),
    );
    backend
        .create_session_task(&fired_one_shot_monitor_task)
        .await
        .expect("Failed to create fired one-shot monitor task");

    let fired_one_shot_task_id = fired_one_shot_monitor_task.id.clone();

    // ------------------------------------------------------------------
    // Fixture: a second monitor task with a *malformed* schedule_id.
    // The regex '^sched_[0-9a-f]{32}$' must reject this, so it should
    // never appear in results.
    // ------------------------------------------------------------------
    let malformed_task = new_session_task(
        CreateSessionTask {
            session_id,
            id: None,
            kind: TASK_KIND_MONITOR.to_string(),
            display_name: "Test monitor (malformed spec)".to_string(),
            spec: json!({ "schedule_id": "not-a-schedule-id" }),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        },
        Utc::now(),
    );
    backend
        .create_session_task(&malformed_task)
        .await
        .expect("Failed to create malformed-spec monitor task");

    let malformed_task_id = malformed_task.id.clone();

    // Helper: filter the global result list to rows belonging to this test.
    let our_ids: std::collections::HashSet<String> = [
        task_id.clone(),
        one_shot_task_id.clone(),
        fired_one_shot_task_id.clone(),
        malformed_task_id.clone(),
    ]
    .into();

    // ------------------------------------------------------------------
    // Step 4: schedule is still enabled → our task must NOT appear.
    // ------------------------------------------------------------------
    let results_before = backend
        .list_monitor_tasks_with_inactive_schedules(500)
        .await
        .expect("list_monitor_tasks_with_inactive_schedules failed");

    let our_results_before: Vec<_> = results_before
        .iter()
        .filter(|(_, tid, _)| our_ids.contains(tid))
        .collect();

    assert!(
        our_results_before.is_empty(),
        "expected no results while schedule is enabled, got: {:?}",
        our_results_before
    );

    // ------------------------------------------------------------------
    // Step 5: disable the recurring and one-shot schedules. The recurring and
    // directly canceled one-shot tasks must appear; the fired one-shot must not.
    // ------------------------------------------------------------------
    backend
        .update_session_schedule(
            TEST_ORG_ID,
            schedule_id,
            UpdateSessionScheduleRow {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("Failed to disable recurring schedule");
    backend
        .update_session_schedule(
            TEST_ORG_ID,
            one_shot_schedule_id,
            UpdateSessionScheduleRow {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("Failed to disable one-shot schedule");
    backend
        .update_session_schedule(
            TEST_ORG_ID,
            fired_one_shot_schedule_id,
            UpdateSessionScheduleRow {
                enabled: Some(false),
                last_triggered_at: Some(Utc::now()),
                trigger_count_increment: true,
                ..Default::default()
            },
        )
        .await
        .expect("Failed to mark fired one-shot schedule");

    let results_after = backend
        .list_monitor_tasks_with_inactive_schedules(500)
        .await
        .expect("list_monitor_tasks_with_inactive_schedules failed after disable");

    let our_results_after: Vec<_> = results_after
        .iter()
        .filter(|(_, tid, _)| our_ids.contains(tid))
        .collect();

    assert_eq!(
        our_results_after.len(),
        2,
        "expected recurring and canceled one-shot results after disabling schedules, got: {:?}",
        our_results_after
    );

    let returned: std::collections::HashSet<_> = our_results_after
        .iter()
        .map(|(_, task_id, schedule_id)| ((*task_id).clone(), (*schedule_id).clone()))
        .collect();
    assert!(
        returned.contains(&(task_id.clone(), schedule_id.to_string())),
        "recurring schedule task should be returned"
    );
    assert!(
        returned.contains(&(one_shot_task_id.clone(), one_shot_schedule_id.to_string())),
        "directly canceled one-shot schedule task should be returned"
    );
    assert!(
        our_results_after
            .iter()
            .all(|(ret_session_id, _, _)| *ret_session_id == session_id),
        "session_id mismatch in results"
    );

    // ------------------------------------------------------------------
    // Step 6: malformed-spec task must NEVER appear (regex filter).
    // ------------------------------------------------------------------
    assert!(
        results_after
            .iter()
            .all(|(_, tid, _)| tid != &malformed_task_id),
        "malformed-spec task must never appear in results"
    );
    assert!(
        results_after
            .iter()
            .all(|(_, tid, _)| tid != &fired_one_shot_task_id),
        "fired disabled one-shot schedule task must never appear in results"
    );
}

/// EVE-586: the boot reaper transitions orphaned `running`/`pending` health
/// check runs to `failed` while leaving terminal runs untouched. Mirrors the
/// in-memory unit test so both backends are proven to behave identically.
#[tokio::test]
async fn test_reap_running_agent_health_check_runs() {
    let backend = create_test_backend().await;

    let health_check_input = |pid: u128| CreateAgentHealthCheckRunRow {
        public_id: format!("healthcheck_{pid:032x}"),
        agent_id: None,
        config_hash: "cfg".to_string(),
        model_id: None,
    };

    // An orphaned run still marked `running`.
    let running = backend
        .create_agent_health_check_run(TEST_ORG_ID, health_check_input(Uuid::now_v7().as_u128()))
        .await
        .expect("create running health check run");
    backend
        .update_agent_health_check_run(
            running.id,
            UpdateAgentHealthCheckRunRow {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("mark running");

    // A run that finished normally must be left untouched.
    let done = backend
        .create_agent_health_check_run(TEST_ORG_ID, health_check_input(Uuid::now_v7().as_u128()))
        .await
        .expect("create completed health check run");
    backend
        .update_agent_health_check_run(
            done.id,
            UpdateAgentHealthCheckRunRow {
                status: Some("completed".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("mark completed");

    let reaped = backend
        .reap_running_agent_health_check_runs()
        .await
        .expect("reap runs");
    assert!(
        reaped >= 1,
        "expected at least the orphaned run to be reaped"
    );

    let running_after = backend
        .get_agent_health_check_run(TEST_ORG_ID, &running.public_id)
        .await
        .expect("fetch running run")
        .expect("running run exists");
    assert_eq!(running_after.status, "failed");
    assert!(running_after.error_message.is_some());
    assert!(running_after.completed_at.is_some());

    let done_after = backend
        .get_agent_health_check_run(TEST_ORG_ID, &done.public_id)
        .await
        .expect("fetch completed run")
        .expect("completed run exists");
    assert_eq!(done_after.status, "completed");
}

/// Org-scoped task listing (EVE-583) against real PostgreSQL.
///
/// Validates the `sessions.org_id` semijoin, the kind/state/created_after
/// filters, and the bounded limit — including strict cross-org isolation: a
/// task owned by another org must never appear in the caller org's listing.
#[tokio::test]
async fn list_org_session_tasks_pg() {
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskState, TASK_KIND_BACKGROUND_TOOL, TASK_KIND_SUBAGENT,
        TaskLinks, TaskWakePolicy, new_session_task,
    };

    let backend = create_test_backend().await;
    let org_b = create_test_org(&backend, "EVE-583 Isolation Org").await;

    // A session in each org.
    let mk_session = |org_id: i64, owner: everruns_core::PrincipalId| {
        let backend = &backend;
        async move {
            backend
                .create_session(CreateSessionRow {
                    source: everruns_platform::SessionSource::Api,
                    workspace_id: None,
                    org_id,
                    app_id: None,
                    harness_id: None,
                    agent_id: None,
                    agent_version_id: None,
                    agent_config_hash: None,
                    agent_identity_id: None,
                    owner_principal_id: owner,
                    resolved_owner_user_id: None,
                    title: Some(format!("eve-583-{}", Uuid::now_v7())),
                    locale: None,
                    tags: vec![],
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
                .expect("create session")
                .id
        }
    };

    let owner_a = create_test_principal(&backend, TEST_ORG_ID).await;
    let owner_b = create_test_principal(&backend, org_b).await;
    let session_a = mk_session(TEST_ORG_ID, owner_a).await;
    let session_b = mk_session(org_b, owner_b).await;

    // Two tasks in org A (distinct kind/state), one in org B.
    let mk_task = |session_id, kind: &str, state, name: &str| {
        let task = new_session_task(
            CreateSessionTask {
                session_id,
                id: None,
                kind: kind.to_string(),
                display_name: name.to_string(),
                spec: json!({}),
                state,
                links: TaskLinks::default(),
                wake_policy: TaskWakePolicy::Silent,
            },
            Utc::now(),
        );
        let backend = &backend;
        async move {
            backend
                .create_session_task(&task)
                .await
                .expect("create task");
            task.id
        }
    };

    let a_sub = mk_task(
        session_a,
        TASK_KIND_SUBAGENT,
        SessionTaskState::Running,
        "A-sub",
    )
    .await;
    let a_bg = mk_task(
        session_a,
        TASK_KIND_BACKGROUND_TOOL,
        SessionTaskState::Queued,
        "A-bg",
    )
    .await;
    let b_sub = mk_task(
        session_b,
        TASK_KIND_SUBAGENT,
        SessionTaskState::Running,
        "B-sub",
    )
    .await;

    let ids = |rows: &[everruns_server::storage::SessionTaskRow]| {
        rows.iter()
            .map(|r| r.id.clone())
            .collect::<std::collections::HashSet<_>>()
    };

    // Org A listing: contains both A tasks, never the B task.
    let a_all = backend
        .list_org_session_tasks(TEST_ORG_ID, None, None, None, None, 500)
        .await
        .expect("list org A");
    let a_all_ids = ids(&a_all);
    assert!(a_all_ids.contains(&a_sub), "A-sub must be listed");
    assert!(a_all_ids.contains(&a_bg), "A-bg must be listed");
    assert!(
        !a_all_ids.contains(&b_sub),
        "org B's task must never leak into org A's listing"
    );

    // Org B listing: contains only the B task, never A's.
    let b_all = backend
        .list_org_session_tasks(org_b, None, None, None, None, 500)
        .await
        .expect("list org B");
    let b_all_ids = ids(&b_all);
    assert!(b_all_ids.contains(&b_sub));
    assert!(!b_all_ids.contains(&a_sub) && !b_all_ids.contains(&a_bg));

    // Kind filter (org A): only the subagent task.
    let a_subs = backend
        .list_org_session_tasks(TEST_ORG_ID, Some("subagent"), None, None, None, 500)
        .await
        .expect("list org A subagents");
    let a_subs_ids = ids(&a_subs);
    assert!(a_subs_ids.contains(&a_sub));
    assert!(!a_subs_ids.contains(&a_bg));

    // State filter (org A): only the queued task.
    let a_queued = backend
        .list_org_session_tasks(TEST_ORG_ID, None, Some("queued"), None, None, 500)
        .await
        .expect("list org A queued");
    let a_queued_ids = ids(&a_queued);
    assert!(a_queued_ids.contains(&a_bg));
    assert!(!a_queued_ids.contains(&a_sub));

    // created_after in the future excludes our just-created tasks.
    let future = Utc::now() + chrono::Duration::hours(1);
    let a_future = backend
        .list_org_session_tasks(TEST_ORG_ID, None, None, Some(future), None, 500)
        .await
        .expect("list org A future");
    let a_future_ids = ids(&a_future);
    assert!(!a_future_ids.contains(&a_sub) && !a_future_ids.contains(&a_bg));

    // Limit is honored.
    let a_limited = backend
        .list_org_session_tasks(TEST_ORG_ID, None, None, None, None, 1)
        .await
        .expect("list org A limited");
    assert!(a_limited.len() <= 1, "limit must bound the result set");

    // root_session_id filter (EVE-680): session_a is its own root (no parent),
    // so filtering on it returns exactly A's two tasks and never B's — and the
    // org boundary still applies alongside the root filter.
    let a_root = backend
        .list_org_session_tasks(TEST_ORG_ID, None, None, None, Some(session_a), 500)
        .await
        .expect("list org A by root");
    let a_root_ids = ids(&a_root);
    assert!(
        a_root_ids.contains(&a_sub) && a_root_ids.contains(&a_bg),
        "root filter must return the whole tree's tasks"
    );
    assert!(
        !a_root_ids.contains(&b_sub),
        "root filter must never cross the org boundary"
    );
}
