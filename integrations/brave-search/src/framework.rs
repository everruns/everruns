//! Application-owned Brave credentials and the Framework capability adapter.

use everruns_capability::{
    CapabilitySpec, IntoCapability,
    definition::{self, Handler},
};
use serde_json::Value;

use crate::{BraveSearchCapability, client::BraveSearchClient, search::SearchInput};
use everruns_core::Capability;

/// Brave Search for `AgentBuilder::capability`.
///
/// The client retains credentials privately; they never enter capability config
/// or metadata. Cloned agents reuse the HTTP connection pool.
pub struct BraveSearch {
    client: BraveSearchClient,
}

impl BraveSearch {
    /// Configure an application-owned API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_client(BraveSearchClient::new(api_key.into()))
    }

    /// Read `BRAVE_SEARCH_API_KEY` once at application startup.
    pub fn from_env() -> Result<Self, std::env::VarError> {
        let key = std::env::var(crate::BRAVE_SEARCH_API_KEY_SECRET)?;
        if key.trim().is_empty() {
            return Err(std::env::VarError::NotPresent);
        }
        Ok(Self::new(key))
    }

    /// Supply a client, including a trusted custom endpoint for tests.
    pub fn with_client(client: BraveSearchClient) -> Self {
        Self { client }
    }
}

impl IntoCapability for BraveSearch {
    fn into_capability(self) -> CapabilitySpec {
        let capability = BraveSearchCapability;
        definition::Definition::new(capability.id(), capability.name(), capability.description())
            .instructions(capability.system_prompt_addition().unwrap_or_default())
            .tool(self)
            .into()
    }
}

#[definition::async_trait]
impl Handler for BraveSearch {
    type Input = SearchInput;
    type Output = Value;
    type Error = definition::Error;

    fn name(&self) -> &str {
        crate::search::TOOL_NAME
    }
    fn description(&self) -> &str {
        crate::search::TOOL_DESCRIPTION
    }
    fn hints(&self) -> definition::Hints {
        definition::Hints::default()
            .readonly(true)
            .idempotent(true)
            .open_world(true)
    }

    async fn execute(
        &self,
        input: SearchInput,
        _context: definition::Context,
    ) -> Result<Value, Self::Error> {
        crate::search::search(&self.client, input)
            .await
            .map_err(|error| definition::Error::user("brave_search_failed", error))
    }
}
