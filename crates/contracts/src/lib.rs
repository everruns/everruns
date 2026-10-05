#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Shared contracts for the [Everruns](https://everruns.com) framework and hosts.
//!
//! Provider and driver boundaries, capability definitions, model profiles,
//! typed IDs, and shared runtime contracts live here. The default build omits
//! HTTP transport; enable `http` for the shared protocol drivers and clients.
//! Application authors normally use the same types through `everruns`.
//!
//! ```
//! use everruns_contracts::{CapabilityRef, ModelSpec};
//! let model = ModelSpec::on("company-gateway", "assistant-v2");
//! let capability = CapabilityRef::new("vendor.search");
//! assert_eq!(model.provider.as_str(), "company-gateway");
//! assert_eq!(capability.id(), "vendor.search");
//! ```

/// Capability identity, configuration, and code-defined authoring.
// Lets moved runtime code and its doctests name this crate by its package name.
extern crate self as everruns_contracts;

#[cfg(feature = "runtime")]
pub mod runtime;
pub mod capability;
/// Model profile metadata and the offline registry, keyed by provider wire ID.
pub mod model_profile_data;

#[cfg(feature = "definition")]
pub use async_trait::async_trait;
pub use capability::{
    ActivationSet, CapabilityError, CapabilityId, CapabilityIdIndex, CapabilityRef, CapabilitySpec,
    CapabilitySpecParts, GENERIC_HARNESS_NAME, IntoCapability, PLUGIN_CAPABILITY_PREFIX,
    RESERVED_CAPABILITY_ID_NAMESPACE, generic_capabilities, is_plugin_capability,
    parse_plugin_capability_id, plugin_capability_id, validate_capability_config,
    validate_capability_id,
};
#[cfg(feature = "definition")]
pub use capability::{Definition, json_schema_for};
#[cfg(feature = "definition")]
pub use schemars;
pub use serde;
pub use serde_json;

