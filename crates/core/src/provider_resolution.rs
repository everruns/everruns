//! Neutral contracts for resolving models and provider configuration.

use crate::error::Result;
use crate::provider::DriverId;
use crate::typed_id::ModelId;
use async_trait::async_trait;

/// Legacy runtime model input retained for the 0.17.x compatibility surface.
///
/// Execution converts this value into a credential-free [`crate::ModelSpec`]
/// plus a runtime provider configuration before selecting a driver. New
/// application code should use `ModelSpec` and `Provider` directly.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub model: String,
    pub provider_type: DriverId,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub provider_metadata: Option<crate::driver_registry::ProviderMetadata>,
}

impl ResolvedModel {
    pub fn provider_key(&self) -> crate::ProviderKey {
        self.provider_metadata
            .as_ref()
            .and_then(|metadata| metadata.extra.as_ref())
            .and_then(|extra| extra.get("provider_id"))
            .and_then(serde_json::Value::as_str)
            .map(crate::ProviderKey::new)
            .unwrap_or_else(|| crate::ProviderKey::new(self.provider_type.as_str()))
    }

    pub fn canonical_parts(&self) -> (crate::ModelSpec, crate::ProviderConfig) {
        let provider = self.provider_key();
        let spec = crate::ModelSpec::on(provider.clone(), self.model.clone());
        let config = crate::ProviderConfig {
            provider,
            provider_type: self.provider_type.clone(),
            api_key: self.api_key.clone(),
            base_url: self.base_url.clone(),
            metadata: self.provider_metadata.clone().unwrap_or_default(),
        };
        (spec, config)
    }
}

/// Trait for retrieving model identity and LLM provider configurations.
///
/// Model lookup is credential-free. Provider configuration is resolved
/// separately by provider identity before execution. The inline credential
/// fields on [`ResolvedModel`] remain only for the 0.17.x compatibility input.
///
/// Implementations can:
/// - Load from a database with encrypted API keys
/// - Use in-memory configurations for testing
/// - Load from environment variables for development
#[async_trait]
pub trait ProviderStore: Send + Sync {
    /// Get model with provider info by model ID
    ///
    /// Hosted implementations return model and provider identity only. Legacy
    /// embedders may still return inline provider configuration during 0.17.x;
    /// execution immediately converts it into the canonical provider path.
    async fn get_resolved_model(&self, model_id: ModelId) -> Result<Option<ResolvedModel>>;

    /// Get the default model with provider info
    ///
    /// Returns the system default model when an agent has no default_model_id set.
    async fn get_default_model(&self) -> Result<Option<ResolvedModel>>;

    /// Resolve runtime service configuration independently from the model.
    /// Application-supplied providers registered directly on the platform do
    /// not need a stored config, so the default maps the open provider id to an
    /// equally named integration kind without credentials.
    async fn get_provider_config(
        &self,
        _provider: &crate::ProviderKey,
    ) -> Result<Option<crate::ProviderConfig>> {
        Ok(None)
    }
}

#[async_trait]
impl<T: ProviderStore + ?Sized> ProviderStore for std::sync::Arc<T> {
    async fn get_resolved_model(&self, model_id: ModelId) -> Result<Option<ResolvedModel>> {
        (**self).get_resolved_model(model_id).await
    }

    async fn get_default_model(&self) -> Result<Option<ResolvedModel>> {
        (**self).get_default_model().await
    }

    async fn get_provider_config(
        &self,
        provider: &crate::ProviderKey,
    ) -> Result<Option<crate::ProviderConfig>> {
        (**self).get_provider_config(provider).await
    }
}
