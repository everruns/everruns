// Open Responses Protocol Driver
//
// Implementation of the Open Responses specification (https://www.openresponses.org/)
// an open-source, vendor-neutral API standard for multi-provider LLM interfaces.
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff, respecting x-ratelimit-reset-* and retry-after headers.
// Retry metadata is included in the response for observability.
//
// The spec is inspired by and interoperable with the OpenAI Responses API, offering:
// - One spec, many providers (OpenAI, Anthropic, Gemini, local models)
// - Agentic loop support with tool calls and state machines
// - Semantic streaming events (not raw text deltas)
// - 40-80% better cache utilization vs Chat Completions API
// - Native stateful conversation support
//
// Specification: https://www.openresponses.org/specification
// GitHub: https://github.com/openresponses/openresponses
//
// The Chat Completions API remains supported for backward compatibility.

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use crate::error::{AgentLoopError, Result};
use crate::llm_driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmContentPart, LlmDriver, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmResponseStream, LlmStreamEvent,
};
use crate::llm_retry::{
    LlmRetryConfig, RateLimitInfo, RetryMetadata, is_rate_limit_status, is_transient_error,
};
use crate::openai_protocol::{is_openai_model_not_found, is_openai_request_too_large};
use crate::openresponses_types::{self as types, StreamingEvent};
use crate::tool_types::{ToolCall, ToolDefinition};

const DEFAULT_API_URL: &str = "https://api.openai.com/v1/responses";

/// Open Responses Protocol Driver (OpenAI implementation)
///
/// Implements `LlmDriver` using the Open Responses specification
/// (<https://www.openresponses.org/>). This driver targets OpenAI's API
/// but follows the vendor-neutral Open Responses standard.
///
/// Rate limit handling: On 429 errors, automatically retries with exponential
/// backoff, respecting `x-ratelimit-reset-*` and `retry-after` headers.
///
/// The Open Responses spec is recommended for new projects, offering:
/// - Better performance with reasoning models (o1, o3, GPT-5)
/// - Provider-agnostic streaming events
/// - Native agentic loop support
///
/// # Example
///
/// ```ignore
/// use everruns_core::OpenResponsesProtocolLlmDriver;
///
/// let driver = OpenResponsesProtocolLlmDriver::from_env()?;
/// // or
/// let driver = OpenResponsesProtocolLlmDriver::new("your-api-key");
/// // or with custom endpoint
/// let driver = OpenResponsesProtocolLlmDriver::with_base_url("your-api-key", "https://api.example.com/v1/responses");
/// // or with custom retry config
/// let driver = OpenResponsesProtocolLlmDriver::new("your-api-key")
///     .with_retry_config(LlmRetryConfig::aggressive());
/// ```
#[derive(Clone)]
pub struct OpenResponsesProtocolLlmDriver {
    client: Client,
    api_key: String,
    api_url: String,
    /// Retry configuration for rate limit errors
    retry_config: LlmRetryConfig,
}

impl OpenResponsesProtocolLlmDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_url: DEFAULT_API_URL.to_string(),
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// Create a new driver from the OPENAI_API_KEY environment variable
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| AgentLoopError::llm("OPENAI_API_KEY environment variable not set"))?;
        Ok(Self::new(api_key))
    }

    /// Create a new driver with a custom API URL
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_url: api_url.into(),
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// Configure retry behavior for rate limit errors
    pub fn with_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Get the API URL
    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    /// Get the API key (for subclass access)
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Get the HTTP client (for subclass access)
    pub fn client(&self) -> &Client {
        &self.client
    }

    fn convert_role(role: &LlmMessageRole) -> &'static str {
        match role {
            LlmMessageRole::System => "developer", // Responses API uses "developer" for system
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        }
    }

    fn convert_message(msg: &LlmMessage, supports_phases: bool) -> ResponsesInputItem {
        // Handle tool result messages differently
        // Note: OpenAI Responses API function_call_output only supports text output.
        // Images in tool results are dropped with a warning.
        if msg.role == LlmMessageRole::Tool
            && let Some(tool_call_id) = &msg.tool_call_id
        {
            let mut has_images = false;
            let output = match &msg.content {
                LlmMessageContent::Text(text) => text.clone(),
                LlmMessageContent::Parts(parts) => {
                    has_images = parts
                        .iter()
                        .any(|p| matches!(p, LlmContentPart::Image { .. }));
                    parts
                        .iter()
                        .filter_map(|p| match p {
                            LlmContentPart::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("")
                }
            };
            if has_images {
                tracing::warn!(
                    tool_call_id = %tool_call_id,
                    "OpenResponses API does not support images in tool results; images dropped"
                );
            }
            return ResponsesInputItem::FunctionCallOutput {
                r#type: "function_call_output".to_string(),
                call_id: tool_call_id.clone(),
                output,
            };
        }

        let content = match &msg.content {
            LlmMessageContent::Text(text) => ResponsesContent::Text(text.clone()),
            LlmMessageContent::Parts(parts) => {
                let responses_parts: Vec<ResponsesContentPart> = parts
                    .iter()
                    .map(|part| match part {
                        LlmContentPart::Text { text } => ResponsesContentPart::InputText {
                            r#type: "input_text".to_string(),
                            text: text.clone(),
                        },
                        LlmContentPart::Image { url } => ResponsesContentPart::InputImage {
                            r#type: "input_image".to_string(),
                            image_url: url.clone(),
                        },
                        LlmContentPart::Audio { url } => ResponsesContentPart::InputAudio {
                            r#type: "input_audio".to_string(),
                            input_audio: ResponsesInputAudio {
                                data: url.clone(),
                                format: "wav".to_string(),
                            },
                        },
                    })
                    .collect();
                ResponsesContent::Parts(responses_parts)
            }
        };

        // Only include phase on assistant messages when the model supports it.
        // Map ExecutionPhase enum to the provider's wire format string.
        let phase = if supports_phases && msg.role == LlmMessageRole::Assistant {
            msg.phase.map(|p| p.as_provider_str().to_string())
        } else {
            None
        };

        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: Self::convert_role(&msg.role).to_string(),
            content,
            phase,
        }
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<ResponsesTool> {
        tools
            .iter()
            .map(|tool| ResponsesTool::Function {
                r#type: "function".to_string(),
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters().clone(),
                defer_loading: None,
            })
            .collect()
    }

    /// Convert tools with tool_search support: groups tools into namespaces,
    /// marks them as deferred, and appends a `tool_search` entry.
    fn convert_tools_with_search(tools: &[ToolDefinition], threshold: usize) -> Vec<ResponsesTool> {
        use crate::tool_types::DeferrablePolicy;
        use std::collections::HashMap;

        // Below threshold: fall back to standard conversion
        if tools.len() < threshold {
            return Self::convert_tools(tools);
        }

        let mut namespaces: HashMap<String, Vec<ResponsesTool>> = HashMap::new();
        let mut ungrouped = vec![];
        let mut never_defer = vec![];

        for tool in tools {
            let should_defer = match tool.deferrable() {
                DeferrablePolicy::Never => false,
                DeferrablePolicy::Automatic | DeferrablePolicy::Always => true,
            };

            let func = ResponsesTool::Function {
                r#type: "function".to_string(),
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters().clone(),
                defer_loading: if should_defer { Some(true) } else { None },
            };

            if !should_defer {
                never_defer.push(func);
            } else {
                match tool.category() {
                    Some(cat) => {
                        namespaces.entry(cat.to_string()).or_default().push(func);
                    }
                    None => ungrouped.push(func),
                }
            }
        }

        let mut result: Vec<ResponsesTool> = Vec::new();

        // Non-deferred tools first (always visible to model)
        result.extend(never_defer);

        // Namespaced tools
        for (name, tools) in namespaces {
            let description = format!("Tools for {name}");
            result.push(ResponsesTool::Namespace {
                r#type: "namespace".to_string(),
                name,
                description,
                tools,
            });
        }

        // Ungrouped deferred tools
        result.extend(ungrouped);

        // Add tool_search activator
        result.push(ResponsesTool::ToolSearch {
            r#type: "tool_search".to_string(),
        });

        result
    }

    /// Compact a conversation to reduce context size
    ///
    /// This method calls the /v1/responses/compact endpoint to compress the conversation
    /// history. User messages are kept verbatim, while assistant messages, tool calls,
    /// and tool results are replaced by an encrypted compaction item.
    ///
    /// # Arguments
    ///
    /// * `request` - The compact request containing the model and input items
    ///
    /// # Returns
    ///
    /// Returns a `CompactResponse` containing the compacted output items.
    /// The output can be used directly as input for the next /v1/responses call.
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_core::{OpenResponsesProtocolLlmDriver, CompactRequest, CompactInputItem, CompactContent};
    ///
    /// let driver = OpenResponsesProtocolLlmDriver::new("your-api-key");
    ///
    /// let request = CompactRequest {
    ///     model: "gpt-4o".to_string(),
    ///     input: vec![
    ///         CompactInputItem::Message {
    ///             role: "user".to_string(),
    ///             content: CompactContent::Text("Hello!".to_string()),
    ///         },
    ///     ],
    ///     previous_response_id: None,
    ///     instructions: None,
    /// };
    ///
    /// let response = driver.compact(request).await?;
    /// // Use response.output as input for the next /v1/responses call
    /// ```
    pub async fn compact(&self, request: CompactRequest) -> Result<CompactResponse> {
        // Build the compact endpoint URL
        // Replace /v1/responses with /v1/responses/compact
        let compact_url = if self.api_url.ends_with("/responses") {
            format!("{}/compact", self.api_url)
        } else if self.api_url.ends_with("/responses/") {
            format!("{}compact", self.api_url)
        } else {
            // Custom URL - just append /compact
            format!("{}/compact", self.api_url.trim_end_matches('/'))
        };

        // Retry loop for rate limit (429) and transient errors
        let mut retry_metadata = RetryMetadata::default();
        let mut last_error: Option<String> = None;

        let response = loop {
            let response = self
                .client
                .post(&compact_url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .map_err(|e| {
                    AgentLoopError::llm(format!("Failed to send compact request: {}", e))
                })?;

            let status = response.status();

            if status.is_success() {
                break response;
            }

            // Check if this is a retryable error
            if is_transient_error(status) && retry_metadata.attempts < self.retry_config.max_retries
            {
                let rate_limit_info = if is_rate_limit_status(status) {
                    Some(RateLimitInfo::from_openai_headers(response.headers()))
                } else {
                    None
                };

                let error_text = response.text().await.unwrap_or_default();

                let wait_duration = rate_limit_info
                    .as_ref()
                    .map(|info| info.recommended_wait(&self.retry_config, retry_metadata.attempts))
                    .unwrap_or_else(|| {
                        self.retry_config.calculate_backoff(retry_metadata.attempts)
                    });

                tracing::warn!(
                    status = %status,
                    attempt = retry_metadata.attempts + 1,
                    max_retries = self.retry_config.max_retries,
                    wait_secs = wait_duration.as_secs_f64(),
                    "OpenResponsesDriver: compact rate limit or transient error, retrying"
                );

                retry_metadata.record_retry(wait_duration, rate_limit_info);
                last_error = Some(error_text);

                tokio::time::sleep(wait_duration).await;
                continue;
            }

            // Non-retryable error or max retries exceeded
            let error_text = response.text().await.unwrap_or_default();

            // Check if this is a model-not-found error
            if is_openai_model_not_found(status, &error_text) {
                return Err(AgentLoopError::model_not_available(request.model.clone()));
            }

            // Check if this is a request-too-large error (context length exceeded)
            if is_openai_request_too_large(status, &error_text) {
                return Err(AgentLoopError::request_too_large(format!(
                    "OpenAI Responses compact API ({}): {}",
                    status, error_text
                )));
            }

            let error_msg = format!(
                "OpenAI Responses compact API error ({}): {}",
                status, error_text
            );

            if retry_metadata.attempts > 0 {
                return Err(AgentLoopError::llm(format!(
                    "{} (after {} retries, last error: {})",
                    error_msg,
                    retry_metadata.attempts,
                    last_error.unwrap_or_default()
                )));
            }

            return Err(AgentLoopError::llm(error_msg));
        };

        if retry_metadata.had_retries() {
            tracing::info!(
                attempts = retry_metadata.attempts,
                total_wait_secs = retry_metadata.total_retry_wait.as_secs_f64(),
                "OpenResponsesDriver: compact request succeeded after retries"
            );
        }

        // Parse the response
        let compact_response: CompactResponse = response
            .json()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to parse compact response: {}", e)))?;

        Ok(compact_response)
    }

    /// Check if this driver supports the compact endpoint
    ///
    /// Returns true for OpenAI's Responses API. Custom endpoints may or may not
    /// support compaction.
    pub fn supports_compact(&self) -> bool {
        // We assume compact is supported for the default OpenAI endpoint
        // For custom endpoints, callers should try and handle errors gracefully
        self.api_url.starts_with("https://api.openai.com/")
    }

    /// Build input items from messages, extracting system/developer instructions
    ///
    /// Handles the conversion of:
    /// - Assistant messages with tool_calls into separate FunctionCall items
    /// - Assistant messages with thinking into Reasoning items (for o-series/GPT-5 models)
    fn build_input(
        messages: &[LlmMessage],
        supports_phases: bool,
    ) -> (Option<String>, Vec<ResponsesInputItem>) {
        let mut instructions: Option<String> = None;
        let mut input_items = Vec::new();
        // Counter for generating reasoning item IDs
        let mut reasoning_counter = 0u32;

        for msg in messages {
            if msg.role == LlmMessageRole::System {
                // Extract system message as instructions
                instructions = Some(match &msg.content {
                    LlmMessageContent::Text(text) => text.clone(),
                    LlmMessageContent::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| match p {
                            LlmContentPart::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join(""),
                });
            } else if msg.role == LlmMessageRole::Assistant {
                // For assistant messages, emit Reasoning item BEFORE message content if present
                // This is required for o-series and GPT-5 models with extended thinking
                if let Some(encrypted_content) = &msg.thinking_signature {
                    reasoning_counter += 1;
                    input_items.push(ResponsesInputItem::Reasoning {
                        r#type: "reasoning".to_string(),
                        id: format!("rs_{:08x}", reasoning_counter),
                        encrypted_content: encrypted_content.clone(),
                    });
                    tracing::debug!(
                        encrypted_len = encrypted_content.len(),
                        "OpenResponses: including reasoning item in request"
                    );
                }

                // Handle tool calls
                if msg.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty()) {
                    // First emit the message content if non-empty
                    let has_content = match &msg.content {
                        LlmMessageContent::Text(text) => !text.is_empty(),
                        LlmMessageContent::Parts(parts) => !parts.is_empty(),
                    };
                    if has_content {
                        input_items.push(Self::convert_message(msg, supports_phases));
                    }

                    // Then emit FunctionCall items for each tool call
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            input_items.push(ResponsesInputItem::FunctionCall {
                                r#type: "function_call".to_string(),
                                call_id: tc.id.clone(),
                                name: tc.name.clone(),
                                arguments: tc.arguments.to_string(),
                            });
                        }
                    }
                } else {
                    input_items.push(Self::convert_message(msg, supports_phases));
                }
            } else {
                input_items.push(Self::convert_message(msg, supports_phases));
            }
        }

        (instructions, input_items)
    }
}

