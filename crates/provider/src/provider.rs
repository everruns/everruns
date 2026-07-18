// Provider entity types (specs/providers.md)
//
// A Provider is an org-scoped instance of a driver: a configured vendor
// account (credentials, endpoint) that powers services like chat. DriverId
// names the driver implementation a provider uses.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::ProviderId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// LLM provider type identifier.
///
/// Built-in variants cover providers shipped with everruns. Any string that
/// does not match a built-in id becomes `External(id)`, so embedders can store
/// and use custom provider ids without schema migrations.
///
/// Serializes to/from a plain string (e.g. `"anthropic"`, `"openai-codex"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DriverId {
    /// OpenAI using Open Responses API (<https://www.openresponses.org/>)
    OpenAI,
    /// OpenRouter using the OpenAI-compatible Responses API
    OpenRouter,
    /// Azure OpenAI using the Azure-hosted OpenAI v1 API
    AzureOpenAI,
    /// OpenAI using Chat Completions API (for backward compatibility)
    OpenAICompletions,
    Anthropic,
    /// Google Gemini API
    Gemini,
    /// LLM simulator for testing
    LlmSim,
    /// AWS Bedrock Runtime (ConverseStream API)
    Bedrock,
    /// Microsoft MAI models (e.g. MAI-Code-1-Flash) served via Azure AI Foundry.
    /// Uses an OpenAI-compatible Chat Completions API and supports either an
    /// Azure AI Foundry API key or Microsoft Entra ID (OAuth) authentication.
    Mai,
    /// Fireworks AI — open-model inference (Llama, Qwen, DeepSeek, GLM, ...)
    /// served via an OpenAI-compatible Chat Completions API.
    Fireworks,
    /// Embedder-defined provider not compiled into everruns-core. The inner id
    /// is the canonical wire string (e.g. `"openai-codex"`).
    External(std::sync::Arc<str>),
}

impl std::fmt::Display for DriverId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl DriverId {
    /// Construct an external driver id from its canonical wire id.
    ///
    /// The id is normalized to lowercase so registration and lookup match
    /// case-insensitively, consistent with built-in parsing.
    pub fn external(id: impl Into<std::sync::Arc<str>>) -> Self {
        let id: std::sync::Arc<str> = id.into();
        // Avoid reallocating when the id is already lowercase.
        if id.bytes().any(|b| b.is_ascii_uppercase()) {
            DriverId::External(std::sync::Arc::from(id.to_lowercase().as_str()))
        } else {
            DriverId::External(id)
        }
    }

    /// Return the canonical string identifier for this provider.
    pub fn as_str(&self) -> &str {
        match self {
            DriverId::OpenAI => "openai",
            DriverId::OpenRouter => "openrouter",
            DriverId::AzureOpenAI => "azure_openai",
            DriverId::OpenAICompletions => "openai_completions",
            DriverId::Anthropic => "anthropic",
            DriverId::Gemini => "gemini",
            DriverId::LlmSim => "llmsim",
            DriverId::Bedrock => "bedrock",
            DriverId::Mai => "mai",
            DriverId::Fireworks => "fireworks",
            DriverId::External(id) => id.as_ref(),
        }
    }

    /// Default trace-link URL templates for this driver, as
    /// `(generation_url_template, session_url_template)`.
    ///
    /// These are best-effort defaults for vendors that expose an observability
    /// dashboard. They are only *defaults*: an org overrides them per provider
    /// (`ProviderTraceConfig`) and must opt in via `enabled`, since most vendors
    /// retain prompt/completion content only when logging is explicitly turned
    /// on. Templates support the `{response_id}`, `{session_id}`, `{turn_id}`
    /// and `{model}` placeholders.
    ///
    /// OpenRouter stores logged generations on its **Logs** page
    /// (<https://openrouter.ai/logs>, gated behind the account's
    /// "Input & Output Logging" Observability setting). OpenRouter does not
    /// document a public deep-link by generation id, so the generation template
    /// passes the id best-effort; worst case it lands on the Logs page where the
    /// generation can be found by recency.
    pub fn default_trace_templates(&self) -> (Option<String>, Option<String>) {
        match self {
            DriverId::OpenRouter => (
                Some("https://openrouter.ai/logs?id={response_id}".to_string()),
                Some("https://openrouter.ai/logs".to_string()),
            ),
            _ => (None, None),
        }
    }
}

impl std::str::FromStr for DriverId {
    // Parsing never fails: unknown ids become `External`.
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Normalize once so built-in matching and the External id share the
        // same lowercased form; casing variance never yields duplicate ids.
        let lower = s.to_lowercase();
        Ok(match lower.as_str() {
            "openai" => DriverId::OpenAI,
            "openrouter" => DriverId::OpenRouter,
            "azure_openai" => DriverId::AzureOpenAI,
            "openai_completions" => DriverId::OpenAICompletions,
            "anthropic" => DriverId::Anthropic,
            "gemini" => DriverId::Gemini,
            "llmsim" => DriverId::LlmSim,
            "bedrock" => DriverId::Bedrock,
            "mai" => DriverId::Mai,
            "fireworks" => DriverId::Fireworks,
            _ => DriverId::External(std::sync::Arc::from(lower.as_str())),
        })
    }
}

impl Serialize for DriverId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DriverId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        // FromStr is infallible (unknown ids become External).
        Ok(s.parse().unwrap_or_else(|_| unreachable!()))
    }
}

// `Arc<str>` does not implement `ToSchema`, so the schema is written by hand.
// It is a plain string at the wire level regardless of the variant.
#[cfg(feature = "openapi")]
impl utoipa::ToSchema for DriverId {
    fn name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("DriverId")
    }
}

#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for DriverId {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::SchemaType::new(
                utoipa::openapi::schema::Type::String,
            ))
            .description(Some(
                "LLM provider type. Built-in: openai, openrouter, azure_openai, \
                 openai_completions, anthropic, gemini, llmsim, bedrock, mai, fireworks. \
                 Any other string is treated as an embedder-defined external provider.",
            ))
            .build()
            .into()
    }
}

/// LLM provider status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Active,
    Disabled,
}

/// Configuration for linking from the chat UI to a provider's observability
/// dashboard ("trace"/"logs").
///
/// This is provider-agnostic: any driver with a dashboard can supply default
/// templates (see [`DriverId::default_trace_templates`]), and an org enables
/// links per provider once it has confirmed logging is on for that account.
/// URL templates support the `{response_id}`, `{session_id}`, `{turn_id}` and
/// `{model}` placeholders, so the same mechanism works for OpenRouter today and
/// for third-party observability backends (Langfuse, Helicone, ...) via an
/// override.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ProviderTraceConfig {
    /// Whether trace links should be shown for this provider. Defaults to
    /// `false`: vendors typically do not retain trace content unless logging is
    /// explicitly enabled, so the org opts in once that is set up.
    pub enabled: bool,
    /// URL template for a single generation's trace, e.g.
    /// `"https://openrouter.ai/logs?id={response_id}"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_url_template: Option<String>,
    /// URL template for a session's grouped trace, e.g.
    /// `"https://openrouter.ai/logs"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_url_template: Option<String>,
}

/// LLM Provider entity (API keys never exposed)
/// Note: This is the entity struct, separate from the Provider trait in llm.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Provider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "provider_01933b5a00007000800000000000001"))]
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
    /// Timestamp of the most recent successful model sync from the provider's API (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Timestamp when this provider was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this provider was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
    /// Resolved trace/observability link configuration: the driver's default
    /// templates overlaid with this provider's stored overrides. `None` when the
    /// driver exposes no dashboard and the org configured nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<ProviderTraceConfig>,
}
