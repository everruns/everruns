// Provider-domain services every command context attaches.
//
// Decision: one bundle, built once in `app_builder` and shared by the
// `/v1/providers` routes, the MCP endpoint, the gRPC worker and the in-process
// worker. Shared rather than rebuilt per surface so the provider resolver whose
// cache provider/model writes invalidate is the one the runtime reads
// (EVE-1233, EVE-1234).

use crate::domains::models::ModelService;
use crate::kernel_imports::contracts::driver_registry::DriverRegistry;
use crate::services::{ModelSyncService, ProviderResolverService};
use crate::storage::{EncryptionService, StorageBackend};
use std::sync::Arc;

use super::ProviderService;

#[derive(Clone)]
pub struct ProviderServices {
    pub provider: Arc<ProviderService>,
    pub model_sync: Arc<ModelSyncService>,
    pub model: Arc<ModelService>,
}

impl ProviderServices {
    /// `provider_resolver` is optional only for tests and bare compositions;
    /// without it provider/model writes cannot invalidate the runtime's cache.
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        driver_registry: Arc<DriverRegistry>,
        provider_resolver: Option<Arc<ProviderResolverService>>,
    ) -> Self {
        let (provider, model) = match provider_resolver {
            Some(resolver) => (
                ProviderService::with_resolver(db.clone(), encryption.clone(), resolver.clone()),
                ModelService::with_resolver(db.clone(), resolver),
            ),
            None => (
                ProviderService::new(db.clone(), encryption.clone()),
                ModelService::new(db.clone()),
            ),
        };
        Self {
            provider: Arc::new(provider),
            model_sync: Arc::new(ModelSyncService::new(db, driver_registry, encryption)),
            model: Arc::new(model),
        }
    }
}

impl crate::domains::common::Ctx {
    /// Attach the provider-domain services, so provider commands behave the
    /// same on every surface: `sync_provider_models` works and create/update
    /// discover models and bootstrap the org's default model.
    pub fn with_provider_services(self, services: &ProviderServices) -> Self {
        self.with_provider_service(services.provider.clone())
            .with_model_sync_service(services.model_sync.clone())
            .with_model_service(services.model.clone())
    }
}
