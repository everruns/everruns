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
//! - `UTILITY_OPENAI_API_KEY` makes the `openai` driver (OpenAI's Decisions
//!   API) available. It answers only when `UTILITY_DECISION_DRIVER=openai` picks it,
//!   so a deployment that already has an OpenAI utility key keeps its current
//!   driver. The preview opt-in was dropped once the API reached public beta
//!   and the wire shape was verified against it (2026-10-06).
//! - `UTILITY_DECISION_DRIVER` picks the default driver. Unset keeps today's
//!   behavior: `typesafe` when its key is present, otherwise disabled. The
//!   `llm` fallback is opt-in, because it spends utility-model tokens on every
//!   `jev` check and answers uncalibrated.
//! - `UTILITY_DECISION_MODEL` is the model the default driver is asked for when a
//!   request names none.
//!
//! The two selectors were `DECISIONS_DRIVER` and `DECISIONS_MODEL` until
//! 2026-10-07. They are deployment utility settings like `UTILITY_LLM_MODEL`,
//! so they took its prefix. The old names stop startup with the new name
//! rather than being silently ignored.
//!
//! Every value here is deployment-owned; none is reachable from agent or
//! session config (THREAT[TM-LLM-037]).

use std::sync::Arc;

use crate::core::{
    DecisionsService, DisabledDecisionsService, SingleDriverService, UtilityLlmService,
};
use crate::host::{DecisionRouter, LLM_DECISION_DRIVER_ID, LlmDecisionDriver};
use everruns_contracts::{ModelSpec, ProviderRegistry};
use everruns_integrations::openai_decisions::OpenAIDecisions;
use everruns_integrations::typesafe::SystemDecisionsConfig;

/// Environment variable naming the default decision driver.
pub const UTILITY_DECISION_DRIVER_ENV: &str = "UTILITY_DECISION_DRIVER";

/// Environment variable naming the default driver's model.
pub const UTILITY_DECISION_MODEL_ENV: &str = "UTILITY_DECISION_MODEL";

/// Names replaced by the two above, refused at startup.
const RENAMED_ENV: [(&str, &str); 2] = [
    ("DECISIONS_DRIVER", UTILITY_DECISION_DRIVER_ENV),
    ("DECISIONS_MODEL", UTILITY_DECISION_MODEL_ENV),
];

/// The deployment's OpenAI key, shared with the utility LLM.
const UTILITY_OPENAI_API_KEY_ENV: &str = "UTILITY_OPENAI_API_KEY";

