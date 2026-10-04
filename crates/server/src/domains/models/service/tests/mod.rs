use super::*;
use crate::storage::models::CreateModelRow;
use crate::storage::{CreateOrganizationRow, CreateProviderRow};
use everruns_core::{DEFAULT_ORG_ID, PolicyError};

fn build_create_request() -> CreateModelRequest {
    CreateModelRequest {
        service: None,
        profile_key: None,
        model_id: "test-model".to_string(),
        display_name: "Test Model".to_string(),
        capabilities: vec!["chat".to_string()],
        enabled: true,
        is_favorite: false,
    }
}

async fn create_second_org(db: &StorageBackend) -> i64 {
    db.create_organization_with_id(
        2,
        CreateOrganizationRow {
            public_id: "org_2".to_string(),
            name: "Org 2".to_string(),
            created_by: None,
        },
    )
    .await
    .unwrap()
    .unwrap()
    .org_id
}

async fn create_provider(
    db: &StorageBackend,
    org_id: i64,
) -> everruns_contracts::typed_id::ProviderId {
    db.create_provider(
        org_id,
        CreateProviderRow {
            name: format!("Provider {org_id}"),
            provider_type: "openai".to_string(),
            base_url: None,
            api_key_encrypted: None,
            settings: None,
        },
    )
    .await
    .unwrap()
    .id
}

async fn create_model_with_foreign_provider(db: &StorageBackend) -> ModelRow {
    let foreign_org_id = create_second_org(db).await;
    let foreign_provider_id = create_provider(db, foreign_org_id).await;
    db.create_model(
        DEFAULT_ORG_ID,
        CreateModelRow {
            provider_id: foreign_provider_id,
            model_id: "cross-org-model".to_string(),
            display_name: "Cross-org Model".to_string(),
            capabilities: vec!["chat".to_string()],
            enabled: true,
            is_favorite: false,
            source: "manual".to_string(),
            provider_metadata: None,
        },
    )
    .await
    .unwrap()
}

async fn mark_provider_managed(db: &StorageBackend, provider_id: ProviderId) {
    assert!(
        db.set_provider_managed(DEFAULT_ORG_ID, provider_id.uuid(), true)
            .await
            .unwrap()
    );
}

fn assert_managed_policy_error(err: anyhow::Error) {
    assert!(err.downcast_ref::<PolicyError>().is_some(), "{err:#}");
}

/// Regression: provider creation now discovers a catalog inline, so an
/// explicit `POST /v1/providers/{id}/models` naming a discovered model used
/// to hit the `(provider_id, model_id)` unique index and 409. Six live
/// workflow tests do exactly that. The caller's settings must win, and the
/// capabilities discovery learned must survive an omitted field.
#[tokio::test]
async fn create_adopts_a_discovered_model_instead_of_conflicting() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let discovered = db
        .create_model(
            DEFAULT_ORG_ID,
            CreateModelRow {
                provider_id,
                model_id: "test-model".to_string(),
                display_name: "test-model".to_string(),
                capabilities: vec!["chat".to_string()],
                enabled: false,
                is_favorite: false,
                source: "discovered".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    let created = service
        .create(
            &caller,
            provider_id.uuid(),
            CreateModelRequest {
                service: None,
                profile_key: None,
                model_id: "test-model".to_string(),
                display_name: "Chosen Name".to_string(),
                capabilities: vec![],
                enabled: true,
                is_favorite: true,
            },
        )
        .await
        .expect("an explicit create must adopt the discovered row");

    assert_eq!(created.id, discovered.id, "adopted, not duplicated");
    assert_eq!(created.display_name, "Chosen Name");
    assert!(created.enabled);
    assert!(created.is_favorite);
    assert_eq!(
        created.capabilities,
        vec!["chat".to_string()],
        "an omitted capabilities list must not blank what discovery found"
    );
    assert_eq!(
        db.list_models_for_provider(DEFAULT_ORG_ID, provider_id.uuid())
            .await
            .unwrap()
            .len(),
        1,
        "adoption must not leave a second row"
    );
}

/// A second explicit create of the same id is a genuine duplicate and must
/// still fail — adoption only ever absorbs a system-discovered row.
#[tokio::test]
async fn create_still_rejects_a_duplicate_of_a_manual_model() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    service
        .create(&caller, provider_id.uuid(), build_create_request())
        .await
        .unwrap();

    let err = service
        .create(&caller, provider_id.uuid(), build_create_request())
        .await
        .unwrap_err();

    assert!(
        err.to_string().contains("duplicate")
            || err.to_string().contains("already")
            || err.to_string().contains("Unique"),
        "expected a duplicate error, got: {err:#}"
    );
}

