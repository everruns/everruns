//! `everruns-provider` — the provider/LLM abstraction foundation.
//!
//! This crate holds the driver-facing types and traits shared by
//! `everruns-core` and the individual provider crates (OpenAI, Anthropic,
//! Gemini, …): the `ChatDriver` interface, the shared OpenAI/OpenResponses
//! protocol drivers, model profiles, retry/stream helpers, typed IDs, the
//! credential form schema, and the LLM error taxonomy.
//!
//! The goal is that provider crates depend on this crate instead of on
//! `everruns-core`, so a provider is a pure `ChatDriver` implementation with no
//! dependency on core's agent-loop runtime.
//!
//! `everruns-core` depends on this crate and re-exports these modules at their
//! original paths, so existing `everruns_core::…` imports keep working. The
//! adapters that convert core's agent-loop domain types (`Message`,
//! `RuntimeAgent`, `ResolvedModel`) into these driver types live in
//! `everruns-core` (`llm_conversions`), keeping the dependency one-directional.

pub mod credential_schema;
pub mod driver_helpers;
pub mod driver_registry;
pub mod error;
pub mod execution_phase;
pub mod llm_retry;
pub mod model;
pub mod model_profiles;
pub mod openai_protocol;
pub mod openresponses_protocol;
pub mod openresponses_types;
pub mod provider;
pub mod stream_accumulator;
pub mod stream_reconnect;
pub mod tool_types;
pub mod typed_id;
pub mod url_validation;
pub mod user_facing_error;

// ============================================================================
// Convenience root re-exports
//
// Mirror the root-level re-exports `everruns-core` exposes for these modules,
// so provider crates can use `everruns_provider::<Symbol>` exactly as they used
// `everruns_core::<Symbol>`.
// ============================================================================

pub use credential_schema::{
    CredentialFormSchema, assemble_credential_document, parse_credential_document,
};
pub use driver_registry::{
    BoxedChatDriver, BoxedEmbeddingsDriver, ChatDriver, DiscoveredModel, DriverDescriptor,
    DriverFactory, DriverId, DriverOAuthConfig, DriverOAuthFlow, DriverRegistry, EmbedRequest,
    EmbedResponse, EmbeddingsDriver, EmbeddingsDriverError, EmbeddingsDriverFactory, LlmCallConfig,
    LlmCallConfigBuilder, LlmCompletionMetadata, LlmContentPart, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmResponse, LlmResponseStream, LlmStreamError, LlmStreamEvent, ProviderConfig,
    ProviderOpaqueContext, ServiceKind, fold_system_messages,
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
pub use model_profiles::{get_model_profile, get_model_vendor};
pub use openai_protocol::{AuthHeaderProvider, OpenAIProtocolChatDriver};
pub use openresponses_protocol::{
    CompactContent, CompactContentPart, CompactInputItem, CompactOutputItem, CompactRequest,
    CompactResponse, CompactUsage, OpenResponsesProtocolChatDriver, OpenResponsesRequestExtension,
    messages_to_compact_input,
};
pub use provider::{Provider, ProviderStatus, ProviderTraceConfig};
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
