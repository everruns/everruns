//! Deployment-owned Jev decisions wiring.
//!
//! Core owns the neutral contract (`DecisionsService`); this module owns the
//! vendor. Nothing above core learns that the decisions come from TypeSafe.
//!
//! It lives in this crate rather than in `everruns-host` so the vendor client
//! needs only one home: the platform composes the service from above
//! (`crates/server/src/platform.rs`, `crates/worker/src/platform.rs`), which is
//! the direction that already works — server and worker depend on integrations,
//! never the reverse.

use std::collections::BTreeMap;

use crate::client::{
    Answer, Evaluation, Question, RetryPolicy, TypeSafeAIClient, question::DEFAULT_MODEL,
};
use async_trait::async_trait;
use everruns_core::{
    DecisionAnswer, DecisionOutcome, DecisionQuestion, DecisionRequest, DecisionUsage,
    DecisionsService, DisabledDecisionsService,
};
use everruns_provider::error::{AgentLoopError, Result};
use std::sync::Arc;

/// Environment variable used by the deployment-owned judgment client.
///
/// Named for the utility role, not the vendor product, to match
/// `UTILITY_OPENAI_API_KEY`: both are platform-owned credentials for internal
/// model work. The agent-facing capability reads the plain `TYPESAFE_API_KEY`
/// session secret instead, and the two must never be the same key.
pub const UTILITY_TYPESAFE_API_KEY_ENV: &str = "UTILITY_TYPESAFE_API_KEY";

/// The model this service asks for when neither the request nor the service
/// names one.
///
/// The platform pins its decisions to this by never naming a model: the knob
/// is absent from the config an agent can write. Other deployments, and
/// embedders, are free to choose — there will be other classifiers and other
/// models, and the type should not be the thing preventing that.
pub const DECISIONS_MODEL: &str = DEFAULT_MODEL;

/// The TypeSafe provider, behind core's provider-neutral decisions.
///
/// Named for the account that issues the key, exactly as `OpenAI` is on the
/// model side: this type is transport and credentials, and the model it reaches
/// is a string id (`jev-latest` by default, any `jev-*` via
/// [`model`](Self::model)). Jev is a model, so it gets an id rather than a type,
/// the way `gpt-5.6-terra` does.
///
/// The agent-facing [`Jev`](crate::Jev) capability, its `jev_decision` tool and
/// the `jev` guardrail engine stay named for the model, because a model is what
/// answers them.
#[derive(Clone)]
pub struct TypeSafeAI {
    client: TypeSafeAIClient,
    model: Option<String>,
}

impl std::fmt::Debug for TypeSafeAI {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypeSafeAI")
            .field("model", &self.model.as_deref().unwrap_or(DECISIONS_MODEL))
            .field("configured", &true)
            .finish()
    }
}

impl TypeSafeAI {
    /// Construct the service from the application's own `TYPESAFE_API_KEY`.
    ///
    /// This is the embedder's path — an application holding its own key, the
    /// same variable [`TypeSafeAIClient::from_env`] reads. The platform's
    /// deployment credential is a different variable and a different account:
    /// see [`SystemDecisionsConfig::from_env`].
    pub fn from_env() -> crate::client::Result<Self> {
        Ok(Self::with_client(TypeSafeAIClient::from_env()?))
    }

    /// Construct the fixed-model service with a deployment-owned key.
    pub fn new(api_key: impl Into<String>) -> Self {
        // THREAT[TM-LLM-037]: Decision credentials remain deployment-owned and
        // never become agent- or session-configurable, the same posture
        // TM-LLM-021 gives the utility LLM key. The agent-facing `jev`
        // capability is a separate surface with its own user-scoped connection,
        // so an agent can neither read nor spend this key.
        Self::with_client(TypeSafeAIClient::new(api_key.into()))
    }

    /// Supply a client, including a trusted custom endpoint for tests.
    pub fn with_client(client: TypeSafeAIClient) -> Self {
        Self {
            client,
            model: None,
        }
    }

    /// Ask a particular model rather than the vendor's default.
    ///
    /// A request that names its own model still wins: this is the default for
    /// callers that name none.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// A client that does not retry, for callers on a latency-critical seam.
    ///
    /// Guardrails sit in front of tool calls and finalized output: a retried
    /// round trip there costs the user more than a fail-open costs the policy.
    pub fn without_retries(api_key: impl Into<String>) -> Self {
        Self::with_client(
            TypeSafeAIClient::builder(api_key.into())
                .retry(RetryPolicy::none())
                .build(),
        )
    }
}