/// Deployment decision configuration, resolved from the environment.
#[derive(Clone, Default)]
pub struct SystemDecisions {
    typesafe: Option<SystemDecisionsConfig>,
    openai_key: Option<String>,
    openrouter_key: Option<String>,
    driver: Option<String>,
    model: Option<String>,
    /// A renamed variable that is still set, with its new name.
    renamed: Option<(&'static str, &'static str)>,
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
            openrouter_key: env_value("UTILITY_OPENROUTER_API_KEY"),
            openai_key: env_value(UTILITY_OPENAI_API_KEY_ENV),
            driver: env_value(UTILITY_DECISION_DRIVER_ENV),
            model: env_value(UTILITY_DECISION_MODEL_ENV),
            renamed: RENAMED_ENV
                .into_iter()
                .find(|(old, _)| env_value(old).is_some()),
        }
    }

    /// Enable the `typesafe` driver with `config`.
    pub fn typesafe(mut self, config: SystemDecisionsConfig) -> Self {
        self.typesafe = Some(config);
        self
    }

    /// Enable the `openai` driver with a deployment-owned key.
    pub fn openai(mut self, api_key: impl Into<String>) -> Self {
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
        if let Some((old, new)) = self.renamed {
            return Err(format!("{old} was renamed to {new}"));
        }
        let mut registry = ProviderRegistry::new();
        if let Some(SystemDecisionsConfig::TypeSafeAI { api_key }) = self.typesafe {
            registry
                .register(everruns_drivers::typesafe::provider("typesafe", api_key))
                .map_err(|e| e.to_string())?;
        }
        if let Some(api_key) = self.openrouter_key {
            registry
                .register(everruns_drivers::openrouter::provider(
                    "openrouter",
                    api_key,
                ))
                .map_err(|e| e.to_string())?;
        }
        if self.driver.as_deref() == Some(LLM_DECISION_DRIVER_ID) {
            if self.model.is_some() {
                return Err(format!(
                    "{UTILITY_DECISION_MODEL_ENV} cannot be combined with {UTILITY_DECISION_DRIVER_ENV}=llm: set UTILITY_LLM_MODEL instead"
                ));
            }
            if !utility.is_configured() {
                return Err("llm needs a configured utility LLM".into());
            }
            return Ok(Arc::new(SingleDriverService(LlmDecisionDriver::new(
                utility,
            ))));
        }
        let default = match self.driver {
            Some(driver) => driver,
            None if registry.get(&"typesafe".into()).is_some() => "typesafe".into(),
            None => return Ok(Arc::new(DisabledDecisionsService)),
        };
        if default == "openai" {
            let key = self
                .openai_key
                .ok_or_else(|| "openai needs UTILITY_OPENAI_API_KEY".to_string())?;
            return Ok(Arc::new(OpenAIDecisions::new(key).model(
                self.model.unwrap_or_else(|| {
                    everruns_integrations::openai_decisions::DEFAULT_MODEL.into()
                }),
            )));
        }
        let router = DecisionRouter::new(registry, ModelSpec::on(default.as_str(), self.model.unwrap_or_else(|| "jev-latest".into())))
            .map_err(|error| format!("{UTILITY_DECISION_DRIVER_ENV}={default}: {error}; typesafe needs UTILITY_TYPESAFE_API_KEY; openrouter needs UTILITY_OPENROUTER_API_KEY"))?;
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
    use crate::core::{DecisionQuestion, DecisionRequest, DisabledUtilityLlmService};
    use everruns_contracts::driver_registry::{
        LlmCompletionMetadata, LlmResponse, LlmResponseStream,
    };
    use everruns_contracts::error::Result;

    /// A utility model that answers yes to everything.
    struct YesModel;

    #[async_trait::async_trait]
    impl UtilityLlmService for YesModel {
        fn is_configured(&self) -> bool {
            true
        }

        async fn chat_completion(
            &self,
            _request: crate::core::UtilityLlmRequest,
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
            _request: crate::core::UtilityLlmRequest,
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
        assert_eq!(service.name(), "ProviderDecisions");
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
        assert!(error.contains("configured utility LLM"), "{error}");

        let error = SystemDecisions::default()
            .driver("nope")
            .into_service(no_utility())
            .err()
            .expect("a configuration error");
        assert!(error.contains("nope"), "{error}");
    }

    #[test]
    fn the_openai_driver_needs_only_the_utility_key() {
        let error = SystemDecisions::default()
            .driver("openai")
            .into_service(no_utility())
            .err()
            .expect("a configuration error");
        assert!(error.contains("UTILITY_OPENAI_API_KEY"), "{error}");

        let service = SystemDecisions::default()
            .openai("sk-test")
            .driver("openai")
            .into_service(no_utility())
            .unwrap();
        assert_eq!(service.name(), "OpenAIDecisions");
    }

    #[test]
    fn a_renamed_variable_stops_startup_with_its_new_name() {
        let error = SystemDecisions {
            renamed: Some(RENAMED_ENV[0]),
            ..SystemDecisions::default()
        }
        .into_service(no_utility())
        .err()
        .expect("a configuration error");
        assert_eq!(
            error,
            "DECISIONS_DRIVER was renamed to UTILITY_DECISION_DRIVER"
        );
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
