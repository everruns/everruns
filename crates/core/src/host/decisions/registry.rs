//! Decision selection uses the same provider registry and explicit model pair as chat.
use crate::error::{AgentLoopError, Result};
use crate::{DecisionOutcome, DecisionRequest, DecisionsService};
use async_trait::async_trait;
use everruns_contracts::{ModelSpec, ProviderRegistry};

#[derive(Clone, Debug)]
pub struct DecisionRouter {
    registry: ProviderRegistry,
    default: ModelSpec,
}

impl DecisionRouter {
    pub fn new(registry: ProviderRegistry, default: ModelSpec) -> Result<Self> {
        let provider = default
            .resolve_provider(&registry)
            .map_err(|e| AgentLoopError::Configuration(e.to_string()))?;
        if !provider.supports_service(everruns_contracts::ServiceKind::Decisions) {
            return Err(AgentLoopError::Configuration(
                "Selected provider does not support decisions".into(),
            ));
        }
        Ok(Self { registry, default })
    }
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }
    pub fn default_driver(&self) -> &str {
        self.default.provider.as_str()
    }
}

#[async_trait]
impl DecisionsService for DecisionRouter {
    fn is_configured(&self) -> bool {
        true
    }
    #[tracing::instrument(name = "decisions.evaluate", skip_all, fields(provider = %request.provider.as_ref().unwrap_or(&self.default.provider), requested_model = %request.model.as_deref().unwrap_or(&self.default.model), question_count = request.questions.len()))]
    async fn evaluate(&self, mut request: DecisionRequest) -> Result<DecisionOutcome> {
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
        let outcome = provider.evaluate_decisions(request).await?;
        tracing::info!(resolved_model = %outcome.model, calibrated = outcome.calibrated, input_tokens = outcome.usage.input_tokens, output_tokens = outcome.usage.output_tokens, "Decision evaluation completed");
        Ok(outcome)
    }
    fn name(&self) -> &'static str {
        "ProviderDecisions"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DecisionDriver, DecisionDriverCapabilities, DecisionQuestion, NativePrimitives};
    use everruns_contracts::{Provider, ProviderEndpoint};
    struct Echo;
    #[async_trait]
    impl DecisionDriver for Echo {
        fn id(&self) -> &str {
            "echo"
        }
        fn capabilities(&self) -> DecisionDriverCapabilities {
            DecisionDriverCapabilities::new(NativePrimitives::ALL, true)
        }
        async fn evaluate(
            &self,
            _: &ProviderEndpoint,
            request: DecisionRequest,
        ) -> Result<DecisionOutcome> {
            Ok(DecisionOutcome {
                model: request.model.unwrap(),
                ..Default::default()
            })
        }
    }
    #[tokio::test]
    async fn model_names_never_route_or_strip_namespaces() {
        let mut providers = ProviderRegistry::new();
        providers
            .register(Provider::services("typesafe").with_decisions(Echo))
            .unwrap();
        providers
            .register(Provider::services("openrouter").with_decisions(Echo))
            .unwrap();
        let router =
            DecisionRouter::new(providers, ModelSpec::on("typesafe", "jev-1.13.0")).unwrap();
        let ask = |model| {
            DecisionRequest::new("state")
                .ask("q", DecisionQuestion::noul("test"))
                .model(model)
        };
        assert_eq!(
            router
                .evaluate(ask("typesafe/jev-1.13"))
                .await
                .unwrap()
                .model,
            "typesafe/jev-1.13"
        );
        assert_eq!(
            router
                .evaluate(ask("jev-1.13.0").on(ModelSpec::on("openrouter", "jev-1.13.0")))
                .await
                .unwrap()
                .model,
            "jev-1.13.0"
        );
        assert!(
            router
                .evaluate(ask("jev-1.13.0").on(ModelSpec::on("missing", "jev-1.13.0")))
                .await
                .is_err()
        );
    }
}
