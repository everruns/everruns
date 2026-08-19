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
    Capability, CapabilityRegistry, EgressService, UtilityLlmService,
    tool_context::ToolContextExtensions,
};
use everruns_provider::driver_registry::DriverRegistry;
use std::sync::{Arc, RwLock};

/// The execution surface a deployment runs with.
///
/// `HostComposition` lets an embedder decide which capabilities, LLM drivers
/// and host services exist at runtime. Server and worker code compose the same
/// shape so the control plane and execution plane stay aligned.
///
/// # Example
///
/// ```rust,ignore
/// use everruns_provider::driver_registry::DriverRegistry;
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
pub struct HostComposition {
    /// Copy-on-write so a capability can be registered through a shared handle
    /// after composition (EVE-917). Readers take a snapshot and never observe a
    /// half-built registry; writers clone, mutate, validate, then swap.
    capability_registry: RwLock<Arc<CapabilityRegistry>>,
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
            capability_registry: RwLock::new(Arc::new(capability_registry)),
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

    /// A consistent snapshot of the capability registry.
    ///
    /// The snapshot is cheap to hold and never changes underneath its holder,
    /// so a turn assembled from one either sees a dynamically registered
    /// capability or does not — never a partially built registry.
    pub fn capability_registry(&self) -> Arc<CapabilityRegistry> {
        self.read_registry().clone()
    }

    /// Register a capability on a live composition (EVE-917).
    ///
    /// Callable through a shared handle, so a host holding
    /// `Arc<InProcessRuntime>` can make a capability discovered mid-session
    /// known to the runtime. Registration is not activation: the id becomes
    /// resolvable, and `activate_capability` still decides per-session
    /// enablement.
    ///
    /// Duplicate canonical ids and alias collisions are rejected and leave the
    /// existing registry untouched.
    pub fn register_capability(
        &self,
        capability: Arc<dyn Capability>,
    ) -> Result<(), everruns_capability::CapabilityError> {
        self.update_registry(|registry| registry.try_register_arc(capability))
    }

    /// Register a capability with composition-time replace semantics.
    ///
    /// Re-registering a canonical id overrides the previous implementation,
    /// matching [`CapabilityRegistry::register_arc`]. Use
    /// [`HostComposition::register_capability`] for anything registered after
    /// the deployment is assembled, where a silent override would hide a bug.
    pub fn register_capability_overriding(&self, capability: Arc<dyn Capability>) {
        let _ = self.update_registry(|registry| {
            registry.register_arc(capability);
            Ok::<(), everruns_capability::CapabilityError>(())
        });
    }

    /// Whether a canonical id or alias already resolves in this composition.
    ///
    /// Lets a host skip registration for a capability that is already present
    /// without matching on error strings.
    pub fn is_capability_registered(&self, id: &str) -> bool {
        self.read_registry().has(id)
    }

    fn read_registry(&self) -> std::sync::RwLockReadGuard<'_, Arc<CapabilityRegistry>> {
        // A panic while a writer holds the lock would poison it. The registry
        // is still coherent in that case — every write is a swap of a fully
        // built value — so recovering beats taking the whole runtime down.
        self.capability_registry
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn update_registry<E>(
        &self,
        mutate: impl FnOnce(&mut CapabilityRegistry) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut guard = self
            .capability_registry
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut next = (**guard).clone();
        mutate(&mut next)?;
        *guard = Arc::new(next);
        Ok(())
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

/// Clones are independent compositions.
///
/// They start from the same registry snapshot — cheap, since the snapshot is
/// shared until one side writes — but a capability registered on a clone is not
/// visible to the original. That keeps the pre-EVE-917 behaviour of
/// `#[derive(Clone)]`, where each clone owned its own registry.
impl Clone for HostComposition {
    fn clone(&self) -> Self {
        Self {
            capability_registry: RwLock::new(self.capability_registry()),
            driver_registry: self.driver_registry.clone(),
            egress_service: self.egress_service.clone(),
            utility_llm_service: self.utility_llm_service.clone(),
            session_file_system_factory: self.session_file_system_factory.clone(),
            extensions: self.extensions.clone(),
        }
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
            .field("capabilities", &self.capability_registry())
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
    pub fn capability_registry(self, registry: CapabilityRegistry) -> Self {
        *self
            .composition
            .capability_registry
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Arc::new(registry);
        self
    }

    /// Register a capability on the composition.
    pub fn capability(self, capability: impl Capability + 'static) -> Self {
        self.composition
            .register_capability_overriding(Arc::new(capability));
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
    impl everruns_provider::driver_registry::ChatDriver for StubChatDriver {
        async fn chat_completion_stream(
            &self,
            _endpoint: &everruns_provider::runtime_provider::ProviderEndpoint,
            _messages: Vec<everruns_provider::driver_registry::LlmMessage>,
            _config: &everruns_provider::driver_registry::LlmCallConfig,
        ) -> everruns_provider::error::Result<everruns_provider::driver_registry::LlmResponseStream>
        {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[test]
    fn composition_builder_registers_capabilities_and_drivers() {
        let mut drivers = DriverRegistry::new();
        let mut descriptor = everruns_provider::driver_registry::DriverDescriptor::chat_only(
            everruns_provider::provider::DriverId::LlmSim,
            |_config| {
                Box::new(StubChatDriver) as everruns_provider::driver_registry::BoxedChatDriver
            },
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
                .has_driver(&everruns_provider::provider::DriverId::LlmSim)
        );
    }

    #[test]
    fn composition_registries_stay_mutable_after_build() {
        let composition = HostComposition::default();
        composition.register_capability_overriding(Arc::new(HumanIntentCapability));

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
