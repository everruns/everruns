//! Organization isolation tests
//!
//! Verifies that all org-scoped entities enforce proper tenant isolation:
//! resources created in one org are invisible to another org.
//!
//! Uses in-memory backend (no PostgreSQL required).
//!
//! Run with: cargo test -p everruns-server --test org_isolation_test

use serde_json::json;
use uuid::Uuid;

use everruns_server::storage::{
    CreateImageRow, CreateLlmModelRow, CreateLlmProviderRow, CreateMcpServerRow,
    CreateOrganizationRow, CreateSessionRow, InMemoryDatabase, StorageBackend, UpdateLlmModel,
    UpdateLlmProvider, UpdateMcpServer,
};

use everruns_server::api::common::verify_session_ownership;
use std::sync::Arc;

/// Helper: create a second org in the in-memory database.
/// The default org (org_id=1) already exists.
async fn create_second_org(db: &InMemoryDatabase) -> i64 {
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Second Org".to_string(),
            created_by: None,
        })
        .await
        .expect("Failed to create second org");
    org.org_id
}

const ORG1: i64 = 1; // Default org

// ============================================
// MCP Server Isolation Tests
// ============================================

#[tokio::test]
async fn test_mcp_server_positive_own_org() {
    let db = InMemoryDatabase::default();

    let server = db
        .create_mcp_server(
            ORG1,
            CreateMcpServerRow {
                name: "My Server".to_string(),
                description: None,
                url: "https://mcp.example.com".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Positive: own org can get, list, update, delete
    assert!(
        db.get_mcp_server(ORG1, server.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.get_mcp_server_by_name(ORG1, "My Server")
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        db.list_mcp_servers(ORG1, None, false).await.unwrap().len(),
        1
    );
    assert_eq!(db.list_active_mcp_servers(ORG1).await.unwrap().len(), 1);

    let updated = db
        .update_mcp_server(
            ORG1,
            server.id.uuid(),
            UpdateMcpServer {
                name: Some("Renamed".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated.is_some());
    assert_eq!(updated.unwrap().name, "Renamed");

    assert!(db.delete_mcp_server(ORG1, server.id.uuid()).await.unwrap());
}

#[tokio::test]
async fn test_mcp_server_negative_cross_org() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let server = db
        .create_mcp_server(
            ORG1,
            CreateMcpServerRow {
                name: "Org1 Server".to_string(),
                description: None,
                url: "https://mcp.example.com".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Negative: other org cannot see, update, or delete
    assert!(
        db.get_mcp_server(org2, server.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_mcp_server_by_name(org2, "Org1 Server")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.list_mcp_servers(org2, None, false)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(db.list_active_mcp_servers(org2).await.unwrap().is_empty());
    assert!(
        db.update_mcp_server(
            org2,
            server.id.uuid(),
            UpdateMcpServer {
                name: Some("Hacked".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(!db.delete_mcp_server(org2, server.id.uuid()).await.unwrap());

    // Verify original is untouched
    let original = db
        .get_mcp_server(ORG1, server.id.uuid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.name, "Org1 Server");
}

// ============================================
// LLM Provider Isolation Tests
// ============================================

#[tokio::test]
async fn test_llm_provider_positive_own_org() {
    let db = InMemoryDatabase::default();

    let provider = db
        .create_llm_provider(
            ORG1,
            CreateLlmProviderRow {
                name: "My Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Positive: own org can get, list, update, delete
    assert!(
        db.get_llm_provider(ORG1, provider.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(db.list_llm_providers(ORG1).await.unwrap().len(), 1);

    let updated = db
        .update_llm_provider(
            ORG1,
            provider.id.uuid(),
            UpdateLlmProvider {
                name: Some("Renamed Provider".to_string()),
                provider_type: None,
                base_url: None,
                api_key_encrypted: None,
                status: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    assert!(updated.is_some());
    assert_eq!(updated.unwrap().name, "Renamed Provider");

    assert!(
        db.delete_llm_provider(ORG1, provider.id.uuid())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_llm_provider_negative_cross_org() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let provider = db
        .create_llm_provider(
            ORG1,
            CreateLlmProviderRow {
                name: "Org1 Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Negative: other org cannot see, update, or delete
    assert!(
        db.get_llm_provider(org2, provider.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(db.list_llm_providers(org2).await.unwrap().is_empty());
    assert!(
        db.update_llm_provider(
            org2,
            provider.id.uuid(),
            UpdateLlmProvider {
                name: Some("Hacked".to_string()),
                provider_type: None,
                base_url: None,
                api_key_encrypted: None,
                status: None,
                settings: None,
            },
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        !db.delete_llm_provider(org2, provider.id.uuid())
            .await
            .unwrap()
    );

    // Verify original is untouched
    let original = db
        .get_llm_provider(ORG1, provider.id.uuid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.name, "Org1 Provider");
}

// ============================================
// LLM Model Isolation Tests
// ============================================

#[tokio::test]
async fn test_llm_model_positive_own_org() {
    let db = InMemoryDatabase::default();

    let provider = db
        .create_llm_provider(
            ORG1,
            CreateLlmProviderRow {
                name: "Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let model = db
        .create_llm_model(
            ORG1,
            CreateLlmModelRow {
                provider_id: provider.id,
                model_id: "gpt-4".to_string(),
                display_name: "GPT-4".to_string(),
                capabilities: vec![],
                installed: false,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    // Positive: own org can get, list, update, delete
    assert!(
        db.get_llm_model(ORG1, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.get_llm_model_with_provider(ORG1, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        db.list_llm_models_for_provider(ORG1, provider.id.uuid())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(db.list_all_llm_models(ORG1).await.unwrap().len(), 1);

    let updated = db
        .update_llm_model(
            ORG1,
            model.id.uuid(),
            UpdateLlmModel {
                display_name: Some("GPT-4 Turbo".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated.is_some());
    assert_eq!(updated.unwrap().display_name, "GPT-4 Turbo");

    assert!(db.delete_llm_model(ORG1, model.id.uuid()).await.unwrap());
}

#[tokio::test]
async fn test_llm_model_negative_cross_org() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let provider = db
        .create_llm_provider(
            ORG1,
            CreateLlmProviderRow {
                name: "Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let model = db
        .create_llm_model(
            ORG1,
            CreateLlmModelRow {
                provider_id: provider.id,
                model_id: "gpt-4".to_string(),
                display_name: "GPT-4".to_string(),
                capabilities: vec![],
                installed: false,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    // Negative: other org cannot see, update, or delete
    assert!(
        db.get_llm_model(org2, model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_llm_model_with_provider(org2, model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.list_llm_models_for_provider(org2, provider.id.uuid())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(db.list_all_llm_models(org2).await.unwrap().is_empty());
    assert!(
        db.update_llm_model(
            org2,
            model.id.uuid(),
            UpdateLlmModel {
                display_name: Some("Hacked".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(!db.delete_llm_model(org2, model.id.uuid()).await.unwrap());

    // Verify original is untouched
    let original = db
        .get_llm_model(ORG1, model.id.uuid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(original.display_name, "GPT-4");
}

#[tokio::test]
async fn test_llm_model_provider_reads_fail_closed_for_corrupt_cross_org_reference() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let foreign_provider = db
        .create_llm_provider(
            org2,
            CreateLlmProviderRow {
                name: "Org2 Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let corrupt_model = db
        .create_llm_model(
            ORG1,
            CreateLlmModelRow {
                provider_id: foreign_provider.id,
                model_id: "cross-org-provider-model".to_string(),
                display_name: "Corrupt Model".to_string(),
                capabilities: vec![],
                installed: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    assert!(
        db.get_llm_model(ORG1, corrupt_model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        db.get_llm_model_with_provider(ORG1, corrupt_model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_llm_model_by_model_id(ORG1, "cross-org-provider-model")
            .await
            .unwrap()
            .is_none()
    );
    assert!(db.list_all_llm_models(ORG1).await.unwrap().is_empty());
}

// ============================================
// Image Isolation Tests
// ============================================

#[tokio::test]
async fn test_image_positive_own_org() {
    let db = InMemoryDatabase::default();

    let image = db
        .create_image(
            ORG1,
            CreateImageRow {
                org_id: ORG1,
                filename: "test.png".to_string(),
                content_type: "image/png".to_string(),
                size_bytes: 1024,
                data: vec![0u8; 1024],
                thumbnail_data: None,
                thumbnail_content_type: None,
                metadata: json!({}),
            },
        )
        .await
        .unwrap();

    // Positive: own org can get, list, delete
    assert!(db.get_image(ORG1, image.id.uuid()).await.unwrap().is_some());
    assert!(
        db.get_image_info(ORG1, image.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(db.list_images(ORG1, 50, 0).await.unwrap().len(), 1);
    assert!(db.delete_image(ORG1, image.id.uuid()).await.unwrap());
}

#[tokio::test]
async fn test_image_negative_cross_org() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let image = db
        .create_image(
            ORG1,
            CreateImageRow {
                org_id: ORG1,
                filename: "secret.png".to_string(),
                content_type: "image/png".to_string(),
                size_bytes: 1024,
                data: vec![0u8; 1024],
                thumbnail_data: None,
                thumbnail_content_type: None,
                metadata: json!({}),
            },
        )
        .await
        .unwrap();

    // Negative: other org cannot see or delete
    assert!(db.get_image(org2, image.id.uuid()).await.unwrap().is_none());
    assert!(
        db.get_image_info(org2, image.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(db.list_images(org2, 50, 0).await.unwrap().is_empty());
    assert!(!db.delete_image(org2, image.id.uuid()).await.unwrap());

    // Verify original still accessible to owner
    assert!(db.get_image(ORG1, image.id.uuid()).await.unwrap().is_some());
}

// ============================================
// Cross-Entity Isolation: Multi-org scenario
// ============================================

#[tokio::test]
async fn test_multi_org_full_isolation() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    // Create resources in org1
    let org1_provider = db
        .create_llm_provider(
            ORG1,
            CreateLlmProviderRow {
                name: "Org1 Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let org1_model = db
        .create_llm_model(
            ORG1,
            CreateLlmModelRow {
                provider_id: org1_provider.id,
                model_id: "gpt-4".to_string(),
                display_name: "GPT-4".to_string(),
                capabilities: vec![],
                installed: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    let org1_server = db
        .create_mcp_server(
            ORG1,
            CreateMcpServerRow {
                name: "Org1 MCP".to_string(),
                description: None,
                url: "https://mcp1.example.com".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let org1_image = db
        .create_image(
            ORG1,
            CreateImageRow {
                org_id: ORG1,
                filename: "org1.png".to_string(),
                content_type: "image/png".to_string(),
                size_bytes: 100,
                data: vec![1u8; 100],
                thumbnail_data: None,
                thumbnail_content_type: None,
                metadata: json!({}),
            },
        )
        .await
        .unwrap();

    // Create resources in org2
    let org2_provider = db
        .create_llm_provider(
            org2,
            CreateLlmProviderRow {
                name: "Org2 Provider".to_string(),
                provider_type: "anthropic".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let org2_model = db
        .create_llm_model(
            org2,
            CreateLlmModelRow {
                provider_id: org2_provider.id,
                model_id: "claude-3".to_string(),
                display_name: "Claude 3".to_string(),
                capabilities: vec![],
                installed: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    let org2_server = db
        .create_mcp_server(
            org2,
            CreateMcpServerRow {
                name: "Org2 MCP".to_string(),
                description: None,
                url: "https://mcp2.example.com".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Verify org1 sees only its own resources
    let org1_providers = db.list_llm_providers(ORG1).await.unwrap();
    assert_eq!(org1_providers.len(), 1);
    assert_eq!(org1_providers[0].name, "Org1 Provider");

    let org1_models = db.list_all_llm_models(ORG1).await.unwrap();
    assert_eq!(org1_models.len(), 1);
    assert_eq!(org1_models[0].model_id, "gpt-4");

    let org1_servers = db.list_mcp_servers(ORG1, None, false).await.unwrap();
    assert_eq!(org1_servers.len(), 1);
    assert_eq!(org1_servers[0].name, "Org1 MCP");

    let org1_images = db.list_images(ORG1, 50, 0).await.unwrap();
    assert_eq!(org1_images.len(), 1);
    assert_eq!(org1_images[0].filename, "org1.png");

    // Verify org2 sees only its own resources
    let org2_providers = db.list_llm_providers(org2).await.unwrap();
    assert_eq!(org2_providers.len(), 1);
    assert_eq!(org2_providers[0].name, "Org2 Provider");

    let org2_models = db.list_all_llm_models(org2).await.unwrap();
    assert_eq!(org2_models.len(), 1);
    assert_eq!(org2_models[0].model_id, "claude-3");

    let org2_servers = db.list_mcp_servers(org2, None, false).await.unwrap();
    assert_eq!(org2_servers.len(), 1);
    assert_eq!(org2_servers[0].name, "Org2 MCP");

    // Verify cross-org access is denied for all entity types
    assert!(
        db.get_llm_provider(org2, org1_provider.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_llm_model(org2, org1_model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_mcp_server(org2, org1_server.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_image(org2, org1_image.id.uuid())
            .await
            .unwrap()
            .is_none()
    );

    assert!(
        db.get_llm_provider(ORG1, org2_provider.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_llm_model(ORG1, org2_model.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_mcp_server(ORG1, org2_server.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
}

// ============================================
// Default model isolation
// ============================================

#[tokio::test]
async fn test_default_model_org_isolation() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let provider = db
        .create_llm_provider(
            ORG1,
            CreateLlmProviderRow {
                name: "Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Set default model in org1
    let _model = db
        .create_llm_model(
            ORG1,
            CreateLlmModelRow {
                provider_id: provider.id,
                model_id: "gpt-4".to_string(),
                display_name: "GPT-4".to_string(),
                capabilities: vec![],
                installed: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    // Positive: org1 has a default model
    assert!(db.get_default_llm_model(ORG1).await.unwrap().is_some());

    // Negative: org2 does not see org1's default model
    assert!(db.get_default_llm_model(org2).await.unwrap().is_none());
}

#[tokio::test]
async fn test_default_llm_model_fails_closed_for_cross_org_provider_reference() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let foreign_provider = db
        .create_llm_provider(
            org2,
            CreateLlmProviderRow {
                name: "Org2 Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let corrupt_model = db
        .create_llm_model(
            ORG1,
            CreateLlmModelRow {
                provider_id: foreign_provider.id,
                model_id: "cross-org-default-model".to_string(),
                display_name: "Corrupt Default".to_string(),
                capabilities: vec![],
                installed: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    db.upsert_organization_settings(ORG1, Some(corrupt_model.id.uuid()))
        .await
        .unwrap();

    assert!(db.get_default_llm_model(ORG1).await.unwrap().is_none());
}

// ============================================
// Update does not leak across orgs
// ============================================

#[tokio::test]
async fn test_update_mcp_server_tools_cross_org() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let server = db
        .create_mcp_server(
            ORG1,
            CreateMcpServerRow {
                name: "Server".to_string(),
                description: None,
                url: "https://example.com".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Negative: org2 cannot update cached tools
    let result = db
        .update_mcp_server_tools(
            org2,
            server.id.uuid(),
            everruns_server::storage::UpdateMcpServerTools {
                cached_tools: json!([{"name": "hacked_tool"}]),
            },
        )
        .await
        .unwrap();
    assert!(result.is_none());

    // Verify original tools untouched (empty array from creation)
    let original = db
        .get_mcp_server(ORG1, server.id.uuid())
        .await
        .unwrap()
        .unwrap();
    let tools: Vec<serde_json::Value> =
        serde_json::from_value(original.cached_tools.clone()).unwrap_or_default();
    assert!(tools.is_empty());
}

// ============================================
// Session Subresource Ownership Tests
// ============================================

/// Helper: create a StorageBackend wrapping a fresh InMemoryDatabase.
fn make_backend() -> Arc<StorageBackend> {
    Arc::new(StorageBackend::in_memory())
}

/// Helper: create a second org via StorageBackend.
async fn create_second_org_backend(backend: &StorageBackend) -> i64 {
    let org = backend
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Second Org".to_string(),
            created_by: None,
        })
        .await
        .expect("Failed to create second org");
    org.org_id
}

/// Helper: create a session in the given org via StorageBackend.
async fn create_session_in_org(backend: &StorageBackend, org_id: i64) -> everruns_core::SessionId {
    let row = backend
        .create_session(CreateSessionRow {
            org_id,
            harness_id: None,
            agent_id: None,
            title: Some("Test Session".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            hints: None,
        })
        .await
        .unwrap();
    row.id
}

#[tokio::test]
async fn test_session_ownership_positive_own_org() {
    let backend = make_backend();

    let session_id = create_session_in_org(&backend, ORG1).await;

    // Same org can access session
    assert!(
        verify_session_ownership(&backend, ORG1, session_id)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn test_session_ownership_negative_cross_org() {
    let backend = make_backend();
    let org2 = create_second_org_backend(&backend).await;

    let session_id = create_session_in_org(&backend, ORG1).await;

    // Different org cannot access session - returns 404
    let result = verify_session_ownership(&backend, org2, session_id).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_session_ownership_negative_nonexistent_session() {
    let backend = make_backend();

    let fake_session_id = everruns_core::SessionId::new();

    // Non-existent session returns 404
    let result = verify_session_ownership(&backend, ORG1, fake_session_id).await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_session_files_cross_org_blocked() {
    // verify_session_ownership is called at the API layer before
    // any file service operation. We test the helper directly.
    let backend = make_backend();
    let org2 = create_second_org_backend(&backend).await;

    // Create session in org1, try to access from org2
    let session_id = create_session_in_org(&backend, ORG1).await;
    assert!(
        verify_session_ownership(&backend, org2, session_id)
            .await
            .is_err()
    );

    // Create session in org2, verify org2 can access
    let session_id_org2 = create_session_in_org(&backend, org2).await;
    assert!(
        verify_session_ownership(&backend, org2, session_id_org2)
            .await
            .is_ok()
    );

    // But org1 cannot access org2's session
    assert!(
        verify_session_ownership(&backend, ORG1, session_id_org2)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_session_storage_cross_org_blocked() {
    // Session storage (keys, secrets) is guarded by verify_session_ownership
    let backend = make_backend();
    let org2 = create_second_org_backend(&backend).await;

    let session_id = create_session_in_org(&backend, ORG1).await;

    // Org2 cannot pass ownership check
    assert!(
        verify_session_ownership(&backend, org2, session_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_session_databases_cross_org_blocked() {
    // Session databases are guarded by verify_session_ownership
    let backend = make_backend();
    let org2 = create_second_org_backend(&backend).await;

    let session_id = create_session_in_org(&backend, ORG1).await;

    // Org2 cannot pass ownership check
    assert!(
        verify_session_ownership(&backend, org2, session_id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_provider_last_synced_cross_org() {
    let db = InMemoryDatabase::default();
    let org2 = create_second_org(&db).await;

    let provider = db
        .create_llm_provider(
            ORG1,
            CreateLlmProviderRow {
                name: "Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Negative: org2 cannot update last_synced_at
    db.update_provider_last_synced(org2, provider.id.uuid(), chrono::Utc::now())
        .await
        .unwrap();

    // Verify original last_synced_at is still None
    let original = db
        .get_llm_provider(ORG1, provider.id.uuid())
        .await
        .unwrap()
        .unwrap();
    assert!(original.last_synced_at.is_none());
}
