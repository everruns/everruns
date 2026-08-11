// Connector Plugin System
//
// Decision (EVE-879): the connector catalog is a hosted control-plane surface
// — the server renders its form schemas and resolves connections; nothing
// consumes a Connector during a turn — so it lives in `everruns-platform`,
// not `everruns-core`.
// Decision: Parallel to IntegrationPlugin, allows integration crates to register
// connectors via inventory::submit! without the kernel knowing about them.
// Decision: Form schema is backend-driven — connectors define their own UI fields
// and instructions, frontend renders generically.
// Decision: Validation is async — connectors can call external APIs to verify credentials.

use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Plugin Registration
// ============================================================================

/// Plugin registration point for connector crates.
///
/// Integration crates use `inventory::submit!` to register their connectors.
/// The server discovers them at runtime to serve form schemas
/// and handle credential submission.
///
/// # Example
///
/// ```ignore
/// inventory::submit! {
///     ConnectorPlugin {
///         experimental_only: true,
///         factory: || Box::new(DaytonaConnector),
///     }
/// }
/// ```
pub struct ConnectorPlugin {
    /// If true, only registered when experimental features are enabled.
    pub experimental_only: bool,
    /// Factory function that creates the provider instance.
    pub factory: fn() -> Box<dyn Connector>,
}

inventory::collect!(ConnectorPlugin);

// ============================================================================
// Connector Trait
// ============================================================================

/// How the user provides credentials for this connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorType {
    /// OAuth flow (redirect-based, e.g. GitHub)
    OAuth,
    /// Direct API key entry (form-based, e.g. Daytona)
    ApiKey,
}

/// A connection provider that can validate credentials and describe its UI form.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Unique provider identifier (e.g. "daytona"). Must match `user_connections.provider`.
    fn provider_id(&self) -> &str;

    /// Human-readable name (e.g. "Daytona").
    fn display_name(&self) -> &str;

    /// Short description for the connections UI.
    fn description(&self) -> &str;

    /// Lucide icon name (e.g. "cloud", "github").
    fn icon(&self) -> &str;

    /// Whether this provider uses OAuth or direct API key entry.
    fn connection_type(&self) -> ConnectorType;

    /// Form schema for API key providers. OAuth providers return None.
    fn form_schema(&self) -> Option<ConnectorFormSchema>;

    /// Validate a credential before saving. Called for API key providers.
    /// Returns Ok with optional metadata on success, Err with user-facing message on failure.
    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String>;

    /// Validate with all form fields. Default delegates to `validate()` using the `api_key` field.
    /// Override this when the provider needs extra fields (e.g. org slug for personal tokens).
    async fn validate_fields(
        &self,
        fields: &HashMap<String, String>,
    ) -> Result<ConnectorValidation, String> {
        let api_key = fields.get("api_key").map(|s| s.as_str()).unwrap_or("");
        self.validate(api_key).await
    }
}

// ============================================================================
// Connector Registry
// ============================================================================

/// Registry of connection providers available to a platform.
///
/// This is the explicit counterpart to `ConnectorPlugin`. Inventory-based
/// discovery remains useful for the default OSS platform, but embedders need a
/// concrete registry they can edit before handing the platform to the server.
#[derive(Clone, Default)]
pub struct ConnectorRegistry {
    providers: HashMap<String, Arc<dyn Connector>>,
}

impl ConnectorRegistry {
    /// Create an empty provider registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a connection provider.
    ///
    /// If a provider with the same `provider_id()` already exists, it is replaced.
    pub fn register(&mut self, provider: impl Connector + 'static) {
        self.providers
            .insert(provider.provider_id().to_string(), Arc::new(provider));
    }

    /// Register a boxed connection provider.
    pub fn register_boxed(&mut self, provider: Box<dyn Connector>) {
        self.providers
            .insert(provider.provider_id().to_string(), Arc::from(provider));
    }

    /// Register an `Arc`-wrapped connection provider.
    pub fn register_arc(&mut self, provider: Arc<dyn Connector>) {
        self.providers
            .insert(provider.provider_id().to_string(), provider);
    }

    /// Remove a provider by ID.
    pub fn unregister(&mut self, provider_id: &str) -> Option<Arc<dyn Connector>> {
        self.providers.remove(provider_id)
    }

    /// Get a provider by ID.
    pub fn get(&self, provider_id: &str) -> Option<&Arc<dyn Connector>> {
        self.providers.get(provider_id)
    }

    /// Check whether a provider is registered.
    pub fn has(&self, provider_id: &str) -> bool {
        self.providers.contains_key(provider_id)
    }

    /// List all registered providers.
    pub fn list(&self) -> Vec<&Arc<dyn Connector>> {
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
    pub fn builder() -> ConnectorRegistryBuilder {
        ConnectorRegistryBuilder::new()
    }
}

impl std::fmt::Debug for ConnectorRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<_> = self.providers.keys().collect();
        f.debug_struct("ConnectorRegistry")
            .field("providers", &ids)
            .finish()
    }
}

/// Builder for creating a `ConnectorRegistry`.
pub struct ConnectorRegistryBuilder {
    registry: ConnectorRegistry,
}

impl ConnectorRegistryBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self {
            registry: ConnectorRegistry::new(),
        }
    }

    /// Add a connection provider to the registry.
    pub fn provider(mut self, provider: impl Connector + 'static) -> Self {
        self.registry.register(provider);
        self
    }

    /// Build the registry.
    pub fn build(self) -> ConnectorRegistry {
        self.registry
    }
}

impl Default for ConnectorRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Form Schema Types
// ============================================================================

// Form schema types are shared with provider drivers; see
// `everruns_core::credential_schema` and knowledge/foundations/providers.md
// "Credentials". The schema itself stays in `everruns-provider`.
pub use everruns_core::credential_schema::{FieldType, FormField};

/// Credential form schema for connectors.
pub type ConnectorFormSchema = everruns_core::credential_schema::CredentialFormSchema;

/// Result of credential validation.
#[derive(Debug, Clone)]
pub struct ConnectorValidation {
    /// Display name from the provider (e.g. organization name, username).
    pub provider_username: Option<String>,
    /// Provider-specific metadata to store alongside the connection (e.g. org slug).
    /// Stored as JSONB in the database and returned via the connection resolver.
    pub provider_metadata: Option<serde_json::Value>,
}
