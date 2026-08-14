//! Execution-surface composition for Everruns hosts.
//!
//! [`HostComposition`] is what an embedder assembles to decide which
//! capabilities, LLM drivers and host services a deployment runs with. It
//! lives here, in the layer that actually executes a turn, rather than in the
//! kernel: `everruns-core` owns the registries and service contracts, and the
//! host owns the bundle that selects a deployment's shape (EVE-887).
//!
//! Each field is a focused component owned by its own layer — the driver
//! registry comes from `everruns-provider`, the capability registry from the
//! neutral capability contract, the egress and utility-LLM services from their
//! own contracts. This type only carries them together for the runtime; it is
//! not a registry of registries and adds no vendor branching.
//!
//! Product presets stay out of here. Built-in harness provisioning, connectors,
//! system email and the hosted service catalog are composed by
//! server/worker/platform code, and inventory-based discovery is confined to
//! those presets so an embedder can build a composition by hand without
//! inheriting a product catalog.
//!
//! Server-only concerns such as route wiring, auth backends and background task
//! scheduling stay outside this module so the type can be reused from any
//! binary crate.

use crate::{DisabledSessionFileSystemFactory, SessionFileSystemFactory};
use everruns_core::{
    Capability, CapabilityRegistry, DriverRegistry, EgressService, UtilityLlmService,
    tool_context::ToolContextExtensions,
};
use std::sync::Arc;

/// The execution surface a deployment runs with.
///
/// `HostComposition` lets an embedder decide which capabilities, LLM drivers
/// and host services exist at runtime. Server and worker code compose the same
/// shape so the control plane and execution plane stay aligned.
///
/// # Example
///
/// ```rust,ignore
/// use everruns_core::DriverRegistry;
/// use everruns_host::HostComposition;
///
/// let mut drivers = DriverRegistry::new();
/// everruns_openai::register_driver(&mut drivers);
///
/// let composition = HostComposition::builder()
///     .driver_registry(drivers)
///     .capability(everruns_builtins::HumanIntentCapability)
///     .build();
/// ```
#[derive(Clone)]
pub struct HostComposition {
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    egress_service: Arc<dyn EgressService>,
    utility_llm_service: Arc<dyn UtilityLlmService>,
    session_file_system_factory: Arc<dyn SessionFileSystemFactory>,
    extensions: ToolContextExtensions,
}

impl HostComposition {
    /// Create a composition from explicit registries.
    pub fn new(capability_registry: CapabilityRegistry, driver_registry: DriverRegistry) -> Self {
        Self {
            capability_registry,
            driver_registry,
            egress_service: Arc::new(everruns_core::DisabledEgressService),
            utility_llm_service: Arc::new(everruns_core::DisabledUtilityLlmService),
            session_file_system_factory: Arc::new(DisabledSessionFileSystemFactory),
            extensions: ToolContextExtensions::default(),
        }
    }

    /// Create a builder for fluent composition.
    pub fn builder() -> HostCompositionBuilder {
        HostCompositionBuilder::new()
    }

    /// Immutable access to the capability registry.
    pub fn capability_registry(&self) -> &CapabilityRegistry {
        &self.capability_registry
    }

    /// Mutable access to the capability registry.
    pub fn capability_registry_mut(&mut self) -> &mut CapabilityRegistry {
        &mut self.capability_registry
    }

    /// Immutable access to the driver registry.
    pub fn driver_registry(&self) -> &DriverRegistry {
        &self.driver_registry
    }

    /// Mutable access to the driver registry.
    pub fn driver_registry_mut(&mut self) -> &mut DriverRegistry {
        &mut self.driver_registry
    }

    /// System-wide outbound network boundary.
    pub fn egress_service(&self) -> Arc<dyn EgressService> {
        self.egress_service.clone()
    }

    /// System-wide utility LLM service for capability internals.
    pub fn utility_llm_service(&self) -> Arc<dyn UtilityLlmService> {
        self.utility_llm_service.clone()
    }

    /// Factory for the composition-selected session filesystem implementation.
    pub fn session_file_system_factory(&self) -> Arc<dyn SessionFileSystemFactory> {
        self.session_file_system_factory.clone()
    }