#[async_trait]
impl DecisionsService for TypeSafeAI {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome> {
        if request.is_empty() {
            return Err(AgentLoopError::llm(
                "decision request must carry at least one question",
            ));
        }
        let mut evaluation = Evaluation::new(request.state.clone());
        // The caller's model when they named one, this service's otherwise.
        // Guardrails never name one, so the deployment stays on its default.
        if let Some(model) = request.model.as_deref().or(self.model.as_deref()) {
            evaluation = evaluation.model(model);
        }
        for (id, question) in &request.questions {
            evaluation = evaluation.ask(id.clone(), to_vendor_question(question));
        }

        let judgment =
            self.client.evaluate(evaluation).await.map_err(|error| {
                AgentLoopError::llm(format!("decision request failed: {error}"))
            })?;

        Ok(DecisionOutcome {
            model: judgment.model.clone(),
            answers: judgment
                .answers
                .iter()
                .map(|(id, answer)| (id.clone(), from_vendor_answer(answer)))
                .collect(),
            usage: DecisionUsage {
                input_tokens: judgment.usage.input_tokens,
                output_tokens: judgment.usage.output_tokens,
            },
        })
    }

    fn name(&self) -> &'static str {
        "TypeSafeAI"
    }
}

fn to_vendor_question(question: &DecisionQuestion) -> Question {
    match question {
        DecisionQuestion::Noul {
            instructions,
            yes,
            no,
        } => {
            let built = Question::noul(instructions.as_str());
            match (yes, no) {
                (Some(yes), Some(no)) => built.criteria(yes, no),
                _ => built,
            }
        }
        DecisionQuestion::Choice {
            instructions,
            options,
        } => Question::choice(
            instructions.as_str(),
            options.iter().map(|(option, rubric)| {
                (
                    option.clone(),
                    rubric
                        .as_ref()
                        .map(|text| serde_json::Value::String(text.clone()))
                        .unwrap_or(serde_json::Value::Null),
                )
            }),
        ),
        DecisionQuestion::Score {
            instructions,
            levels,
        } => Question::score(instructions.as_str(), levels.clone()),
    }
}

fn from_vendor_answer(answer: &Answer) -> DecisionAnswer {
    match answer {
        Answer::Noul(noul) => DecisionAnswer::Noul {
            probability: noul.noul,
        },
        Answer::Choice(choice) => DecisionAnswer::Choice {
            selected: choice.choice.clone(),
            probabilities: choice.probabilities.clone(),
            confidence: choice.confidence,
        },
        Answer::Score(score) => DecisionAnswer::Score {
            score: score.score,
            // Level keys arrive as strings; unparseable keys are dropped rather
            // than defaulted, so a malformed level never reads as level 0.
            probabilities: score
                .probabilities
                .iter()
                .filter_map(|(level, probability)| {
                    level
                        .parse::<usize>()
                        .ok()
                        .map(|level| (level, *probability))
                })
                .collect::<BTreeMap<usize, f64>>(),
            confidence: score.confidence,
        },
    }
}

/// Deployment startup configuration for the concrete decisions.
#[derive(Clone, PartialEq, Eq)]
pub enum SystemDecisionsConfig {
    /// Decision calls are unavailable; dependent checks fail open.
    Disabled,
    /// Enable the Jev decisions with a system-owned TypeSafe API key.
    TypeSafeAI {
        /// Deployment-owned credential.
        api_key: String,
    },
}

impl std::fmt::Debug for SystemDecisionsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.debug_struct("SystemDecisionsConfig::Disabled").finish(),
            Self::TypeSafeAI { .. } => f
                .debug_struct("SystemDecisionsConfig::TypeSafeAI")
                .field("api_key", &"<redacted>")
                .finish(),
        }
    }
}

