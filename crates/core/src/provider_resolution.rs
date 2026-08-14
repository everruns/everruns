//! Neutral host effects for resolving model identity and provider configuration.

use crate::error::Result;
use crate::model_spec::ModelSpec;
use crate::runtime_provider::ProviderKey;
use crate::typed_id::ModelId;
use async_trait::async_trait;

/// Trait for retrieving model identity and LLM provider configurations.
///
/// Model lookup is always credential-free. A host resolves provider
/// configuration separately by the open provider identity immediately before
/// it constructs or selects a driver. This split prevents model values from
/// carrying credentials through snapshots, logs, events, or public APIs.
#[async_trait]
pub trait ProviderStore: Send + Sync {
    /// Resolve a configured model ID to its credential-free model selection.
    async fn get_model_spec(&self, model_id: ModelId) -> Result<Option<ModelSpec>>;

    /// Return the default credential-free model selection.
    async fn get_default_model_spec(&self) -> Result<Option<ModelSpec>>;

    /// Resolve runtime service configuration independently from the model.
    /// Application-supplied providers registered directly on the platform do
    /// not need a stored config, so the default maps the open provider id to an
    /// equally named integration kind without credentials.
    async fn get_provider_config(
        &self,
        _provider: &ProviderKey,
    ) -> Result<Option<crate::driver_registry::ProviderConfig>> {
        Ok(None)
    }
}

#[async_trait]
impl<T: ProviderStore + ?Sized> ProviderStore for std::sync::Arc<T> {
    async fn get_model_spec(&self, model_id: ModelId) -> Result<Option<ModelSpec>> {
        (**self).get_model_spec(model_id).await
    }

    async fn get_default_model_spec(&self) -> Result<Option<ModelSpec>> {
        (**self).get_default_model_spec().await
    }

    async fn get_provider_config(
        &self,
        provider: &ProviderKey,
    ) -> Result<Option<crate::driver_registry::ProviderConfig>> {
        (**self).get_provider_config(provider).await
    }
}