    /// Resolve a type-keyed service supplied by a crate layered above core.
    pub fn extension<T: std::any::Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.extensions.get::<T>()
    }
}

impl Default for HostComposition {
    fn default() -> Self {
        Self::new(CapabilityRegistry::new(), DriverRegistry::new())
    }
}

impl std::fmt::Debug for HostComposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostComposition")
            .field("capabilities", &self.capability_registry)
            .field("drivers", &self.driver_registry.registered_providers())
            .field("egress_service", &self.egress_service.name())
            .field("utility_llm_service", &self.utility_llm_service.name())
            .field(
                "session_file_system_factory",
                &self.session_file_system_factory.name(),
            )
            .field("extensions", &self.extensions)
            .finish()
    }
}

/// Builder for [`HostComposition`].
pub struct HostCompositionBuilder {
    composition: HostComposition,
}

impl HostCompositionBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self {
            composition: HostComposition::default(),
        }
    }

    /// Replace the capability registry.
    pub fn capability_registry(mut self, registry: CapabilityRegistry) -> Self {
        self.composition.capability_registry = registry;
        self
    }

    /// Register a capability on the composition.
    pub fn capability(mut self, capability: impl Capability + 'static) -> Self {
        self.composition.capability_registry.register(capability);
        self
    }

    /// Replace the driver registry.
    pub fn driver_registry(mut self, registry: DriverRegistry) -> Self {
        self.composition.driver_registry = registry;
        self
    }

    /// Set the system-wide outbound egress service.
    pub fn egress_service(mut self, service: Arc<dyn EgressService>) -> Self {
        self.composition.egress_service = service;
        self
    }

    /// Set the system-wide utility LLM service.
    pub fn utility_llm_service(mut self, service: Arc<dyn UtilityLlmService>) -> Self {
        self.composition.utility_llm_service = service;
        self
    }

    /// Set the host-wide session filesystem factory.
    pub fn session_file_system_factory(
        mut self,
        factory: Arc<dyn SessionFileSystemFactory>,
    ) -> Self {
        self.composition.session_file_system_factory = factory;
        self
    }

    /// Insert a type-keyed service supplied by a crate layered above core.
    pub fn extension<T: std::any::Any + Send + Sync>(mut self, value: Arc<T>) -> Self {
        self.composition.extensions.insert(value);
        self
    }

    /// Build the composition.
    pub fn build(self) -> HostComposition {
        self.composition
    }
}

impl Default for HostCompositionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use everruns_builtins::HumanIntentCapability;
    use everruns_core::CapabilityStatus;

    /// Chat driver stub: registration-only, never invoked in these tests.
    struct StubChatDriver;

    #[async_trait]
    impl everruns_core::ChatDriver for StubChatDriver {
        async fn chat_completion_stream(
            &self,
            _endpoint: &everruns_core::ProviderEndpoint,
            _messages: Vec<everruns_core::LlmMessage>,
            _config: &everruns_core::LlmCallConfig,
        ) -> everruns_core::Result<everruns_core::LlmResponseStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[test]
    fn composition_builder_registers_capabilities_and_drivers() {
        let mut drivers = DriverRegistry::new();
        let mut descriptor = everruns_core::driver_registry::DriverDescriptor::chat_only(
            everruns_core::DriverId::LlmSim,
            |_config| Box::new(StubChatDriver) as everruns_core::BoxedChatDriver,
        );
        descriptor.display_name = "Stub".into();
        drivers.register_descriptor_or_replace(descriptor);

        let composition = HostComposition::builder()
            .driver_registry(drivers.clone())
            .capability(HumanIntentCapability)
            .build();

        assert!(composition.capability_registry().has("human_intent"));
        assert!(
            composition
                .driver_registry()
                .has_driver(&everruns_core::DriverId::LlmSim)
        );
    }

    #[test]
    fn composition_registries_stay_mutable_after_build() {
        let mut composition = HostComposition::default();
        composition
            .capability_registry_mut()
            .register(HumanIntentCapability);

        let info = everruns_core::CapabilityInfo::from_core(
            composition
                .capability_registry()
                .get("human_intent")
                .expect("human_intent registered")
                .as_ref(),
        );
        assert_eq!(info.status, CapabilityStatus::Available);
    }
}
