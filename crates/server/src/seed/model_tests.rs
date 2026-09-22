//! Model catalog seeding: the platform default, the current-generation
//! models, and idempotent upserts of seeded model rows.

use super::*;
use crate::storage::StorageBackend;

fn make_db() -> StorageBackend {
    StorageBackend::in_memory()
}

#[tokio::test]
async fn test_seed_all_leaves_the_org_model_override_unset() {
    let db = make_db();
    seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();

    // GPT-6 Luna is platform-owned, not an organization override.
    let settings = db
        .get_organization_settings(DEFAULT_ORG_ID)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settings.default_model_id, None);

    assert_eq!(
        crate::platform::PLATFORM_DEFAULT_MODEL_ID,
        seed_ids::GPT_6_LUNA
    );
    let luna = db
        .get_model(DEFAULT_ORG_ID, seed_ids::GPT_6_LUNA)
        .await
        .unwrap()
        .expect("gpt-6-luna should be seeded");
    assert_eq!(luna.model_id, "gpt-6-luna");
    assert!(luna.enabled, "gpt-6-luna should be enabled");
    assert!(luna.is_favorite, "gpt-6-luna should be favorite");
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

/// End-to-end catalog check: after seeding, the current-generation models
/// must surface for the picker. `claude-sonnet-5` (and its 1M twin) is the
/// enabled favorite Sonnet while the superseded `claude-sonnet-4-6` is
/// disabled, and the Gemini 3.x models are catalogued. Guards against the
/// profile registry and seed catalog drifting apart again.
#[tokio::test]
async fn test_seed_surfaces_current_gen_models() {
    let db = make_db();
    seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();

    let by_id = |models: &[crate::storage::models::ModelRow]| {
        models
            .iter()
            .map(|m| (m.model_id.clone(), (m.enabled, m.is_favorite)))
            .collect::<std::collections::HashMap<_, _>>()
    };

    let openai = db
        .list_models_for_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
        .await
        .unwrap();
    let embedding = openai
        .iter()
        .find(|model| model.model_id == "text-embedding-3-small")
        .expect("default embedding model must be catalogued");
    assert!(embedding.enabled);
    assert_eq!(embedding.capabilities, serde_json::json!(["embeddings"]));

    let openai = by_id(&openai);
    assert_eq!(
        openai.get("gpt-6-astra"),
        Some(&(true, true)),
        "GPT-6 Astra must be seeded as an enabled favorite"
    );
    assert_eq!(
        openai.get("gpt-6-sol"),
        Some(&(true, true)),
        "GPT-6 Sol must be seeded as an enabled favorite"
    );
    assert_eq!(
        openai.get("gpt-6-luna"),
        Some(&(true, true)),
        "GPT-6 Luna must be seeded as an enabled favorite"
    );

    let anthropic = db
        .list_models_for_provider(DEFAULT_ORG_ID, seed_ids::ANTHROPIC_PROVIDER)
        .await
        .unwrap();
    let anthropic = by_id(&anthropic);
    assert_eq!(
        anthropic.get("claude-fable-5-1"),
        Some(&(true, true)),
        "Fable 5.1 must be seeded as an enabled favorite"
    );
    assert_eq!(
        anthropic.get("claude-fable-5-1[1m]"),
        Some(&(true, true)),
        "Fable 5.1 (1M) twin must be seeded and enabled"
    );
    assert_eq!(
        anthropic.get("claude-opus-5-5"),
        Some(&(true, true)),
        "Opus 5.5 must be the enabled favorite Opus"
    );
    assert_eq!(
        anthropic.get("claude-opus-5-5[1m]"),
        Some(&(true, true)),
        "Opus 5.5 (1M) twin must be seeded and enabled"
    );
    assert_eq!(
        anthropic.get("claude-opus-5"),
        Some(&(true, true)),
        "Opus 5 must stay enabled for existing agents"
    );
    assert_eq!(
        anthropic.get("claude-opus-5[1m]"),
        Some(&(true, true)),
        "Opus 5 (1M) twin must be seeded and enabled"
    );
    assert_eq!(
        anthropic.get("claude-sonnet-5"),
        Some(&(true, true)),
        "Sonnet 5 must be the enabled favorite Sonnet"
    );
    assert_eq!(
        anthropic.get("claude-sonnet-5[1m]"),
        Some(&(true, true)),
        "Sonnet 5 (1M) twin must be seeded and enabled"
    );
    assert_eq!(
        anthropic.get("claude-sonnet-4-6").map(|v| v.0),
        Some(false),
        "Sonnet 4.6 must be demoted to disabled once superseded by Sonnet 5"
    );

    let gemini = db
        .list_models_for_provider(DEFAULT_ORG_ID, seed_ids::GEMINI_PROVIDER)
        .await
        .unwrap();
    let gemini = by_id(&gemini);
    assert!(
        gemini.contains_key("gemini-3.1-pro-preview"),
        "Gemini 3.1 Pro Preview must be catalogued"
    );
    assert_eq!(
        gemini.get("gemini-3.5-flash").map(|v| v.1),
        Some(true),
        "Gemini 3.5 Flash must be catalogued as a favorite"
    );
    assert!(
        gemini.contains_key("gemini-3.1-flash-lite"),
        "Gemini 3.1 Flash Lite must be catalogued"
    );
}
