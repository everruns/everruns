//! Model catalog seeding: the platform default, the current-generation
//! models, and idempotent upserts of seeded model rows.

use super::*;
use crate::storage::StorageBackend;

fn make_db() -> StorageBackend {
    StorageBackend::test_database()
}

/// Discovery can catalogue a model under a generated id before the seed
/// entry for it exists. Seeding must adopt that row rather than trip the
/// `(provider_id, model_id)` unique index on every boot.
#[tokio::test]
async fn test_seed_all_adopts_model_discovered_before_seed() {
    let db = make_db();
    // Seed once so the provider rows exist, then plant a discovered twin
    // of a seed model under a different id.
    seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();
    let seeded = db
        .get_model(DEFAULT_ORG_ID, seed_ids::TEXT_EMBEDDING_3_SMALL)
        .await
        .unwrap()
        .expect("seed model present");
    assert!(
        db.delete_model(DEFAULT_ORG_ID, seeded.id.uuid())
            .await
            .unwrap()
    );
    let discovered = db
        .create_model(
            DEFAULT_ORG_ID,
            CreateModelRow {
                provider_id: seed_ids::OPENAI_PROVIDER.into(),
                model_id: "text-embedding-3-small".to_string(),
                display_name: "text-embedding-3-small".to_string(),
                capabilities: vec![],
                is_favorite: false,
                enabled: false,
                source: "discovered".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    assert_ne!(discovered.id.uuid(), seed_ids::TEXT_EMBEDDING_3_SMALL);

    let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .expect("seeding must not fail on a discovered twin");
    assert!(result.updated >= 1, "discovered row must be updated");

    let models = db
        .list_models_for_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
        .await
        .unwrap();
    let twins: Vec<_> = models
        .iter()
        .filter(|m| m.model_id == "text-embedding-3-small")
        .collect();
    assert_eq!(twins.len(), 1, "exactly one row per provider/model");
    assert_eq!(twins[0].id, discovered.id, "discovered row is kept");
    assert!(
        twins[0].enabled && twins[0].is_favorite,
        "seed fields applied"
    );
    assert_eq!(twins[0].display_name, "Text Embedding 3 Small");

    // A further run is a no-op.
    let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();
    assert_eq!(result.updated, 0);
}

// --- Model upsert ---

#[tokio::test]
async fn test_model_seed_detects_display_name_change() {
    let db = make_db();
    let _ = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();

    // Mutate model display_name via public API
    db.update_model(
        DEFAULT_ORG_ID,
        seed_ids::GPT_5_2,
        UpdateModel {
            display_name: Some("STALE".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let result = seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();
    assert!(
        result.updated >= 1,
        "should detect model display_name change"
    );
}
