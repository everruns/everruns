//! The decision-driver registry and the router that serves it.
//!
//! Shaped like the LLM `DriverRegistry`: drivers register by id, and a model
//! id picks one. Routing, in order:
//!
//! 1. `driver/model`, when `driver` is a registered id: that driver, asked
//!    for `model`. The prefix is stripped before the vendor sees the id.
//! 2. A model-id prefix a driver declares (`jev-` for TypeSafe): that driver,
//!    with the id untouched.
//! 3. Anything else, including no model at all: the deployment default.
//!
//! An unregistered `x/...` falls through to the default rather than failing,
//! because some vendors' own ids carry a slash; the vendor then rejects an id
//! it does not know, exactly as it would without the router.
//!
//! The router is picked once at startup. A default naming a driver that is not
//! registered is a [`DecisionRoutingError`] there, never a surprise on the
//! first guardrail check.

use std::fmt;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use everruns_core::{DecisionDriver, DecisionOutcome, DecisionRequest, DecisionsService};
use everruns_provider::error::Result;
use tracing::Instrument;

/// Why a registry could not produce a router.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecisionRoutingError {
    /// The default names a driver nobody registered.
    UnknownDriver {
        /// The id asked for.
        id: String,
        /// Ids that are registered.
        available: Vec<String>,
    },
    /// A second driver tried to take an id already in use.
    DuplicateDriver(String),
}

impl fmt::Display for DecisionRoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownDriver { id, available } if available.is_empty() => write!(
                f,
                "decision driver '{id}' is not configured, and no decision driver is"
            ),
            Self::UnknownDriver { id, available } => write!(
                f,
                "decision driver '{id}' is not configured; configured drivers: {}",
                available.join(", ")
            ),
            Self::DuplicateDriver(id) => {
                write!(f, "decision driver '{id}' is already registered")
            }
        }
    }
}

impl std::error::Error for DecisionRoutingError {}

/// Decision drivers by id.
#[derive(Clone, Default)]
pub struct DecisionDriverRegistry {
    // Registration order is prefix-match order.
    drivers: Vec<Arc<dyn DecisionDriver>>,
}

impl fmt::Debug for DecisionDriverRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecisionDriverRegistry")
            .field("drivers", &self.ids())
            .finish()
    }
}

impl DecisionDriverRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `driver`. Ids are unique.
    pub fn register(
        &mut self,
        driver: Arc<dyn DecisionDriver>,
    ) -> std::result::Result<(), DecisionRoutingError> {
        if self.get(driver.id()).is_some() {
            return Err(DecisionRoutingError::DuplicateDriver(
                driver.id().to_string(),
            ));
        }
        self.drivers.push(driver);
        Ok(())
    }

    /// Register `driver`, builder style.
    ///
    /// # Panics
    ///
    /// When the id is already taken; composing two drivers under one id is a
    /// programming error, not a runtime condition.
    pub fn with(mut self, driver: impl DecisionDriver + 'static) -> Self {
        if let Err(error) = self.register(Arc::new(driver)) {
            panic!("{error}");
        }
        self
    }

    /// The driver registered under `id`.
    pub fn get(&self, id: &str) -> Option<Arc<dyn DecisionDriver>> {
        self.drivers
            .iter()
            .find(|driver| driver.id() == id)
            .cloned()
    }

    /// Registered ids, in registration order.
    pub fn ids(&self) -> Vec<String> {
        self.drivers
            .iter()
            .map(|driver| driver.id().to_string())
            .collect()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.drivers.is_empty()
    }

    /// The driver a model id names by itself, explicitly or by prefix, and
    /// the id to send it. `None` means the id belongs to the default.
    pub fn resolve(&self, model: &str) -> Option<(Arc<dyn DecisionDriver>, String)> {
        if let Some((id, rest)) = model.split_once('/')
            && let Some(driver) = self.get(id)
        {
            return Some((driver, rest.to_string()));
        }
        self.drivers
            .iter()
            .find(|driver| {
                driver
                    .model_prefixes()
                    .iter()
                    .any(|prefix| model.starts_with(prefix))
            })
            .map(|driver| (driver.clone(), model.to_string()))
    }

    /// A router with `default_driver` answering whatever names no driver.
    ///
    /// `default_model` is what the default driver is asked for when a request
    /// names no model; `None` leaves that to the driver.
    pub fn router(
        self,
        default_driver: &str,
        default_model: Option<String>,
    ) -> std::result::Result<DecisionRouter, DecisionRoutingError> {
        let default =
            self.get(default_driver)
                .ok_or_else(|| DecisionRoutingError::UnknownDriver {
                    id: default_driver.to_string(),
                    available: self.ids(),
                })?;
        Ok(DecisionRouter {
            registry: self,
            default,
            default_model,
        })
    }
}