// --- first-run intelligence bootstrap ---

async fn create_keyed_provider(
    db: &StorageBackend,
    org_id: i64,
) -> everruns_contracts::typed_id::ProviderId {
    db.create_provider(
        org_id,
        CreateProviderRow {
            name: "OpenAI".to_string(),
            provider_type: "openai".to_string(),
            base_url: None,
            api_key_encrypted: Some(b"encrypted".to_vec()),
            settings: None,
        },
    )
    .await
    .unwrap()
    .id
}

/// Mimics model discovery: catalog rows land disabled and unstarred.
async fn discover_model(
    db: &StorageBackend,
    org_id: i64,
    provider_id: everruns_contracts::typed_id::ProviderId,
    model_id: &str,
    capabilities: &[&str],
) -> ModelRow {
    db.create_model(
        org_id,
        CreateModelRow {
            provider_id,
            model_id: model_id.to_string(),
            display_name: model_id.to_string(),
            capabilities: capabilities.iter().map(|c| c.to_string()).collect(),
            enabled: false,
            is_favorite: false,
            source: "discovered".to_string(),
            provider_metadata: None,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn bootstrap_enables_favourites_and_elects_a_default() {
    let db = Arc::new(StorageBackend::in_memory());
    let org_id = create_second_org(&db).await;
    let service = ModelService::new(db.clone());
    let provider_id = create_keyed_provider(&db, org_id).await;

    for model_id in ["gpt-5.6-terra", "gpt-5.6-sol", "gpt-5.4-mini"] {
        discover_model(&db, org_id, provider_id, model_id, &["chat"]).await;
    }
    let embedding = discover_model(
        &db,
        org_id,
        provider_id,
        "text-embedding-3-small",
        &["embeddings"],
    )
    .await;

    // Precondition: discovery alone leaves the org unable to resolve a model.
    assert!(db.get_default_model(org_id).await.unwrap().is_none());

    let outcome = service
        .bootstrap_intelligence(org_id, provider_id.uuid())
        .await
        .unwrap();

    assert_eq!(outcome.enabled_models, 3);
    assert_eq!(outcome.favorited_models, 3);
    let default = db
        .get_default_model(org_id)
        .await
        .unwrap()
        .expect("default elected");
    assert_eq!(outcome.default_model_id, Some(default.id.uuid()));

    let rows = db
        .list_models_for_provider(org_id, provider_id.uuid())
        .await
        .unwrap();
    for row in &rows {
        if row.id == embedding.id {
            assert!(!row.enabled, "embedding model must stay disabled");
            assert!(!row.is_favorite);
        } else {
            assert!(row.enabled, "{} should be enabled", row.model_id);
            assert!(row.is_favorite, "{} should be starred", row.model_id);
        }
    }
}

/// No other OpenAI model wins the default election over the curated pick,
/// whether newer or pricier: GPT-6 Luna is the everyday default, GPT-6
/// Astra and Sol are the stronger tiers an operator opts into.
#[tokio::test]
async fn bootstrap_elects_luna_over_other_openai_models() {
    let db = Arc::new(StorageBackend::in_memory());
    let org_id = create_second_org(&db).await;
    let service = ModelService::new(db.clone());
    let provider_id = create_keyed_provider(&db, org_id).await;

    for model_id in ["gpt-6-astra", "gpt-6.1-sol", "gpt-5.6-terra", "gpt-6-luna"] {
        discover_model(&db, org_id, provider_id, model_id, &["chat"]).await;
    }

    service
        .bootstrap_intelligence(org_id, provider_id.uuid())
        .await
        .unwrap();

    let default = db
        .get_default_model(org_id)
        .await
        .unwrap()
        .expect("default elected");
    assert_eq!(default.model_id, "gpt-6-luna");
}

/// Disabling the org default must hand the org another *chat* model. An
/// embedding model is enabled like any other row, and electing one leaves a
/// default that resolves but no chat can use.
#[tokio::test]
async fn electing_a_new_default_skips_embedding_models() {
    let db = Arc::new(StorageBackend::in_memory());
    let org_id = create_second_org(&db).await;
    let service = ModelService::new(db.clone());
    let provider_id = create_keyed_provider(&db, org_id).await;

    let chat = discover_model(&db, org_id, provider_id, "gpt-5.6-terra", &["chat"]).await;
    let embedding = discover_model(
        &db,
        org_id,
        provider_id,
        "text-embedding-3-small",
        &["embeddings"],
    )
    .await;
    let doomed = discover_model(&db, org_id, provider_id, "gpt-5.4", &["chat"]).await;
    // `text-embedding-3-small` is seeded as an enabled favorite, and the
    // catalog lists favorites first — which is exactly how it won the
    // election on a real stack.
    for (id, favorite) in [
        (chat.id.uuid(), false),
        (embedding.id.uuid(), true),
        (doomed.id.uuid(), false),
    ] {
        db.update_model(
            org_id,
            id,
            UpdateModel {
                enabled: Some(true),
                is_favorite: favorite.then_some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }
    service.set_default(org_id, doomed.id.uuid()).await.unwrap();

    // Disabling the current default forces a re-election.
    service
        .update(
            &Caller::internal(org_id),
            doomed.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: None,
                model_id: None,
                display_name: None,
                capabilities: None,
                enabled: Some(false),
                is_favorite: None,
            },
        )
        .await
        .unwrap();

    let elected = db
        .get_default_model(org_id)
        .await
        .unwrap()
        .expect("a new default must be elected");
    assert_eq!(
        elected.model_id, "gpt-5.6-terra",
        "election must skip the embedding model"
    );
}

#[tokio::test]
async fn bootstrap_leaves_a_configured_org_alone() {
    let db = Arc::new(StorageBackend::in_memory());
    let org_id = create_second_org(&db).await;
    let service = ModelService::new(db.clone());
    let provider_id = create_keyed_provider(&db, org_id).await;

    let chosen = discover_model(&db, org_id, provider_id, "gpt-5.6-terra", &["chat"]).await;
    let other = discover_model(&db, org_id, provider_id, "gpt-5.6-sol", &["chat"]).await;
    db.update_model(
        org_id,
        chosen.id.uuid(),
        UpdateModel {
            enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    db.upsert_organization_settings(org_id, Some(chosen.id.uuid()))
        .await
        .unwrap();

    let outcome = service
        .bootstrap_intelligence(org_id, provider_id.uuid())
        .await
        .unwrap();

    assert_eq!(outcome, IntelligenceBootstrap::default());
    let rows = db
        .list_models_for_provider(org_id, provider_id.uuid())
        .await
        .unwrap();
    let other_row = rows.iter().find(|row| row.id == other.id).unwrap();
    assert!(
        !other_row.enabled,
        "a deliberate opt-out must not be overridden"
    );
}

#[tokio::test]
async fn bootstrap_skips_a_provider_without_a_key() {
    let db = Arc::new(StorageBackend::in_memory());
    let org_id = create_second_org(&db).await;
    let service = ModelService::new(db.clone());
    let provider_id = create_provider(&db, org_id).await;
    discover_model(&db, org_id, provider_id, "gpt-5.6-terra", &["chat"]).await;

    let outcome = service
        .bootstrap_intelligence(org_id, provider_id.uuid())
        .await
        .unwrap();

    assert_eq!(outcome, IntelligenceBootstrap::default());
    assert!(db.get_default_model(org_id).await.unwrap().is_none());
}

#[tokio::test]
async fn bootstrap_falls_back_to_one_model_for_an_unknown_catalog() {
    let db = Arc::new(StorageBackend::in_memory());
    let org_id = create_second_org(&db).await;
    let service = ModelService::new(db.clone());
    let provider_id = create_keyed_provider(&db, org_id).await;
    for model_id in ["house-llm-a", "house-llm-b"] {
        discover_model(&db, org_id, provider_id, model_id, &["chat"]).await;
    }

    let outcome = service
        .bootstrap_intelligence(org_id, provider_id.uuid())
        .await
        .unwrap();

    assert_eq!(outcome.enabled_models, 1);
    let default = db
        .get_default_model(org_id)
        .await
        .unwrap()
        .expect("default elected");
    assert_eq!(default.model_id, "house-llm-a");
}

#[tokio::test]
async fn create_rejects_managed_provider() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    mark_provider_managed(&db, provider_id).await;

    let err = service
        .create(&caller, provider_id.uuid(), build_create_request())
        .await
        .unwrap_err();

    assert_managed_policy_error(err);
}

#[tokio::test]
async fn managed_model_allows_preference_updates_only() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let model = service
        .create(&caller, provider_id.uuid(), build_create_request())
        .await
        .unwrap();
    mark_provider_managed(&db, provider_id).await;

    let updated = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: None,
                model_id: None,
                display_name: None,
                capabilities: None,
                enabled: Some(false),
                is_favorite: Some(true),
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert!(!updated.enabled);
    assert!(updated.is_favorite);
}

/// Regression: enabling a disabled model 404'd because the precondition
/// read used the resolution-path `get_model`, which filters
/// `enabled = TRUE`. Every freshly discovered model is disabled, so the
/// models page could never enable anything.
#[tokio::test]
async fn update_can_enable_a_disabled_model() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let model = service
        .create(
            &caller,
            provider_id.uuid(),
            CreateModelRequest {
                enabled: false,
                ..build_create_request()
            },
        )
        .await
        .unwrap();
    assert!(!model.enabled);

    let updated = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: None,
                model_id: None,
                display_name: None,
                capabilities: None,
                enabled: Some(true),
                is_favorite: None,
            },
        )
        .await
        .unwrap()
        .expect("disabled model must be updatable");

    assert!(updated.enabled);
}

/// The managed-catalog guard must also cover disabled rows: reading the
/// existing model through an enabled-only lookup silently skipped it.
#[tokio::test]
async fn managed_disabled_model_rejects_catalog_update_and_delete() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let model = service
        .create(
            &caller,
            provider_id.uuid(),
            CreateModelRequest {
                enabled: false,
                ..build_create_request()
            },
        )
        .await
        .unwrap();
    mark_provider_managed(&db, provider_id).await;

    let err = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: None,
                model_id: None,
                display_name: Some("Renamed".to_string()),
                capabilities: None,
                enabled: None,
                is_favorite: None,
            },
        )
        .await
        .unwrap_err();
    assert_managed_policy_error(err);

    let err = service.delete(&caller, model.id.uuid()).await.unwrap_err();
    assert_managed_policy_error(err);
    assert!(
        db.get_model_for_mutation(DEFAULT_ORG_ID, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn model_with_foreign_provider_link_cannot_be_deleted() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let model = create_model_with_foreign_provider(&db).await;

    let err = service.delete(&caller, model.id.uuid()).await.unwrap_err();

    assert_eq!(err.to_string(), "Provider not found");
    assert!(
        db.get_model_for_mutation(DEFAULT_ORG_ID, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn model_with_foreign_provider_link_cannot_be_updated() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let model = create_model_with_foreign_provider(&db).await;

    let err = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: None,
                model_id: None,
                display_name: Some("Renamed".to_string()),
                capabilities: None,
                enabled: None,
                is_favorite: None,
            },
        )
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "Provider not found");
    assert_eq!(
        db.get_model_for_mutation(DEFAULT_ORG_ID, model.id.uuid())
            .await
            .unwrap()
            .unwrap()
            .display_name,
        "Cross-org Model"
    );
}

