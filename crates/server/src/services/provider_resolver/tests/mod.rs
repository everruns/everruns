use super::*;
use everruns_core::DEFAULT_ORG_ID;

fn mock_env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        vars.iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.to_string())
    }
}

#[test]
fn test_get_default_api_key_openai() {
    // Not set
    assert_eq!(
        get_default_api_key_with_lookup("openai", mock_env(&[])),
        None
    );
    assert_eq!(
        get_default_api_key_with_lookup("OpenAI", mock_env(&[])),
        None
    );

    // Set
    let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "sk-test-key")]);
    assert_eq!(
        get_default_api_key_with_lookup("openai", &env),
        Some("sk-test-key".to_string())
    );
    assert_eq!(
        get_default_api_key_with_lookup("OpenAI", &env),
        Some("sk-test-key".to_string())
    );
}

#[test]
fn test_get_default_api_key_anthropic() {
    // Not set
    assert_eq!(
        get_default_api_key_with_lookup("anthropic", mock_env(&[])),
        None
    );

    // Set
    let env = mock_env(&[("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test-key")]);
    assert_eq!(
        get_default_api_key_with_lookup("anthropic", &env),
        Some("sk-ant-test-key".to_string())
    );
    assert_eq!(
        get_default_api_key_with_lookup("Anthropic", &env),
        Some("sk-ant-test-key".to_string())
    );
}

#[test]
fn test_get_default_api_key_unknown_provider() {
    let env = mock_env(&[
        ("DEFAULT_OPENAI_API_KEY", "sk-test"),
        ("DEFAULT_OPENROUTER_API_KEY", "sk-or-test"),
        ("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test"),
    ]);
    // Unknown providers and providers without defaults return None
    assert_eq!(get_default_api_key_with_lookup("unknown", &env), None);
    assert_eq!(
        get_default_api_key_with_lookup("openai_completions", &env),
        None
    );
    assert_eq!(
        get_default_api_key_with_lookup("openrouter", &env),
        Some("sk-or-test".to_string())
    );
}

#[test]
fn test_get_default_api_key_meta() {
    let explicit = mock_env(&[("DEFAULT_META_API_KEY", "meta-default")]);
    assert_eq!(
        get_default_api_key_with_lookup("meta", explicit),
        Some("meta-default".to_string())
    );
}

#[test]
fn test_get_default_api_key_empty_value() {
    let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "")]);
    assert_eq!(get_default_api_key_with_lookup("openai", &env), None);
}

// --- Integration tests with in-memory storage ---

use crate::storage::StorageBackend;
use crate::storage::models::{CreateModelRow, CreateProviderRow};