impl SystemDecisionsConfig {
    /// Resolve judgment configuration from the process environment.
    pub fn from_env() -> Self {
        match std::env::var(UTILITY_TYPESAFE_API_KEY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
        {
            Some(api_key) => Self::TypeSafeAI { api_key },
            None => Self::Disabled,
        }
    }

    /// Materialize the configured service behind core's neutral trait.
    pub fn into_service(self) -> Arc<dyn DecisionsService> {
        match self {
            Self::Disabled => Arc::new(DisabledDecisionsService),
            // Guardrails are the primary caller and sit on latency-critical
            // seams, so the deployment client does not retry.
            Self::TypeSafeAI { api_key } => Arc::new(TypeSafeAI::without_retries(api_key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_config_debug_redacts_api_key() {
        let debug = format!(
            "{:?}",
            SystemDecisionsConfig::TypeSafeAI {
                api_key: "ts-secret-value".to_string(),
            }
        );
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("ts-secret-value"));
    }

    #[test]
    fn service_debug_never_renders_the_key() {
        let service = TypeSafeAI::new("ts-secret-value");
        assert!(!format!("{service:?}").contains("ts-secret-value"));
    }

    #[test]
    fn questions_translate_to_the_vendor_shape() {
        let noul = to_vendor_question(&DecisionQuestion::Noul {
            instructions: "Is it urgent?".to_string(),
            yes: Some("Time-sensitive".to_string()),
            no: Some("No urgency".to_string()),
        });
        let wire = serde_json::to_value(&noul).expect("serializes");
        assert_eq!(wire["type"], "noul");
        assert_eq!(wire["criteria"]["true"], "Time-sensitive");

        let choice = to_vendor_question(&DecisionQuestion::Choice {
            instructions: "Which team?".to_string(),
            options: vec![
                ("billing".to_string(), Some("Money".to_string())),
                ("technical".to_string(), None),
            ],
        });
        let wire = serde_json::to_value(&choice).expect("serializes");
        assert_eq!(wire["criteria"]["billing"], "Money");
        assert_eq!(wire["criteria"]["technical"], serde_json::Value::Null);

        let score = to_vendor_question(&DecisionQuestion::score("How bad?", ["Fine", "Bad"]));
        let wire = serde_json::to_value(&score).expect("serializes");
        assert_eq!(wire["criteria"][1], "Bad");
    }

    #[test]
    fn score_answers_keep_their_distribution_and_drop_unparseable_levels() {
        let answer: Answer = serde_json::from_value(serde_json::json!({
            "type": "score",
            "score": 1.5,
            "legend": {"0": "Fine", "1": "Bad", "2": "Awful"},
            "probabilities": {"0": 0.1, "1": 0.3, "2": 0.6, "oops": 0.9},
            "confidence": 0.7
        }))
        .expect("answer");
        let converted = from_vendor_answer(&answer);
        // Summed float mass, so compare with a tolerance rather than exactly.
        let tail = converted
            .probability_at_or_above(1)
            .expect("a score answer");
        assert!((tail - 0.9).abs() < 1e-9, "{tail}");
        assert_eq!(converted.confidence(), Some(0.7));
        let DecisionAnswer::Score { probabilities, .. } = &converted else {
            panic!("expected a score answer");
        };
        assert_eq!(probabilities.len(), 3);
    }

    #[test]
    fn noul_and_choice_answers_round_trip() {
        let noul: Answer =
            serde_json::from_value(serde_json::json!({"type": "noul", "noul": 0.92}))
                .expect("answer");
        assert_eq!(from_vendor_answer(&noul).probability_yes(), Some(0.92));

        let choice: Answer = serde_json::from_value(serde_json::json!({
            "type": "choice",
            "choice": "technical",
            "probabilities": {"billing": 0.1, "technical": 0.9},
            "confidence": 0.8
        }))
        .expect("answer");
        let DecisionAnswer::Choice {
            selected,
            probabilities,
            confidence,
        } = from_vendor_answer(&choice)
        else {
            panic!("expected a choice answer");
        };
        assert_eq!(selected, "technical");
        assert_eq!(probabilities["technical"], 0.9);
        assert_eq!(confidence, 0.8);
    }

    #[tokio::test]
    async fn empty_requests_are_rejected_before_any_round_trip() {
        let service = TypeSafeAI::new("unused");
        let error = service
            .evaluate(DecisionRequest::new("state"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("at least one question"));
    }

    #[test]
    fn env_config_is_disabled_without_a_key() {
        // Set/unset is process-global; assert the branch logic directly instead.
        assert!(
            !SystemDecisionsConfig::Disabled
                .into_service()
                .is_configured()
        );
        assert!(
            SystemDecisionsConfig::TypeSafeAI {
                api_key: "k".to_string()
            }
            .into_service()
            .is_configured()
        );
    }
}