/// A [`DecisionsService`] that routes each request to a registered driver.
#[derive(Clone)]
pub struct DecisionRouter {
    registry: DecisionDriverRegistry,
    default: Arc<dyn DecisionDriver>,
    default_model: Option<String>,
}

impl fmt::Debug for DecisionRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DecisionRouter")
            .field("drivers", &self.registry.ids())
            .field("default_driver", &self.default.id())
            .field("default_model", &self.default_model)
            .finish()
    }
}

impl DecisionRouter {
    /// Id of the driver that answers requests naming no driver.
    pub fn default_driver(&self) -> &str {
        self.default.id()
    }

    /// Model the default driver is asked for when a request names none.
    pub fn default_model(&self) -> Option<&str> {
        self.default_model.as_deref()
    }

    /// The registry behind this router.
    pub fn registry(&self) -> &DecisionDriverRegistry {
        &self.registry
    }

    /// The driver `model` routes to, and the model id that driver receives.
    pub fn route(&self, model: Option<&str>) -> (Arc<dyn DecisionDriver>, Option<String>) {
        match model {
            Some(model) => match self.registry.resolve(model) {
                Some((driver, model)) => (driver, Some(model)),
                None => (self.default.clone(), Some(model.to_string())),
            },
            None => (self.default.clone(), self.default_model.clone()),
        }
    }
}

#[async_trait]
impl DecisionsService for DecisionRouter {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(&self, mut request: DecisionRequest) -> Result<DecisionOutcome> {
        let (driver, model) = self.route(request.model.as_deref());
        request.model = model;
        driver.capabilities().check(driver.id(), &request)?;

        let primitives = request
            .questions
            .iter()
            .map(|(_, question)| question.kind())
            .collect::<Vec<_>>()
            .join(",");
        // One span per evaluation. It reaches OTel through the tracing bridge
        // `init_telemetry` installs, which is how vendors get compared on the
        // guardrail path: same span name, driver id as an attribute.
        let span = tracing::info_span!(
            "decisions.evaluate",
            decision.driver = driver.id(),
            decision.requested_model = request.model.as_deref().unwrap_or(""),
            decision.primitives = %primitives,
            decision.questions = request.len(),
            decision.model = tracing::field::Empty,
            decision.calibrated = tracing::field::Empty,
            decision.input_tokens = tracing::field::Empty,
            decision.output_tokens = tracing::field::Empty,
            decision.latency_ms = tracing::field::Empty,
            otel.status_code = tracing::field::Empty,
        );
        let started = Instant::now();
        let result = driver.evaluate(request).instrument(span.clone()).await;
        span.record("decision.latency_ms", started.elapsed().as_millis() as u64);
        match &result {
            Ok(outcome) => {
                span.record("decision.model", outcome.model.as_str());
                span.record("decision.calibrated", outcome.calibrated);
                span.record("decision.input_tokens", outcome.usage.input_tokens);
                span.record("decision.output_tokens", outcome.usage.output_tokens);
                span.record("otel.status_code", "OK");
            }
            Err(_) => {
                span.record("otel.status_code", "ERROR");
            }
        }
        result
    }

    fn name(&self) -> &'static str {
        "DecisionRouter"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::{
        DecisionAnswer, DecisionDriverCapabilities, DecisionQuestion, NativePrimitives,
    };
    use std::sync::Mutex;

