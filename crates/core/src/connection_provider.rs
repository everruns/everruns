// Connection Provider Plugin System
//
// Decision: Parallel to IntegrationPlugin, allows integration crates to register
// connection providers via inventory::submit! without core knowing about them.
// Decision: Form schema is backend-driven — providers define their own UI fields
// and instructions, frontend renders generically.
// Decision: Validation is async — providers can call external APIs to verify credentials.

use async_trait::async_trait;
use serde::Serialize;

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
