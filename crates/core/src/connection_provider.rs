// Connection Provider Plugin System
//
// Decision: Parallel to IntegrationPlugin, allows integration crates to register
// connection providers via inventory::submit! without core knowing about them.
// Decision: Form schema is backend-driven — providers define their own UI fields
// and instructions, frontend renders generically.
// Decision: Validation is async — providers can call external APIs to verify credentials.

use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Plugin Registration
// ============================================================================

/// Plugin registration point for connection provider crates.
///
/// Integration crates use `inventory::submit!` to register their connection
/// providers. The server discovers them at runtime to serve form schemas
/// and handle credential submission.
///
/// # Example
///
/// ```ignore
/// inventory::submit! {
///     ConnectionProviderPlugin {
///         experimental_only: true,
///         factory: || Box::new(DaytonaConnectionProvider),
///     }
/// }
/// ```
pub struct ConnectionProviderPlugin {
    /// If true, only registered when experimental features are enabled.
    pub experimental_only: bool,
    /// Factory function that creates the provider instance.
    pub factory: fn() -> Box<dyn ConnectionProvider>,
}

inventory::collect!(ConnectionProviderPlugin);

// ============================================================================
// ConnectionProvider Trait
// ============================================================================

/// How the user provides credentials for this connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionType {
    /// OAuth flow (redirect-based, e.g. GitHub)
    OAuth,
    /// Direct API key entry (form-based, e.g. Daytona)
    ApiKey,
}

/// A connection provider that can validate credentials and describe its UI form.
#[async_trait]
pub trait ConnectionProvider: Send + Sync {
    /// Unique provider identifier (e.g. "daytona"). Must match `user_connections.provider`.
    fn provider_id(&self) -> &str;

    /// Human-readable name (e.g. "Daytona").
    fn display_name(&self) -> &str;

    /// Short description for the connections UI.
    fn description(&self) -> &str;

    /// Lucide icon name (e.g. "cloud", "github").
    fn icon(&self) -> &str;

    /// Whether this provider uses OAuth or direct API key entry.
    fn connection_type(&self) -> ConnectionType;

    /// Form schema for API key providers. OAuth providers return None.
    fn form_schema(&self) -> Option<ConnectionFormSchema>;

    /// Validate a credential before saving. Called for API key providers.
    /// Returns Ok with optional metadata on success, Err with user-facing message on failure.
    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String>;
}

// ============================================================================
// ConnectionProvider Registry
// ============================================================================

/// Registry of connection providers available to a platform.
///
/// This is the explicit counterpart to `ConnectionProviderPlugin`. Inventory-based
/// discovery remains useful for the default OSS platform, but embedders need a
/// concrete registry they can edit before handing the platform to the server.
#[derive(Clone, Default)]
pub struct ConnectionProviderRegistry {
    providers: HashMap<String, Arc<dyn ConnectionProvider>>,
}

impl ConnectionProviderRegistry {
    /// Create an empty provider registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a connection provider.
    ///
    /// If a provider with the same `provider_id()` already exists, it is replaced.
    pub fn register(&mut self, provider: impl ConnectionProvider + 'static) {
        self.providers
            .insert(provider.provider_id().to_string(), Arc::new(provider));
    }

    /// Register a boxed connection provider.
    pub fn register_boxed(&mut self, provider: Box<dyn ConnectionProvider>) {
        self.providers
            .insert(provider.provider_id().to_string(), Arc::from(provider));
    }

    /// Register an `Arc`-wrapped connection provider.
    pub fn register_arc(&mut self, provider: Arc<dyn ConnectionProvider>) {
        self.providers
            .insert(provider.provider_id().to_string(), provider);
    }

    /// Remove a provider by ID.
    pub fn unregister(&mut self, provider_id: &str) -> Option<Arc<dyn ConnectionProvider>> {
        self.providers.remove(provider_id)
    }

    /// Get a provider by ID.
    pub fn get(&self, provider_id: &str) -> Option<&Arc<dyn ConnectionProvider>> {
        self.providers.get(provider_id)
    }

    /// Check whether a provider is registered.
    pub fn has(&self, provider_id: &str) -> bool {
        self.providers.contains_key(provider_id)
    }

    /// List all registered providers.
    pub fn list(&self) -> Vec<&Arc<dyn ConnectionProvider>> {
        self.providers.values().collect()
    }

    /// Number of registered providers.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Create a builder for fluent registration.
    pub fn builder() -> ConnectionProviderRegistryBuilder {
        ConnectionProviderRegistryBuilder::new()
    }
}

impl std::fmt::Debug for ConnectionProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<_> = self.providers.keys().collect();
        f.debug_struct("ConnectionProviderRegistry")
            .field("providers", &ids)
            .finish()
    }
}

/// Builder for creating a `ConnectionProviderRegistry`.
pub struct ConnectionProviderRegistryBuilder {
    registry: ConnectionProviderRegistry,
}

impl ConnectionProviderRegistryBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self {
            registry: ConnectionProviderRegistry::new(),
        }
    }

    /// Add a connection provider to the registry.
    pub fn provider(mut self, provider: impl ConnectionProvider + 'static) -> Self {
        self.registry.register(provider);
        self
    }

    /// Build the registry.
    pub fn build(self) -> ConnectionProviderRegistry {
        self.registry
    }
}

impl Default for ConnectionProviderRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Form Schema Types
// ============================================================================

/// Describes the form fields and instructions for an API key connection.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionFormSchema {
    /// Input fields to render.
    pub fields: Vec<FormField>,
    /// Markdown instructions shown above the form (how to get the key, etc.).
    pub instructions_markdown: String,
}

/// A single form field.
#[derive(Debug, Clone, Serialize)]
pub struct FormField {
    /// Field name used as the key when submitting (e.g. "api_key").
    pub name: String,
    /// Label shown next to the input.
    pub label: String,
    /// Input type.
    pub field_type: FieldType,
    /// Whether the field is required.
    pub required: bool,
    /// Placeholder text inside the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Help text shown below the input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

/// Input field type for rendering.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    /// Masked password/secret input.
    Password,
    /// Plain text input.
    Text,
    /// URL input.
    Url,
}

/// Result of credential validation.
#[derive(Debug, Clone)]
pub struct ConnectionValidation {
    /// Display name from the provider (e.g. organization name, username).
    pub provider_username: Option<String>,
}
