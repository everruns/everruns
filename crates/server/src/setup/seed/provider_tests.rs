//! Provider seeding: built-in providers exist only once connected, env keys
//! connect them, and keyless catalog models stay disabled.

use super::*;
use crate::storage::{EncryptionService, StorageBackend, UpdateProvider};

fn make_db() -> StorageBackend {
    StorageBackend::test_database()
}

fn test_encryption() -> EncryptionService {
    EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap()
}

/// Rows an older version seeded for every built-in driver, without keys.
pub(super) async fn create_keyless_seed_providers(db: &StorageBackend) {
    seed_default_organization(db).await.unwrap();
    for seed in SEED_PROVIDERS {
        db.create_provider_with_id(
            DEFAULT_ORG_ID,
            seed.id,
            CreateProviderRow {
                name: seed.name.to_string(),
                provider_type: seed.provider_type.to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    }
}

/// Materialization fills an empty default-org provider with the env key,
/// and the key is resolvable through the normal fail-closed DB path.
#[tokio::test]
async fn materialize_fills_empty_provider_and_resolves() {
    let db = make_db();
    let encryption = test_encryption();
    let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);

    // Seed the bare provider rows (no keys).
    create_keyless_seed_providers(&db).await;

    let result = seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |ty| {
        (ty == "openai").then(|| "sk-from-env".to_string())
    })
    .await
    .unwrap();
    assert_eq!(result.updated, 1, "exactly the openai provider is filled");

    // The key round-trips through the same decrypt path the resolver uses.
    let provider = db
        .get_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
        .await
        .unwrap()
        .expect("openai provider seeded");
    assert!(provider.api_key_set);
    let resolved = crate::services::provider_resolver::resolve_provider_api_key(
        &db,
        Some(&encryption),
        &provider,
    )
    .unwrap();
    assert_eq!(resolved, Some("sk-from-env".to_string()));
}

/// Materialization never overwrites an explicitly-configured key.
#[tokio::test]
async fn materialize_does_not_clobber_existing_key() {
    let db = make_db();
    let encryption = test_encryption();
    let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);
    create_keyless_seed_providers(&db).await;

    // User configures their own key.
    let user_key = encryption.encrypt_string("sk-user-set").unwrap();
    db.update_provider(
        DEFAULT_ORG_ID,
        seed_ids::OPENAI_PROVIDER,
        UpdateProvider {
            name: None,
            provider_type: None,
            base_url: None,
            api_key_encrypted: Some(user_key),
            status: None,
            settings: None,
        },
    )
    .await
    .unwrap();

    let result = seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |ty| {
        (ty == "openai").then(|| "sk-from-env".to_string())
    })
    .await
    .unwrap();
    assert_eq!(result.updated, 0);
    assert_eq!(result.unchanged, 1);

    let provider = db
        .get_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
        .await
        .unwrap()
        .unwrap();
    let resolved = crate::services::provider_resolver::resolve_provider_api_key(
        &db,
        Some(&encryption),
        &provider,
    )
    .unwrap();
    assert_eq!(
        resolved,
        Some("sk-user-set".to_string()),
        "user key must be preserved"
    );
}

/// With no env key set, nothing is materialized (multitenant-safe default
/// even if the gate were somehow on).
#[tokio::test]
async fn materialize_no_env_key_is_noop() {
    let db = make_db();
    let encryption = test_encryption();
    let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);
    create_keyless_seed_providers(&db).await;

    let result = seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |_| None)
        .await
        .unwrap();
    assert_eq!(result.updated, 0);

    let provider = db
        .get_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
        .await
        .unwrap()
        .unwrap();
    assert!(!provider.api_key_set);
}

/// A fresh install connects nothing: no provider rows, so no catalog
/// models either.
#[tokio::test]
async fn seed_all_creates_no_providers_or_models() {
    let db = make_db();
    seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();
    assert!(db.list_providers(DEFAULT_ORG_ID).await.unwrap().is_empty());
    assert!(db.list_all_models(DEFAULT_ORG_ID).await.unwrap().is_empty());
}