#[async_trait]
impl LlmDriver for OpenResponsesProtocolLlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Check model profile to determine if phases should be sent in the wire format.
        // Only GPT-5.4+ models support native execution phases.
        let supports_phases = crate::llm_model_profiles::get_model_profile(
            &crate::llm_models::LlmProviderType::Openai,
            &config.model,
        )
        .is_some_and(|p| p.supports_phases);

        let (instructions, input_items) = Self::build_input(&messages, supports_phases);

        let tools = if config.tools.is_empty() {
            None
        } else if let Some(ref ts_config) = config.tool_search {
            if ts_config.enabled {
                Some(Self::convert_tools_with_search(
                    &config.tools,
                    ts_config.threshold,
                ))
            } else {
                Some(Self::convert_tools(&config.tools))
            }
        } else {
            Some(Self::convert_tools(&config.tools))
        };

        // Build reasoning config if specified.
        // Skip when effort is "none" — sending reasoning params to models that
        // don't support them (or with effort=none) causes OpenAI API errors.
        let reasoning = config
            .reasoning_effort
            .as_ref()
            .filter(|e| !e.eq_ignore_ascii_case("none"))
            .map(|effort| ResponsesReasoning {
                effort: effort.clone(),
                summary: "detailed".to_string(),
            });

        // Build metadata for request tracking
        let metadata = if config.metadata.is_empty() {
            None
        } else {
            Some(config.metadata.clone())
        };

        let request = ResponsesRequest {
            model: config.model.clone(),
            input: input_items,
            instructions,
            previous_response_id: config.previous_response_id.clone(),
            temperature: config.temperature,
            max_output_tokens: config.max_tokens,
            stream: true,
            tools,
            reasoning,
            metadata,
        };

        // Retry loop for rate limit (429) and transient errors
        let mut retry_metadata = RetryMetadata::default();
        let mut last_error: Option<String> = None;

        let response = loop {
            let response = self
                .client
                .post(&self.api_url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Content-Type", "application/json")
                .json(&request)
                .send()
                .await
                .map_err(|e| AgentLoopError::llm(format!("Failed to send request: {}", e)))?;

            let status = response.status();

            if status.is_success() {
                // Success - exit retry loop
                break response;
            }

            // Check if this is a retryable error
            if is_transient_error(status) && retry_metadata.attempts < self.retry_config.max_retries
            {
                // Parse rate limit info from headers before consuming response body
                let rate_limit_info = if is_rate_limit_status(status) {
                    Some(RateLimitInfo::from_openai_headers(response.headers()))
                } else {
                    None
                };

                let error_text = response.text().await.unwrap_or_default();

                // Calculate wait duration
                let wait_duration = rate_limit_info
                    .as_ref()
                    .map(|info| info.recommended_wait(&self.retry_config, retry_metadata.attempts))
                    .unwrap_or_else(|| {
                        self.retry_config.calculate_backoff(retry_metadata.attempts)
                    });

                tracing::warn!(
                    status = %status,
                    attempt = retry_metadata.attempts + 1,
                    max_retries = self.retry_config.max_retries,
                    wait_secs = wait_duration.as_secs_f64(),
                    retry_after = ?rate_limit_info.as_ref().and_then(|i| i.retry_after_secs),
                    "OpenResponsesDriver: rate limit or transient error, retrying"
                );

                // Record retry attempt
                retry_metadata.record_retry(wait_duration, rate_limit_info);
                last_error = Some(error_text);

                // Wait before retry
                tokio::time::sleep(wait_duration).await;
                continue;
            }

            // Non-retryable error or max retries exceeded
            let error_text = response.text().await.unwrap_or_default();

            // Check if this is a model-not-found error
            if is_openai_model_not_found(status, &error_text) {
                return Err(AgentLoopError::model_not_available(config.model.clone()));
            }

            // Check if this is a request-too-large error (context length exceeded)
            if is_openai_request_too_large(status, &error_text) {
                return Err(AgentLoopError::request_too_large(format!(
                    "OpenAI Responses API ({}): {}",
                    status, error_text
                )));
            }

            let error_msg = format!("OpenAI Responses API error ({}): {}", status, error_text);

            // If we exhausted retries, include that in the error message
            if retry_metadata.attempts > 0 {
                return Err(AgentLoopError::llm(format!(
                    "{} (after {} retries, last error: {})",
                    error_msg,
                    retry_metadata.attempts,
                    last_error.unwrap_or_default()
                )));
            }

            return Err(AgentLoopError::llm(error_msg));
        };

        // Log successful retry recovery
        if retry_metadata.had_retries() {
            tracing::info!(
                attempts = retry_metadata.attempts,
                total_wait_secs = retry_metadata.total_retry_wait.as_secs_f64(),
                "OpenResponsesDriver: request succeeded after retries"
            );
        }

        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream.eventsource();

        let model = config.model.clone();
        let input_tokens = Arc::new(Mutex::new(0u32));
        let output_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCallAccumulator>::new()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));
        // Share retry metadata with stream closure (only set if retries occurred)
        let shared_retry_metadata = if retry_metadata.had_retries() {
            Some(Arc::new(retry_metadata))
        } else {
            None
        };

        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let input_tokens = Arc::clone(&input_tokens);
            let output_tokens = Arc::clone(&output_tokens);
            let cache_read_tokens = Arc::clone(&cache_read_tokens);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            let finish_reason = Arc::clone(&finish_reason);
            let retry_metadata_for_done = shared_retry_metadata.clone();

            async move {
                match result {
                    Ok(event) => {
                        let event_data = &event.data;

                        // Try to parse as typed StreamingEvent first for type safety
                        if let Ok(streaming_event) =
                            serde_json::from_str::<StreamingEvent>(event_data)
                        {
                            return Ok(handle_streaming_event(
                                streaming_event,
                                &input_tokens,
                                &output_tokens,
                                &cache_read_tokens,
                                &accumulated_tool_calls,
                                &finish_reason,
                                model,
                                retry_metadata_for_done,
                            ));
                        }

                        // Fallback: parse as generic JSON for backwards compatibility
                        let parsed: std::result::Result<Value, _> =
                            serde_json::from_str(event_data);

                        match parsed {
                            Ok(json) => {
                                let event_type = json.get("type").and_then(|t| t.as_str());

                                match event_type {
                                    Some("response.output_text.delta") => {
                                        // Text delta
                                        if let Some(delta) =
                                            json.get("delta").and_then(|d| d.as_str())
                                        {
                                            Ok(LlmStreamEvent::TextDelta(delta.to_string()))
                                        } else {
                                            Ok(LlmStreamEvent::TextDelta(String::new()))
                                        }
                                    }

                                    Some("response.function_call_arguments.delta") => {
                                        // Function call arguments delta
                                        if let (Some(item_id), Some(delta)) = (
                                            json.get("item_id").and_then(|c| c.as_str()),
                                            json.get("delta").and_then(|d| d.as_str()),
                                        ) {
                                            let mut acc = accumulated_tool_calls.lock().unwrap();
                                            // Find or create accumulator for this item_id
                                            if let Some(tc) =
                                                acc.iter_mut().find(|t| t.id == item_id)
                                            {
                                                tc.arguments.push_str(delta);
                                            } else {
                                                acc.push(ToolCallAccumulator {
                                                    id: item_id.to_string(),
                                                    call_id: String::new(),
                                                    name: String::new(),
                                                    arguments: delta.to_string(),
                                                });
                                            }
                                        }
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }

                                    Some("response.output_item.added") => {
                                        // New output item added - may be function call
                                        if let Some(item) = json.get("item")
                                            && item.get("type").and_then(|t| t.as_str())
                                                == Some("function_call")
                                        {
                                            let id = item
                                                .get("id")
                                                .and_then(|c| c.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let call_id = item
                                                .get("call_id")
                                                .and_then(|c| c.as_str())
                                                .unwrap_or("")
                                                .to_string();
                                            let name = item
                                                .get("name")
                                                .and_then(|n| n.as_str())
                                                .unwrap_or("")
                                                .to_string();

                                            let mut acc = accumulated_tool_calls.lock().unwrap();
                                            if let Some(tc) = acc.iter_mut().find(|t| t.id == id) {
                                                tc.name = name;
                                                tc.call_id = call_id;
                                            } else {
                                                acc.push(ToolCallAccumulator {
                                                    id,
                                                    call_id,
                                                    name,
                                                    arguments: String::new(),
                                                });
                                            }
                                        }
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }

                                    Some("response.output_item.done") => {
                                        // Output item completed - check if it's a function call
                                        if let Some(item) = json.get("item")
                                            && item.get("type").and_then(|t| t.as_str())
                                                == Some("function_call")
                                        {
                                            // Function call completed, emit ToolCalls event
                                            let acc = accumulated_tool_calls.lock().unwrap();
                                            if !acc.is_empty() {
                                                let tool_calls: Vec<ToolCall> = acc
                                                    .iter()
                                                    .filter(|tc| !tc.name.is_empty())
                                                    .map(|tc| {
                                                        let arguments: Value =
                                                            serde_json::from_str(&tc.arguments)
                                                                .unwrap_or(json!({}));
                                                        ToolCall {
                                                            id: tc.call_id.clone(),
                                                            name: tc.name.clone(),
                                                            arguments,
                                                        }
                                                    })
                                                    .collect();

                                                if !tool_calls.is_empty() {
                                                    *finish_reason.lock().unwrap() =
                                                        Some("tool_calls".to_string());
                                                    return Ok(LlmStreamEvent::ToolCalls(
                                                        tool_calls,
                                                    ));
                                                }
                                            }
                                        }
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }

                                    Some("response.completed") | Some("response.done") => {
                                        // Response completed - extract usage
                                        let response_obj = json.get("response").unwrap_or(&json);

                                        if let Some(usage) = response_obj.get("usage") {
                                            if let Some(input) =
                                                usage.get("input_tokens").and_then(|t| t.as_u64())
                                            {
                                                *input_tokens.lock().unwrap() = input as u32;
                                            }
                                            if let Some(output) =
                                                usage.get("output_tokens").and_then(|t| t.as_u64())
                                            {
                                                *output_tokens.lock().unwrap() = output as u32;
                                            }
                                            // Check for cached tokens
                                            if let Some(details) = usage.get("input_tokens_details")
                                                && let Some(cached) = details
                                                    .get("cached_tokens")
                                                    .and_then(|t| t.as_u64())
                                            {
                                                *cache_read_tokens.lock().unwrap() =
                                                    Some(cached as u32);
                                            }
                                        }

                                        // Determine finish reason from status
                                        let status = response_obj
                                            .get("status")
                                            .and_then(|s| s.as_str())
                                            .unwrap_or("completed");

                                        let reason = match status {
                                            "completed" => {
                                                // Check if there were tool calls
                                                let existing_reason =
                                                    finish_reason.lock().unwrap().clone();
                                                existing_reason
                                                    .unwrap_or_else(|| "stop".to_string())
                                            }
                                            "failed" => "error".to_string(),
                                            "cancelled" => "stop".to_string(),
                                            _ => "stop".to_string(),
                                        };

                                        // Extract phase from the last assistant message in output items
                                        let phase = response_obj
                                            .get("output")
                                            .and_then(|o| o.as_array())
                                            .and_then(|items| {
                                                items.iter().rev().find_map(|item| {
                                                    if item.get("type")?.as_str()? == "message"
                                                        && item.get("role")?.as_str()?
                                                            == "assistant"
                                                    {
                                                        item.get("phase")?
                                                            .as_str()
                                                            .map(String::from)
                                                    } else {
                                                        None
                                                    }
                                                })
                                            });

                                        let input = *input_tokens.lock().unwrap();
                                        let output = *output_tokens.lock().unwrap();
                                        let cached = *cache_read_tokens.lock().unwrap();

                                        Ok(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                                            total_tokens: Some(input + output),
                                            prompt_tokens: Some(input),
                                            completion_tokens: Some(output),
                                            cache_read_tokens: cached,
                                            cache_creation_tokens: None,
                                            model: Some(model),
                                            finish_reason: Some(reason),
                                            retry_metadata: retry_metadata_for_done
                                                .map(|arc| (*arc).clone()),
                                            response_id: None,
                                            phase,
                                        })))
                                    }

                                    Some("error") => {
                                        // Error event
                                        let error_msg = json
                                            .get("error")
                                            .and_then(|e| e.get("message"))
                                            .and_then(|m| m.as_str())
                                            .unwrap_or("Unknown error");
                                        Ok(LlmStreamEvent::Error(error_msg.to_string()))
                                    }

                                    _ => {
                                        // Other event types - ignore
                                        Ok(LlmStreamEvent::TextDelta(String::new()))
                                    }
                                }
                            }
                            Err(e) => Ok(LlmStreamEvent::Error(format!(
                                "Failed to parse event: {}",
                                e
                            ))),
                        }
                    }
                    Err(e) => Ok(LlmStreamEvent::Error(format!("Stream error: {}", e))),
                }
            }
        }));

        Ok(converted_stream)
    }

    fn supports_compact(&self) -> bool {
        // Delegate to the inherent method
        OpenResponsesProtocolLlmDriver::supports_compact(self)
    }

    async fn compact(
        &self,
        request: crate::openresponses_protocol::CompactRequest,
    ) -> Result<Option<crate::openresponses_protocol::CompactResponse>> {
        // Delegate to the inherent method and wrap in Some
        Ok(Some(
            OpenResponsesProtocolLlmDriver::compact(self, request).await?,
        ))
    }
}

