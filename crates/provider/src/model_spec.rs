//! Credential-free model selection.

use serde::{Deserialize, Serialize};

use crate::runtime_provider::{ProviderKey, RuntimeProviderRegistry};

/// A model name paired with the runtime provider that serves it.
///
/// This is ordinary serializable configuration: credentials, endpoints, and
/// service authentication live exclusively on the runtime provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModelSpec {
    pub provider: ProviderKey,
    pub model: String,
}

impl ModelSpec {
    pub fn on(provider: impl Into<ProviderKey>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }

    pub fn resolve_provider(
        &self,
        registry: &RuntimeProviderRegistry,
    ) -> Result<std::sync::Arc<crate::RuntimeProvider>, UnknownProvider> {
        registry.get(&self.provider).ok_or_else(|| UnknownProvider {
            requested: self.provider.clone(),
            registered: registry.ids(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownProvider {
    pub requested: ProviderKey,
    pub registered: Vec<String>,
}

impl std::fmt::Display for UnknownProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "provider '{}' is not registered; registered providers: [{}]",
            self.requested,
            self.registered.join(", ")
        )
    }
}

impl std::error::Error for UnknownProvider {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Noop;

    #[async_trait::async_trait]
    impl crate::ChatDriver for Noop {
        async fn chat_completion_stream(
            &self,
            _endpoint: &crate::ProviderEndpoint,
            _messages: Vec<crate::LlmMessage>,
            _config: &crate::LlmCallConfig,
        ) -> crate::Result<crate::LlmResponseStream> {
            unreachable!()
        }
    }

    #[test]
    fn model_spec_is_credential_and_endpoint_free() {
        let spec = ModelSpec::on("openai-prod", "gpt-5");
        let json = serde_json::to_string(&spec).unwrap();
        let debug = format!("{spec:?}");
        assert_eq!(json, r#"{"provider":"openai-prod","model":"gpt-5"}"#);
        assert!(!debug.contains("api_key"));
        assert!(!debug.contains("base_url"));
    }

    #[test]
    fn two_models_select_distinct_providers_over_one_protocol() {
        let driver: std::sync::Arc<dyn crate::ChatDriver> = std::sync::Arc::new(Noop);
        let mut registry = RuntimeProviderRegistry::new();
        registry
            .register(crate::Provider::from_driver("east", driver.clone()))
            .unwrap();
        registry
            .register(crate::Provider::from_driver("west", driver))
            .unwrap();

        let east = ModelSpec::on("east", "shared-model")
            .resolve_provider(&registry)
            .unwrap();
        let west = ModelSpec::on("west", "shared-model")
            .resolve_provider(&registry)
            .unwrap();
        assert_eq!(east.id().as_str(), "east");
        assert_eq!(west.id().as_str(), "west");
        assert!(std::sync::Arc::ptr_eq(east.driver(), west.driver()));
    }
}
