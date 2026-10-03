//! The deployment's decisions service, composed from its decision drivers.
//!
//! Server and worker both compose it here, so a distributed worker answers
//! `jev` guardrail checks with the same driver the in-process server path
//! does.
//!
//! Environment:
//!
//! - `UTILITY_TYPESAFE_API_KEY` registers the `typesafe` driver.
//! - A configured utility LLM (`UTILITY_OPENAI_API_KEY` or
//!   `UTILITY_OPENROUTER_API_KEY`) registers the `llm` driver.
//! - `DECISIONS_OPENAI_PREVIEW=1` with `UTILITY_OPENAI_API_KEY` registers the
//!   `openai` driver (OpenAI's Decisions API). Opt-in because the API is in
//!   limited preview and the driver's wire shape is unverified (EVE-1118).
//! - `DECISIONS_DRIVER` picks the default driver. Unset keeps today's
//!   behavior: `typesafe` when its key is present, otherwise disabled. The
//!   `llm` fallback is opt-in, because it spends utility-model tokens on every
//!   `jev` check and answers uncalibrated.
//! - `DECISIONS_MODEL` is the model the default driver is asked for when a
//!   request names none.
//!
//! Every value here is deployment-owned; none is reachable from agent or
//! session config (THREAT[TM-LLM-037]).

use std::sync::Arc;

use everruns_core::{DecisionsService, DisabledDecisionsService, UtilityLlmService};
use everruns_host::{DecisionDriverRegistry, LLM_DECISION_DRIVER_ID, LlmDecisionDriver};
use everruns_integrations_openai_decisions::{OPENAI_DECISION_DRIVER_ID, OpenAIDecisions};
use everruns_integrations_typesafe::{SystemDecisionsConfig, TYPESAFE_DECISION_DRIVER_ID};

/// Environment variable naming the default decision driver.
pub const DECISIONS_DRIVER_ENV: &str = "DECISIONS_DRIVER";

/// Environment variable naming the default driver's model.
pub const DECISIONS_MODEL_ENV: &str = "DECISIONS_MODEL";

/// Environment variable that opts into the preview `openai` driver.
pub const DECISIONS_OPENAI_PREVIEW_ENV: &str = "DECISIONS_OPENAI_PREVIEW";

/// The deployment's OpenAI key, shared with the utility LLM.
const UTILITY_OPENAI_API_KEY_ENV: &str = "UTILITY_OPENAI_API_KEY";

/// Deployment decision configuration, resolved from the environment.
#[derive(Clone, Default)]
pub struct SystemDecisions {
    typesafe: Option<SystemDecisionsConfig>,
    openai_key: Option<String>,
    driver: Option<String>,
    model: Option<String>,
}

impl std::fmt::Debug for SystemDecisions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `SystemDecisionsConfig`'s own Debug redacts the key.
        f.debug_struct("SystemDecisions")
            .field("typesafe", &self.typesafe)
            .field("openai", &self.openai_key.as_ref().map(|_| "<redacted>"))
            .field("driver", &self.driver)
            .field("model", &self.model)
            .finish()
    }
}

