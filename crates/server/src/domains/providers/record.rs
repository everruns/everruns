//! Persisted provider connection records.
use chrono::{DateTime, Utc};
use everruns_contracts::provider::{DriverId, ProviderRequestOptions, ProviderTraceConfig};
use everruns_contracts::typed_id::ProviderId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// LLM provider status
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Active,
    Disabled,
}

/// LLM Provider entity (API keys never exposed)
/// This is the persisted provider entity, separate from the runtime provider trait.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Provider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[schema(value_type = String, example = "provider_01933b5a00007000800000000000001")]
    pub id: ProviderId,
    /// Human-readable provider name. Safe to render in user-facing messages.
    pub name: String,
    /// Provider implementation type (OpenAI, Anthropic, Gemini, etc.).
    pub provider_type: DriverId,
    /// Custom base URL for self-hosted / proxied providers. `None` means use the provider's default endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Whether an API key is configured. The key itself is never returned.
    pub api_key_set: bool,
    /// Current lifecycle status of this provider.
    pub status: ProviderStatus,
    /// Whether this provider is host-managed (EVE-810). A managed provider is
    /// provisioned by the host/embedder; the OSS API rejects tenant PATCH/DELETE
    /// on it (403). Read-only to org admins. Defaults to `false`.
    pub managed: bool,
    /// Timestamp of the most recent successful model sync from the provider's API (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<DateTime<Utc>>,
    /// When someone last reviewed this provider's discovered models (RFC 3339).
    /// A discovered model created later that is still disabled is reported as
    /// `is_new` on the model. `None` means never reviewed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models_reviewed_at: Option<DateTime<Utc>>,
    /// Timestamp when this provider was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this provider was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
    /// Resolved trace/observability link configuration: the driver's default
    /// templates overlaid with this provider's stored overrides. `None` when the
    /// driver exposes no dashboard and the org configured nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<ProviderTraceConfig>,
    /// Extra headers and diagnostics options applied to every request sent to
    /// this provider. `None` when the org configured nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_options: Option<ProviderRequestOptions>,
}
