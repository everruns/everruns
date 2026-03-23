//! Platform definition for embeddable Everruns deployments.
//!
//! `PlatformDefinition` is the shared composition root for server and worker
//! runtime surface. Embedders can add or remove capabilities, LLM drivers,
//! connection providers, and built-in harness templates without patching
//! internal startup code.
//!
//! Server-only concerns such as route wiring, auth backends, and background
//! task scheduling stay outside this module so the type can be reused from any
//! binary crate.

use crate::{
    Capability, CapabilityRegistry, ConnectionProvider, ConnectionProviderRegistry, DriverRegistry,
};
use serde_json::Value;
use uuid::Uuid;

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
#[derive(Debug, Clone)]
pub struct BuiltInHarnessDefinition {
    /// Stable key for the template inside the platform definition.
    ///
    /// This key is for code-level identification and should not be shown to
    /// users. Use values like `base`, `generic`, or `platform_chat`.
    pub key: String,
    /// Fixed UUID used for the default org when backward compatibility matters.
    ///
    /// Other organizations still receive fresh UUIDs; the template name and
    /// `is_built_in` flag are used to reconcile them.
    pub seed_id: Option<Uuid>,
    /// Display name shown in the UI and API.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Base system prompt for the harness.
    pub system_prompt: String,
    /// Optional parent harness key to inherit from during provisioning.
    pub parent_key: Option<String>,
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
        key: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        system_prompt: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            seed_id: None,
            name: name.into(),
            description: description.into(),
            system_prompt: system_prompt.into(),
            parent_key: None,
            tags: Vec::new(),
            capabilities: Vec::new(),
            roles: Vec::new(),
        }
    }

    /// Set the default-org seed UUID used for backward compatibility.
    pub fn with_seed_id(mut self, seed_id: Uuid) -> Self {
        self.seed_id = Some(seed_id);
        self
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

    /// Set the parent harness key used for inheritance during provisioning.
    pub fn with_parent_key(mut self, parent_key: impl Into<String>) -> Self {
        self.parent_key = Some(parent_key.into());
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
/// drivers, connection providers, and built-in harness templates exist at
/// runtime. Server and worker code should consume the same definition so the
/// control plane and execution plane stay aligned.
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
    connection_providers: ConnectionProviderRegistry,
    built_in_harnesses: Vec<BuiltInHarnessDefinition>,
}

impl PlatformDefinition {
    /// Create a platform definition from explicit registries.
    pub fn new(capability_registry: CapabilityRegistry, driver_registry: DriverRegistry) -> Self {
        Self {
            capability_registry,
            driver_registry,
            connection_providers: ConnectionProviderRegistry::new(),
            built_in_harnesses: Vec::new(),
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

    /// Immutable access to the connection-provider registry.
    pub fn connection_providers(&self) -> &ConnectionProviderRegistry {
        &self.connection_providers
    }

    /// Mutable access to the connection-provider registry.
    pub fn connection_providers_mut(&mut self) -> &mut ConnectionProviderRegistry {
        &mut self.connection_providers
    }

    /// Built-in harness templates provisioned by this platform.
    pub fn built_in_harnesses(&self) -> &[BuiltInHarnessDefinition] {
        &self.built_in_harnesses
    }

    /// Mutable access to the built-in harness templates.
    pub fn built_in_harnesses_mut(&mut self) -> &mut Vec<BuiltInHarnessDefinition> {
        &mut self.built_in_harnesses
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
        let harness_keys: Vec<_> = self.built_in_harnesses.iter().map(|h| &h.key).collect();
        f.debug_struct("PlatformDefinition")
            .field("capabilities", &self.capability_registry)
            .field("drivers", &self.driver_registry.registered_providers())
            .field("connection_providers", &self.connection_providers)
            .field("built_in_harnesses", &harness_keys)
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
    pub fn connection_providers(mut self, registry: ConnectionProviderRegistry) -> Self {
        self.platform.connection_providers = registry;
        self
    }

    /// Register a connection provider on the platform.
    pub fn connection_provider(mut self, provider: impl ConnectionProvider + 'static) -> Self {
        self.platform.connection_providers.register(provider);
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
    use crate::connection_provider::{
        ConnectionFormSchema, ConnectionType, ConnectionValidation, FieldType, FormField,
    };
    use crate::{CapabilityStatus, CurrentTimeCapability};
    use async_trait::async_trait;

    struct TestProvider;

    #[async_trait]
    impl ConnectionProvider for TestProvider {
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

        fn connection_type(&self) -> ConnectionType {
            ConnectionType::ApiKey
        }

        fn form_schema(&self) -> Option<ConnectionFormSchema> {
            Some(ConnectionFormSchema {
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

        async fn validate(&self, _credential: &str) -> Result<ConnectionValidation, String> {
            Ok(ConnectionValidation {
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
            .connection_provider(TestProvider)
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
        assert!(platform.connection_providers().has("test_provider"));
        assert_eq!(
            platform
                .harness_for_role(BuiltInHarnessRole::Base)
                .unwrap()
                .name,
            "Minimal"
        );
        assert!(
            platform
                .driver_registry()
                .has_driver(&crate::ProviderType::LlmSim)
        );
    }

    #[test]
    fn test_platform_definition_mutation() {
        let mut platform = PlatformDefinition::default();
        platform
            .capability_registry_mut()
            .register(CurrentTimeCapability);
        platform.connection_providers_mut().register(TestProvider);
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
        assert!(platform.connection_providers().has("test_provider"));
        assert_eq!(
            platform
                .harness_for_role(BuiltInHarnessRole::Chat)
                .unwrap()
                .key,
            "chat"
        );
    }
}