impl SystemDecisions {
    /// Read the process environment.
    pub fn from_env() -> Self {
        Self {
            typesafe: Some(SystemDecisionsConfig::from_env()),
            openai_key: env_value(DECISIONS_OPENAI_PREVIEW_ENV)
                .filter(|flag| matches!(flag.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
                .and_then(|_| env_value(UTILITY_OPENAI_API_KEY_ENV)),
            driver: env_value(DECISIONS_DRIVER_ENV),
            model: env_value(DECISIONS_MODEL_ENV),
        }
    }

    /// Enable the `typesafe` driver with `config`.
    pub fn typesafe(mut self, config: SystemDecisionsConfig) -> Self {
        self.typesafe = Some(config);
        self
    }

    /// Enable the preview `openai` driver with a deployment-owned key.
    pub fn openai_preview(mut self, api_key: impl Into<String>) -> Self {
        self.openai_key = Some(api_key.into());
        self
    }

    /// Pick the default driver.
    pub fn driver(mut self, driver: impl Into<String>) -> Self {
        self.driver = Some(driver.into());
        self
    }

    /// Pick the default driver's model.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Compose the service, or explain which setting is wrong.
    ///
    /// `utility` is the deployment's utility LLM; the `llm` driver is
    /// registered only when it is configured.
    pub fn into_service(
        self,
        utility: Arc<dyn UtilityLlmService>,
    ) -> Result<Arc<dyn DecisionsService>, String> {
        let mut registry = DecisionDriverRegistry::new();
        if let Some(driver) = self.typesafe.and_then(SystemDecisionsConfig::into_driver) {
            registry = registry.with(driver);
        }
        if let Some(api_key) = self.openai_key {
            // Guardrails sit on latency-critical seams: no retries, as with
            // the TypeSafe deployment client.
            registry = registry.with(OpenAIDecisions::new(api_key).max_attempts(1));
        }
        if utility.is_configured() {
            registry = registry.with(LlmDecisionDriver::new(utility));
        }

        let default = match self.driver {
            Some(driver) => driver,
            None if registry.get(TYPESAFE_DECISION_DRIVER_ID).is_some() => {
                TYPESAFE_DECISION_DRIVER_ID.to_string()
            }
            // Nothing chosen and no vendor: the disabled service, whose error
            // names the settings that enable it. Guardrails fail open on it.
            None => return Ok(Arc::new(DisabledDecisionsService)),
        };
        if default == LLM_DECISION_DRIVER_ID && self.model.is_some() {
            return Err(format!(
                "{DECISIONS_MODEL_ENV} cannot be combined with {DECISIONS_DRIVER_ENV}=llm: the \
                 llm driver answers with the utility model (set UTILITY_LLM_MODEL instead)"
            ));
        }
        let router = registry.router(&default, self.model).map_err(|error| {
            format!(
                "{DECISIONS_DRIVER_ENV}={default}: {error} (typesafe needs \
                 UTILITY_TYPESAFE_API_KEY; llm needs UTILITY_OPENAI_API_KEY or \
                 UTILITY_OPENROUTER_API_KEY; {OPENAI_DECISION_DRIVER_ID} needs \
                 {DECISIONS_OPENAI_PREVIEW_ENV}=1 and UTILITY_OPENAI_API_KEY)"
            )
        })?;
        tracing::info!(
            decisions.default_driver = router.default_driver(),
            decisions.drivers = ?router.registry().ids(),
            "decisions router active"
        );
        Ok(Arc::new(router))
    }
}

fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::driver_registry::{
        LlmCompletionMetadata, LlmResponse, LlmResponseStream,
    };
    use everruns_contracts::error::Result;
    use everruns_core::{DecisionQuestion, DecisionRequest, DisabledUtilityLlmService};

    /// A utility model that answers yes to everything.
    struct YesModel;

    #[async_trait::async_trait]
    impl UtilityLlmService for YesModel {
        fn is_configured(&self) -> bool {
            true
        }

        async fn chat_completion(
            &self,
            _request: everruns_core::UtilityLlmRequest,
        ) -> Result<LlmResponse> {
            Ok(LlmResponse {
                text: r#"{"answers": {"q1": "yes"}}"#.to_string(),
                reasoning: Vec::new(),
                tool_calls: None,
                metadata: LlmCompletionMetadata::default(),
            })
        }

        async fn chat_completion_stream(
            &self,
            _request: everruns_core::UtilityLlmRequest,
        ) -> Result<LlmResponseStream> {
            unreachable!()
        }
    }

    fn typesafe_key() -> SystemDecisionsConfig {
        SystemDecisionsConfig::TypeSafeAI {
            api_key: "ts-key".into(),
        }
    }

    fn no_utility() -> Arc<dyn UtilityLlmService> {
        Arc::new(DisabledUtilityLlmService)
    }

    #[test]
    fn nothing_configured_is_the_disabled_service() {
        let service = SystemDecisions::default()
            .into_service(no_utility())
            .unwrap();
        assert!(!service.is_configured());
        assert_eq!(service.name(), "DisabledDecisionsService");

        // A utility model alone does not opt deployments into the llm driver.
        let service = SystemDecisions::default()
            .into_service(Arc::new(YesModel))
            .unwrap();
        assert!(!service.is_configured());
    }

    #[test]
    fn a_typesafe_key_alone_routes_to_typesafe_as_before() {
        let service = SystemDecisions::default()
            .typesafe(typesafe_key())
            .into_service(no_utility())
            .unwrap();
        assert!(service.is_configured());
        assert_eq!(service.name(), "DecisionRouter");
    }

    #[tokio::test]
    async fn the_llm_driver_answers_jev_checks_without_a_vendor_key() {
        let service = SystemDecisions::default()
            .driver("llm")
            .into_service(Arc::new(YesModel))
            .unwrap();
        assert!(service.is_configured());
        let outcome = service
            .evaluate(
                DecisionRequest::new("rm -rf /").ask("q", DecisionQuestion::noul("Destructive?")),
            )
            .await
            .expect("a real answer, not a disabled-service error");
        assert_eq!(outcome.get("q").unwrap().probability_yes(), Some(1.0));
        assert!(!outcome.calibrated);
    }

    #[test]
    fn a_driver_that_is_not_configured_is_a_startup_error() {
        let error = SystemDecisions::default()
            .driver("llm")
            .typesafe(typesafe_key())
            .into_service(no_utility())
            .err()
            .expect("a configuration error");
        assert!(error.contains("DECISIONS_DRIVER=llm"), "{error}");
        assert!(error.contains("configured drivers: typesafe"), "{error}");

        let error = SystemDecisions::default()
            .driver("nope")
            .into_service(no_utility())
            .err()
            .expect("a configuration error");
        assert!(error.contains("'nope' is not configured"), "{error}");
    }

    #[test]
    fn the_openai_driver_is_preview_opt_in() {
        let error = SystemDecisions::default()
            .driver("openai")
            .into_service(no_utility())
            .err()
            .expect("a configuration error");
        assert!(error.contains("DECISIONS_OPENAI_PREVIEW=1"), "{error}");

        let service = SystemDecisions::default()
            .openai_preview("sk-test")
            .driver("openai")
            .into_service(no_utility())
            .unwrap();
        assert_eq!(service.name(), "DecisionRouter");
    }

    #[test]
    fn a_model_for_the_llm_driver_is_refused() {
        let error = SystemDecisions::default()
            .driver("llm")
            .model("gpt-6-astra")
            .into_service(Arc::new(YesModel))
            .err()
            .expect("a configuration error");
        assert!(error.contains("UTILITY_LLM_MODEL"), "{error}");
    }
}
