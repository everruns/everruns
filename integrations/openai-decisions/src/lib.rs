//! Compatibility application helper for the deployment-opted-in Decisions preview.
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::{BearerAuth, Provider};
use everruns_contracts::runtime::{DecisionOutcome, DecisionRequest, DecisionsService};
pub use everruns_drivers::openai::decisions::{
    DEFAULT_BASE_URL, DEFAULT_MODEL, OPENAI_DECISION_DRIVER_ID, wire,
};
#[derive(Clone)]
pub struct OpenAIDecisions {
    provider: Provider,
    model: String,
}
impl OpenAIDecisions {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            provider: Provider::services("openai")
                .with_decisions(
                    everruns_drivers::openai::decisions::OpenAIDecisionDriver::new()
                        .max_attempts(1),
                )
                .base_url(DEFAULT_BASE_URL)
                .auth(BearerAuth::new(api_key)),
            model: DEFAULT_MODEL.into(),
        }
    }
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.provider = self.provider.base_url(url);
        self
    }
    pub fn max_attempts(mut self, attempts: u32) -> Self {
        self.provider = self.provider.with_decisions(
            everruns_drivers::openai::decisions::OpenAIDecisionDriver::new().max_attempts(attempts),
        );
        self
    }
}
#[async_trait]
impl DecisionsService for OpenAIDecisions {
    fn is_configured(&self) -> bool {
        true
    }
    fn name(&self) -> &'static str {
        "OpenAIDecisionsPreview"
    }
    async fn evaluate(&self, mut request: DecisionRequest) -> Result<DecisionOutcome> {
        request.model = request.model.or_else(|| Some(self.model.clone()));
        self.provider.evaluate_decisions(request).await
    }
}
