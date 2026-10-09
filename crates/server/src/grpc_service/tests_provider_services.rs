//! EVE-1234: the gRPC worker's command context carries the provider services.

use super::tests::test_worker_service;
use super::*;

/// Provider commands the worker dispatches (`sync_provider_models`,
/// `create_provider`, `update_provider`) need the services `/v1/providers`
/// attaches. Without them sync answered 503 "Model sync is not available on
/// this endpoint" and create/update skipped model discovery and bootstrap.
#[tokio::test]
async fn grpc_worker_context_carries_provider_services() {
    let caller = || everruns_core::Caller::internal(everruns_core::DEFAULT_ORG_ID);
    let mut service = test_worker_service().await;

    // Standalone (tests, no `app_builder`): built over the shared resolver.
    let ctx = service.domain_ctx_for_caller(caller());
    assert!(ctx.provider_service.is_some(), "provider service missing");
    assert!(
        ctx.model_sync_service.is_some(),
        "model sync service missing"
    );
    assert!(ctx.model_service.is_some(), "model service missing");

    // Composed: exactly the `/v1/providers` instances, so their resolver cache
    // invalidation reaches the runtime.
    let shared = crate::domains::providers::ProviderServices::new(
        service.db.clone(),
        None,
        Arc::new(crate::kernel_imports::contracts::driver_registry::DriverRegistry::new()),
        Some(service.provider_resolver_service.clone()),
    );
    service.set_provider_services(shared.clone());
    let ctx = service.domain_ctx_for_caller(caller());
    assert!(Arc::ptr_eq(
        ctx.provider_service.as_ref().unwrap(),
        &shared.provider
    ));
    assert!(Arc::ptr_eq(
        ctx.model_sync_service.as_ref().unwrap(),
        &shared.model_sync
    ));
    assert!(Arc::ptr_eq(
        ctx.model_service.as_ref().unwrap(),
        &shared.model
    ));
}