#[tokio::test]
async fn managed_model_rejects_catalog_update_and_delete() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let model = service
        .create(&caller, provider_id.uuid(), build_create_request())
        .await
        .unwrap();
    mark_provider_managed(&db, provider_id).await;

    let err = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: None,
                model_id: Some("unauthorized-model".to_string()),
                display_name: None,
                capabilities: None,
                enabled: None,
                is_favorite: None,
            },
        )
        .await
        .unwrap_err();
    assert_managed_policy_error(err);

    let err = service.delete(&caller, model.id.uuid()).await.unwrap_err();
    assert_managed_policy_error(err);
    assert!(
        db.get_model(DEFAULT_ORG_ID, model.id.uuid())
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn update_cannot_move_model_to_managed_provider() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let first_provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let managed_provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    mark_provider_managed(&db, managed_provider_id).await;
    let model = service
        .create(&caller, first_provider_id.uuid(), build_create_request())
        .await
        .unwrap();

    let err = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: Some(managed_provider_id.to_string()),
                model_id: None,
                display_name: None,
                capabilities: None,
                enabled: None,
                is_favorite: None,
            },
        )
        .await
        .unwrap_err();

    assert_managed_policy_error(err);
}

#[tokio::test]
async fn create_rejects_provider_from_another_org() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let other_org_id = create_second_org(&db).await;
    let other_provider_id = create_provider(&db, other_org_id).await;

    let err = service
        .create(&caller, other_provider_id.uuid(), build_create_request())
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "Provider not found");
}

