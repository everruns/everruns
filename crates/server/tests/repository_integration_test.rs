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

use everruns_server::api::common::Pagination;
use everruns_server::storage::{
    CreateAgentCapabilityRow, CreateAgentRow, CreateEventRow, CreateImageRow, CreateLlmModelRow,
    CreateLlmProviderRow, CreateMcpServerRow, CreateOrganizationRow, CreateSessionFileRow,
    CreateSessionRow, Database, StorageBackend, UpdateAgent, UpdateLlmModel, UpdateLlmProvider,
    UpdateOrganization, UpdateSession, UpdateSessionFile,
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
                name: "Repo Test Agent".to_string(),
                description: Some("Test description".to_string()),
                system_prompt: "Test prompt".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
            },
        )
        .await
        .expect("Failed to create agent");

    assert_eq!(agent.name, "Repo Test Agent");

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
    let agents = backend
        .list_agents(TEST_ORG_ID)
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
async fn test_agent_get_by_name() {
    let backend = create_test_backend().await;

    // Create unique agent
    let unique_name = format!("NameTest_{}", Uuid::now_v7());
    let agent = backend
        .create_agent(
            TEST_ORG_ID,
            CreateAgentRow {
                public_id: everruns_core::AgentId::new().to_string(),
                name: unique_name.clone(),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
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
                name: "Session Test Agent".to_string(),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
            },
        )
        .await
        .expect("Failed to create agent");

    // Create session
    let session = backend
        .create_session(CreateSessionRow {
            org_id: TEST_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            title: Some("Test Session".to_string()),
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
        })
        .await
        .expect("Failed to create session");

    assert_eq!(session.agent_id, Some(agent.id));

    // Get session
    let fetched = backend
        .get_session(TEST_ORG_ID, session.id)
        .await
        .expect("Failed to get session")
        .expect("Session not found");
    assert_eq!(fetched.id, session.id);

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

    // List sessions with pagination
    let (sessions, total) = backend
        .list_sessions(
            TEST_ORG_ID,
            Some(agent.id),
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
                name: "Event Test Agent".to_string(),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
            },
        )
        .await
        .expect("Failed to create agent");

    let session = backend
        .create_session(CreateSessionRow {
            org_id: TEST_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            title: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
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
        .list_events(session.id, None, None, &[])
        .await
        .expect("Failed to list events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, event.id);

    // List with since_id filter
    let events_since = backend
        .list_events(session.id, None, Some(event.id), &[])
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
                name: "Event Exclude Test Agent".to_string(),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
            },
        )
        .await
        .expect("Failed to create agent");

    let session = backend
        .create_session(CreateSessionRow {
            org_id: TEST_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            title: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
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
            &["output.message.delta".to_string()],
        )
        .await
        .expect("Failed to list events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "input.message");

    // Note: No cleanup - events are append-only so sessions with events cannot be deleted
}

// ============================================
// LLM Provider Repository Tests
// ============================================

#[tokio::test]
async fn test_llm_provider_crud() {
    let backend = create_test_backend().await;

    // Create provider
    let provider = backend
        .create_llm_provider(
            TEST_ORG_ID,
            CreateLlmProviderRow {
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
        .get_llm_provider(TEST_ORG_ID, provider.id.into())
        .await
        .expect("Failed to get provider")
        .expect("Provider not found");
    assert_eq!(fetched.id, provider.id);

    // Update provider
    let updated = backend
        .update_llm_provider(
            TEST_ORG_ID,
            provider.id.into(),
            UpdateLlmProvider {
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
        .list_llm_providers(TEST_ORG_ID)
        .await
        .expect("Failed to list providers");
    assert!(providers.iter().any(|p| p.id == provider.id));

    // Delete provider
    let deleted = backend
        .delete_llm_provider(TEST_ORG_ID, provider.id.into())
        .await
        .expect("Failed to delete provider");
    assert!(deleted);
}

// ============================================
// LLM Model Repository Tests
// ============================================

#[tokio::test]
async fn test_llm_model_crud() {
    let backend = create_test_backend().await;

    // Create provider first
    let provider = backend
        .create_llm_provider(
            TEST_ORG_ID,
            CreateLlmProviderRow {
                name: "Model Test Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("Failed to create provider");

    // Create model
    let model = backend
        .create_llm_model(
            TEST_ORG_ID,
            CreateLlmModelRow {
                provider_id: provider.id,
                model_id: "gpt-4-test".to_string(),
                display_name: "GPT-4 Test".to_string(),
                capabilities: vec!["chat".to_string()],
                is_default: false,
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
        .get_llm_model(TEST_ORG_ID, model.id.into())
        .await
        .expect("Failed to get model")
        .expect("Model not found");
    assert_eq!(fetched.id, model.id);

    // Get model with provider
    let with_provider = backend
        .get_llm_model_with_provider(TEST_ORG_ID, model.id.into())
        .await
        .expect("Failed to get model with provider")
        .expect("Model not found");
    assert_eq!(with_provider.provider_id, provider.id);

    // Update model
    let updated = backend
        .update_llm_model(
            TEST_ORG_ID,
            model.id.into(),
            UpdateLlmModel {
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
        .list_llm_models_for_provider(TEST_ORG_ID, provider.id.into())
        .await
        .expect("Failed to list models");
    assert!(models.iter().any(|m| m.id == model.id));

    // Cleanup
    backend
        .delete_llm_model(TEST_ORG_ID, model.id.into())
        .await
        .unwrap();
    backend
        .delete_llm_provider(TEST_ORG_ID, provider.id.into())
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
                name: "File Test Agent".to_string(),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
            },
        )
        .await
        .expect("Failed to create agent");

    let session = backend
        .create_session(CreateSessionRow {
            org_id: TEST_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            title: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
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
        .list_mcp_servers(TEST_ORG_ID)
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
                name: "Capability Test Agent".to_string(),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
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

    // Delete organization
    let deleted = backend
        .delete_organization(org.org_id)
        .await
        .expect("Failed to delete organization");
    assert!(deleted);
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
                name: "Usage Test Agent".to_string(),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
            },
        )
        .await
        .expect("Failed to create agent");

    let session = backend
        .create_session(CreateSessionRow {
            org_id: TEST_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            title: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
        })
        .await
        .expect("Failed to create session");

    // Increment session usage
    backend
        .increment_session_usage(session.id.into(), 100, 50, 0, 0)
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
        .increment_session_usage(session.id.into(), 200, 100, 50, 10)
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
                name: "Preview Test Agent".to_string(),
                description: None,
                system_prompt: "Test".to_string(),
                default_model_id: None,
                tags: vec![],
                tools: serde_json::json!([]),
            },
        )
        .await
        .expect("Failed to create agent");

    let session = backend
        .create_session(CreateSessionRow {
            org_id: TEST_ORG_ID,
            harness_id: None,
            agent_id: Some(agent.id),
            title: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
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
            .list_mcp_servers(TEST_ORG_ID)
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
            .list_mcp_servers(org2)
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
async fn test_llm_provider_org_isolation_postgres() {
    let backend = create_test_backend().await;
    let org2 = create_test_org(&backend, "Provider Isolation Org").await;

    let provider = backend
        .create_llm_provider(
            TEST_ORG_ID,
            CreateLlmProviderRow {
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
            .get_llm_provider(TEST_ORG_ID, provider.id.uuid())
            .await
            .unwrap()
            .is_some()
    );

    // Negative
    assert!(
        backend
            .get_llm_provider(org2, provider.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        !backend
            .list_llm_providers(org2)
            .await
            .unwrap()
            .iter()
            .any(|p| p.id == provider.id)
    );
    assert!(
        !backend
            .delete_llm_provider(org2, provider.id.uuid())
            .await
            .unwrap()
    );

    // Cleanup
    backend
        .delete_llm_provider(TEST_ORG_ID, provider.id.uuid())
        .await
        .unwrap();
}

#[tokio::test]
async fn test_llm_model_org_isolation_postgres() {
    let backend = create_test_backend().await;
    let org2 = create_test_org(&backend, "Model Isolation Org").await;

    let provider = backend
        .create_llm_provider(
            TEST_ORG_ID,
            CreateLlmProviderRow {
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
        .create_llm_model(
            TEST_ORG_ID,
            CreateLlmModelRow {
                provider_id: provider.id,
                model_id: format!("model-{}", Uuid::now_v7()),
                display_name: "Test Model".to_string(),
                capabilities: vec![],
                is_default: false,
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
            .get_llm_model(TEST_ORG_ID, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        backend
            .get_llm_model_with_provider(TEST_ORG_ID, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );

    // Negative
    assert!(
        backend
            .get_llm_model(org2, model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .get_llm_model_with_provider(org2, model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        backend
            .list_llm_models_for_provider(org2, provider.id.uuid())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !backend
            .delete_llm_model(org2, model.id.uuid())
            .await
            .unwrap()
    );

    // Cleanup
    backend
        .delete_llm_model(TEST_ORG_ID, model.id.uuid())
        .await
        .unwrap();
    backend
        .delete_llm_provider(TEST_ORG_ID, provider.id.uuid())
        .await
        .unwrap();
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
