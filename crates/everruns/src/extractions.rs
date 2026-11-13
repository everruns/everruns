//! Stability: alpha — may change without a major bump; see [`crate::stability`].
//!
//! Typed extraction without creating an agent session. Unlike structured chat,
//! extraction preserves the provider's evidence, absence and uncertainty.
use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
pub use everruns_contracts::extractions::*;
use everruns_contracts::{ModelSpec, ProviderRegistry, ServiceKind};
use std::sync::Arc;

/// A model and its extraction service, analogous to [`crate::Decisions`].
#[derive(Clone)]
pub struct Extractions {
    service: Arc<dyn ExtractionsService>,
    model: Option<String>,
}
impl Extractions {
    /// Bind a default model to a concrete extraction service.
    pub fn new(model: impl Into<String>, service: impl ExtractionsService + 'static) -> Self {
        Self {
            service: Arc::new(service),
            model: Some(model.into()),
        }
    }
    /// Bind a default model to an already shared extraction service.
    pub fn shared(model: impl Into<String>, service: Arc<dyn ExtractionsService>) -> Self {
        Self {
            service,
            model: Some(model.into()),
        }
    }
    /// Resolve an extraction-capable provider and model from a registry.
    pub fn from_registry(registry: ProviderRegistry, model: ModelSpec) -> Result<Self> {
        let provider = model
            .resolve_provider(&registry)
            .map_err(|e| AgentLoopError::Configuration(e.to_string()))?;
        if !provider.supports_service(ServiceKind::Extractions) {
            return Err(AgentLoopError::Configuration(
                "Selected provider does not support extractions".into(),
            ));
        }
        Ok(Self {
            service: Arc::new(ExtractionRouter {
                registry,
                default: model,
            }),
            model: None,
        })
    }
    /// Whether the bound extraction service is configured.
    pub fn is_configured(&self) -> bool {
        self.service.is_configured()
    }
    /// Start a fluent extraction request for the supplied source text.
    pub fn about(&self, state: impl Into<String>) -> Extraction {
        Extraction {
            client: self.clone(),
            request: ExtractionRequest::new(state),
        }
    }
    /// Execute a typed request. Documents never become agent instructions and
    /// no chat fallback silently invents evidence or confidence.
    pub async fn extract(&self, mut request: ExtractionRequest) -> Result<ExtractionOutcome> {
        if !self.is_configured() {
            return Err(AgentLoopError::Configuration(
                "Extractions service is not configured".into(),
            ));
        }
        if request.model.is_none() {
            request.model = self.model.clone();
        }
        request.validate()?;
        let outcome = self.service.extract(request.clone()).await?;
        outcome.validate(&request)?;
        Ok(outcome)
    }
}
/// Fluent request builder; [`send`](Self::send) returns the complete evidence.
pub struct Extraction {
    client: Extractions,
    request: ExtractionRequest,
}
impl Extraction {
    /// Add a named typed field to extract.
    pub fn field(mut self, name: impl Into<String>, field: ExtractionField) -> Self {
        self.request.fields.insert(name.into(), field);
        self
    }
    /// Attach an encoded image or document supported by the provider.
    pub fn asset(mut self, name: impl Into<String>, asset: ExtractionAsset) -> Self {
        self.request.assets.insert(name.into(), asset);
        self
    }
    /// Set the date used to interpret relative dates in the source.
    pub fn reference_date(mut self, date: impl Into<String>) -> Self {
        self.request.reference_date = Some(date.into());
        self
    }
    /// Execute the request and retain all provider evidence and uncertainty.
    pub async fn send(self) -> Result<ExtractionOutcome> {
        self.client.extract(self.request).await
    }
}
struct ExtractionRouter {
    registry: ProviderRegistry,
    default: ModelSpec,
}
#[async_trait]
impl ExtractionsService for ExtractionRouter {
    fn is_configured(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "ProviderExtractions"
    }
    async fn extract(&self, mut request: ExtractionRequest) -> Result<ExtractionOutcome> {
        let spec = ModelSpec::on(
            request
                .provider
                .clone()
                .unwrap_or_else(|| self.default.provider.clone()),
            request
                .model
                .clone()
                .unwrap_or_else(|| self.default.model.clone()),
        );
        let provider = spec
            .resolve_provider(&self.registry)
            .map_err(|e| AgentLoopError::Configuration(e.to_string()))?;
        request.model = Some(spec.model);
        provider.extract(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct TestService;
    #[async_trait]
    impl ExtractionsService for TestService {
        fn is_configured(&self) -> bool {
            true
        }
        fn name(&self) -> &'static str {
            "test"
        }
        async fn extract(&self, request: ExtractionRequest) -> Result<ExtractionOutcome> {
            assert_eq!(request.model.as_deref(), Some("test-model"));
            Ok(serde_json::from_value(serde_json::json!({"model":"test-model","answers":{"invoice":{"type":"text","value":"42","span":[0,2]}}})).unwrap())
        }
    }
    #[tokio::test]
    async fn typed_builder_preserves_evidence() {
        let result = Extractions::new("test-model", TestService)
            .about("42")
            .field("invoice", ExtractionField::text())
            .send()
            .await
            .unwrap();
        assert_eq!(result.answers["invoice"].span, Some([0, 2]));
        assert_eq!(result.answers["invoice"].confidence, None);
    }
    #[tokio::test]
    async fn empty_request_fails_before_service() {
        assert!(
            Extractions::new("test-model", TestService)
                .about("42")
                .send()
                .await
                .is_err()
        );
    }
}