impl std::fmt::Debug for OpenResponsesProtocolLlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenResponsesProtocolLlmDriver")
            .field("api_url", &self.api_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Helper Types
// ============================================================================

/// Accumulator for tool call arguments during streaming
#[derive(Clone, Default)]
struct ToolCallAccumulator {
    /// Item ID in the stream
    id: String,
    /// Unique call ID for the function call
    call_id: String,
    /// Function name
    name: String,
    /// Accumulated JSON arguments
    arguments: String,
}

/// Handle typed streaming events from the OpenResponses API
#[allow(clippy::too_many_arguments)]
fn handle_streaming_event(
    event: StreamingEvent,
    input_tokens: &Mutex<u32>,
    output_tokens: &Mutex<u32>,
    cache_read_tokens: &Mutex<Option<u32>>,
    accumulated_tool_calls: &Mutex<Vec<ToolCallAccumulator>>,
    finish_reason: &Mutex<Option<String>>,
    model: String,
    retry_metadata: Option<Arc<RetryMetadata>>,
) -> LlmStreamEvent {
    match event {
        StreamingEvent::OutputTextDelta { delta, .. } => LlmStreamEvent::TextDelta(delta),

        StreamingEvent::ReasoningDelta { delta, .. } => LlmStreamEvent::ThinkingDelta(delta),

        StreamingEvent::ReasoningSummaryDelta { delta, .. } => LlmStreamEvent::ThinkingDelta(delta),

        StreamingEvent::FunctionCallArgumentsDelta { item_id, delta, .. } => {
            let mut acc = accumulated_tool_calls.lock().unwrap();
            if let Some(tc) = acc.iter_mut().find(|t| t.id == item_id) {
                tc.arguments.push_str(&delta);
            } else {
                acc.push(ToolCallAccumulator {
                    id: item_id,
                    call_id: String::new(),
                    name: String::new(),
                    arguments: delta,
                });
            }
            LlmStreamEvent::TextDelta(String::new())
        }

        StreamingEvent::OutputItemAdded { item, .. } => {
            if let Some(types::OutputItem::FunctionCall {
                id, call_id, name, ..
            }) = item
            {
                let mut acc = accumulated_tool_calls.lock().unwrap();
                if let Some(tc) = acc.iter_mut().find(|t| t.id == id) {
                    tc.name = name;
                    tc.call_id = call_id;
                } else {
                    acc.push(ToolCallAccumulator {
                        id,
                        call_id,
                        name,
                        arguments: String::new(),
                    });
                }
            }
            LlmStreamEvent::TextDelta(String::new())
        }

        StreamingEvent::OutputItemDone { item, .. } => {
            match item {
                Some(types::OutputItem::FunctionCall { .. }) => {
                    let acc = accumulated_tool_calls.lock().unwrap();
                    if !acc.is_empty() {
                        let tool_calls: Vec<ToolCall> = acc
                            .iter()
                            .filter(|tc| !tc.name.is_empty())
                            .map(|tc| {
                                let arguments: Value =
                                    serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
                                ToolCall {
                                    id: tc.call_id.clone(),
                                    name: tc.name.clone(),
                                    arguments,
                                }
                            })
                            .collect();

                        if !tool_calls.is_empty() {
                            *finish_reason.lock().unwrap() = Some("tool_calls".to_string());
                            return LlmStreamEvent::ToolCalls(tool_calls);
                        }
                    }
                    LlmStreamEvent::TextDelta(String::new())
                }
                Some(types::OutputItem::Reasoning {
                    encrypted_content: Some(encrypted),
                    ..
                }) => {
                    // Emit encrypted reasoning content as ThinkingSignature
                    // This is the OpenAI equivalent of Anthropic's thinking signature
                    // and must be included in subsequent API calls to preserve reasoning context
                    tracing::debug!(
                        encrypted_len = encrypted.len(),
                        "OpenResponses: received encrypted reasoning content"
                    );
                    LlmStreamEvent::ThinkingSignature(encrypted)
                }
                _ => LlmStreamEvent::TextDelta(String::new()),
            }
        }

        StreamingEvent::ResponseCompleted { response, .. } => {
            // Extract usage
            if let Some(usage) = &response.usage {
                *input_tokens.lock().unwrap() = usage.input_tokens;
                *output_tokens.lock().unwrap() = usage.output_tokens;
                if let Some(details) = &usage.input_tokens_details {
                    *cache_read_tokens.lock().unwrap() = Some(details.cached_tokens);
                }
            }

            let reason = match response.status {
                types::ResponseStatus::Completed => {
                    let existing = finish_reason.lock().unwrap().clone();
                    existing.unwrap_or_else(|| "stop".to_string())
                }
                types::ResponseStatus::Failed => "error".to_string(),
                types::ResponseStatus::Cancelled => "stop".to_string(),
                _ => "stop".to_string(),
            };

            // Extract phase from the last assistant message in output items.
            // The API assigns the phase; we preserve it as-is for subsequent requests.
            let phase = response.output.iter().rev().find_map(|item| {
                if let types::OutputItem::Message { phase, .. } = item {
                    phase.clone()
                } else {
                    None
                }
            });

            let input = *input_tokens.lock().unwrap();
            let output = *output_tokens.lock().unwrap();
            let cached = *cache_read_tokens.lock().unwrap();

            LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                total_tokens: Some(input + output),
                prompt_tokens: Some(input),
                completion_tokens: Some(output),
                cache_read_tokens: cached,
                cache_creation_tokens: None,
                model: Some(model),
                finish_reason: Some(reason),
                retry_metadata: retry_metadata.map(|arc| (*arc).clone()),
                response_id: Some(response.id),
                phase,
            }))
        }

        StreamingEvent::Error { error, .. } => {
            let msg = if let Some(code) = &error.code {
                format!("{}: {}", code, error.message)
            } else {
                error.message
            };
            LlmStreamEvent::Error(msg)
        }

        StreamingEvent::RefusalDelta { delta, .. } => {
            // Treat refusal as an error message
            LlmStreamEvent::Error(format!("Model refused: {}", delta))
        }

        // All other events: emit empty delta to maintain stream continuity
        _ => LlmStreamEvent::TextDelta(String::new()),
    }
}

