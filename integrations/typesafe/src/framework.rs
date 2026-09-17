//! Application-owned TypeSafe credentials and the Framework capability adapter.

use everruns_capability::{
    CapabilitySpec, IntoCapability,
    definition::{self, Handler},
};
use serde_json::Value;

use everruns_core::Capability;

use crate::{TypeSafeCapability, TypeSafeClient, evaluate, evaluate::EvaluateInput};

/// TypeSafe judgments for `AgentBuilder::capability`.
///
/// The client retains the credential privately; it never enters capability
/// config or metadata. Cloned agents reuse the HTTP connection pool.
pub struct TypeSafe {
    client: TypeSafeClient,
}

impl TypeSafe {
    /// Configure an application-owned API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_client(TypeSafeClient::new(api_key.into()))
    }

    /// Read `TYPESAFE_API_KEY` once at application startup.
    pub fn from_env() -> crate::Result<Self> {
        Ok(Self::with_client(TypeSafeClient::from_env()?))
    }

    /// Supply a client, including a trusted custom endpoint for tests.
    pub fn with_client(client: TypeSafeClient) -> Self {
        Self { client }
    }
}

impl IntoCapability for TypeSafe {
    fn into_capability(self) -> CapabilitySpec {
        let capability = TypeSafeCapability;
        definition::Definition::new(capability.id(), capability.name(), capability.description())
            .instructions(capability.system_prompt_addition().unwrap_or_default())
            .tool(self)
            .into()
    }
}

#[definition::async_trait]
impl Handler for TypeSafe {
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
        evaluate::evaluate(&self.client, input)
            .await
            .map_err(|error| definition::Error::user("typesafe_evaluate_failed", error))
    }
}