/// Helper: create resolver with in-memory storage and seed a provider + model.
/// Returns (resolver, model_uuid).
async fn setup_resolver_with_model() -> (ProviderResolverService, Uuid) {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = ProviderResolverService::new(db.clone(), None);
    let org_id = DEFAULT_ORG_ID;

    let provider_row = db
        .create_provider(
            org_id,
            CreateProviderRow {
                name: "Test OpenAI".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let model_row = db
        .create_model(
            org_id,
            CreateModelRow {
                provider_id: provider_row.id,
                model_id: "gpt-5.2".to_string(),
                display_name: "GPT-5.2".to_string(),
                capabilities: vec!["chat".to_string()],
                // Resolver paths require `enabled = TRUE`; these tests
                // exercise successful resolution, so create as enabled.
                enabled: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    (resolver, model_row.id.uuid())
}

#[tokio::test]
async fn test_resolve_model_cache_miss_then_hit() {
    let (resolver, model_id) = setup_resolver_with_model().await;

    // Cache starts empty
    assert_eq!(resolver.cache_entry_count(), 0);

    // First call: cache miss -> populates cache
    let result = resolver
        .resolve_model(DEFAULT_ORG_ID, model_id)
        .await
        .unwrap();
    assert!(result.is_some());
    let resolved = result.unwrap();
    assert_eq!(resolved.model_id, "gpt-5.2");
    assert_eq!(resolved.provider_type, "openai");

    // Run pending moka tasks so entry_count updates
    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 1);

    // Second call: cache hit (same result)
    let result2 = resolver
        .resolve_model(DEFAULT_ORG_ID, model_id)
        .await
        .unwrap();
    assert!(result2.is_some());
    assert_eq!(result2.unwrap().model_id, "gpt-5.2");

    // Still one entry
    assert_eq!(resolver.cache_entry_count(), 1);
}

#[tokio::test]
async fn test_resolve_model_not_found_is_cached() {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = ProviderResolverService::new(db, None);

    let missing_id = Uuid::new_v4();

    // First call: miss, returns None, caches it
    let result = resolver
        .resolve_model(DEFAULT_ORG_ID, missing_id)
        .await
        .unwrap();
    assert!(result.is_none());

    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 1);

    // Second call: cache hit (still None)
    let result2 = resolver
        .resolve_model(DEFAULT_ORG_ID, missing_id)
        .await
        .unwrap();
    assert!(result2.is_none());
}

#[tokio::test]
async fn test_invalidate_cache_clears_entries() {
    let (resolver, model_id) = setup_resolver_with_model().await;

    // Populate cache
    resolver
        .resolve_model(DEFAULT_ORG_ID, model_id)
        .await
        .unwrap();
    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 1);

    // Invalidate
    resolver.invalidate_cache(DEFAULT_ORG_ID).await;
    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 0);
}