/// An env key connects its provider on a fresh install, and only that one.
#[tokio::test]
async fn materialize_creates_missing_provider_with_key() {
    let db = make_db();
    let encryption = test_encryption();
    let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);
    seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();

    let result = seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |ty| {
        (ty == "openrouter").then(|| "sk-or-from-env".to_string())
    })
    .await
    .unwrap();
    assert_eq!(result.created, 1);
    assert_eq!(result.connected, vec![seed_ids::OPENROUTER_PROVIDER]);

    let providers = db.list_providers(DEFAULT_ORG_ID).await.unwrap();
    assert_eq!(
        providers.len(),
        1,
        "only the provider with an env key exists"
    );
    assert!(providers[0].api_key_set);

    // A second boot leaves it alone.
    let again = seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |ty| {
        (ty == "openrouter").then(|| "sk-or-from-env".to_string())
    })
    .await
    .unwrap();
    assert!(again.connected.is_empty());
    assert_eq!(again.unchanged, 1);
}

/// Rows older versions seeded without a key keep their catalog, but none
/// of it is enabled: those models cannot run.
#[tokio::test]
async fn keyless_seed_provider_models_are_disabled() {
    let db = make_db();
    create_keyless_seed_providers(&db).await;
    seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();
    let models = db.list_all_models(DEFAULT_ORG_ID).await.unwrap();
    assert!(!models.is_empty(), "catalog is still listed");
    assert!(
        models.iter().all(|model| !model.enabled),
        "no model of a keyless provider is enabled"
    );
}

/// A connected provider gets the catalog's default-enabled models.
#[tokio::test]
async fn keyed_seed_provider_models_follow_catalog_defaults() {
    let db = make_db();
    let encryption = test_encryption();
    let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);
    seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();
    seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |ty| {
        (ty == "openai").then(|| "sk-from-env".to_string())
    })
    .await
    .unwrap();
    seed_models_with_host_composition(&db, &platform)
        .await
        .unwrap();

    let models = db
        .list_models_for_provider(DEFAULT_ORG_ID, seed_ids::OPENAI_PROVIDER)
        .await
        .unwrap();
    let expected = SEED_MODELS
        .iter()
        .filter(|seed| seed.provider_id == seed_ids::OPENAI_PROVIDER && seed.enabled)
        .count();
    assert!(expected > 0);
    assert_eq!(
        models.iter().filter(|model| model.enabled).count(),
        expected
    );
}

/// An org that connected only OpenRouter must not keep defaulting to the
/// platform's OpenAI model that an older version seeded without a key.
#[tokio::test]
async fn default_model_moves_to_a_connected_provider() {
    let db = Arc::new(make_db());
    let encryption = test_encryption();
    let platform = crate::platform::oss_host_composition_for_grade(DeploymentGrade::Dev);
    create_keyless_seed_providers(&db).await;
    seed_default_provider_keys_with_lookup(&db, &encryption, &platform, |ty| {
        (ty == "openrouter").then(|| "sk-or".to_string())
    })
    .await
    .unwrap();
    seed_all(&db, DeploymentGrade::Dev, &SeedAuthContext::default())
        .await
        .unwrap();

    // What a sync would have discovered.
    db.create_model(
        DEFAULT_ORG_ID,
        CreateModelRow {
            provider_id: seed_ids::OPENROUTER_PROVIDER.into(),
            model_id: "openai/gpt-6-luna".to_string(),
            display_name: "GPT-6 Luna".to_string(),
            capabilities: vec![],
            is_favorite: false,
            enabled: false,
            source: "discovered".to_string(),
            provider_metadata: None,
        },
    )
    .await
    .unwrap();
    // The keyless OpenAI platform default no longer resolves.
    assert!(
        db.get_default_model(DEFAULT_ORG_ID)
            .await
            .unwrap()
            .is_none()
    );

    ensure_default_org_default_model(&db, None, &platform).await;

    let default = db
        .get_default_model(DEFAULT_ORG_ID)
        .await
        .unwrap()
        .expect("the default resolves to a runnable model");
    assert_eq!(default.provider_id.uuid(), seed_ids::OPENROUTER_PROVIDER);
}
