use super::{
    ProviderService,
    types::{CreateProviderRequest, UpdateProviderRequest},
};
use crate::{
    kernel_imports::{Caller, OrgRole},
    services::ProviderResolverService,
    storage::StorageBackend,
};
use everruns_core::{DEFAULT_ORG_ID, provider_resolution::ProviderStore};
use serde_json::json;
use std::sync::Arc;

fn caller() -> Caller {
    Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: format!("org_{DEFAULT_ORG_ID:032}"),
        user_id: None,
        role: OrgRole::Owner,
        is_platform_user: false,
        is_internal: false,
        read_only: false,
    }
}

#[tokio::test]
async fn selection_round_trips_through_storage_resolution_and_patch() {
    let db = Arc::new(StorageBackend::test_database());
    let resolver = Arc::new(
        ProviderResolverService::new(db.clone(), None)
            .with_driver_registry(everruns_worker::create_driver_registry()),
    );
    let service = ProviderService::with_resolver(db.clone(), None, resolver.clone());
    let caller = caller();
    let request: CreateProviderRequest = serde_json::from_value(json!({
        "name":"mistral-responses", "provider_type":"mistral", "api":"responses",
        "request_options":{"cache_diagnostics":true}
    }))
    .unwrap();
    let provider = service.create(&caller, request).await.unwrap();
    assert_eq!(provider.api.as_deref(), Some("responses"));
    let id = provider.id.uuid();
    let resolved = resolver
        .resolve_runtime_provider_config(caller.org_id, &provider.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.api.as_deref(), Some("responses"));
    assert!(resolved.request_options.cache_diagnostics);
    let store = crate::storage::DbProviderStore::new(
        db.database().clone(),
        crate::storage::EncryptionService::new(
            "test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            &[],
        )
        .unwrap(),
        caller.org_id,
    );
    let config = store
        .get_provider_config(&provider.id.to_string().into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(config.api.as_deref(), Some("responses"));
    assert!(
        resolver
            .resolve_runtime_provider_config(999, &provider.id.to_string())
            .await
            .unwrap()
            .is_none()
    );

    let update: UpdateProviderRequest = serde_json::from_value(json!({"name":"renamed"})).unwrap();
    let updated = service.update(&caller, id, update).await.unwrap().unwrap();
    assert_eq!(updated.api.as_deref(), Some("responses"));
    let invalid: UpdateProviderRequest =
        serde_json::from_value(json!({"provider_type":"anthropic"})).unwrap();
    assert!(service.update(&caller, id, invalid).await.is_err());
    let reset: UpdateProviderRequest = serde_json::from_value(json!({"api":"default"})).unwrap();
    service.update(&caller, id, reset).await.unwrap().unwrap();
    let resolved = resolver
        .resolve_runtime_provider_config(caller.org_id, &provider.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.api.as_deref(), Some("default"));
    assert!(resolved.request_options.cache_diagnostics);
}

#[tokio::test]
async fn unsupported_selections_fail_before_persistence() {
    let db = Arc::new(StorageBackend::test_database());
    let service = ProviderService::new(db.clone(), None);
    let before = service.list(&caller()).await.unwrap().len();
    for (driver, api) in [
        ("mistral", "invalid"),
        ("openai", "messages"),
        ("anthropic", "responses"),
        ("mistral", ""),
    ] {
        let request: CreateProviderRequest =
            serde_json::from_value(json!({"name":"invalid", "provider_type":driver, "api":api}))
                .unwrap();
        assert!(service.create(&caller(), request).await.is_err());
    }
    assert_eq!(service.list(&caller()).await.unwrap().len(), before);
    assert!(
        serde_json::from_value::<CreateProviderRequest>(
            json!({"name":"invalid","provider_type":"mistral","api":true})
        )
        .is_err()
    );
}
