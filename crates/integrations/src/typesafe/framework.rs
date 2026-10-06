//! Application-owned TypeSafe credentials and the Framework capability adapter.

use everruns_contracts::capability::{
    CapabilitySpec, IntoCapability,
    definition::{self, Handler},
};
use serde_json::Value;

use everruns_contracts::runtime::Capability;

use crate::typesafe::client::TypeSafeAIClient;

use crate::typesafe::{JevCapability, evaluate, evaluate::EvaluateInput};

/// Jev judgments for `AgentBuilder::capability`.
///
/// The client retains the credential privately; it never enters capability
/// config or metadata. Cloned agents reuse the HTTP connection pool.
pub struct Jev {
    service: std::sync::Arc<dyn everruns_contracts::runtime::DecisionsService>,
    model: everruns_contracts::ModelSpec,
}

impl Jev {
    /// Use the same authenticated account as other model services.
    pub fn with_provider(
        provider: everruns_contracts::Provider,
        model: everruns_contracts::ModelSpec,
    ) -> Self {
        Self {
            service: std::sync::Arc::new(provider),
            model,
        }
    }

    /// Configure an application-owned API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_client(TypeSafeAIClient::new(api_key.into()))
    }

    /// Read `TYPESAFE_API_KEY` once at application startup.
    pub fn from_env() -> crate::typesafe::client::Result<Self> {
        Ok(Self::with_client(TypeSafeAIClient::from_env()?))
    }

    /// Supply a client, including a trusted custom endpoint for tests.
    pub fn with_client(client: TypeSafeAIClient) -> Self {
        Self {
            service: std::sync::Arc::new(crate::typesafe::TypeSafeAI::with_client(client)),
            model: everruns_contracts::ModelSpec::on(
                "typesafe",
                crate::typesafe::client::DEFAULT_MODEL,
            ),
        }
    }
}

impl IntoCapability for Jev {
    fn into_capability(self) -> CapabilitySpec {
        let capability = JevCapability;
        definition::Definition::new(capability.id(), capability.name(), capability.description())
            .instructions(capability.system_prompt_addition().unwrap_or_default())
            .tool(self)
            .into()
    }
}

#[definition::async_trait]
impl Handler for Jev {
    type Input = EvaluateInput;
    type Output = Value;
    type Error = definition::Error;

    fn name(&self) -> &str {
        evaluate::TOOL_NAME
    }
    fn description(&self) -> &str {
        evaluate::TOOL_DESCRIPTION
    }
    fn hints(&self) -> definition::Hints {
        definition::Hints::default().readonly(true).open_world(true)
    }

    async fn execute(
        &self,
        input: EvaluateInput,
        _context: definition::Context,
    ) -> Result<Value, Self::Error> {
        let mut request = crate::typesafe::bound::decision_request(input, &self.model.model)
            .map_err(|error| definition::Error::user("jev_decision_failed", error))?;
        request.provider = Some(self.model.provider.clone());
        let outcome = self
            .service
            .evaluate(request.clone())
            .await
            .map_err(|error| definition::Error::user("jev_decision_failed", error.to_string()))?;
        crate::typesafe::bound::render_outcome(outcome, &request)
            .map_err(|error| definition::Error::user("jev_decision_failed", error))
    }
}
