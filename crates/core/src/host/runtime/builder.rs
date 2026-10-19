//! Runtime definition and query assembly configuration.
use super::*;

impl InProcessRuntimeBuilder {
    /// Assemble context against query-local metadata without authoring the
    /// canonical first-session event. Pair with a read-only event log.
    pub fn for_context_query(mut self) -> Self {
        self.initialize_session = false;
        self
    }

    /// Set the auth provider used to acquire credentials for scoped MCP
    /// servers. Defaults to no credentials, suitable
    /// for unauthenticated servers or servers carrying literal auth headers.
    #[cfg(feature = "mcp")]
    pub fn mcp_auth_provider(mut self, provider: Arc<dyn crate::mcp::McpAuthProvider>) -> Self {
        self.mcp_auth_provider = Some(provider);
        self
    }

    /// Replace the platform definition used by the runtime.
    pub fn host_composition(mut self, host_composition: HostComposition) -> Self {
        self.host_composition = host_composition;
        self
    }

    /// Register an additional capability on the runtime platform.
    pub fn capability<C: Capability + 'static>(self, capability: C) -> Self {
        self.host_composition
            .register_capability_overriding(Arc::new(capability));
        self
    }

    /// Replace the platform driver registry.
    pub fn driver_registry(mut self, driver_registry: DriverRegistry) -> Self {
        *self.host_composition.driver_registry_mut() = driver_registry;
        self
    }

    /// Register a canonical runtime provider selected by [`ModelSpec`](everruns_contracts::model_spec::ModelSpec).
    pub fn provider(mut self, provider: everruns_contracts::runtime_provider::Provider) -> Self {
        self.providers.push(provider);
        self
    }

    /// Register a provider, replacing any same-id registration, without
    /// changing the selected default model.
    ///
    /// This is useful for deterministic test/simulation adapters. Ordinary
    /// applications should prefer [`Self::provider`], which rejects duplicate
    /// identities instead of silently replacing them.
    pub fn replace_provider(
        mut self,
        provider: everruns_contracts::runtime_provider::Provider,
    ) -> Self {
        self.replacement_providers.push(provider);
        self
    }

    /// Register `provider` at build time — replacing any provider already
    /// registered under the same name — and default the runtime model to
    /// `model_id` on that provider when no other default was configured.
    ///
    /// This is the seam behind deterministic simulated runtimes: the
    /// `everruns-llmsim` crate's `.llm_sim_as_default(...)` extension builds an
    /// `llmsim` provider and routes it through here.
    pub fn provider_with_default_model(
        mut self,
        provider: everruns_contracts::runtime_provider::Provider,
        model_id: impl Into<String>,
    ) -> Self {
        self.default_provider = Some((provider, model_id.into()));
        self
    }

    /// Select the runtime's default credential-free model specification.
    pub fn default_model(mut self, model: everruns_contracts::model_spec::ModelSpec) -> Self {
        self.model_spec = Some(model);
        self
    }

    /// Select a credential-free model served by a provider registered on this builder.
    pub fn model_spec(mut self, model: everruns_contracts::model_spec::ModelSpec) -> Self {
        self.model_spec = Some(model);
        self
    }
}
