//! Lean provider and LLM abstractions shared by the
//! [Everruns Framework](https://docs.everruns.com/framework/) and provider crates.
//!
//! This crate owns credential-free model identity, the [`ChatDriver`] boundary,
//! provider assembly, shared protocol drivers, stream/retry helpers, model
//! profiles, typed IDs, credential schemas, and the LLM error taxonomy. It is a
//! focused implementation crate in the [Everruns](https://everruns.com)
//! ecosystem. Low-level consumers import provider-owned contracts from this
//! crate directly; Framework users normally use the `everruns` facade.
//!
//! # Example
//!
//! ```
//! use everruns_provider::ModelSpec;
//!
//! let model = ModelSpec::on("company-gateway", "assistant-v2");
//! assert_eq!(model.provider.as_str(), "company-gateway");
//! assert_eq!(model.model, "assistant-v2");
//! ```

pub mod compact;
pub mod credential_provider;
pub mod credential_schema;
#[cfg(feature = "http")]
pub mod driver_helpers;
pub mod driver_registry;
pub mod error;
pub mod execution_phase;
pub mod llm_retry;
pub mod model;
pub mod model_discovery;
pub mod model_profiles;
pub mod model_spec;
#[cfg(feature = "http")]
pub mod openai_protocol;
#[cfg(feature = "http")]
pub mod openresponses_protocol;
pub mod openresponses_types;
pub mod provider;
pub mod runtime_provider;
pub mod stream_accumulator;
#[cfg(feature = "http")]
pub mod stream_reconnect;
pub mod tool_schema_compat;
pub mod tool_types;
pub mod typed_id;
pub mod url_validation;
pub mod user_facing_error;

// ============================================================================
// Convenience root re-exports
//
// These root exports are the canonical low-level provider API. `everruns-core`
// consumes a narrow subset privately and deliberately does not mirror them.
// ============================================================================

pub use compact::{
    CompactContent, CompactContentPart, CompactInputItem, CompactOutputItem, CompactRequest,
    CompactResponse, CompactUsage, messages_to_compact_input,
};
pub use credential_provider::{CredentialProvider, EnvCredentialProvider, ProviderCredentials};
pub use credential_schema::{
    CredentialFormSchema, assemble_credential_document, parse_credential_document,
};
pub use driver_registry::{
    BoxedChatDriver, BoxedEmbeddingsDriver, ChatDriver, DiscoveredModel, DriverDescriptor,
    DriverFactory, DriverId, DriverOAuthConfig, DriverOAuthFlow, DriverRegistry, EmbedRequest,
    EmbedResponse, EmbeddingsDriver, EmbeddingsDriverError, EmbeddingsDriverFactory, LlmCallConfig,
    LlmCallConfigBuilder, LlmCompletionMetadata, LlmContentPart, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmResponse, LlmResponseStream, LlmStreamError, LlmStreamEvent, ProviderConfig,
    ProviderMetadata, ProviderOpaqueContext, ServiceKind, fold_system_messages,
};
pub use error::{
    AgentLoopError, FileSystemError, FileSystemErrorClass, LlmError, LlmErrorKind, Result,
    StoreResultExt, classify_fs_error, from_json, json_val,
};
pub use execution_phase::ExecutionPhase;
pub use llm_retry::{LlmRetryConfig, RateLimitInfo, RateLimitType, RetryMetadata};
pub use model::{
    CostTier, Modality, Model, ModelCost, ModelLimits, ModelModalities, ModelProfile, ModelSource,
    ModelVendor, ModelWithProvider, ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue,
};
#[cfg(feature = "http")]
pub use model_discovery::list_openai_compatible_models;
pub use model_discovery::{
    DiscoveredProviderModel, ModelSearchMatch, ModelSearchResult, RankedDiscoveredModels,
    discover_provider_models, enrich_with_profiles, match_models, normalize_and_enrich,
    rank_discovered_models, search_provider_models,
};
pub use model_profiles::{get_model_profile, get_model_vendor};
pub use model_spec::{ModelSpec, UnknownProvider};
#[cfg(feature = "http")]
pub use openai_protocol::OpenAIProtocolChatDriver;
#[cfg(feature = "http")]
pub use openresponses_protocol::{OpenResponsesProtocolChatDriver, OpenResponsesRequestExtension};
pub use provider::{Provider as ProviderRecord, ProviderStatus, ProviderTraceConfig};
pub use runtime_provider::{
    BearerAuth, Provider, ProviderAuth, ProviderAuthRequest, ProviderEndpoint, ProviderKey,
    ProviderRegistry, ResolvedProviderRequest, RuntimeProvider, RuntimeProviderRegistry,
    StaticHeaderAuth,
};
pub use tool_types::{
    BuiltinTool, ClientSideTool, DeferrablePolicy, SideEffectClass, ToolCall, ToolDefinition,
    ToolHints, ToolPolicy, ToolResult, ToolResultImage,
};
pub use url_validation::{
    UrlValidationError, is_blocked_ip, validate_safe_url, validate_url_dns_pinned,
};
pub use user_facing_error::{
    ErrorDisclosure, UserFacingError, UserFacingErrorContext, UserFacingErrorFields,
    classify_runtime_error_message, codes as user_facing_error_codes, is_provider_quota_message,
    is_usage_limit_message, metadata_keys as user_facing_error_metadata_keys,
    parse_usage_limit_reset_at, trim_error_chain_prefixes,
};

/// Select ring as the process-wide rustls crypto provider.
///
/// Products that link more than one rustls provider call this once during
/// startup, before constructing TLS clients. Repeated and concurrent calls are
/// safe: rustls accepts the first installation and later calls are no-ops.
#[cfg(feature = "tls-ring")]
pub fn install_ring_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