    /// Records the model each request reached it with.
    struct Stub {
        id: &'static str,
        prefixes: &'static [&'static str],
        seen: Mutex<Vec<Option<String>>>,
        max_questions: Option<usize>,
    }

    impl Stub {
        fn new(id: &'static str, prefixes: &'static [&'static str]) -> Self {
            Self {
                id,
                prefixes,
                seen: Mutex::new(Vec::new()),
                max_questions: None,
            }
        }
    }

    #[async_trait]
    impl DecisionDriver for Stub {
        fn id(&self) -> &str {
            self.id
        }

        fn capabilities(&self) -> DecisionDriverCapabilities {
            let caps = DecisionDriverCapabilities::new(NativePrimitives::ALL, true);
            match self.max_questions {
                Some(max) => caps.with_max_questions(max),
                None => caps,
            }
        }

        fn model_prefixes(&self) -> &[&str] {
            self.prefixes
        }

        async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome> {
            self.seen.lock().unwrap().push(request.model.clone());
            Ok(DecisionOutcome {
                model: format!("{}:{}", self.id, request.model.unwrap_or_default()),
                answers: request
                    .questions
                    .into_iter()
                    .map(|(id, _)| (id, DecisionAnswer::noul_label(true)))
                    .collect(),
                calibrated: true,
                ..DecisionOutcome::default()
            })
        }
    }

    fn router() -> DecisionRouter {
        DecisionDriverRegistry::new()
            .with(Stub::new("typesafe", &["jev-"]))
            .with(Stub::new("openai", &[]))
            .router("typesafe", None)
            .expect("typesafe is registered")
    }

    fn ask(model: Option<&str>) -> DecisionRequest {
        let request = DecisionRequest::new("state").ask("q", DecisionQuestion::noul("Yes?"));
        match model {
            Some(model) => request.model(model),
            None => request,
        }
    }

    #[tokio::test]
    async fn jev_ids_route_to_typesafe_by_prefix_untouched() {
        let outcome = router().evaluate(ask(Some("jev-latest"))).await.unwrap();
        assert_eq!(outcome.model, "typesafe:jev-latest");
    }

    #[tokio::test]
    async fn explicit_driver_form_routes_and_strips_the_prefix() {
        let outcome = router()
            .evaluate(ask(Some("openai/decisions-1")))
            .await
            .unwrap();
        assert_eq!(outcome.model, "openai:decisions-1");
    }

    #[tokio::test]
    async fn unnamed_and_unknown_models_go_to_the_default() {
        let router = DecisionDriverRegistry::new()
            .with(Stub::new("typesafe", &["jev-"]))
            .with(Stub::new("llm", &[]))
            .router("llm", Some("utility".to_string()))
            .unwrap();
        let outcome = router.evaluate(ask(None)).await.unwrap();
        assert_eq!(outcome.model, "llm:utility", "default model applies");
        let outcome = router.evaluate(ask(Some("vendor/x"))).await.unwrap();
        assert_eq!(outcome.model, "llm:vendor/x", "unregistered prefix is kept");
    }

    #[test]
    fn unknown_default_driver_fails_at_construction_and_names_the_options() {
        let error = DecisionDriverRegistry::new()
            .with(Stub::new("typesafe", &["jev-"]))
            .router("openai", None)
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "decision driver 'openai' is not configured; configured drivers: typesafe"
        );
        let error = DecisionDriverRegistry::new()
            .router("llm", None)
            .unwrap_err();
        assert!(
            error.to_string().contains("no decision driver is"),
            "{error}"
        );
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let mut registry = DecisionDriverRegistry::new().with(Stub::new("llm", &[]));
        let error = registry
            .register(Arc::new(Stub::new("llm", &[])))
            .unwrap_err();
        assert_eq!(error, DecisionRoutingError::DuplicateDriver("llm".into()));
    }

    #[tokio::test]
    async fn declared_limits_are_enforced_before_the_driver_is_called() {
        let mut stub = Stub::new("typesafe", &[]);
        stub.max_questions = Some(1);
        let router = DecisionDriverRegistry::new()
            .with(stub)
            .router("typesafe", None)
            .unwrap();
        let request = ask(None).ask("second", DecisionQuestion::noul("And?"));
        let error = router.evaluate(request).await.unwrap_err();
        assert!(error.to_string().contains("at most 1 questions"), "{error}");
    }
}
