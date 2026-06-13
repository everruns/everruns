//! Platform definition for embeddable Everruns deployments.
//!
//! `PlatformDefinition` is the shared composition root for server and worker
//! runtime surface. Embedders can add or remove capabilities, LLM drivers,
//! connection providers, and built-in harness templates without patching
//! internal startup code.
//!
//! Server-only concerns such as route wiring, auth backends, and background
//! task scheduling stay outside this module so the type can be reused from any
//! binary crate. Platform-wide host services belong here as factories, not as
//! server-owned concrete dependencies.

use crate::{
    Capability, CapabilityRegistry, Connector, ConnectorRegistry, DriverRegistry, EgressService,
    EmailSender, UtilityLlmService,
    traits::{DisabledSessionFileSystemFactory, SessionFileSystemFactory},
};
use serde_json::Value;
use std::sync::Arc;

/// Stable role assigned to a built-in harness template.
///
/// Roles let the server resolve special harness behavior without assuming a
/// specific harness name. For example, a platform can provide a base harness
/// named "Minimal" and still mark it as the `Base` harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltInHarnessRole {
    /// Harness used when session creation omits `harness_id` and the org has no
    /// explicit `base_harness_id` configured yet.
    Base,
    /// Harness selected as the default in organization settings when the org is
    /// first initialized.
    Default,
    /// Harness used by the global chat endpoint.
    Chat,
}

/// Capability entry for a built-in harness template.
#[derive(Debug, Clone)]
pub struct BuiltInCapabilityDefinition {
    /// Capability identifier.
    pub id: String,
    /// Per-harness capability config passed to capability resolution.
    pub config: Value,
}

impl BuiltInCapabilityDefinition {
    /// Create a capability entry with an empty config object.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            config: serde_json::json!({}),
        }
    }

    /// Create a capability entry with explicit config.
    pub fn with_config(id: impl Into<String>, config: Value) -> Self {
        Self {
            id: id.into(),
            config,
        }
    }
}

/// Built-in harness template provisioned by a platform definition.
///
/// Built-in harnesses are identified by `name` (unique per org). IDs are
/// assigned by the seeder at provisioning time — never hardcoded.
#[derive(Debug, Clone)]
pub struct BuiltInHarnessDefinition {
    /// Name, unique per org (e.g. "generic").
    pub name: String,
    /// Human-readable display name shown in UI.
    pub display_name: String,
    /// Human-readable description.
    pub description: String,
    /// Base system prompt for the harness.
    pub system_prompt: String,
    /// Optional parent harness name to inherit from during provisioning.
    pub parent_name: Option<String>,
    /// Tags applied to the harness.
    pub tags: Vec<String>,
    /// Capabilities enabled by default for the harness.
    pub capabilities: Vec<BuiltInCapabilityDefinition>,
    /// Special roles for platform behavior.
    pub roles: Vec<BuiltInHarnessRole>,
}

impl BuiltInHarnessDefinition {
    /// Create a built-in harness template.
    pub fn new(
        name: impl Into<String>,
        display_name: impl Into<String>,
        description: impl Into<String>,
        system_prompt: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            display_name: display_name.into(),
            description: description.into(),
            system_prompt: system_prompt.into(),
            parent_name: None,
            tags: Vec::new(),
            capabilities: Vec::new(),
            roles: Vec::new(),
        }
    }

    /// Replace the harness tags.
    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    /// Set the parent harness name used for inheritance during provisioning.
    pub fn with_parent_name(mut self, parent_name: impl Into<String>) -> Self {
        self.parent_name = Some(parent_name.into());
        self
    }

    /// Replace the harness capabilities.
    pub fn with_capabilities<I>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = BuiltInCapabilityDefinition>,
    {
        self.capabilities = capabilities.into_iter().collect();
        self
    }

    /// Replace the harness roles.
    pub fn with_roles<I>(mut self, roles: I) -> Self
    where
        I: IntoIterator<Item = BuiltInHarnessRole>,
    {
        self.roles = roles.into_iter().collect();
        self
    }

    /// Check whether this harness has a specific role.
    pub fn has_role(&self, role: BuiltInHarnessRole) -> bool {
        self.roles.contains(&role)
    }
}

/// Shared definition of the Everruns platform surface.
///
/// `PlatformDefinition` lets an embedder decide which capabilities, LLM
/// drivers, connection providers, built-in harness templates, and platform
/// service factories exist at runtime. Server and worker code should consume
/// the same definition so the control plane and execution plane stay aligned.
///
/// # Example
///
/// ```rust,ignore
/// use everruns_core::{
///     BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole,
///     DriverRegistry, PlatformDefinition,
/// };
///
/// let mut drivers = DriverRegistry::new();
/// everruns_openai::register_driver(&mut drivers);
///
/// let platform = PlatformDefinition::builder()
///     .driver_registry(drivers)
///     .capability(everruns_core::CurrentTimeCapability)
///     .add_built_in_harness(
///         BuiltInHarnessDefinition::new(
///             "minimal",
///             "Minimal",
///             "Small default harness for an embedded deployment.",
///             "You are a helpful assistant.",
///         )
///         .with_roles([BuiltInHarnessRole::Base, BuiltInHarnessRole::Default])
///         .with_capabilities([BuiltInCapabilityDefinition::new("current_time")]),
///     )
///     .build();
/// ```
#[derive(Clone)]
pub struct PlatformDefinition {
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    connectors: ConnectorRegistry,
    built_in_harnesses: Vec<BuiltInHarnessDefinition>,
    egress_service: Arc<dyn EgressService>,
    email_sender: Arc<dyn EmailSender>,
    utility_llm_service: Arc<dyn UtilityLlmService>,
    session_file_system_factory: Arc<dyn SessionFileSystemFactory>,
}

