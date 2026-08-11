//! Platform definition for embeddable Everruns deployments.
//!
//! `PlatformDefinition` is the shared composition root for server and worker
//! runtime surface. Embedders can add or remove capabilities, LLM drivers,
//! and connection providers without patching internal startup code.
//!
//! Built-in harness provisioning templates are product composition, not
//! Framework execution configuration; they moved to `everruns-platform`
//! (EVE-881) and are wired by the server/platform layer.
//!
//! Server-only concerns such as route wiring, auth backends, and background
//! task scheduling stay outside this module so the type can be reused from any
//! binary crate. Platform-wide host services belong here as factories, not as
//! server-owned concrete dependencies.

use crate::{
    Capability, CapabilityRegistry, Connector, ConnectorRegistry, DriverRegistry, EgressService,
    EmailSender, UtilityLlmService,
    traits::{DisabledSessionFileSystemFactory, SessionFileSystemFactory},
    vector_store::{InMemoryVectorStore, VectorStore},
};
use std::sync::Arc;

/// Shared definition of the Everruns platform surface.
///
/// `PlatformDefinition` lets an embedder decide which capabilities, LLM
/// drivers, connection providers, and platform service factories exist at
/// runtime. Server and worker code should consume the same definition so the
/// control plane and execution plane stay aligned.
///
/// # Example
///
/// ```rust,ignore
/// use everruns_core::{DriverRegistry, PlatformDefinition};
///
/// let mut drivers = DriverRegistry::new();
/// everruns_openai::register_driver(&mut drivers);
///
/// let platform = PlatformDefinition::builder()
///     .driver_registry(drivers)
///     .capability(everruns_core::CurrentTimeCapability)
///     .build();
/// ```
#[derive(Clone)]
pub struct PlatformDefinition {
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    connectors: ConnectorRegistry,
    egress_service: Arc<dyn EgressService>,
    email_sender: Arc<dyn EmailSender>,
    utility_llm_service: Arc<dyn UtilityLlmService>,
    session_file_system_factory: Arc<dyn SessionFileSystemFactory>,
    vector_store: Arc<dyn VectorStore>,
}

impl PlatformDefinition {
    /// Create a platform definition from explicit registries.
    pub fn new(capability_registry: CapabilityRegistry, driver_registry: DriverRegistry) -> Self {
        Self {
            capability_registry,
            driver_registry,
            connectors: ConnectorRegistry::new(),
            egress_service: Arc::new(crate::DirectEgressService::default()),
            email_sender: Arc::new(crate::DisabledEmailSender),
            utility_llm_service: Arc::new(crate::DisabledUtilityLlmService),
            session_file_system_factory: Arc::new(DisabledSessionFileSystemFactory),
            vector_store: Arc::new(InMemoryVectorStore::new()),
        }
    }

    /// Create a builder for fluent platform composition.
    pub fn builder() -> PlatformDefinitionBuilder {
        PlatformDefinitionBuilder::new()
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

    /// Immutable access to the connector registry.
    pub fn connectors(&self) -> &ConnectorRegistry {
        &self.connectors
    }

    /// Mutable access to the connector registry.
    pub fn connectors_mut(&mut self) -> &mut ConnectorRegistry {
        &mut self.connectors
    }

    /// System-wide outbound network boundary.
    pub fn egress_service(&self) -> Arc<dyn EgressService> {
        self.egress_service.clone()
    }

    /// System-wide email sender for product and operational flows.
    pub fn email_sender(&self) -> Arc<dyn EmailSender> {
        self.email_sender.clone()
    }

    /// System-wide utility LLM service for capability internals.
    pub fn utility_llm_service(&self) -> Arc<dyn UtilityLlmService> {
        self.utility_llm_service.clone()
    }

    /// Factory for the platform-selected session filesystem implementation.
    pub fn session_file_system_factory(&self) -> Arc<dyn SessionFileSystemFactory> {
        self.session_file_system_factory.clone()
    }

    /// Platform-selected vector store backing Knowledge Index embeddings.
    /// Defaults to the in-memory backend; production deployments override it.
    pub fn vector_store(&self) -> Arc<dyn VectorStore> {
        self.vector_store.clone()
    }
}

impl Default for PlatformDefinition {
    fn default() -> Self {
        Self::new(CapabilityRegistry::new(), DriverRegistry::new())
    }
}

impl std::fmt::Debug for PlatformDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformDefinition")
            .field("capabilities", &self.capability_registry)
            .field("drivers", &self.driver_registry.registered_providers())
            .field("connectors", &self.connectors)
            .field("egress_service", &self.egress_service.name())
            .field("email_sender", &self.email_sender.name())
            .field("utility_llm_service", &self.utility_llm_service.name())
            .field(
                "session_file_system_factory",
                &self.session_file_system_factory.name(),
            )
            .field("vector_store", &"<dyn VectorStore>")
            .finish()
    }
}

/// Builder for `PlatformDefinition`.
pub struct PlatformDefinitionBuilder {
    platform: PlatformDefinition,
}

