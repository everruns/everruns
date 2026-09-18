//! Payloads describing one LLM generation: request options, metadata and outcome.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use super::*;

/// LLM generation output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationOutput {
    /// Text response from the model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Tool calls requested by the model
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

/// Request options applied to an LLM generation.
///
/// These fields capture request-side intent such as prompt caching or deferred
/// tool loading. They complement `usage`, which captures what actually happened.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmRequestOptions {
    /// Sampling temperature sent with the request, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Maximum output tokens requested, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    /// Reasoning / thinking effort level requested, as the string sent to the
    /// provider (`low`, `medium`, `high`, ...), when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Whether the request used the provider's streaming mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Prompt caching configuration for this request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache: Option<LlmPromptCacheInfo>,
    /// Deferred tool-loading configuration for this request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_search: Option<LlmToolSearchInfo>,
    /// Provider-specific request options that do not warrant dedicated fields.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub provider_options: HashMap<String, Value>,
    /// General request metadata passed to the LLM provider for tracking and observability.
    /// Includes embedder-supplied labels merged with system tracking keys (session_id, turn_id, etc.).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

impl LlmRequestOptions {
    pub fn is_empty(&self) -> bool {
        self.temperature.is_none()
            && self.max_tokens.is_none()
            && self.reasoning_effort.is_none()
            && self.stream.is_none()
            && self.prompt_cache.is_none()
            && self.tool_search.is_none()
            && self.provider_options.is_empty()
            && self.metadata.is_empty()
    }
}

/// Request-side prompt cache settings for an LLM generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmPromptCacheInfo {
    /// Whether prompt caching was enabled on the request.
    pub enabled: bool,
    /// Strategy used to enable prompt caching.
    pub strategy: crate::driver_registry::PromptCacheStrategy,
    /// Provider-specific prompt-cache mode used by the driver.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_mode: Option<String>,
}

/// Request-side tool_search settings for an LLM generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmToolSearchInfo {
    /// Whether tool_search was enabled on the request.
    pub enabled: bool,
    /// Minimum number of tools before deferred loading activates.
    pub threshold: usize,
}

/// Metadata about an LLM generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationMetadata {
    /// Model identifier used for generation
    #[cfg_attr(feature = "openapi", schema(example = "claude-sonnet-4-5"))]
    pub model: String,

    /// Provider type (openai, anthropic, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "anthropic"))]
    pub provider: Option<String>,

    /// Token usage statistics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,

    /// Duration of the generation in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 1_842u64))]
    pub duration_ms: Option<u64>,

    /// Time to first token in milliseconds (streaming latency)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = 312u64))]
    pub time_to_first_token_ms: Option<u64>,

    /// Whether the generation was successful
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub success: bool,

    /// Error message if generation failed
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "provider returned 503"))]
    pub error: Option<String>,

    /// Finish reasons from the LLM (e.g., ["stop"], ["tool_calls"])
    /// Required for gen-ai semantic conventions
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = json!(["tool_calls"])))]
    pub finish_reasons: Option<Vec<String>>,

    /// Unique response identifier from the LLM provider
    /// Required for gen-ai semantic conventions
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "msg_01ABCDef0123456789"))]
    pub response_id: Option<String>,

    /// Retry information if rate limit retries occurred
    /// Contains number of retries and total wait time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<LlmRetryInfo>,

    /// Compaction information if context was compressed before generation
    /// Occurs when the conversation context exceeded the model's limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compaction: Option<LlmCompactionInfo>,

    /// Request-side driver options that were enabled for this generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_options: Option<LlmRequestOptions>,
}

/// Information about rate limit retries during LLM generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmRetryInfo {
    /// Number of retry attempts made (0 = succeeded on first try)
    pub attempts: u32,

    /// Total time spent waiting between retries in milliseconds
    pub total_wait_ms: u64,
}

/// Information about context compaction performed before LLM generation
///
/// When the conversation context exceeds the model's limit, compaction is
/// automatically triggered to compress the context before retrying.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmCompactionInfo {
    /// Whether compaction was performed
    pub compacted: bool,

    /// Number of input tokens before compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_before: Option<u32>,

    /// Number of input tokens after compaction
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_after: Option<u32>,

    /// Duration of the compaction operation in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,

    /// Provider-reported cost of the compaction call itself, in USD.
    ///
    /// Compaction is a separate billable model call, so its cost is also folded
    /// into the generation's `usage.actual_cost_usd` — that is what budgets and
    /// `llm_generations` read. This field keeps the split visible, so an
    /// operator can see how much of a turn's spend was compaction (EVE-895).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl LlmCompactionInfo {
    /// Create info for a successful compaction
    pub fn new(
        input_tokens_before: Option<u32>,
        input_tokens_after: Option<u32>,
        duration_ms: Option<u64>,
        cost_usd: Option<f64>,
    ) -> Self {
        Self {
            compacted: true,
            input_tokens_before,
            input_tokens_after,
            duration_ms,
            cost_usd,
        }
    }
}

