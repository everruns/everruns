//! Tests for LLM model enabled flag and organization default model
//!
//! Verifies that:
//! - Models can be enabled/disabled independently
//! - Organization settings control the default model
//! - Disabling the org default model elects a new one
//!
//! Uses in-memory backend (no PostgreSQL required).
//!
//! Run with: cargo test -p everruns-server --test llm_model_default_test

use uuid::Uuid;

use everruns_server::storage::{
    CreateLlmModelRow, CreateLlmProviderRow, InMemoryDatabase, UpdateLlmModel,
};

const TEST_ORG_ID: i64 = 1;

async fn setup() -> (InMemoryDatabase, everruns_core::ProviderId) {
    let db = InMemoryDatabase::default();
    let provider = db
        .create_llm_provider(
            TEST_ORG_ID,
            CreateLlmProviderRow {
                name: "Test Provider".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .expect("create provider");
    (db, provider.id)
}

fn model_input(
    provider_id: everruns_core::ProviderId,
    name: &str,
    enabled: bool,
) -> CreateLlmModelRow {
    CreateLlmModelRow {
        provider_id,
        model_id: name.to_string(),
        display_name: name.to_string(),
        capabilities: vec![],
        enabled,
        is_favorite: false,
        source: "manual".to_string(),
        provider_metadata: None,
    }
}

#[tokio::test]
async fn test_create_installed_model() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");
    assert!(m1.enabled);

    let m2 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-b", false))
        .await
        .expect("create model-b");
    assert!(!m2.enabled);
}

#[tokio::test]
async fn test_multiple_models_can_be_installed() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");
    let m2 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-b", true))
        .await
        .expect("create model-b");

    // Both should be enabled (unlike is_default, multiple models can be enabled)
    assert!(m1.enabled);
    assert!(m2.enabled);

    // Verify both are still enabled after fetch
    let m1_fetched = db
        .get_llm_model(TEST_ORG_ID, m1.id.into())
        .await
        .expect("get model-a")
        .expect("model-a exists");
    let m2_fetched = db
        .get_llm_model(TEST_ORG_ID, m2.id.into())
        .await
        .expect("get model-b")
        .expect("model-b exists");
    assert!(m1_fetched.enabled);
    assert!(m2_fetched.enabled);
}

#[tokio::test]
async fn test_update_installed_flag() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", false))
        .await
        .expect("create model-a");
    assert!(!m1.enabled);

    // Install the model
    let updated = db
        .update_llm_model(
            TEST_ORG_ID,
            m1.id.into(),
            UpdateLlmModel {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("update model-a")
        .expect("model-a exists");
    assert!(updated.enabled);

    // Disable
    let updated2 = db
        .update_llm_model(
            TEST_ORG_ID,
            m1.id.into(),
            UpdateLlmModel {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("update model-a")
        .expect("model-a exists");
    assert!(!updated2.enabled);
}

#[tokio::test]
async fn test_org_settings_default_model() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");

    // Set org default model
    let settings = db
        .upsert_organization_settings(TEST_ORG_ID, Some(m1.id.uuid()))
        .await
        .expect("upsert org settings");
    assert_eq!(settings.default_model_id.unwrap().uuid(), m1.id.uuid());

    // Get default model should return model-a
    let default = db
        .get_default_llm_model(TEST_ORG_ID)
        .await
        .expect("get default model")
        .expect("default exists");
    assert_eq!(default.id.uuid(), m1.id.uuid());
}

#[tokio::test]
async fn test_org_settings_upsert_updates_existing() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");
    let m2 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-b", true))
        .await
        .expect("create model-b");

    // Set default to m1
    db.upsert_organization_settings(TEST_ORG_ID, Some(m1.id.uuid()))
        .await
        .expect("set default to m1");

    // Update default to m2
    let settings = db
        .upsert_organization_settings(TEST_ORG_ID, Some(m2.id.uuid()))
        .await
        .expect("set default to m2");
    assert_eq!(settings.default_model_id.unwrap().uuid(), m2.id.uuid());

    // Verify default is now m2
    let default = db
        .get_default_llm_model(TEST_ORG_ID)
        .await
        .expect("get default model")
        .expect("default exists");
    assert_eq!(default.id.uuid(), m2.id.uuid());
}

#[tokio::test]
async fn test_seed_upsert_sets_installed() {
    let (db, pid) = setup().await;

    let seed_id = Uuid::new_v4();
    let result = db
        .create_llm_model_with_id(TEST_ORG_ID, seed_id, model_input(pid, "seed-model", true))
        .await
        .expect("seed model");
    assert!(result.is_some(), "seed should create the model");
    assert!(result.unwrap().enabled);

    // Re-seed should detect no change
    let reseed = db
        .create_llm_model_with_id(TEST_ORG_ID, seed_id, model_input(pid, "seed-model", true))
        .await
        .expect("re-seed");
    assert!(reseed.is_none(), "no change on identical re-seed");
}

#[tokio::test]
async fn test_get_default_model_returns_none_without_org_settings() {
    let (db, pid) = setup().await;

    // Create an enabled model but don't set org settings
    db.create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");

    // No org settings → no default
    let default = db
        .get_default_llm_model(TEST_ORG_ID)
        .await
        .expect("get default model");
    assert!(default.is_none(), "no default without org settings");
}

#[tokio::test]
async fn test_org_settings_clear_default() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");

    // Set default
    db.upsert_organization_settings(TEST_ORG_ID, Some(m1.id.uuid()))
        .await
        .expect("set default");

    // Clear default to None
    let settings = db
        .upsert_organization_settings(TEST_ORG_ID, None)
        .await
        .expect("clear default");
    assert!(settings.default_model_id.is_none());

    // get_default should return None
    let default = db
        .get_default_llm_model(TEST_ORG_ID)
        .await
        .expect("get default model");
    assert!(default.is_none(), "default should be cleared");
}

#[tokio::test]
async fn test_get_organization_settings_returns_none_initially() {
    let db = InMemoryDatabase::default();

    let settings = db
        .get_organization_settings(TEST_ORG_ID)
        .await
        .expect("get org settings");
    assert!(settings.is_none(), "no settings initially");
}

#[tokio::test]
async fn test_delete_model_that_is_org_default() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");

    // Set as org default
    db.upsert_organization_settings(TEST_ORG_ID, Some(m1.id.uuid()))
        .await
        .expect("set default");

    // Delete the model
    let deleted = db
        .delete_llm_model(TEST_ORG_ID, m1.id.into())
        .await
        .expect("delete model");
    assert!(deleted);

    // Default model should now return None (model is gone)
    let default = db
        .get_default_llm_model(TEST_ORG_ID)
        .await
        .expect("get default model");
    assert!(
        default.is_none(),
        "default should be None after deleting the model"
    );
}
