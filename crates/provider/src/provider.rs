// Provider entity types (knowledge/foundations/providers.md)
//
// A Provider is an org-scoped instance of a driver: a configured vendor
// account (credentials, endpoint) that powers services like chat. DriverId
// names the driver implementation a provider uses.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::ProviderId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Open string identifier retained as the 0.17.x integration-kind name.
///
/// Despite the legacy type name, this is not a built-in-provider enum: any
/// normalized string is valid. The associated constants keep source
/// compatibility for the 0.17.x runtime adapter and persisted HTTP shapes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriverId(std::borrow::Cow<'static, str>);

impl std::fmt::Display for DriverId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl DriverId {
    #[allow(non_upper_case_globals)]
    pub const OpenAI: Self = Self(std::borrow::Cow::Borrowed("openai"));
    #[allow(non_upper_case_globals)]
    pub const OpenRouter: Self = Self(std::borrow::Cow::Borrowed("openrouter"));
    #[allow(non_upper_case_globals)]
    pub const AzureOpenAI: Self = Self(std::borrow::Cow::Borrowed("azure_openai"));
    #[allow(non_upper_case_globals)]
    pub const OpenAICompletions: Self = Self(std::borrow::Cow::Borrowed("openai_completions"));
    #[allow(non_upper_case_globals)]
    pub const Anthropic: Self = Self(std::borrow::Cow::Borrowed("anthropic"));
    #[allow(non_upper_case_globals)]
    pub const Gemini: Self = Self(std::borrow::Cow::Borrowed("gemini"));
    #[allow(non_upper_case_globals)]
    pub const LlmSim: Self = Self(std::borrow::Cow::Borrowed("llmsim"));
    #[allow(non_upper_case_globals)]
    pub const Bedrock: Self = Self(std::borrow::Cow::Borrowed("bedrock"));
    #[allow(non_upper_case_globals)]
    pub const Mai: Self = Self(std::borrow::Cow::Borrowed("mai"));
    #[allow(non_upper_case_globals)]
    pub const Fireworks: Self = Self(std::borrow::Cow::Borrowed("fireworks"));
    #[allow(non_upper_case_globals)]
    pub const Meta: Self = Self(std::borrow::Cow::Borrowed("meta"));

    /// Construct an external driver id from its canonical wire id.
    ///
    /// The id is normalized to lowercase so registration and lookup match
    /// case-insensitively, consistent with built-in parsing.
    pub fn external(id: impl AsRef<str>) -> Self {
        Self(std::borrow::Cow::Owned(
            id.as_ref().trim().to_ascii_lowercase(),
        ))
    }

    /// Return the canonical string identifier for this provider.
    pub fn as_str(&self) -> &str {
        self.0.as_ref()
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
        if self == &DriverId::OpenRouter {
            (
                Some("https://openrouter.ai/logs?id={response_id}".to_string()),
                Some("https://openrouter.ai/logs".to_string()),
            )
        } else {
            (None, None)
        }
    }
}

impl std::str::FromStr for DriverId {
    // Parsing never fails: unknown ids become `External`.
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Normalize once so built-in matching and the External id share the
        // same lowercased form; casing variance never yields duplicate ids.
        Ok(DriverId::external(s))
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
        if s.trim().is_empty() {
            return Err(serde::de::Error::custom("provider type cannot be empty"));
        }
        if s != s.trim() {
            return Err(serde::de::Error::custom(
                "provider type cannot have leading or trailing whitespace",
            ));
        }
        // FromStr is infallible (unknown ids become External).
        Ok(s.parse().unwrap_or_else(|_| unreachable!()))
    }
}

#[cfg(test)]
mod tests {
    use super::DriverId;

    #[test]
    fn wire_ids_reject_empty_and_padded_values() {
        for value in ["", " ", "\t\n"] {
            let error = serde_json::from_value::<DriverId>(serde_json::json!(value)).unwrap_err();
            assert!(error.to_string().contains("provider type cannot be empty"));
        }
        let error = serde_json::from_value::<DriverId>(serde_json::json!(" openai ")).unwrap_err();
        assert!(error.to_string().contains("leading or trailing whitespace"));
    }

    #[test]
    fn wire_ids_normalize_case_and_accept_extensions() {
        assert_eq!(
            serde_json::from_str::<DriverId>(r#""Custom-Driver""#)
                .unwrap()
                .as_str(),
            "custom-driver"
        );
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
                 openai_completions, anthropic, gemini, llmsim, bedrock, mai, fireworks, meta. \
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

/// One extra HTTP header sent with every request to a provider connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ProviderRequestHeader {
    /// Header name, e.g. `x-gateway-tenant`.
    pub name: String,
    /// Header value, sent verbatim.
    pub value: String,
}

/// Per-connection request options: what an org wants added to every outbound
/// request to this provider, beyond endpoint and credentials.
///
/// These are connection-level on purpose. A gateway header or a diagnostics
/// opt-in describes the *service* an org talks to, not one agent's behavior, so
/// it belongs next to the base URL and credentials rather than on every agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ProviderRequestOptions {
    /// Extra HTTP headers added to every request to this provider. They
    /// override the driver's and the connection's own headers by name, but
    /// connection-level headers (`host`, `content-length`, ...) are ignored.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<ProviderRequestHeader>,
    /// Ask the provider to explain unexpected prompt-cache misses. Honored by
    /// drivers with a diagnostics protocol (today: Anthropic's
    /// `cache-diagnosis` beta); ignored elsewhere.
    #[serde(default)]
    pub cache_diagnostics: bool,
}

impl ProviderRequestOptions {
    /// Whether these options change anything about an outbound request.
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && !self.cache_diagnostics
    }

    /// The headers as the `(name, value)` pairs drivers merge onto a request.
    pub fn header_pairs(&self) -> Vec<(String, String)> {
        self.headers
            .iter()
            .map(|header| (header.name.clone(), header.value.clone()))
            .collect()
    }
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
    /// Whether this provider is host-managed (EVE-810). A managed provider is
    /// provisioned by the host/embedder; the OSS API rejects tenant PATCH/DELETE
    /// on it (403). Read-only to org admins. Defaults to `false`.
    pub managed: bool,
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
    /// Extra headers and diagnostics options applied to every request sent to
    /// this provider. `None` when the org configured nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_options: Option<ProviderRequestOptions>,
}