// ============================================================================
// Compact Endpoint Types (Public API)
// ============================================================================

/// Request for the /v1/responses/compact endpoint
///
/// This endpoint compacts a conversation by replacing prior assistant messages,
/// tool calls, and tool results with an encrypted compaction item that preserves
/// latent context but is opaque. User messages are kept verbatim.
#[derive(Debug, Clone, Serialize)]
pub struct CompactRequest {
    /// Model to use for compaction (required)
    pub model: String,
    /// Input items to compact (the current conversation window)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<CompactInputItem>,
    /// Previous response ID (optional, alternative to input)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// System instructions (optional, applies only to the compaction request)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Input item for compact request
///
/// These are the same types as ResponsesInputItem but exposed publicly
/// for callers to construct compact requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompactInputItem {
    /// A message (user, assistant, or developer)
    #[serde(rename = "message")]
    Message {
        role: String,
        content: CompactContent,
    },
    /// A function call from the assistant
    #[serde(rename = "function_call")]
    FunctionCall {
        call_id: String,
        name: String,
        arguments: String,
    },
    /// Output from a function call
    #[serde(rename = "function_call_output")]
    FunctionCallOutput { call_id: String, output: String },
    /// A compaction item (from a previous compact call)
    #[serde(rename = "compaction")]
    Compaction { encrypted_content: String },
}

/// Content for compact input items
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompactContent {
    /// Simple text content
    Text(String),
    /// Array of content parts
    Parts(Vec<CompactContentPart>),
}

/// Content part for compact input
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompactContentPart {
    /// Text content
    #[serde(rename = "input_text")]
    InputText { text: String },
    /// Image content
    #[serde(rename = "input_image")]
    InputImage { image_url: String },
}

/// Response from the /v1/responses/compact endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct CompactResponse {
    /// The compacted output items
    pub output: Vec<CompactOutputItem>,
    /// Token usage information
    pub usage: Option<CompactUsage>,
}

/// Output item from compact response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompactOutputItem {
    /// A user message (kept verbatim)
    #[serde(rename = "message")]
    Message {
        role: String,
        content: CompactContent,
    },
    /// An encrypted compaction item
    #[serde(rename = "compaction")]
    Compaction {
        /// Encrypted content that preserves latent context
        encrypted_content: String,
    },
}

/// Token usage from compact response
#[derive(Debug, Clone, Deserialize)]
pub struct CompactUsage {
    /// Input tokens processed
    pub input_tokens: Option<u32>,
    /// Output tokens generated
    pub output_tokens: Option<u32>,
    /// Total tokens used
    pub total_tokens: Option<u32>,
}

// ============================================================================
// Compaction Conversion Utilities
// ============================================================================

