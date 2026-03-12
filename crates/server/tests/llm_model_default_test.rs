//! Tests for LLM model is_default uniqueness constraint
//!
//! Verifies that only one model can be the default at a time,
//! and that setting a new default clears the previous one.
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

fn model_input(provider_id: everruns_core::ProviderId, name: &str, is_default: bool) -> CreateLlmModelRow {
    CreateLlmModelRow {
        provider_id,
        model_id: name.to_string(),
        display_name: name.to_string(),
        capabilities: vec![],
        is_default,
        is_favorite: false,
        source: "manual".to_string(),
        provider_metadata: None,
    }
}

#[tokio::test]
async fn test_create_model_clears_previous_default() {
    let (db, pid) = setup().await;

    // Create first model as default
    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");
    assert!(m1.is_default);

    // Create second model as default — should clear the first
    let m2 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-b", true))
        .await
        .expect("create model-b");
    assert!(m2.is_default);

    // Verify first model is no longer default
    let m1_fetched = db
        .get_llm_model(TEST_ORG_ID, m1.id.into())
        .await
        .expect("get model-a")
        .expect("model-a exists");
    assert!(!m1_fetched.is_default, "previous default should be cleared");
}

#[tokio::test]
async fn test_create_non_default_does_not_clear_existing_default() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");
    assert!(m1.is_default);

    // Create non-default model — should not affect existing default
    db.create_llm_model(TEST_ORG_ID, model_input(pid, "model-b", false))
        .await
        .expect("create model-b");

    let m1_fetched = db
        .get_llm_model(TEST_ORG_ID, m1.id.into())
        .await
        .expect("get model-a")
        .expect("model-a exists");
    assert!(m1_fetched.is_default, "default should remain unchanged");
}

#[tokio::test]
async fn test_update_model_to_default_clears_previous() {
    let (db, pid) = setup().await;

    let m1 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-a", true))
        .await
        .expect("create model-a");
    let m2 = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "model-b", false))
        .await
        .expect("create model-b");

    // Update m2 to be default
    let updated = db
        .update_llm_model(
            TEST_ORG_ID,
            m2.id.into(),
            UpdateLlmModel {
                is_default: Some(true),
                ..Default::default()
            },
        )
        .await
        .expect("update model-b")
        .expect("model-b exists");
    assert!(updated.is_default);

    // m1 should no longer be default
    let m1_fetched = db
        .get_llm_model(TEST_ORG_ID, m1.id.into())
        .await
        .expect("get model-a")
        .expect("model-a exists");
    assert!(!m1_fetched.is_default, "previous default should be cleared after update");
}

#[tokio::test]
async fn test_seed_upsert_clears_previous_default() {
    let (db, pid) = setup().await;

    // Simulate: user manually set a model as default
    let user_model = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "user-model", true))
        .await
        .expect("create user model");
    assert!(user_model.is_default);

    // Seed inserts a model with is_default=true using a fixed ID
    let seed_id = Uuid::new_v4();
    let seed_result = db
        .create_llm_model_with_id(
            TEST_ORG_ID,
            seed_id,
            model_input(pid, "seed-default", true),
        )
        .await
        .expect("seed model");
    assert!(seed_result.is_some(), "seed should create the model");
    assert!(seed_result.unwrap().is_default);

    // User's model should no longer be default
    let user_fetched = db
        .get_llm_model(TEST_ORG_ID, user_model.id.into())
        .await
        .expect("get user model")
        .expect("user model exists");
    assert!(!user_fetched.is_default, "user default should be cleared by seed");
}

#[tokio::test]
async fn test_seed_upsert_update_path_clears_previous_default() {
    let (db, pid) = setup().await;

    // First seed: model-a is default
    let seed_id = Uuid::new_v4();
    db.create_llm_model_with_id(
        TEST_ORG_ID,
        seed_id,
        model_input(pid, "seed-model", true),
    )
    .await
    .expect("first seed");

    // User sets another model as default
    let user_model = db
        .create_llm_model(TEST_ORG_ID, model_input(pid, "user-choice", true))
        .await
        .expect("user creates default");

    // Verify seed model lost default
    let seed_fetched = db
        .get_llm_model(TEST_ORG_ID, seed_id)
        .await
        .expect("get seed")
        .expect("seed exists");
    assert!(!seed_fetched.is_default);

    // Re-seed: updates existing seed model back to default
    let reseed = db
        .create_llm_model_with_id(
            TEST_ORG_ID,
            seed_id,
            model_input(pid, "seed-model", true),
        )
        .await
        .expect("re-seed");
    assert!(reseed.is_some(), "should detect is_default changed");
    assert!(reseed.unwrap().is_default);

    // User model should no longer be default
    let user_fetched = db
        .get_llm_model(TEST_ORG_ID, user_model.id.into())
        .await
        .expect("get user model")
        .expect("user model exists");
    assert!(!user_fetched.is_default, "user default cleared by re-seed");
}
