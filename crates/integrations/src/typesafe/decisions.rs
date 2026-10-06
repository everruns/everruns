//! Deployment-owned Jev decisions wiring.
//!
//! Contracts owns the neutral service; drivers owns the credential-free protocol.
//! This application helper retains TypeSafe client conveniences. Host composition
//! selects deployment credentials separately from tenant provider accounts.

use crate::typesafe::client::{RetryPolicy, TypeSafeAIClient, question::DEFAULT_MODEL};
use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::{DecisionOutcome, DecisionRequest, DecisionsService};

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

/// Provider driver id for explicit TypeSafe account selection.
pub const TYPESAFE_DECISION_DRIVER_ID: &str = "typesafe";

/// The TypeSafe provider, behind core's provider-neutral decisions.
///
/// Named for the account that issues the key, exactly as `OpenAI` is on the
/// model side: this type is transport and credentials, and the model it reaches
/// is a string id (`jev-latest` by default, any `jev-*` via
/// [`model`](Self::model)). Jev is a model, so it gets an id rather than a type,
/// the way `gpt-5.6-terra` does.
///
/// The agent-facing [`Jev`](crate::typesafe::Jev) capability, its `jev_decision` tool and
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
    pub fn from_env() -> crate::typesafe::client::Result<Self> {
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
    async fn evaluate(&self, mut request: DecisionRequest) -> Result<DecisionOutcome> {
        if request
            .provider
            .as_ref()
            .is_some_and(|key| key.as_str() != "typesafe")
        {
            return Err(AgentLoopError::llm(
                "A single-account service cannot select another provider",
            ));
        }
        request.model = request
            .model
            .or_else(|| self.model.clone())
            .or_else(|| Some(DECISIONS_MODEL.into()));
        let evaluation = everruns_drivers::systemone::evaluation(&request)?;
        let judgment = self.client.evaluate(evaluation).await.map_err(|error| {
            // Keep status actionable without copying a provider body into logs.
            AgentLoopError::llm(match error.status() {
                Some(status) => format!("TypeSafe decision request failed (HTTP {status})"),
                None => "TypeSafe decision request failed".into(),
            })
        })?;
        everruns_drivers::systemone::decode(
            &request,
            serde_json::to_value(judgment)
                .map_err(|_| AgentLoopError::llm("Invalid decision response"))?,
        )
    }
    fn name(&self) -> &'static str {
        "TypeSafeAI"
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

    /// The deployment's TypeSafe driver, when a key is configured.
    ///
    /// The platform registers it with the other decision drivers; which one
    /// answers by default is `DECISIONS_DRIVER`'s call, not this crate's.
    pub fn into_driver(self) -> Option<TypeSafeAI> {
        match self {
            Self::Disabled => None,
            // Guardrails are the primary caller and sit on latency-critical
            // seams, so the deployment client does not retry.
            Self::TypeSafeAI { api_key } => Some(TypeSafeAI::without_retries(api_key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_redacted() {
        assert!(!format!("{:?}", TypeSafeAI::new("secret-key")).contains("secret-key"));
        assert!(
            !format!(
                "{:?}",
                SystemDecisionsConfig::TypeSafeAI {
                    api_key: "secret-key".into()
                }
            )
            .contains("secret-key")
        );
    }
    #[tokio::test]
    async fn empty_request_is_rejected_locally() {
        assert!(
            TypeSafeAI::new("unused")
                .evaluate(DecisionRequest::new("state"))
                .await
                .is_err()
        );
    }
}