impl CompactInputItem {
    /// Convert an LlmMessage to CompactInputItem(s)
    ///
    /// An assistant message with tool_calls is expanded into multiple items:
    /// one Message for the text content and one FunctionCall for each tool call.
    pub fn from_llm_message(msg: &LlmMessage) -> Vec<Self> {
        let mut items = Vec::new();

        let role = match msg.role {
            LlmMessageRole::System => "developer",
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        };

        // Handle tool result messages differently
        if msg.role == LlmMessageRole::Tool
            && let Some(tool_call_id) = &msg.tool_call_id
        {
            let output = match &msg.content {
                LlmMessageContent::Text(text) => text.clone(),
                LlmMessageContent::Parts(parts) => parts
                    .iter()
                    .filter_map(|p| match p {
                        LlmContentPart::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            };
            items.push(CompactInputItem::FunctionCallOutput {
                call_id: tool_call_id.clone(),
                output,
            });
            return items;
        }

        // Add message content (if non-empty)
        let content = Self::content_from_llm_message(msg);
        let has_content = match &content {
            CompactContent::Text(t) => !t.is_empty(),
            CompactContent::Parts(p) => !p.is_empty(),
        };

        if has_content || msg.tool_calls.is_none() {
            items.push(CompactInputItem::Message {
                role: role.to_string(),
                content,
            });
        }

        // Add function calls for assistant messages
        if msg.role == LlmMessageRole::Assistant
            && let Some(tool_calls) = &msg.tool_calls
        {
            for tc in tool_calls {
                items.push(CompactInputItem::FunctionCall {
                    call_id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.to_string(),
                });
            }
        }

        items
    }

    /// Convert LlmMessageContent to CompactContent
    fn content_from_llm_message(msg: &LlmMessage) -> CompactContent {
        match &msg.content {
            LlmMessageContent::Text(text) => CompactContent::Text(text.clone()),
            LlmMessageContent::Parts(parts) => {
                let compact_parts: Vec<CompactContentPart> = parts
                    .iter()
                    .filter_map(|part| match part {
                        LlmContentPart::Text { text } => {
                            Some(CompactContentPart::InputText { text: text.clone() })
                        }
                        LlmContentPart::Image { url } => {
                            // URL is already in data URL format (data:image/png;base64,...)
                            Some(CompactContentPart::InputImage {
                                image_url: url.clone(),
                            })
                        }
                        LlmContentPart::Audio { .. } => None, // Audio not supported in compact
                    })
                    .collect();
                if compact_parts.len() == 1
                    && let CompactContentPart::InputText { text } = &compact_parts[0]
                {
                    return CompactContent::Text(text.clone());
                }
                CompactContent::Parts(compact_parts)
            }
        }
    }
}

impl CompactOutputItem {
    /// Convert a CompactOutputItem to LlmMessage
    ///
    /// Compaction items are converted to a special system message containing
    /// the encrypted context that will be included in subsequent requests.
    pub fn to_llm_message(&self) -> Option<LlmMessage> {
        match self {
            CompactOutputItem::Message { role, content } => {
                let llm_role = match role.as_str() {
                    "user" => LlmMessageRole::User,
                    "assistant" => LlmMessageRole::Assistant,
                    "developer" | "system" => LlmMessageRole::System,
                    "tool" => LlmMessageRole::Tool,
                    _ => LlmMessageRole::User, // Default to user
                };

                let llm_content = match content {
                    CompactContent::Text(text) => LlmMessageContent::Text(text.clone()),
                    CompactContent::Parts(parts) => {
                        let llm_parts: Vec<LlmContentPart> = parts
                            .iter()
                            .map(|p| match p {
                                CompactContentPart::InputText { text } => {
                                    LlmContentPart::Text { text: text.clone() }
                                }
                                CompactContentPart::InputImage { image_url } => {
                                    // Pass the URL directly - it's already in data URL format
                                    LlmContentPart::Image {
                                        url: image_url.clone(),
                                    }
                                }
                            })
                            .collect();
                        LlmMessageContent::Parts(llm_parts)
                    }
                };

                Some(LlmMessage {
                    role: llm_role,
                    content: llm_content,
                    tool_calls: None,
                    tool_call_id: None,
                    phase: None,
                    thinking: None,
                    thinking_signature: None,
                })
            }
            CompactOutputItem::Compaction { .. } => {
                // Compaction items are handled separately - they're passed as-is
                // to the next request, not converted to messages
                None
            }
        }
    }
}

/// Convert a slice of LlmMessages to CompactInputItems
pub fn messages_to_compact_input(messages: &[LlmMessage]) -> Vec<CompactInputItem> {
    messages
        .iter()
        .flat_map(CompactInputItem::from_llm_message)
        .collect()
}

/// Convert CompactResponse output to LlmMessages plus any compaction items
///
/// Returns a tuple of (regular messages, compaction items).
/// The compaction items should be preserved and included in subsequent compact requests.
pub fn compact_output_to_messages(
    output: &[CompactOutputItem],
) -> (Vec<LlmMessage>, Vec<CompactInputItem>) {
    let mut messages = Vec::new();
    let mut compaction_items = Vec::new();

    for item in output {
        match item {
            CompactOutputItem::Message { role, content } => {
                if let Some(msg) = item.to_llm_message() {
                    messages.push(msg);
                } else {
                    // Re-add as compact input for next request
                    compaction_items.push(CompactInputItem::Message {
                        role: role.clone(),
                        content: content.clone(),
                    });
                }
            }
            CompactOutputItem::Compaction { encrypted_content } => {
                compaction_items.push(CompactInputItem::Compaction {
                    encrypted_content: encrypted_content.clone(),
                });
            }
        }
    }

    (messages, compaction_items)
}

// ============================================================================
// OpenAI Responses API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct ResponsesRequest {
    model: String,
    input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<ResponsesReasoning>,
    /// Metadata for tracking API usage (up to 16 key-value pairs).
    /// Useful for correlating requests with session_id, agent_id, org_id, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct ResponsesReasoning {
    effort: String,
    /// Request reasoning summary to get thinking tokens streamed back.
    /// Without this, reasoning happens internally but tokens are not exposed.
    summary: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ResponsesInputItem {
    Message {
        r#type: String,
        role: String,
        content: ResponsesContent,
        /// Execution phase for assistant messages (e.g., "in_progress", "completed").
        /// Helps GPT-5.x distinguish intermediate working commentary from final answers.
        /// Only set on assistant messages; must be preserved when replaying history.
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    FunctionCall {
        r#type: String,
        call_id: String,
        name: String,
        arguments: String,
    },
    FunctionCallOutput {
        r#type: String,
        call_id: String,
        output: String,
    },
    /// Reasoning item for o-series and GPT-5 models
    /// Contains encrypted reasoning content that must be included in subsequent API calls
    /// to preserve reasoning context across turns (similar to Anthropic's thinking signature)
    Reasoning {
        r#type: String,
        /// Unique ID for this reasoning item
        id: String,
        /// Encrypted reasoning content (required for multi-turn conversations)
        encrypted_content: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum ResponsesContent {
    Text(String),
    Parts(Vec<ResponsesContentPart>),
}

// The "Input" prefix matches OpenAI's Responses API naming convention
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
#[allow(clippy::enum_variant_names)]
enum ResponsesContentPart {
    InputText {
        r#type: String,
        text: String,
    },
    InputImage {
        r#type: String,
        image_url: String,
    },
    InputAudio {
        r#type: String,
        input_audio: ResponsesInputAudio,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponsesInputAudio {
    data: String,
    format: String,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ResponsesTool {
    /// Standard function tool (or deferred function with defer_loading)
    Function {
        r#type: String,
        name: String,
        description: String,
        parameters: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        defer_loading: Option<bool>,
    },
    /// Namespace grouping for tool_search (groups related deferred tools)
    Namespace {
        r#type: String,
        name: String,
        description: String,
        tools: Vec<ResponsesTool>,
    },
    /// Activates tool_search on the request
    ToolSearch { r#type: String },
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_with_api_key() {
        let driver = OpenResponsesProtocolLlmDriver::new("test-key");
        assert!(format!("{:?}", driver).contains("OpenResponsesProtocolLlmDriver"));
    }

    #[test]
    fn test_driver_with_base_url() {
        let driver = OpenResponsesProtocolLlmDriver::with_base_url(
            "test-key",
            "https://custom.api.com/v1/responses",
        );
        assert!(format!("{:?}", driver).contains("OpenResponsesProtocolLlmDriver"));
        assert_eq!(driver.api_url(), "https://custom.api.com/v1/responses");
    }

    #[test]
    fn test_request_serialization() {
        let request = ResponsesRequest {
            model: "gpt-4o".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Hello".to_string()),
                phase: None,
            }],
            instructions: Some("You are helpful".to_string()),
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: None,
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["stream"], true);
        assert_eq!(json["instructions"], "You are helpful");
        assert!(json["input"].is_array());
    }

    #[test]
    fn test_request_with_reasoning() {
        let request = ResponsesRequest {
            model: "o3".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Think about this".to_string()),
                phase: None,
            }],
            instructions: None,
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: Some(ResponsesReasoning {
                effort: "high".to_string(),
                summary: "detailed".to_string(),
            }),
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["reasoning"]["effort"], "high");
        assert_eq!(json["reasoning"]["summary"], "detailed");
    }

    #[test]
    fn test_request_with_metadata() {
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("session_id".to_string(), "session_abc123".to_string());
        metadata.insert("agent_id".to_string(), "agent_xyz789".to_string());

        let request = ResponsesRequest {
            model: "gpt-4o".to_string(),
            input: vec![ResponsesInputItem::Message {
                r#type: "message".to_string(),
                role: "user".to_string(),
                content: ResponsesContent::Text("Hello".to_string()),
                phase: None,
            }],
            instructions: None,
            previous_response_id: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: None,
            metadata: Some(metadata),
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["metadata"]["session_id"], "session_abc123");
        assert_eq!(json["metadata"]["agent_id"], "agent_xyz789");
    }

    #[test]
    fn test_function_call_output_serialization() {
        let item = ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".to_string(),
            call_id: "call_123".to_string(),
            output: r#"{"result": 42}"#.to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "function_call_output");
        assert_eq!(json["call_id"], "call_123");
        assert_eq!(json["output"], r#"{"result": 42}"#);
    }

    #[test]
    fn test_multipart_content_serialization() {
        let content = ResponsesContent::Parts(vec![
            ResponsesContentPart::InputText {
                r#type: "input_text".to_string(),
                text: "Look at this image".to_string(),
            },
            ResponsesContentPart::InputImage {
                r#type: "input_image".to_string(),
                image_url: "data:image/png;base64,abc123".to_string(),
            },
        ]);

        let json = serde_json::to_value(&content).unwrap();
        assert!(json.is_array());
        assert_eq!(json[0]["type"], "input_text");
        assert_eq!(json[1]["type"], "input_image");
    }

    #[test]
    fn test_tool_serialization() {
        let tool = ResponsesTool::Function {
            r#type: "function".to_string(),
            name: "get_weather".to_string(),
            description: "Get weather for a location".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "location": {"type": "string"}
                },
                "required": ["location"]
            }),
            defer_loading: None,
        };

        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["type"], "function");
        assert_eq!(json["name"], "get_weather");
        assert!(json["parameters"]["properties"]["location"].is_object());
    }

    #[test]
    fn test_build_input_extracts_system_as_instructions() {
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are a helpful assistant"),
            LlmMessage::text(LlmMessageRole::User, "Hello"),
        ];

        let (instructions, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);

        assert_eq!(
            instructions,
            Some("You are a helpful assistant".to_string())
        );
        assert_eq!(input.len(), 1); // Only user message, system converted to instructions
    }

    #[test]
    fn test_convert_role() {
        assert_eq!(
            OpenResponsesProtocolLlmDriver::convert_role(&LlmMessageRole::System),
            "developer"
        );
        assert_eq!(
            OpenResponsesProtocolLlmDriver::convert_role(&LlmMessageRole::User),
            "user"
        );
        assert_eq!(
            OpenResponsesProtocolLlmDriver::convert_role(&LlmMessageRole::Assistant),
            "assistant"
        );
        assert_eq!(
            OpenResponsesProtocolLlmDriver::convert_role(&LlmMessageRole::Tool),
            "tool"
        );
    }

    #[test]
    fn test_function_call_serialization() {
        let item = ResponsesInputItem::FunctionCall {
            r#type: "function_call".to_string(),
            call_id: "call_abc123".to_string(),
            name: "get_current_time".to_string(),
            arguments: r#"{"timezone":"UTC"}"#.to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_abc123");
        assert_eq!(json["name"], "get_current_time");
        assert_eq!(json["arguments"], r#"{"timezone":"UTC"}"#);
    }

    #[test]
    fn test_build_input_with_tool_calls() {
        use crate::tool_types::ToolCall;

        // Simulate a conversation with tool calls:
        // 1. User asks a question
        // 2. Assistant calls a tool
        // 3. Tool returns result
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are helpful"),
            LlmMessage::text(LlmMessageRole::User, "What time is it?"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text(String::new()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_xyz789".to_string(),
                    name: "get_current_time".to_string(),
                    arguments: json!({"timezone": "UTC"}),
                }]),
                tool_call_id: None,
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("2025-01-19T10:30:00Z".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_xyz789".to_string()),
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
        ];

        let (instructions, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);

        // System message becomes instructions
        assert_eq!(instructions, Some("You are helpful".to_string()));

        // Should have: user message, function_call, function_call_output
        assert_eq!(input.len(), 3);

        // Verify the function_call is present (second item, since assistant had empty content)
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_xyz789");
        assert_eq!(json["name"], "get_current_time");

        // Verify the function_call_output is present
        let json = serde_json::to_value(&input[2]).unwrap();
        assert_eq!(json["type"], "function_call_output");
        assert_eq!(json["call_id"], "call_xyz789");
        assert_eq!(json["output"], "2025-01-19T10:30:00Z");
    }

    #[test]
    fn test_build_input_with_tool_calls_and_text() {
        use crate::tool_types::ToolCall;

        // Assistant message with both text content and tool calls
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "What time is it?"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Let me check the time for you.".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_abc".to_string(),
                    name: "get_time".to_string(),
                    arguments: json!({}),
                }]),
                tool_call_id: None,
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
        ];

        let (_, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);

        // Should have: user message, assistant message, function_call
        assert_eq!(input.len(), 3);

        // First is user message
        let json = serde_json::to_value(&input[0]).unwrap();
        assert_eq!(json["role"], "user");

        // Second is assistant message with text
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["role"], "assistant");

        // Third is function_call
        let json = serde_json::to_value(&input[2]).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_abc");
    }

    // ========================================================================
    // Compact endpoint tests
    // ========================================================================

    #[test]
    fn test_compact_request_serialization() {
        let request = CompactRequest {
            model: "gpt-4o".to_string(),
            input: vec![
                CompactInputItem::Message {
                    role: "user".to_string(),
                    content: CompactContent::Text("Hello!".to_string()),
                },
                CompactInputItem::Message {
                    role: "assistant".to_string(),
                    content: CompactContent::Text("Hi there!".to_string()),
                },
            ],
            previous_response_id: None,
            instructions: Some("Be helpful".to_string()),
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "gpt-4o");
        assert_eq!(json["instructions"], "Be helpful");
        assert!(json["input"].is_array());
        assert_eq!(json["input"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_compact_input_item_message_serialization() {
        let item = CompactInputItem::Message {
            role: "user".to_string(),
            content: CompactContent::Text("Test message".to_string()),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "message");
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Test message");
    }

    #[test]
    fn test_compact_input_item_function_call_serialization() {
        let item = CompactInputItem::FunctionCall {
            call_id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: r#"{"city":"NYC"}"#.to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_123");
        assert_eq!(json["name"], "get_weather");
        assert_eq!(json["arguments"], r#"{"city":"NYC"}"#);
    }

    #[test]
    fn test_compact_input_item_compaction_serialization() {
        let item = CompactInputItem::Compaction {
            encrypted_content: "encrypted_data_here".to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "compaction");
        assert_eq!(json["encrypted_content"], "encrypted_data_here");
    }

    #[test]
    fn test_compact_output_item_deserialization() {
        let json = r#"{
            "type": "message",
            "role": "user",
            "content": "Hello"
        }"#;

        let item: CompactOutputItem = serde_json::from_str(json).unwrap();
        match item {
            CompactOutputItem::Message { role, content } => {
                assert_eq!(role, "user");
                match content {
                    CompactContent::Text(text) => assert_eq!(text, "Hello"),
                    _ => panic!("Expected text content"),
                }
            }
            _ => panic!("Expected Message item"),
        }
    }

    #[test]
    fn test_compact_output_compaction_deserialization() {
        let json = r#"{
            "type": "compaction",
            "encrypted_content": "abc123encrypted"
        }"#;

        let item: CompactOutputItem = serde_json::from_str(json).unwrap();
        match item {
            CompactOutputItem::Compaction { encrypted_content } => {
                assert_eq!(encrypted_content, "abc123encrypted");
            }
            _ => panic!("Expected Compaction item"),
        }
    }

    #[test]
    fn test_compact_response_deserialization() {
        let json = r#"{
            "output": [
                {"type": "message", "role": "user", "content": "Hello"},
                {"type": "compaction", "encrypted_content": "xyz789"}
            ],
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "total_tokens": 150
            }
        }"#;

        let response: CompactResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.output.len(), 2);
        assert!(response.usage.is_some());
        let usage = response.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.total_tokens, Some(150));
    }

    #[test]
    fn test_compact_content_parts_serialization() {
        let content = CompactContent::Parts(vec![
            CompactContentPart::InputText {
                text: "Check this image".to_string(),
            },
            CompactContentPart::InputImage {
                image_url: "data:image/png;base64,abc".to_string(),
            },
        ]);

        let json = serde_json::to_value(&content).unwrap();
        assert!(json.is_array());
        assert_eq!(json[0]["type"], "input_text");
        assert_eq!(json[0]["text"], "Check this image");
        assert_eq!(json[1]["type"], "input_image");
    }

    #[test]
    fn test_supports_compact_default_url() {
        let driver = OpenResponsesProtocolLlmDriver::new("test-key");
        // Default URL is OpenAI, should support compact
        assert!(driver.supports_compact());
    }

    #[test]
    fn test_supports_compact_custom_url() {
        let driver = OpenResponsesProtocolLlmDriver::with_base_url(
            "test-key",
            "https://custom.api.com/v1/responses",
        );
        // Custom URL, compact support unknown
        assert!(!driver.supports_compact());
    }

    // ========================================================================
    // OpenAI Thinking/Reasoning Support Tests
    // ========================================================================

    #[test]
    fn test_reasoning_input_item_serialization() {
        let item = ResponsesInputItem::Reasoning {
            r#type: "reasoning".to_string(),
            id: "rs_00000001".to_string(),
            encrypted_content: "encrypted_reasoning_context_here".to_string(),
        };

        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["type"], "reasoning");
        assert_eq!(json["id"], "rs_00000001");
        assert_eq!(
            json["encrypted_content"],
            "encrypted_reasoning_context_here"
        );
    }

    #[test]
    fn test_build_input_with_thinking_signature() {
        // Assistant message with thinking and thinking_signature (encrypted_content)
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "Think about this deeply"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("I have thought about this.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: Some("This is my chain of thought reasoning...".to_string()),
                thinking_signature: Some("encrypted_reasoning_token_123".to_string()),
            },
            LlmMessage::text(LlmMessageRole::User, "What else?"),
        ];

        let (_, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);

        // Should have: user message, reasoning item, assistant message, user message
        assert_eq!(input.len(), 4);

        // First is user message
        let json = serde_json::to_value(&input[0]).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Think about this deeply");

        // Second is reasoning item (before assistant message)
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["type"], "reasoning");
        assert_eq!(json["encrypted_content"], "encrypted_reasoning_token_123");

        // Third is assistant message
        let json = serde_json::to_value(&input[2]).unwrap();
        assert_eq!(json["role"], "assistant");
        assert_eq!(json["content"], "I have thought about this.");

        // Fourth is second user message
        let json = serde_json::to_value(&input[3]).unwrap();
        assert_eq!(json["role"], "user");
    }

    #[test]
    fn test_build_input_with_thinking_signature_and_tool_calls() {
        use crate::tool_types::ToolCall;

        // Assistant message with thinking, tool calls, and thinking_signature
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "What time is it? Think carefully."),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Let me check.".to_string()),
                tool_calls: Some(vec![ToolCall {
                    id: "call_123".to_string(),
                    name: "get_time".to_string(),
                    arguments: json!({}),
                }]),
                tool_call_id: None,
                phase: None,
                thinking: Some("I need to call the get_time tool...".to_string()),
                thinking_signature: Some("encrypted_token_xyz".to_string()),
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("10:30 AM".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_123".to_string()),
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
        ];

        let (_, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);

        // Should have: user, reasoning, assistant, function_call, function_call_output
        assert_eq!(input.len(), 5);

        // Reasoning item comes before assistant message
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["type"], "reasoning");
        assert_eq!(json["encrypted_content"], "encrypted_token_xyz");

        // Assistant message
        let json = serde_json::to_value(&input[2]).unwrap();
        assert_eq!(json["role"], "assistant");

        // Function call
        let json = serde_json::to_value(&input[3]).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_123");

        // Function call output
        let json = serde_json::to_value(&input[4]).unwrap();
        assert_eq!(json["type"], "function_call_output");
    }

    #[test]
    fn test_build_input_without_thinking_signature() {
        // Assistant message with thinking but NO thinking_signature should not emit reasoning item
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "Hello"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Hi there!".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: Some("Some thinking...".to_string()),
                thinking_signature: None, // No signature!
            },
        ];

        let (_, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);

        // Should have: user message, assistant message (no reasoning item)
        assert_eq!(input.len(), 2);

        // Verify no reasoning item
        let json = serde_json::to_value(&input[0]).unwrap();
        assert_eq!(json["role"], "user");

        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["role"], "assistant");
    }

    #[test]
    fn test_handle_streaming_event_reasoning_encrypted_content() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // Create an OutputItemDone event with Reasoning item containing encrypted_content
        let event = StreamingEvent::OutputItemDone {
            sequence_number: 5,
            output_index: 0,
            item: Some(types::OutputItem::Reasoning {
                id: "rs_001".to_string(),
                summary: vec![],
                content: None,
                encrypted_content: Some("encrypted_reasoning_data".to_string()),
            }),
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "gpt-5".to_string(),
            None,
        );

        // Should emit ThinkingSignature with the encrypted content
        match result {
            LlmStreamEvent::ThinkingSignature(sig) => {
                assert_eq!(sig, "encrypted_reasoning_data");
            }
            _ => panic!("Expected ThinkingSignature event, got {:?}", result),
        }
    }

    #[test]
    fn test_handle_streaming_event_reasoning_without_encrypted_content() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // Create an OutputItemDone event with Reasoning item but NO encrypted_content
        let event = StreamingEvent::OutputItemDone {
            sequence_number: 5,
            output_index: 0,
            item: Some(types::OutputItem::Reasoning {
                id: "rs_001".to_string(),
                summary: vec![types::ContentPart::SummaryText {
                    text: "Some summary".to_string(),
                }],
                content: None,
                encrypted_content: None, // No encrypted content
            }),
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "gpt-5".to_string(),
            None,
        );

        // Should emit empty TextDelta (not ThinkingSignature)
        match result {
            LlmStreamEvent::TextDelta(text) => {
                assert!(text.is_empty());
            }
            _ => panic!("Expected empty TextDelta, got {:?}", result),
        }
    }

    #[test]
    fn test_handle_streaming_event_reasoning_delta() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // ReasoningDelta (opaque reasoning from o-series) maps to ThinkingDelta
        let event = StreamingEvent::ReasoningDelta {
            sequence_number: 3,
            item_id: "rs_001".to_string(),
            output_index: 0,
            content_index: 0,
            delta: "Let me reason about this...".to_string(),
            obfuscation: None,
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "o3".to_string(),
            None,
        );

        match result {
            LlmStreamEvent::ThinkingDelta(text) => {
                assert_eq!(text, "Let me reason about this...");
            }
            _ => panic!("Expected ThinkingDelta, got {:?}", result),
        }
    }

    #[test]
    fn test_handle_streaming_event_reasoning_summary_delta() {
        use std::sync::Mutex;

        let input_tokens = Mutex::new(0u32);
        let output_tokens = Mutex::new(0u32);
        let cache_read_tokens = Mutex::new(None);
        let accumulated_tool_calls = Mutex::new(Vec::new());
        let finish_reason = Mutex::new(None);

        // ReasoningSummaryDelta (readable summary from GPT-5.x) maps to ThinkingDelta
        let event = StreamingEvent::ReasoningSummaryDelta {
            sequence_number: 4,
            item_id: "rs_002".to_string(),
            output_index: 0,
            summary_index: 0,
            delta: "Breaking down the problem...".to_string(),
            obfuscation: None,
        };

        let result = handle_streaming_event(
            event,
            &input_tokens,
            &output_tokens,
            &cache_read_tokens,
            &accumulated_tool_calls,
            &finish_reason,
            "gpt-5.2".to_string(),
            None,
        );

        match result {
            LlmStreamEvent::ThinkingDelta(text) => {
                assert_eq!(text, "Breaking down the problem...");
            }
            _ => panic!("Expected ThinkingDelta, got {:?}", result),
        }
    }

    #[test]
    fn test_request_reasoning_none_is_omitted() {
        // When reasoning effort is "none", the reasoning field should be omitted
        // to avoid API errors on models that don't support reasoning params
        let config = LlmCallConfig {
            model: "gpt-5.2".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: Some("none".to_string()),
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            tool_search: None,
        };

        // Simulate the driver's filter logic
        let reasoning = config
            .reasoning_effort
            .as_ref()
            .filter(|e| !e.eq_ignore_ascii_case("none"))
            .map(|effort| ResponsesReasoning {
                effort: effort.clone(),
                summary: "detailed".to_string(),
            });

        assert!(
            reasoning.is_none(),
            "reasoning should be None for effort=none"
        );
    }

    #[test]
    fn test_request_reasoning_high_is_included() {
        // When reasoning effort is "high", the reasoning field should be present
        let config = LlmCallConfig {
            model: "gpt-5.2".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: Some("high".to_string()),
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            tool_search: None,
        };

        let reasoning = config
            .reasoning_effort
            .as_ref()
            .filter(|e| !e.eq_ignore_ascii_case("none"))
            .map(|effort| ResponsesReasoning {
                effort: effort.clone(),
                summary: "detailed".to_string(),
            });

        assert!(
            reasoning.is_some(),
            "reasoning should be present for effort=high"
        );
        let r = reasoning.unwrap();
        assert_eq!(r.effort, "high");
        assert_eq!(r.summary, "detailed");
    }

    #[test]
    fn test_request_reasoning_none_case_insensitive() {
        // "None", "NONE", "none" should all be filtered out
        for effort in &["none", "None", "NONE"] {
            let reasoning = Some(effort.to_string())
                .as_ref()
                .filter(|e| !e.eq_ignore_ascii_case("none"))
                .cloned();

            assert!(
                reasoning.is_none(),
                "effort={effort:?} should be filtered out"
            );
        }
    }

    #[test]
    fn test_build_input_assistant_without_thinking_or_tools() {
        // Plain assistant message (no thinking, no tool calls) should just be a message
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "Hello"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Hi there!".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
        ];

        let (_, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);

        assert_eq!(input.len(), 2);
        let json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(json["role"], "assistant");
        assert!(json.get("type").is_none() || json["type"] == "message");
    }

    #[test]
    fn test_build_input_multiple_reasoning_items_get_unique_ids() {
        // Multiple assistant messages with thinking_signature should get unique reasoning IDs
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "First question"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("First answer.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: Some("thinking 1".to_string()),
                thinking_signature: Some("encrypted_1".to_string()),
            },
            LlmMessage::text(LlmMessageRole::User, "Second question"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Second answer.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: Some("thinking 2".to_string()),
                thinking_signature: Some("encrypted_2".to_string()),
            },
        ];

        let (_, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);

        // Should have: user, reasoning_1, assistant, user, reasoning_2, assistant
        assert_eq!(input.len(), 6);

        let r1 = serde_json::to_value(&input[1]).unwrap();
        let r2 = serde_json::to_value(&input[4]).unwrap();

        assert_eq!(r1["type"], "reasoning");
        assert_eq!(r2["type"], "reasoning");
        assert_ne!(r1["id"], r2["id"], "Reasoning items should have unique IDs");
        assert_eq!(r1["encrypted_content"], "encrypted_1");
        assert_eq!(r2["encrypted_content"], "encrypted_2");
    }

    #[test]
    fn test_build_input_with_phases_enabled() {
        use crate::message::ExecutionPhase;

        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "You are helpful"),
            LlmMessage::text(LlmMessageRole::User, "Hello"),
            LlmMessage {
                role: LlmMessageRole::Assistant,
                content: LlmMessageContent::Text("Working on it...".to_string()),
                tool_calls: Some(vec![crate::tool_types::ToolCall {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    arguments: json!({}),
                }]),
                tool_call_id: None,
                phase: Some(ExecutionPhase::Commentary),
                thinking: None,
                thinking_signature: None,
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("result".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_1".to_string()),
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
        ];

        // With supports_phases=true, assistant message should include phase
        let (_, input) = OpenResponsesProtocolLlmDriver::build_input(&messages, true);
        let assistant_json = serde_json::to_value(&input[1]).unwrap();
        assert_eq!(assistant_json["phase"], "commentary");

        // With supports_phases=false, phase should be absent
        let (_, input_no_phases) = OpenResponsesProtocolLlmDriver::build_input(&messages, false);
        let assistant_json_no = serde_json::to_value(&input_no_phases[1]).unwrap();
        assert!(assistant_json_no.get("phase").is_none() || assistant_json_no["phase"].is_null());
    }

    // ========================================================================
    // tool_search / convert_tools_with_search tests
    // ========================================================================

    /// Helper: create a ToolDefinition with optional category and deferrable policy
    fn make_tool(
        name: &str,
        category: Option<&str>,
        deferrable: crate::tool_types::DeferrablePolicy,
    ) -> ToolDefinition {
        ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: format!("{} description", name),
            parameters: json!({"type": "object", "properties": {}}),
            policy: crate::tool_types::ToolPolicy::Auto,
            category: category.map(|s| s.to_string()),
            deferrable,
        })
    }

    #[test]
    fn test_convert_tools_with_search_below_threshold_falls_back() {
        use crate::tool_types::DeferrablePolicy;

        let tools: Vec<ToolDefinition> = (0..5)
            .map(|i| {
                make_tool(
                    &format!("tool_{i}"),
                    Some("cat"),
                    DeferrablePolicy::Automatic,
                )
            })
            .collect();

        // threshold=15, only 5 tools → should fall back to standard convert_tools
        let result = OpenResponsesProtocolLlmDriver::convert_tools_with_search(&tools, 15);
        assert_eq!(result.len(), 5);
        // No ToolSearch entry, no namespaces
        let json = serde_json::to_value(&result).unwrap();
        for item in json.as_array().unwrap() {
            assert_eq!(item["type"], "function");
            assert!(item.get("defer_loading").is_none() || item["defer_loading"].is_null());
        }
    }

    #[test]
    fn test_convert_tools_with_search_groups_by_category() {
        use crate::tool_types::DeferrablePolicy;

        let mut tools = vec![];
        // 10 "FileSystem" tools + 6 "Weather" tools = 16, threshold=15
        for i in 0..10 {
            tools.push(make_tool(
                &format!("fs_tool_{i}"),
                Some("FileSystem"),
                DeferrablePolicy::Automatic,
            ));
        }
        for i in 0..6 {
            tools.push(make_tool(
                &format!("weather_tool_{i}"),
                Some("Weather"),
                DeferrablePolicy::Automatic,
            ));
        }

        let result = OpenResponsesProtocolLlmDriver::convert_tools_with_search(&tools, 15);
        let json = serde_json::to_value(&result).unwrap();
        let arr = json.as_array().unwrap();

        // Should have: 2 namespace entries + 1 tool_search entry = 3
        assert_eq!(arr.len(), 3);

        // Last entry should be tool_search
        assert_eq!(arr.last().unwrap()["type"], "tool_search");

        // The two namespace entries
        let ns: Vec<&Value> = arr.iter().filter(|v| v["type"] == "namespace").collect();
        assert_eq!(ns.len(), 2);

        let ns_names: Vec<&str> = ns.iter().map(|v| v["name"].as_str().unwrap()).collect();
        assert!(ns_names.contains(&"FileSystem"));
        assert!(ns_names.contains(&"Weather"));

        // Check tool counts inside namespaces
        for n in &ns {
            let inner_tools = n["tools"].as_array().unwrap();
            match n["name"].as_str().unwrap() {
                "FileSystem" => assert_eq!(inner_tools.len(), 10),
                "Weather" => assert_eq!(inner_tools.len(), 6),
                other => panic!("Unexpected namespace: {other}"),
            }
            // All inner tools should have defer_loading: true
            for t in inner_tools {
                assert_eq!(t["defer_loading"], true);
            }
        }
    }

    #[test]
    fn test_convert_tools_with_search_never_defer_stays_top_level() {
        use crate::tool_types::DeferrablePolicy;

        let mut tools = vec![];
        // 2 Never-defer tools
        tools.push(make_tool(
            "write_todos",
            Some("Productivity"),
            DeferrablePolicy::Never,
        ));
        tools.push(make_tool(
            "get_session_info",
            Some("Session"),
            DeferrablePolicy::Never,
        ));
        // 14 Automatic tools in "FileSystem" category
        for i in 0..14 {
            tools.push(make_tool(
                &format!("fs_tool_{i}"),
                Some("FileSystem"),
                DeferrablePolicy::Automatic,
            ));
        }

        let result = OpenResponsesProtocolLlmDriver::convert_tools_with_search(&tools, 15);
        let json = serde_json::to_value(&result).unwrap();
        let arr = json.as_array().unwrap();

        // 2 never-defer functions + 1 FileSystem namespace + 1 tool_search = 4
        assert_eq!(arr.len(), 4);

        // First two should be non-deferred functions
        let funcs: Vec<&Value> = arr.iter().filter(|v| v["type"] == "function").collect();
        assert_eq!(funcs.len(), 2);
        for f in &funcs {
            // No defer_loading on never-defer tools
            assert!(f.get("defer_loading").is_none() || f["defer_loading"].is_null());
        }

        // Namespace
        let ns: Vec<&Value> = arr.iter().filter(|v| v["type"] == "namespace").collect();
        assert_eq!(ns.len(), 1);
        assert_eq!(ns[0]["name"], "FileSystem");
        assert_eq!(ns[0]["tools"].as_array().unwrap().len(), 14);
    }

    #[test]
    fn test_convert_tools_with_search_ungrouped_tools() {
        use crate::tool_types::DeferrablePolicy;

        let mut tools = vec![];
        // 10 categorized tools
        for i in 0..10 {
            tools.push(make_tool(
                &format!("cat_tool_{i}"),
                Some("Cat"),
                DeferrablePolicy::Automatic,
            ));
        }
        // 6 uncategorized tools (no category → ungrouped)
        for i in 0..6 {
            tools.push(make_tool(
                &format!("misc_tool_{i}"),
                None,
                DeferrablePolicy::Automatic,
            ));
        }

        let result = OpenResponsesProtocolLlmDriver::convert_tools_with_search(&tools, 15);
        let json = serde_json::to_value(&result).unwrap();
        let arr = json.as_array().unwrap();

        // 1 namespace + 6 ungrouped functions + 1 tool_search = 8
        assert_eq!(arr.len(), 8);

        let ns: Vec<&Value> = arr.iter().filter(|v| v["type"] == "namespace").collect();
        assert_eq!(ns.len(), 1);
        assert_eq!(ns[0]["tools"].as_array().unwrap().len(), 10);

        let funcs: Vec<&Value> = arr.iter().filter(|v| v["type"] == "function").collect();
        assert_eq!(funcs.len(), 6);
        // These ungrouped tools should still have defer_loading: true
        for f in &funcs {
            assert_eq!(f["defer_loading"], true);
        }

        assert_eq!(arr.last().unwrap()["type"], "tool_search");
    }

    #[test]
    fn test_convert_tools_with_search_always_policy() {
        use crate::tool_types::DeferrablePolicy;

        let mut tools = vec![];
        // 14 Automatic tools
        for i in 0..14 {
            tools.push(make_tool(
                &format!("tool_{i}"),
                Some("General"),
                DeferrablePolicy::Automatic,
            ));
        }
        // 1 Always tool (should be deferred even if only at threshold)
        tools.push(make_tool(
            "always_tool",
            Some("General"),
            DeferrablePolicy::Always,
        ));

        // Exactly at threshold (15 tools, threshold=15)
        let result = OpenResponsesProtocolLlmDriver::convert_tools_with_search(&tools, 15);
        let json = serde_json::to_value(&result).unwrap();
        let arr = json.as_array().unwrap();

        // 1 namespace (General) + 1 tool_search = 2
        assert_eq!(arr.len(), 2);

        let ns = &arr[0];
        assert_eq!(ns["type"], "namespace");
        let inner = ns["tools"].as_array().unwrap();
        assert_eq!(inner.len(), 15);
        // All should have defer_loading: true
        for t in inner {
            assert_eq!(t["defer_loading"], true);
        }
    }

    #[test]
    fn test_tool_search_serialization_format() {
        // Verify the ToolSearch entry serializes correctly
        let ts = ResponsesTool::ToolSearch {
            r#type: "tool_search".to_string(),
        };
        let json = serde_json::to_value(&ts).unwrap();
        assert_eq!(json, json!({"type": "tool_search"}));
    }

    #[test]
    fn test_namespace_serialization_format() {
        let ns = ResponsesTool::Namespace {
            r#type: "namespace".to_string(),
            name: "FileSystem".to_string(),
            description: "Tools for FileSystem".to_string(),
            tools: vec![ResponsesTool::Function {
                r#type: "function".to_string(),
                name: "read_file".to_string(),
                description: "Read a file".to_string(),
                parameters: json!({}),
                defer_loading: Some(true),
            }],
        };
        let json = serde_json::to_value(&ns).unwrap();
        assert_eq!(json["type"], "namespace");
        assert_eq!(json["name"], "FileSystem");
        assert_eq!(json["tools"][0]["name"], "read_file");
        assert_eq!(json["tools"][0]["defer_loading"], true);
    }
}
