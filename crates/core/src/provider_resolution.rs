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
    ///
    /// Implementors must make credential ownership explicit. Return a config
    /// when this store owns endpoint/authentication material; return `None`
    /// only when the matching provider is registered directly in the host's
    /// driver registry or is intentionally selected but not configured yet.
    /// The latter remains constructible for configuration commands, but every
    /// provider operation fails locally at the credential boundary.
    async fn get_provider_config(
        &self,
        provider: &ProviderKey,
    ) -> Result<Option<crate::driver_registry::ProviderConfig>>;
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
