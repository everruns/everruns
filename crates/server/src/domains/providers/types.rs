// LLM providers domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::kernel_imports::{contracts::provider::DriverId, contracts::provider::ProviderStatus};
use everruns_contracts::provider::{ProviderRequestOptions, ProviderTraceConfig};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Request to create a new LLM provider
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProviderRequest {
    /// Display name for the provider.
    #[schema(example = "OpenAI Production")]
    pub name: String,
    /// The type of LLM provider (e.g., openai, anthropic).
    pub provider_type: DriverId,
    /// Base URL for the provider's API. Required for custom endpoints.
    /// For standard providers, this can be omitted to use the default URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://api.openai.com/v1")]
    pub base_url: Option<String>,
    /// API key for authenticating with the provider.
    /// Will be encrypted at rest if encryption is configured.
    ///
    /// Single-field convenience for simple providers and programmatic clients.
    /// Multi-field drivers (Bedrock, MAI) should send `credentials` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Typed credential fields keyed by the driver's declared credential-schema
    /// field names. Validated against the schema and assembled into the stored
    /// credential document. Takes precedence over `api_key` when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<std::collections::BTreeMap<String, String>>,
    /// Trace/observability link configuration. Stored as a per-provider override
    /// of the driver's default templates; omit to keep driver defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<ProviderTraceConfig>,
    /// Extra headers and diagnostics options applied to every request sent to
    /// this provider. Omit to configure none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_options: Option<ProviderRequestOptions>,
}

/// Response from syncing models from a provider
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncModelsResponse {
    /// Sync completed successfully
    Success {
        /// Number of new models discovered
        created: usize,
        /// Number of existing models updated
        updated: usize,
        /// Number of models marked as stale (not seen in this sync)
        stale: usize,
    },
    /// Provider doesn't support model discovery
    NotSupported,
}

/// Request to update an LLM provider. Only provided fields will be updated.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProviderRequest {
    /// Display name for the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "OpenAI Development")]
    pub name: Option<String>,
    /// The type of LLM provider (e.g., openai, anthropic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<DriverId>,
    /// Base URL for the provider's API.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://api.openai.com/v1")]
    pub base_url: Option<String>,
    /// API key for authenticating with the provider.
    /// Will be encrypted at rest if encryption is configured.
    ///
    /// Single-field convenience for simple providers and programmatic clients.
    /// Multi-field drivers (Bedrock, MAI) should send `credentials` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Typed credential fields keyed by the driver's declared credential-schema
    /// field names. Validated against the schema and assembled into the stored
    /// credential document. Takes precedence over `api_key` when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<std::collections::BTreeMap<String, String>>,
    /// The status of the provider. Set to "inactive" to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ProviderStatus>,
    /// Trace/observability link configuration override. Merged into the
    /// provider's stored settings, preserving other settings keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<ProviderTraceConfig>,
    /// Extra headers and diagnostics options applied to every request sent to
    /// this provider. Replaces the stored options wholesale; omit to leave them
    /// unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_options: Option<ProviderRequestOptions>,
}