#[tokio::test]
async fn test_different_models_cached_independently() {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = ProviderResolverService::new(db.clone(), None);
    let org_id = DEFAULT_ORG_ID;

    let provider_row = db
        .create_provider(
            org_id,
            CreateProviderRow {
                name: "Anthropic".to_string(),
                provider_type: "anthropic".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let model_a = db
        .create_model(
            org_id,
            CreateModelRow {
                provider_id: provider_row.id,
                model_id: "claude-opus-5".to_string(),
                display_name: "Claude Opus 5".to_string(),
                capabilities: vec![],
                // Resolver paths require `enabled = TRUE`.
                enabled: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    let model_b = db
        .create_model(
            org_id,
            CreateModelRow {
                provider_id: provider_row.id,
                model_id: "claude-sonnet-5".to_string(),
                display_name: "Claude Sonnet 5".to_string(),
                capabilities: vec![],
                enabled: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    // Resolve both models
    let ra = resolver
        .resolve_model(DEFAULT_ORG_ID, model_a.id.uuid())
        .await
        .unwrap();
    let rb = resolver
        .resolve_model(DEFAULT_ORG_ID, model_b.id.uuid())
        .await
        .unwrap();

    assert_eq!(ra.unwrap().model_id, "claude-opus-5");
    assert_eq!(rb.unwrap().model_id, "claude-sonnet-5");

    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 2);
}

#[tokio::test]
async fn test_resolve_default_model_cached() {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = ProviderResolverService::new(db.clone(), None);
    let org_id = DEFAULT_ORG_ID;

    let provider_row = db
        .create_provider(
            org_id,
            CreateProviderRow {
                name: "OpenAI".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let model = db
        .create_model(
            org_id,
            CreateModelRow {
                provider_id: provider_row.id,
                model_id: "gpt-5.2".to_string(),
                display_name: "GPT-5.2".to_string(),
                capabilities: vec![],
                enabled: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    // Set org default model
    db.upsert_organization_settings(org_id, Some(model.id.uuid()))
        .await
        .unwrap();

    // First call: populates cache
    let result = resolver
        .resolve_default_model(DEFAULT_ORG_ID)
        .await
        .unwrap();
    assert!(result.is_some());
    assert_eq!(result.unwrap().model_id, "gpt-5.2");

    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 1);

    // Second call: cache hit
    let result2 = resolver
        .resolve_default_model(DEFAULT_ORG_ID)
        .await
        .unwrap();
    assert!(result2.is_some());
    assert_eq!(result2.unwrap().model_id, "gpt-5.2");
}

#[tokio::test]
async fn test_invalidation_forces_fresh_resolution() {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = ProviderResolverService::new(db.clone(), None);
    let org_id = DEFAULT_ORG_ID;

    // Resolve a missing model -> cached as None
    let missing_id = Uuid::new_v4();
    let result = resolver
        .resolve_model(DEFAULT_ORG_ID, missing_id)
        .await
        .unwrap();
    assert!(result.is_none());

    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 1);

    // Invalidate cache
    resolver.invalidate_cache(org_id).await;
    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 0);

    // Next resolve goes to DB again (still None since model doesn't exist)
    let result2 = resolver
        .resolve_model(DEFAULT_ORG_ID, missing_id)
        .await
        .unwrap();
    assert!(result2.is_none());

    // But entry is re-cached
    resolver.cache.run_pending_tasks().await;
    assert_eq!(resolver.cache_entry_count(), 1);
}

// --- resolve_provider_api_key shared function tests ---

use crate::storage::EncryptionService;

fn test_encryption() -> Arc<EncryptionService> {
    Arc::new(
        EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[]).unwrap(),
    )
}

#[tokio::test]
async fn resolve_provider_api_key_decrypts_from_db() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();

    let encrypted = encryption.encrypt_string("sk-from-db").unwrap();
    let provider = db
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: "OpenAI".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: Some(encrypted),
                settings: None,
            },
        )
        .await
        .unwrap();

    let result = resolve_provider_api_key(&db, Some(&*encryption), &provider).unwrap();
    assert_eq!(result, Some("sk-from-db".to_string()));
}

#[tokio::test]
async fn resolve_provider_api_key_falls_back_without_encryption() {
    let db = Arc::new(StorageBackend::test_database());

    let provider = db
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: "OpenAI".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: Some(vec![1, 2, 3]),
                settings: None,
            },
        )
        .await
        .unwrap();

    // No encryption service, no env var -> None
    let result = resolve_provider_api_key(&db, None, &provider).unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn resolve_provider_api_key_no_db_key_returns_none() {
    let db = Arc::new(StorageBackend::test_database());

    let provider = db
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: "Anthropic".to_string(),
                provider_type: "anthropic".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // No encrypted key in DB -> None, regardless of env
    let result = resolve_provider_api_key(&db, None, &provider).unwrap();
    assert!(result.is_none());
}

/// EVE-511: resolver must not spend platform env keys for tenant execution.
/// Sets DEFAULT_OPENAI_API_KEY in the process env so the test would fail
/// against the old env-fallback implementation.
#[tokio::test]
async fn resolve_provider_api_key_env_key_set_does_not_leak() {
    let db = Arc::new(StorageBackend::test_database());

    let provider = db
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: "OpenAI".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    // Safety: test-only, single-threaded assertion.
    // set_var/remove_var are unsafe in Rust 2024 because they are not
    // thread-safe; this test serialises the env mutation via the
    // variable going out of scope before any assertion.
    unsafe {
        std::env::set_var("DEFAULT_OPENAI_API_KEY", "sk-platform-key-must-not-leak");
    }
    let result = resolve_provider_api_key(&db, None, &provider).unwrap();
    unsafe {
        std::env::remove_var("DEFAULT_OPENAI_API_KEY");
    }

    assert!(
        result.is_none(),
        "resolve_provider_api_key must not fall back to DEFAULT_OPENAI_API_KEY"
    );
}

/// EVE-511: resolve_provider_credentials must also fail closed.
/// Sets DEFAULT_OPENAI_API_KEY to verify it is never consulted.
#[tokio::test]
async fn resolve_provider_credentials_env_key_set_does_not_leak() {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = ProviderResolverService::new(db.clone(), None);

    // No provider configured for this org at all.
    // Safety: test-only env mutation, same rationale as above.
    unsafe {
        std::env::set_var("DEFAULT_OPENAI_API_KEY", "sk-platform-key-must-not-leak");
    }
    let result = resolver
        .resolve_provider_credentials(DEFAULT_ORG_ID, "openai")
        .await
        .unwrap();
    unsafe {
        std::env::remove_var("DEFAULT_OPENAI_API_KEY");
    }

    assert!(
        result.is_none(),
        "resolve_provider_credentials must not fall back to DEFAULT_OPENAI_API_KEY"
    );
}

#[tokio::test]
async fn resolve_provider_credentials_ignores_disabled_provider() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let provider = seed_active_provider(&db, &encryption, "azure_openai").await;
    db.update_provider(
        DEFAULT_ORG_ID,
        provider.uuid(),
        crate::storage::models::UpdateProvider {
            status: Some("disabled".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let resolver = ProviderResolverService::new(db, Some(encryption));

    let result = resolver
        .resolve_provider_credentials(DEFAULT_ORG_ID, "azure_openai")
        .await
        .unwrap();

    assert!(result.is_none(), "disabled providers must not be resolved");
}

// =========================================================================
// Cross-org isolation regression tests (EVE-59)
// =========================================================================

/// Regression: resolve_model must scope lookups to the given org_id.
/// A model created in org 1 must not be visible when resolved with org 999.
#[tokio::test]
async fn resolve_model_scoped_to_org() {
    let (resolver, model_id) = setup_resolver_with_model().await;

    // Model belongs to DEFAULT_ORG_ID — should resolve
    let result = resolver
        .resolve_model(DEFAULT_ORG_ID, model_id)
        .await
        .unwrap();
    assert!(result.is_some(), "model should resolve in its own org");

    // Same model UUID with a different org — should NOT resolve
    let result = resolver.resolve_model(999, model_id).await.unwrap();
    assert!(result.is_none(), "model must not resolve in another org");
}

/// Regression: resolve_default_model must scope to the given org_id.
#[tokio::test]
async fn resolve_default_model_scoped_to_org() {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = ProviderResolverService::new(db.clone(), None);
    let org_id = DEFAULT_ORG_ID;

    let provider_row = db
        .create_provider(
            org_id,
            CreateProviderRow {
                name: "OpenAI".to_string(),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();

    let model = db
        .create_model(
            org_id,
            CreateModelRow {
                provider_id: provider_row.id,
                model_id: "gpt-5.2".to_string(),
                display_name: "GPT-5.2".to_string(),
                capabilities: vec![],
                enabled: true,
                is_favorite: false,
                source: "manual".to_string(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();

    // Set org default model
    db.upsert_organization_settings(org_id, Some(model.id.uuid()))
        .await
        .unwrap();

    // Default model belongs to DEFAULT_ORG_ID — should resolve
    let result = resolver
        .resolve_default_model(DEFAULT_ORG_ID)
        .await
        .unwrap();
    assert!(
        result.is_some(),
        "default model should resolve in its own org"
    );

    // Different org — should NOT resolve
    let result = resolver.resolve_default_model(999).await.unwrap();
    assert!(
        result.is_none(),
        "default model must not resolve in another org"
    );
}

// --- resolve_service (service-bound resolution) tests ---

/// Resolver wired with the real OSS driver registry: `openai` declares
/// `Realtime`/`Chat`, `openrouter` is chat-only — exactly the asymmetry the
/// service-kind selection must respect.
fn service_resolver(
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
) -> ProviderResolverService {
    ProviderResolverService::new(db, encryption)
        .with_driver_registry(everruns_worker::create_driver_registry())
}

async fn seed_active_provider(
    db: &StorageBackend,
    encryption: &EncryptionService,
    provider_type: &str,
) -> everruns_contracts::typed_id::ProviderId {
    use crate::storage::models::CreateProviderRow;
    let encrypted = encryption.encrypt_string("sk-test").unwrap();
    db.create_provider(
        DEFAULT_ORG_ID,
        CreateProviderRow {
            name: provider_type.to_string(),
            provider_type: provider_type.to_string(),
            base_url: None,
            api_key_encrypted: Some(encrypted),
            settings: None,
        },
    )
    .await
    .unwrap()
    .id
}

#[tokio::test]
async fn exact_runtime_provider_resolution_is_org_scoped() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let provider = seed_active_provider(&db, &encryption, "openai").await;
    let resolver = ProviderResolverService::new(db, Some(encryption));

    let own = resolver
        .resolve_runtime_provider(DEFAULT_ORG_ID, &provider.to_string())
        .await
        .unwrap();
    assert!(own.is_some());

    let cross_org = resolver
        .resolve_runtime_provider(999, &provider.to_string())
        .await
        .unwrap();
    assert!(
        cross_org.is_none(),
        "provider must not cross org boundaries"
    );
}

#[tokio::test]
async fn runtime_provider_config_preserves_credentialless_drivers() {
    use crate::storage::models::CreateProviderRow;

    let db = Arc::new(StorageBackend::test_database());
    let provider = db
        .create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: "llmsim".to_string(),
                provider_type: "llmsim".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap();
    let resolver = ProviderResolverService::new(db, Some(test_encryption()));

    let resolved = resolver
        .resolve_runtime_provider_config(DEFAULT_ORG_ID, &provider.id.to_string())
        .await
        .unwrap()
        .expect("credentialless provider remains resolvable");
    assert_eq!(resolved.provider_type, "llmsim");
    assert!(resolved.api_key.is_none());

    assert!(
        resolver
            .resolve_runtime_provider_config(999, &provider.id.to_string())
            .await
            .unwrap()
            .is_none(),
        "provider config must remain org-scoped"
    );
}

#[tokio::test]
async fn resolve_service_selects_active_provider_declaring_service() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    seed_active_provider(&db, &encryption, "openai").await;
    let resolver = service_resolver(db, Some(encryption));

    let resolved = resolver
        .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
        .await
        .expect("openai declares Realtime and has a key");
    assert_eq!(resolved.provider_type, "openai");
    assert_eq!(resolved.credentials.api_key, "sk-test");
}

#[tokio::test]
async fn resolve_service_fails_closed_when_no_provider() {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = service_resolver(db, Some(test_encryption()));

    let err = resolver
        .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
        .await
        .expect_err("no provider configured");
    assert!(
        err.to_string().contains("no provider configured"),
        "got: {err}"
    );
}

#[tokio::test]
async fn resolve_service_skips_driver_without_service() {
    // OpenRouter is chat-only; it must not satisfy a Realtime request,
    // but it must still serve Chat.
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    seed_active_provider(&db, &encryption, "openrouter").await;
    let resolver = service_resolver(db, Some(encryption));

    let err = resolver
        .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
        .await
        .expect_err("openrouter does not declare Realtime");
    assert!(
        err.to_string().contains("no provider configured"),
        "got: {err}"
    );

    resolver
        .resolve_service(DEFAULT_ORG_ID, ServiceKind::Chat, None)
        .await
        .expect("openrouter declares Chat");
}

#[tokio::test]
async fn resolve_service_binding_requires_service_support() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let openrouter = seed_active_provider(&db, &encryption, "openrouter").await;
    let resolver = service_resolver(db, Some(encryption));

    let err = resolver
        .resolve_service(
            DEFAULT_ORG_ID,
            ServiceKind::Realtime,
            Some(&openrouter.to_string()),
        )
        .await
        .expect_err("bound provider's driver lacks Realtime");
    assert!(err.to_string().contains("does not provide"), "got: {err}");
}

#[tokio::test]
async fn resolve_service_binding_fails_closed_when_provider_disabled() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let provider = seed_active_provider(&db, &encryption, "openai").await;
    db.update_provider(
        DEFAULT_ORG_ID,
        provider.uuid(),
        crate::storage::models::UpdateProvider {
            status: Some("disabled".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let resolver = service_resolver(db, Some(encryption));

    let err = resolver
        .resolve_service(
            DEFAULT_ORG_ID,
            ServiceKind::Realtime,
            Some(&provider.to_string()),
        )
        .await
        .expect_err("disabled explicit binding fails closed");
    assert!(err.to_string().contains("not active"), "got: {err}");
}

#[tokio::test]
async fn resolve_service_binding_selects_explicit_provider() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    // Two realtime-capable providers; the binding must pick the named one,
    // not just the first active match.
    let first = seed_active_provider(&db, &encryption, "openai").await;
    let second = seed_active_provider(&db, &encryption, "openai").await;
    let resolver = service_resolver(db, Some(encryption));

    let resolved = resolver
        .resolve_service(
            DEFAULT_ORG_ID,
            ServiceKind::Realtime,
            Some(&second.to_string()),
        )
        .await
        .expect("explicit binding resolves");
    assert_eq!(resolved.provider_id, second.to_string());
    assert_ne!(resolved.provider_id, first.to_string());
}

// --- Tier 2: org-level default provider per service (EVE-569) ---

/// Pin `provider` as the org default for `service`.
async fn set_service_default(
    db: &StorageBackend,
    service: ServiceKind,
    provider: everruns_contracts::typed_id::ProviderId,
) {
    let mut defaults = crate::storage::models::ServiceProviderDefaults::new();
    defaults.insert(service, provider);
    db.patch_organization_settings(
        DEFAULT_ORG_ID,
        crate::storage::models::UpdateOrganizationSettings {
            default_provider_per_service: crate::storage::UpdateField::Set(defaults),
            ..Default::default()
        },
    )
    .await
    .unwrap();
}

#[test]
fn service_provider_defaults_json_round_trips() {
    // The Postgres path stores this map as JSONB; assert ServiceKind keys
    // serialize snake_case and ProviderId values round-trip as strings.
    let mut map = crate::storage::models::ServiceProviderDefaults::new();
    let pid = everruns_contracts::typed_id::ProviderId::new();
    map.insert(ServiceKind::Realtime, pid);
    let value = serde_json::to_value(&map).unwrap();
    assert_eq!(value, serde_json::json!({ "realtime": pid.to_string() }));
    let back: crate::storage::models::ServiceProviderDefaults =
        serde_json::from_value(value).unwrap();
    assert_eq!(back.get(&ServiceKind::Realtime), Some(&pid));
}

#[tokio::test]
async fn resolve_service_uses_org_default_before_active_fallback() {
    // Two realtime-capable providers; the org default (tier 2) must win over
    // the first-active scan (tier 3).
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let _first = seed_active_provider(&db, &encryption, "openai").await;
    let second = seed_active_provider(&db, &encryption, "openai").await;
    set_service_default(&db, ServiceKind::Realtime, second).await;
    let resolver = service_resolver(db, Some(encryption));

    let resolved = resolver
        .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
        .await
        .expect("org default resolves");
    assert_eq!(resolved.provider_id, second.to_string());
}

#[tokio::test]
async fn resolve_service_binding_overrides_org_default() {
    // Precedence: explicit binding (tier 1) wins over the org default (tier 2).
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let bound = seed_active_provider(&db, &encryption, "openai").await;
    let default = seed_active_provider(&db, &encryption, "openai").await;
    set_service_default(&db, ServiceKind::Realtime, default).await;
    let resolver = service_resolver(db, Some(encryption));

    let resolved = resolver
        .resolve_service(
            DEFAULT_ORG_ID,
            ServiceKind::Realtime,
            Some(&bound.to_string()),
        )
        .await
        .expect("binding resolves");
    assert_eq!(resolved.provider_id, bound.to_string());
    assert_ne!(resolved.provider_id, default.to_string());
}

#[tokio::test]
async fn resolve_service_org_default_fails_closed_when_missing() {
    // A default that points at a non-existent provider must error, not
    // silently fall through to an otherwise-usable active provider.
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    seed_active_provider(&db, &encryption, "openai").await;
    set_service_default(
        &db,
        ServiceKind::Realtime,
        everruns_contracts::typed_id::ProviderId::new(),
    )
    .await;
    let resolver = service_resolver(db, Some(encryption));

    let err = resolver
        .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
        .await
        .expect_err("missing org default fails closed");
    assert!(err.to_string().contains("not found"), "got: {err}");
}

#[tokio::test]
async fn resolve_service_org_default_fails_closed_when_inactive() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let provider = seed_active_provider(&db, &encryption, "openai").await;
    set_service_default(&db, ServiceKind::Realtime, provider).await;
    db.update_provider(
        DEFAULT_ORG_ID,
        provider.uuid(),
        crate::storage::models::UpdateProvider {
            status: Some("disabled".to_string()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let resolver = service_resolver(db, Some(encryption));

    let err = resolver
        .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
        .await
        .expect_err("inactive org default fails closed");
    assert!(err.to_string().contains("not active"), "got: {err}");
}

#[tokio::test]
async fn resolve_service_org_default_fails_closed_when_service_unsupported() {
    // openrouter is chat-only; pinning it as the Realtime default is invalid.
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let provider = seed_active_provider(&db, &encryption, "openrouter").await;
    set_service_default(&db, ServiceKind::Realtime, provider).await;
    let resolver = service_resolver(db, Some(encryption));

    let err = resolver
        .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
        .await
        .expect_err("incompatible org default fails closed");
    assert!(err.to_string().contains("does not provide"), "got: {err}");
}

#[tokio::test]
async fn decision_binding_is_exact_org_scoped_and_fails_closed() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let provider = seed_active_provider(&db, &encryption, "openrouter").await;
    let session = db
        .create_session(crate::storage::models::CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();
    let model = db
        .create_model(
            DEFAULT_ORG_ID,
            CreateModelRow {
                provider_id: provider,
                model_id: "typesafe/jev-1.13".into(),
                display_name: "Jev".into(),
                capabilities: vec!["decisions".into()],
                enabled: true,
                is_favorite: false,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    let resolver = service_resolver(db.clone(), Some(encryption));
    assert!(
        resolver
            .resolve_decision_model(DEFAULT_ORG_ID, None, session.id.uuid())
            .await
            .unwrap()
            .is_none()
    );
    db.set_decision_default(DEFAULT_ORG_ID, Some(model.id.uuid()))
        .await
        .unwrap();
    let bound = resolver
        .resolve_decision_model(DEFAULT_ORG_ID, None, session.id.uuid())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bound.provider_id, provider.to_string());
    assert_eq!(bound.api_key, "sk-test");
    assert_eq!(bound.profile_key, "typesafe/jev-1.13.0");
    assert!(
        resolver
            .resolve_decision_model(
                DEFAULT_ORG_ID + 1,
                Some(&model.id.to_string()),
                session.id.uuid()
            )
            .await
            .is_err()
    );
    assert!(
        db.set_decision_default(DEFAULT_ORG_ID + 1, Some(model.id.uuid()))
            .await
            .is_err()
    );
    let invalid = everruns_contracts::typed_id::ModelId::new().to_string();
    assert!(
        resolver
            .resolve_decision_model(DEFAULT_ORG_ID, Some(&invalid), session.id.uuid())
            .await
            .is_err()
    );
    db.update_model(
        DEFAULT_ORG_ID,
        model.id.uuid(),
        crate::storage::models::UpdateModel {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        resolver
            .resolve_decision_model(DEFAULT_ORG_ID, None, session.id.uuid())
            .await
            .is_err()
    );
    // A selected broken default stays visible so an operator can repair it.
    assert_eq!(
        db.get_decision_default(DEFAULT_ORG_ID).await.unwrap(),
        Some(model.id.uuid())
    );
}

#[tokio::test]
async fn an_openai_provider_serves_gpt_6_luna_as_a_decision_model() {
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let provider = seed_active_provider(&db, &encryption, "openai").await;
    let session = db
        .create_session(crate::storage::models::CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();
    // The chat model and the decision model share a provider under distinct ids.
    for (model_id, capability) in [
        ("gpt-6-luna", "chat"),
        ("gpt-6-luna-decisions", "decisions"),
    ] {
        db.create_model(
            DEFAULT_ORG_ID,
            CreateModelRow {
                provider_id: provider,
                model_id: model_id.into(),
                display_name: model_id.into(),
                capabilities: vec![capability.into()],
                enabled: true,
                is_favorite: false,
                source: "predefined".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    }
    let models = db
        .list_models_for_provider(DEFAULT_ORG_ID, provider.uuid())
        .await
        .unwrap();
    let decision = models
        .iter()
        .find(|m| m.model_id == "gpt-6-luna-decisions")
        .unwrap();
    let chat = models.iter().find(|m| m.model_id == "gpt-6-luna").unwrap();
    let resolver = service_resolver(db.clone(), Some(encryption));
    let bound = resolver
        .resolve_decision_model(
            DEFAULT_ORG_ID,
            Some(&decision.id.to_string()),
            session.id.uuid(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bound.provider_type, "openai");
    assert_eq!(bound.model, "gpt-6-luna-decisions");
    assert_eq!(bound.profile_key, "openai/gpt-6-luna-decisions");
    // The chat row is not a decision model.
    assert!(
        resolver
            .resolve_decision_model(
                DEFAULT_ORG_ID,
                Some(&chat.id.to_string()),
                session.id.uuid()
            )
            .await
            .is_err()
    );
}

// THREAT[TM-LLM-037]: an org that answers deployment-owned checks itself is
// never handed back to the deployment, even when its model cannot serve.
#[tokio::test]
async fn system_decisions_follow_the_org_choice_and_never_fall_back() {
    use everruns_core::connection_services::SystemDecisionModel;
    let db = Arc::new(StorageBackend::test_database());
    let encryption = test_encryption();
    let provider = seed_active_provider(&db, &encryption, "openrouter").await;
    let model = db
        .create_model(
            DEFAULT_ORG_ID,
            CreateModelRow {
                provider_id: provider,
                model_id: "typesafe/jev-1.13".into(),
                display_name: "Jev".into(),
                capabilities: vec!["decisions".into()],
                enabled: true,
                is_favorite: false,
                source: "manual".into(),
                provider_metadata: None,
            },
        )
        .await
        .unwrap();
    db.set_decision_default(DEFAULT_ORG_ID, Some(model.id.uuid()))
        .await
        .unwrap();
    let resolver = service_resolver(db.clone(), Some(encryption));
    // The default leaves these checks to the deployment, model or not.
    assert!(matches!(
        resolver
            .resolve_system_decision_model(DEFAULT_ORG_ID, None)
            .await
            .unwrap(),
        SystemDecisionModel::Deployment
    ));
    let choose = |choice| crate::storage::models::UpdateOrganizationSettings {
        system_decisions: Some(choice),
        ..Default::default()
    };
    db.patch_organization_settings(
        DEFAULT_ORG_ID,
        choose(crate::storage::SystemDecisions::Organization),
    )
    .await
    .unwrap();
    // Session-less, as Slack asks before a thread has a session.
    let SystemDecisionModel::Organization(bound) = resolver
        .resolve_system_decision_model(DEFAULT_ORG_ID, None)
        .await
        .unwrap()
    else {
        panic!("the org's own model answers");
    };
    assert_eq!(bound.model_id, model.id.to_string());
    assert_eq!(bound.api_key, "sk-test");
    // A disabled or cleared default is an error, not the deployment.
    db.update_model(
        DEFAULT_ORG_ID,
        model.id.uuid(),
        crate::storage::models::UpdateModel {
            enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        resolver
            .resolve_system_decision_model(DEFAULT_ORG_ID, None)
            .await
            .is_err()
    );
    db.set_decision_default(DEFAULT_ORG_ID, None).await.unwrap();
    assert!(
        resolver
            .resolve_system_decision_model(DEFAULT_ORG_ID, None)
            .await
            .is_err()
    );
    db.patch_organization_settings(
        DEFAULT_ORG_ID,
        choose(crate::storage::SystemDecisions::Deployment),
    )
    .await
    .unwrap();
    assert!(matches!(
        resolver
            .resolve_system_decision_model(DEFAULT_ORG_ID, None)
            .await
            .unwrap(),
        SystemDecisionModel::Deployment
    ));
}