impl PlatformDefinitionBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self {
            platform: PlatformDefinition::default(),
        }
    }

    /// Replace the capability registry.
    pub fn capability_registry(mut self, registry: CapabilityRegistry) -> Self {
        self.platform.capability_registry = registry;
        self
    }

    /// Register a capability on the platform.
    pub fn capability(mut self, capability: impl Capability + 'static) -> Self {
        self.platform.capability_registry.register(capability);
        self
    }

    /// Replace the driver registry.
    pub fn driver_registry(mut self, registry: DriverRegistry) -> Self {
        self.platform.driver_registry = registry;
        self
    }

    /// Replace the connection-provider registry.
    pub fn connectors(mut self, registry: ConnectorRegistry) -> Self {
        self.platform.connectors = registry;
        self
    }

    /// Register a connector on the platform.
    pub fn connector(mut self, provider: impl Connector + 'static) -> Self {
        self.platform.connectors.register(provider);
        self
    }

    /// Set the system-wide outbound egress service.
    pub fn egress_service(mut self, service: Arc<dyn EgressService>) -> Self {
        self.platform.egress_service = service;
        self
    }

    /// Set the system-wide email sender.
    pub fn email_sender(mut self, sender: Arc<dyn EmailSender>) -> Self {
        self.platform.email_sender = sender;
        self
    }

    /// Set the system-wide utility LLM service.
    pub fn utility_llm_service(mut self, service: Arc<dyn UtilityLlmService>) -> Self {
        self.platform.utility_llm_service = service;
        self
    }

    /// Set the platform-wide session filesystem factory.
    pub fn session_file_system_factory(
        mut self,
        factory: Arc<dyn SessionFileSystemFactory>,
    ) -> Self {
        self.platform.session_file_system_factory = factory;
        self
    }

    /// Set the platform-wide vector store for Knowledge Index embeddings.
    pub fn vector_store(mut self, vector_store: Arc<dyn VectorStore>) -> Self {
        self.platform.vector_store = vector_store;
        self
    }

    /// Build the platform definition.
    pub fn build(self) -> PlatformDefinition {
        self.platform
    }
}

impl Default for PlatformDefinitionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connector::{ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField};
    use crate::{CapabilityStatus, CurrentTimeCapability};
    use async_trait::async_trait;

    struct TestProvider;

    #[async_trait]
    impl Connector for TestProvider {
        fn provider_id(&self) -> &str {
            "test_provider"
        }

        fn display_name(&self) -> &str {
            "Test Provider"
        }

        fn description(&self) -> &str {
            "Test connection provider"
        }

        fn icon(&self) -> &str {
            "plug"
        }

        fn connection_type(&self) -> ConnectorType {
            ConnectorType::ApiKey
        }

        fn form_schema(&self) -> Option<ConnectorFormSchema> {
            Some(ConnectorFormSchema {
                fields: vec![FormField::password("api_key", "API Key").required()],
                instructions_markdown: "Enter the API key.".to_string(),
            })
        }

        async fn validate(&self, _credential: &str) -> Result<ConnectorValidation, String> {
            Ok(ConnectorValidation {
                provider_username: Some("test-user".to_string()),
                provider_metadata: None,
            })
        }
    }

    /// Chat driver stub: registration-only, never invoked in these tests.
    struct StubChatDriver;

    #[async_trait]
    impl crate::ChatDriver for StubChatDriver {
        async fn chat_completion_stream(
            &self,
            _endpoint: &crate::ProviderEndpoint,
            _messages: Vec<crate::LlmMessage>,
            _config: &crate::LlmCallConfig,
        ) -> crate::Result<crate::LlmResponseStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    #[test]
    fn test_platform_definition_builder() {
        let mut drivers = DriverRegistry::new();
        let mut descriptor = crate::driver_registry::DriverDescriptor::chat_only(
            crate::DriverId::LlmSim,
            |_config| Box::new(StubChatDriver) as crate::BoxedChatDriver,
        );
        descriptor.display_name = "Stub".into();
        drivers.register_descriptor_or_replace(descriptor);

        let platform = PlatformDefinition::builder()
            .driver_registry(drivers.clone())
            .capability(CurrentTimeCapability)
            .connector(TestProvider)
            .build();

        assert!(platform.capability_registry().has("current_time"));
        assert!(platform.connectors().has("test_provider"));
        assert!(
            platform
                .driver_registry()
                .has_driver(&crate::DriverId::LlmSim)
        );
    }

    #[test]
    fn test_platform_definition_mutation() {
        let mut platform = PlatformDefinition::default();
        platform
            .capability_registry_mut()
            .register(CurrentTimeCapability);
        platform.connectors_mut().register(TestProvider);

        let info = crate::CapabilityInfo::from_core(
            platform
                .capability_registry()
                .get("current_time")
                .expect("current_time registered")
                .as_ref(),
        );
        assert_eq!(info.status, CapabilityStatus::Available);
        assert!(platform.connectors().has("test_provider"));
    }
}
