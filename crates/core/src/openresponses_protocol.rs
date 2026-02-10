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
use crate::openai_protocol::is_openai_request_too_large;
use crate::openresponses_types::{self as types, StreamingEvent};
use crate::tool_types::{ToolCall, ToolDefinition};

const DEFAULT_API_URL: &str = "https://api.openai.com/v1/responses";

/// Open Responses Protocol Driver (OpenAI implementation)
///
/// Implements `LlmDriver` using the Open Responses specification
/// (https://www.openresponses.org/). This driver targets OpenAI's API
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

    fn convert_message(msg: &LlmMessage) -> ResponsesInputItem {
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

        ResponsesInputItem::Message {
            r#type: "message".to_string(),
            role: Self::convert_role(&msg.role).to_string(),
            content,
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
            })
            .collect()
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
    /// Handles the conversion of assistant messages with tool_calls into separate
    /// FunctionCall items, which is required by the Open Responses API.
    fn build_input(messages: &[LlmMessage]) -> (Option<String>, Vec<ResponsesInputItem>) {
        let mut instructions: Option<String> = None;
        let mut input_items = Vec::new();

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
            } else if msg.role == LlmMessageRole::Assistant
                && msg.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty())
            {
                // Assistant message with tool calls - emit FunctionCall items
                // First emit the message content if non-empty
                let has_content = match &msg.content {
                    LlmMessageContent::Text(text) => !text.is_empty(),
                    LlmMessageContent::Parts(parts) => !parts.is_empty(),
                };
                if has_content {
                    input_items.push(Self::convert_message(msg));
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
                input_items.push(Self::convert_message(msg));
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
        let (instructions, input_items) = Self::build_input(&messages);

        let tools = if config.tools.is_empty() {
            None
        } else {
            Some(Self::convert_tools(&config.tools))
        };

        // Build reasoning config if specified
        let reasoning = config
            .reasoning_effort
            .as_ref()
            .map(|effort| ResponsesReasoning {
                effort: effort.clone(),
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

                                        let input = *input_tokens.lock().unwrap();
                                        let output = *output_tokens.lock().unwrap();
                                        let cached = *cache_read_tokens.lock().unwrap();

                                        Ok(LlmStreamEvent::Done(LlmCompletionMetadata {
                                            total_tokens: Some(input + output),
                                            prompt_tokens: Some(input),
                                            completion_tokens: Some(output),
                                            cache_read_tokens: cached,
                                            cache_creation_tokens: None,
                                            model: Some(model),
                                            finish_reason: Some(reason),
                                            retry_metadata: retry_metadata_for_done
                                                .map(|arc| (*arc).clone()),
                                        }))
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
            if let Some(types::OutputItem::FunctionCall { .. }) = item {
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
            }
            LlmStreamEvent::TextDelta(String::new())
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

            let input = *input_tokens.lock().unwrap();
            let output = *output_tokens.lock().unwrap();
            let cached = *cache_read_tokens.lock().unwrap();

            LlmStreamEvent::Done(LlmCompletionMetadata {
                total_tokens: Some(input + output),
                prompt_tokens: Some(input),
                completion_tokens: Some(output),
                cache_read_tokens: cached,
                cache_creation_tokens: None,
                model: Some(model),
                finish_reason: Some(reason),
                retry_metadata: retry_metadata.map(|arc| (*arc).clone()),
            })
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
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum ResponsesInputItem {
    Message {
        r#type: String,
        role: String,
        content: ResponsesContent,
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
    Function {
        r#type: String,
        name: String,
        description: String,
        parameters: Value,
    },
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
            }],
            instructions: Some("You are helpful".to_string()),
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
            }],
            instructions: None,
            temperature: None,
            max_output_tokens: None,
            stream: true,
            tools: None,
            reasoning: Some(ResponsesReasoning {
                effort: "high".to_string(),
            }),
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["reasoning"]["effort"], "high");
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
            }],
            instructions: None,
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

        let (instructions, input) = OpenResponsesProtocolLlmDriver::build_input(&messages);

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
                thinking: None,
                thinking_signature: None,
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("2025-01-19T10:30:00Z".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_xyz789".to_string()),
                thinking: None,
                thinking_signature: None,
            },
        ];

        let (instructions, input) = OpenResponsesProtocolLlmDriver::build_input(&messages);

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
                thinking: None,
                thinking_signature: None,
            },
        ];

        let (_, input) = OpenResponsesProtocolLlmDriver::build_input(&messages);

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
}