/// Data for llm.generation event
///
/// Emitted after each LLM API call to provide full visibility into
/// the messages sent to the model and the response received.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmGenerationData {
    /// Messages sent to the LLM (including system prompt)
    pub messages: Vec<RuntimeMessage>,

    /// Tools available to the LLM for this generation
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinitionSummary>,

    /// Output from the LLM
    pub output: LlmGenerationOutput,

    /// Metadata about the generation
    pub metadata: LlmGenerationMetadata,
}

impl LlmGenerationData {
    /// Create a successful generation event
    #[allow(clippy::too_many_arguments)]
    pub fn success(
        messages: Vec<RuntimeMessage>,
        tools: Vec<ToolDefinitionSummary>,
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        model: String,
        provider: Option<String>,
        usage: Option<TokenUsage>,
        duration_ms: Option<u64>,
        time_to_first_token_ms: Option<u64>,
    ) -> Self {
        // Infer finish reasons from content
        let finish_reasons = if !tool_calls.is_empty() {
            Some(vec!["tool_calls".to_string()])
        } else {
            Some(vec!["stop".to_string()])
        };

        Self {
            messages,
            tools,
            output: LlmGenerationOutput { text, tool_calls },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage,
                duration_ms,
                time_to_first_token_ms,
                success: true,
                error: None,
                finish_reasons,
                response_id: None,
                retry: None,
                compaction: None,
                request_options: None,
            },
        }
    }

    /// Create a successful generation event with full metadata
    #[allow(clippy::too_many_arguments)]
    pub fn success_with_metadata(
        messages: Vec<RuntimeMessage>,
        tools: Vec<ToolDefinitionSummary>,
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        model: String,
        provider: Option<String>,
        usage: Option<TokenUsage>,
        duration_ms: Option<u64>,
        time_to_first_token_ms: Option<u64>,
        finish_reasons: Option<Vec<String>>,
        response_id: Option<String>,
    ) -> Self {
        Self {
            messages,
            tools,
            output: LlmGenerationOutput { text, tool_calls },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage,
                duration_ms,
                time_to_first_token_ms,
                success: true,
                error: None,
                finish_reasons,
                response_id,
                retry: None,
                compaction: None,
                request_options: None,
            },
        }
    }

    /// Create a successful generation event with retry information
    #[allow(clippy::too_many_arguments)]
    pub fn success_with_retry(
        messages: Vec<RuntimeMessage>,
        tools: Vec<ToolDefinitionSummary>,
        text: Option<String>,
        tool_calls: Vec<ToolCall>,
        model: String,
        provider: Option<String>,
        usage: Option<TokenUsage>,
        duration_ms: Option<u64>,
        time_to_first_token_ms: Option<u64>,
        finish_reasons: Option<Vec<String>>,
        response_id: Option<String>,
        retry: Option<LlmRetryInfo>,
    ) -> Self {
        Self {
            messages,
            tools,
            output: LlmGenerationOutput { text, tool_calls },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage,
                duration_ms,
                time_to_first_token_ms,
                success: true,
                error: None,
                finish_reasons,
                response_id,
                retry,
                compaction: None,
                request_options: None,
            },
        }
    }

    /// Create a failed generation event
    pub fn failure(
        messages: Vec<RuntimeMessage>,
        tools: Vec<ToolDefinitionSummary>,
        model: String,
        provider: Option<String>,
        error: String,
        duration_ms: Option<u64>,
        time_to_first_token_ms: Option<u64>,
    ) -> Self {
        Self {
            messages,
            tools,
            output: LlmGenerationOutput {
                text: None,
                tool_calls: vec![],
            },
            metadata: LlmGenerationMetadata {
                model,
                provider,
                usage: None,
                duration_ms,
                time_to_first_token_ms,
                success: false,
                error: Some(error),
                finish_reasons: Some(vec!["error".to_string()]),
                response_id: None,
                retry: None,
                compaction: None,
                request_options: None,
            },
        }
    }

    /// Set compaction info on this generation event
    ///
    /// Call this when context was compacted before a successful retry.
    pub fn with_compaction(mut self, compaction: LlmCompactionInfo) -> Self {
        self.metadata.compaction = Some(compaction);
        self
    }

    /// Set retry info on this generation event
    pub fn with_retry(mut self, retry: LlmRetryInfo) -> Self {
        self.metadata.retry = Some(retry);
        self
    }

    /// Set request-side options on this generation event.
    pub fn with_request_options(mut self, request_options: LlmRequestOptions) -> Self {
        if !request_options.is_empty() {
            self.metadata.request_options = Some(request_options);
        }
        self
    }
}

// ============================================================================
// Extended Thinking Event Data Types
// ============================================================================
