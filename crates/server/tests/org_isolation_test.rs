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
    CreateOrganizationRow, InMemoryDatabase, UpdateLlmModel, UpdateLlmProvider, UpdateMcpServer,
};

/// Helper: create a second org in the in-memory database.
/// The default org (org_id=1) already exists.
async fn create_second_org(db: &InMemoryDatabase) -> i64 {
    let org = db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: "Second Org".to_string(),
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
                auth_type: "none".to_string(),
                oauth_authorization_url: None,
                oauth_token_url: None,
                oauth_client_id: None,
                oauth_client_secret_encrypted: None,
                oauth_scopes: None,
                oauth_resource_metadata_url: None,
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
    assert_eq!(db.list_mcp_servers(ORG1).await.unwrap().len(), 1);
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
                auth_type: "none".to_string(),
                oauth_authorization_url: None,
                oauth_token_url: None,
                oauth_client_id: None,
                oauth_client_secret_encrypted: None,
                oauth_scopes: None,
                oauth_resource_metadata_url: None,
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
    assert!(db.list_mcp_servers(org2).await.unwrap().is_empty());
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
                is_default: false,
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
                is_default: false,
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
                is_default: true,
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
                auth_type: "none".to_string(),
                oauth_authorization_url: None,
                oauth_token_url: None,
                oauth_client_id: None,
                oauth_client_secret_encrypted: None,
                oauth_scopes: None,
                oauth_resource_metadata_url: None,
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
                is_default: true,
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
                auth_type: "none".to_string(),
                oauth_authorization_url: None,
                oauth_token_url: None,
                oauth_client_id: None,
                oauth_client_secret_encrypted: None,
                oauth_scopes: None,
                oauth_resource_metadata_url: None,
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

    let org1_servers = db.list_mcp_servers(ORG1).await.unwrap();
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

    let org2_servers = db.list_mcp_servers(org2).await.unwrap();
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
                is_default: true,
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
                auth_type: "none".to_string(),
                oauth_authorization_url: None,
                oauth_token_url: None,
                oauth_client_id: None,
                oauth_client_secret_encrypted: None,
                oauth_scopes: None,
                oauth_resource_metadata_url: None,
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