pub mod background_call;
pub mod compact;
pub mod credential_provider;
pub mod credential_schema;
#[cfg(feature = "http")]
pub mod driver_helpers;
mod driver_oauth;
pub mod driver_registry;
pub mod error;
pub mod execution_phase;
pub mod form_elicitation_types;
pub mod hosted_mcp;
mod llm_call_config_builder;
pub mod llm_error;
pub mod llm_retry;
pub mod message;
pub mod model;
pub mod model_discovery;
pub mod model_profiles;
pub mod model_spec;
pub mod native_async;
pub mod native_computer;
pub mod openai_compat;
pub mod openai_computer;
#[cfg(feature = "http")]
pub mod openai_errors;
pub mod openai_hosted_tools;
#[cfg(feature = "http")]
mod openai_message_convert;
#[cfg(feature = "http")]
pub mod openai_protocol;
#[cfg(feature = "http")]
mod openai_types;
pub mod openai_wire;
#[cfg(feature = "http")]
pub mod openresponses_protocol;
pub mod openresponses_types;
pub mod provider;
mod provider_managed;
pub mod reasoning;
pub mod rt;
pub mod runtime_provider;
pub mod stream_accumulator;
mod stream_error;
pub mod stream_event;
#[cfg(feature = "http")]
pub mod stream_reconnect;
pub mod structured_output;
pub mod tool_approval_types;
pub mod tool_schema_compat;
pub mod tool_types;
pub mod turn_collector;
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
    LlmCallConfigBuilder, LlmCompletionMetadata, LlmContentPart, LlmResponse, LlmResponseStream,
    LlmStreamError, LlmStreamEvent, Message, MessageContent, MessageRole, ProviderConfig,
    ProviderMetadata, ProviderOpaqueContext, ServiceKind, fold_system_messages,
};
// Pre-0.31 names, kept as deprecated aliases.
pub use error::{
    AgentLoopError, BillingPressureReason, FileSystemError, FileSystemErrorClass, LlmError,
    LlmErrorKind, Result, StoreResultExt, classify_fs_error, from_json, json_val,
};
pub use execution_phase::{ExecutionPhase, PhaseSource};
pub use llm_error::RejectedProviderCapability;
pub use llm_retry::{LlmRetryConfig, RateLimitInfo, RateLimitType, RetryMetadata};
pub use message::ProviderOpaqueContent;
#[allow(deprecated)]
pub use message::{LlmMessage, LlmMessageContent, LlmMessageRole};
pub use model::{
    CostTier, Modality, ModelCost, ModelLimits, ModelModalities, ModelProfile, ModelVendor,
    ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue,
};
#[cfg(feature = "http")]
pub use model_discovery::list_openai_compatible_models;
pub use model_discovery::{
    DiscoveredProviderModel, ModelSearchMatch, ModelSearchResult, RankedDiscoveredModels,
    discover_provider_models, enrich_with_profiles, match_models, normalize_and_enrich,
    rank_discovered_models, search_provider_models,
};
pub use model_profiles::{
    ModelProfileEntry, all_profile_entries, all_profiles, get_model_profile, get_model_vendor,
    profile_entries_for_provider, profiles_for_provider, selected_profiles,
    selected_profiles_for_provider,
};
pub use model_spec::{ModelSpec, UnknownProvider};
#[cfg(feature = "http")]
pub use openai_protocol::OpenAIProtocolChatDriver;
pub use openai_wire::OpenAiWireError;
#[cfg(feature = "http")]
pub use openresponses_protocol::{
    OPENAI_BACKGROUND_OPTION, OPENAI_WEBSOCKET_OPTION, OpenResponsesProtocolChatDriver,
    OpenResponsesRequestExtension,
};
pub use provider::ProviderTraceConfig;
pub use reasoning::{ReasoningContentPart, ReasoningText};
pub use runtime_provider::{
    BearerAuth, Provider, ProviderAuth, ProviderAuthRequest, ProviderEndpoint, ProviderKey,
    ProviderRegistry, ResolvedProviderRequest, RuntimeProvider, RuntimeProviderRegistry,
    StaticHeaderAuth,
};
pub use tool_approval_types::{
    APPROVE_TOOL_CALL_TOOL, TOOL_APPROVAL_CALL_ID_PREFIX, TOOL_APPROVAL_REQUIRED_CODE,
    ToolApprovalRequired,
};
pub use tool_types::{
    ASK_USER_TOOL_NAME, BuiltinTool, CONFIRM_URL_ELICITATION_TOOL, ClientSideTool,
    ConnectionRequired, ConnectionRequiredSubject, DeferrablePolicy,
    FORM_ELICITATION_CALL_ID_PREFIX, FORM_ELICITATION_REQUIRED_CODE, FormElicitationRequired,
    MCP_ELICITATION_ARGUMENT, SideEffectClass, ToolCall, ToolDefinition, ToolHints, ToolPolicy,
    ToolResult, ToolResultImage, URL_ELICITATION_REQUIRED_CODE, UrlElicitationRequired,
    unattended_ask_user_result,
};
pub use turn_collector::{CollectedTurn, TurnLimits, TurnTiming, collect_turn, limit_stream};
pub use url_validation::{
    UrlValidationError, is_blocked_ip, validate_safe_url, validate_url_dns_pinned,
};
pub use user_facing_error::{
    ErrorDisclosure, UserFacingError, UserFacingErrorContext, UserFacingErrorFields,
    classify_runtime_error_message, codes as user_facing_error_codes, is_provider_quota_message,
    is_usage_limit_message, metadata_keys as user_facing_error_metadata_keys,
    parse_usage_limit_reset_at, trim_error_chain_prefixes,
};

/// Select the workspace crypto backend as the process-wide rustls provider.
///
/// The backend is a build-time choice — the workspace standardises on
/// `aws-lc-rs` (EVE-924), so only one is ever linked. Products still call this
/// once during startup, before constructing TLS clients, because rustls refuses
/// to pick a default implicitly when a crate in the graph could install another.
/// Repeated and concurrent calls are safe: rustls accepts the first
/// installation and later calls are no-ops.
#[cfg(feature = "tls-aws-lc-rs")]
pub fn install_default_crypto_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

pub mod reasoning_updates;

pub mod connector;
pub mod knowledge_store;
pub mod session_sqldb;
pub mod vector_store;

pub mod sandbox_checkpoint;
pub mod session_sandbox;
pub mod tools;

/// Session-bound Slack effects; credentials stay with the control plane.
pub mod slack_action;

pub mod decision_driver;
pub mod decisions;