#[tokio::test]
async fn update_can_move_model_to_another_provider_in_same_org() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let first_provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let second_provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let model = service
        .create(&caller, first_provider_id.uuid(), build_create_request())
        .await
        .unwrap();

    let updated = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: Some(second_provider_id.to_string()),
                model_id: None,
                display_name: None,
                capabilities: None,
                enabled: None,
                is_favorite: None,
            },
        )
        .await
        .unwrap()
        .unwrap();

    assert_eq!(updated.provider_id, second_provider_id);
}

#[tokio::test]
async fn update_rejects_provider_from_another_org() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let other_org_id = create_second_org(&db).await;
    let other_provider_id = create_provider(&db, other_org_id).await;
    let model = service
        .create(&caller, provider_id.uuid(), build_create_request())
        .await
        .unwrap();

    let err = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                service: None,
                profile_key: None,
                provider_id: Some(other_provider_id.to_string()),
                model_id: None,
                display_name: None,
                capabilities: None,
                enabled: None,
                is_favorite: None,
            },
        )
        .await
        .unwrap_err();

    assert_eq!(err.to_string(), "Provider not found");
}

mod private_provider_tests;
mod profile_merge_tests;

#[tokio::test]
async fn explicit_profile_survives_preference_updates_with_unchanged_identity() {
    let db = Arc::new(StorageBackend::in_memory());
    let service = ModelService::new(db.clone());
    let caller = Caller::internal(DEFAULT_ORG_ID);
    let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
    let mut request = build_create_request();
    request.profile_key = Some("openai/gpt-6-sol".into());
    let model = service
        .create(&caller, provider_id.uuid(), request)
        .await
        .unwrap();
    let updated = service
        .update(
            &caller,
            model.id.uuid(),
            UpdateModelRequest {
                provider_id: Some(provider_id.to_string()),
                model_id: Some(model.model_id.clone()),
                service: Some(everruns_contracts::ServiceKind::Chat),
                is_favorite: Some(true),
                display_name: Some("Preferred label".into()),
                profile_key: None,
                capabilities: None,
                enabled: None,
            },
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.profile_key, "openai/gpt-6-sol");
    assert!(updated.is_favorite);
    assert_eq!(updated.display_name, "Preferred label");
}