impl PlatformDefinition {
    /// Create a platform definition from explicit registries.
    pub fn new(capability_registry: CapabilityRegistry, driver_registry: DriverRegistry) -> Self {
        Self {
            capability_registry,
            driver_registry,
            connectors: ConnectorRegistry::new(),
            built_in_harnesses: Vec::new(),
            egress_service: Arc::new(crate::DirectEgressService::default()),
            email_sender: Arc::new(crate::DisabledEmailSender),
            utility_llm_service: Arc::new(crate::DisabledUtilityLlmService),
            session_file_system_factory: Arc::new(DisabledSessionFileSystemFactory),
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

    /// Built-in harness templates provisioned by this platform.
    pub fn built_in_harnesses(&self) -> &[BuiltInHarnessDefinition] {
        &self.built_in_harnesses
    }

    /// Mutable access to the built-in harness templates.
    pub fn built_in_harnesses_mut(&mut self) -> &mut Vec<BuiltInHarnessDefinition> {
        &mut self.built_in_harnesses
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

    /// Append a built-in harness template.
    pub fn add_built_in_harness(&mut self, harness: BuiltInHarnessDefinition) {
        self.built_in_harnesses.push(harness);
    }

    /// Find the first built-in harness with the requested role.
    pub fn harness_for_role(&self, role: BuiltInHarnessRole) -> Option<&BuiltInHarnessDefinition> {
        self.built_in_harnesses.iter().find(|h| h.has_role(role))
    }
}

impl Default for PlatformDefinition {
    fn default() -> Self {
        Self::new(CapabilityRegistry::new(), DriverRegistry::new())
    }
}

impl std::fmt::Debug for PlatformDefinition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let harness_keys: Vec<_> = self.built_in_harnesses.iter().map(|h| &h.name).collect();
        f.debug_struct("PlatformDefinition")
            .field("capabilities", &self.capability_registry)
            .field("drivers", &self.driver_registry.registered_providers())
            .field("connectors", &self.connectors)
            .field("built_in_harnesses", &harness_keys)
            .field("egress_service", &self.egress_service.name())
            .field("email_sender", &self.email_sender.name())
            .field("utility_llm_service", &self.utility_llm_service.name())
            .field(
                "session_file_system_factory",
                &self.session_file_system_factory.name(),
            )
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

    /// Replace the built-in harness templates.
    pub fn built_in_harnesses<I>(mut self, harnesses: I) -> Self
    where
        I: IntoIterator<Item = BuiltInHarnessDefinition>,
    {
        self.platform.built_in_harnesses = harnesses.into_iter().collect();
        self
    }

    /// Append a built-in harness template.
    pub fn add_built_in_harness(mut self, harness: BuiltInHarnessDefinition) -> Self {
        self.platform.built_in_harnesses.push(harness);
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
    use crate::connector::{
        ConnectorFormSchema, ConnectorType, ConnectorValidation, FieldType, FormField,
    };
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
                fields: vec![FormField {
                    name: "api_key".to_string(),
                    label: "API Key".to_string(),
                    field_type: FieldType::Password,
                    required: true,
                    placeholder: None,
                    help_text: None,
                }],
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

    #[test]
    fn test_platform_definition_builder() {
        let mut drivers = DriverRegistry::new();
        crate::llmsim_driver::register_driver(&mut drivers);

        let platform = PlatformDefinition::builder()
            .driver_registry(drivers.clone())
            .capability(CurrentTimeCapability)
            .connector(TestProvider)
            .add_built_in_harness(
                BuiltInHarnessDefinition::new(
                    "minimal",
                    "Minimal",
                    "Minimal harness",
                    "You are helpful.",
                )
                .with_roles([BuiltInHarnessRole::Base, BuiltInHarnessRole::Default]),
            )
            .build();

        assert!(platform.capability_registry().has("current_time"));
        assert!(platform.connectors().has("test_provider"));
        assert_eq!(
            platform
                .harness_for_role(BuiltInHarnessRole::Base)
                .unwrap()
                .name,
            "minimal"
        );
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
        platform.add_built_in_harness(
            BuiltInHarnessDefinition::new("chat", "Chat", "Chat harness", "You are helpful.")
                .with_roles([BuiltInHarnessRole::Chat]),
        );

        let info = crate::CapabilityInfo::from_core(
            platform
                .capability_registry()
                .get("current_time")
                .expect("current_time registered")
                .as_ref(),
        );
        assert_eq!(info.status, CapabilityStatus::Available);
        assert!(platform.connectors().has("test_provider"));
        assert_eq!(
            platform
                .harness_for_role(BuiltInHarnessRole::Chat)
                .unwrap()
                .name,
            "chat"
        );
    }
}
