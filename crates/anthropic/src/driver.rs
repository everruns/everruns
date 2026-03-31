// Anthropic Claude LLM Driver
//
// Implementation of LlmDriver for Anthropic's Claude API.
// Uses the Messages API with streaming support.
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff, respecting the retry-after header if provided.
// Retry metadata is included in the response for observability.
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// llm.generation events are emitted by ReasonAtom, and OtelEventListener
// creates the appropriate gen-ai spans. No direct tracing in drivers.

use async_trait::async_trait;
use chrono::DateTime;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_helpers::{
    self, ANTHROPIC_NOT_FOUND_PATTERNS, ANTHROPIC_TOO_LARGE_PATTERNS, AUDIO_CONTENT_PLACEHOLDER,
    parse_data_url,
};
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverRegistry, LlmCallConfig, LlmCompletionMetadata,
    LlmContentPart, LlmDriver, LlmMessage, LlmMessageContent, LlmMessageRole, LlmResponseStream,
    LlmStreamEvent, ProviderType,
};
use everruns_core::llm_retry::{
    LlmRetryConfig, RateLimitInfo, RetryMetadata, is_rate_limit_status, is_transient_error,
};
use everruns_core::tool_types::{ToolCall, ToolDefinition};

const DEFAULT_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Claude LLM Driver
///
/// Implements `LlmDriver` for Anthropic's Messages API.
/// Supports streaming responses and tool calls.
///
/// Rate limit handling: On 429 errors, automatically retries with exponential
/// backoff, respecting the `retry-after` header if provided by Anthropic.
///
/// # Example
///
/// ```ignore
/// use everruns_anthropic::AnthropicLlmDriver;
///
/// let driver = AnthropicLlmDriver::from_env()?;
/// // or
/// let driver = AnthropicLlmDriver::new("your-api-key");
/// // or with custom endpoint
/// let driver = AnthropicLlmDriver::with_base_url("your-api-key", "https://api.example.com/v1/messages");
/// // or with custom retry config
/// let driver = AnthropicLlmDriver::new("your-api-key")
///     .with_retry_config(LlmRetryConfig::aggressive());
/// ```
#[derive(Clone)]
pub struct AnthropicLlmDriver {
    client: Client,
    api_key: String,
    api_url: String,
    /// Whether using a custom base URL (not Anthropic's API)
    uses_custom_url: bool,
    /// Retry configuration for rate limit errors
    retry_config: LlmRetryConfig,
}

impl AnthropicLlmDriver {
    /// Create a new provider with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_url: DEFAULT_API_URL.to_string(),
            uses_custom_url: false,
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// Create a new provider from the ANTHROPIC_API_KEY environment variable
    pub fn from_env() -> Result<Self> {
        let api_key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| AgentLoopError::llm("ANTHROPIC_API_KEY environment variable not set"))?;
        Ok(Self::new(api_key))
    }

    /// Create a new provider with a custom API URL
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_url: api_url.into(),
            uses_custom_url: true,
            retry_config: LlmRetryConfig::default(),
        }
    }

    /// Configure retry behavior for rate limit errors
    pub fn with_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.retry_config = config;
        self
    }

    /// Check if using a custom base URL
    pub fn uses_custom_url(&self) -> bool {
        self.uses_custom_url
    }

    fn convert_role(role: &LlmMessageRole) -> &'static str {
        match role {
            LlmMessageRole::System => "user", // System is handled separately in Anthropic
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "user", // Tool results are sent as user messages
        }
    }

    fn convert_content(content: &LlmMessageContent) -> Vec<AnthropicContentBlock> {
        match content {
            LlmMessageContent::Text(text) => {
                // Skip empty text to avoid Anthropic API error
                if text.is_empty() {
                    vec![]
                } else {
                    vec![AnthropicContentBlock::Text { text: text.clone() }]
                }
            }
            LlmMessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|part| match part {
                    LlmContentPart::Text { text } => {
                        // Skip empty text to avoid Anthropic API error
                        if text.is_empty() {
                            None
                        } else {
                            Some(AnthropicContentBlock::Text { text: text.clone() })
                        }
                    }
                    LlmContentPart::Image { url } => {
                        if let Some(parsed) = parse_data_url(url) {
                            Some(AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Base64 {
                                    media_type: parsed.media_type,
                                    data: parsed.data,
                                },
                            })
                        } else if url.starts_with("data:") {
                            // Malformed data URL — fall back to image/jpeg
                            Some(AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Base64 {
                                    media_type: "image/jpeg".to_string(),
                                    data: url.clone(),
                                },
                            })
                        } else {
                            // HTTP URL
                            Some(AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Url { url: url.clone() },
                            })
                        }
                    }
                    LlmContentPart::Audio { .. } => Some(AnthropicContentBlock::Text {
                        text: AUDIO_CONTENT_PLACEHOLDER.to_string(),
                    }),
                })
                .collect(),
        }
    }

    fn convert_messages(messages: &[LlmMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system_prompt = None;
        let mut converted = Vec::new();

        for msg in messages {
            match msg.role {
                LlmMessageRole::System => {
                    // Extract system prompt (Anthropic handles it separately)
                    system_prompt = Some(msg.content.to_text());
                }
                LlmMessageRole::Tool => {
                    // Tool results in Anthropic are user messages with tool_result content blocks.
                    // When the message contains images, we use the array form with content blocks.
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        let has_images = match &msg.content {
                            LlmMessageContent::Parts(parts) => parts
                                .iter()
                                .any(|p| matches!(p, LlmContentPart::Image { .. })),
                            _ => false,
                        };

                        let content = if has_images {
                            // Build array of text + image content blocks
                            let blocks = match &msg.content {
                                LlmMessageContent::Parts(parts) => parts
                                    .iter()
                                    .filter_map(|p| match p {
                                        LlmContentPart::Text { text } => {
                                            Some(AnthropicToolResultBlock::Text {
                                                text: text.clone(),
                                            })
                                        }
                                        LlmContentPart::Image { url } => {
                                            let source = if let Some(parsed) = parse_data_url(url) {
                                                AnthropicImageSource::Base64 {
                                                    media_type: parsed.media_type,
                                                    data: parsed.data,
                                                }
                                            } else {
                                                AnthropicImageSource::Url { url: url.clone() }
                                            };
                                            Some(AnthropicToolResultBlock::Image { source })
                                        }
                                        _ => None,
                                    })
                                    .collect(),
                                LlmMessageContent::Text(text) => {
                                    vec![AnthropicToolResultBlock::Text { text: text.clone() }]
                                }
                            };
                            AnthropicToolResultContent::Blocks(blocks)
                        } else {
                            AnthropicToolResultContent::Text(msg.content.to_text())
                        };

                        converted.push(AnthropicMessage {
                            role: "user".to_string(),
                            content: vec![AnthropicContentBlock::ToolResult {
                                tool_use_id: tool_call_id.clone(),
                                content,
                                is_error: None,
                            }],
                        });
                    }
                }
                LlmMessageRole::Assistant => {
                    let mut content = Vec::new();

                    // Debug: log thinking state for assistant messages
                    tracing::debug!(
                        has_thinking = msg.thinking.is_some(),
                        has_signature = msg.thinking_signature.is_some(),
                        thinking_len = msg.thinking.as_ref().map(|t| t.len()),
                        signature_len = msg.thinking_signature.as_ref().map(|s| s.len()),
                        has_tool_calls = msg.tool_calls.is_some(),
                        tool_calls_count = msg.tool_calls.as_ref().map(|tc| tc.len()),
                        "AnthropicDriver: converting assistant message"
                    );

                    // Add thinking block first if present (required by Anthropic when thinking is enabled)
                    // Both thinking content and signature are required when sending thinking back
                    if let (Some(thinking), Some(signature)) =
                        (&msg.thinking, &msg.thinking_signature)
                    {
                        tracing::debug!(
                            thinking_len = thinking.len(),
                            signature_len = signature.len(),
                            "AnthropicDriver: adding thinking block to assistant message"
                        );
                        content.push(AnthropicContentBlock::Thinking {
                            thinking: thinking.clone(),
                            signature: signature.clone(),
                        });
                    } else if msg.thinking.is_some() || msg.thinking_signature.is_some() {
                        // Warn if one is present but not the other
                        tracing::warn!(
                            has_thinking = msg.thinking.is_some(),
                            has_signature = msg.thinking_signature.is_some(),
                            "AnthropicDriver: assistant message has partial thinking data (need both)"
                        );
                    }

                    // Add text/image content
                    content.extend(Self::convert_content(&msg.content));

                    // Add tool_use blocks if present
                    if let Some(tool_calls) = &msg.tool_calls {
                        for tc in tool_calls {
                            content.push(AnthropicContentBlock::ToolUse {
                                id: tc.id.clone(),
                                name: tc.name.clone(),
                                input: tc.arguments.clone(),
                            });
                        }
                    }

                    converted.push(AnthropicMessage {
                        role: Self::convert_role(&msg.role).to_string(),
                        content,
                    });
                }
                _ => {
                    converted.push(AnthropicMessage {
                        role: Self::convert_role(&msg.role).to_string(),
                        content: Self::convert_content(&msg.content),
                    });
                }
            }
        }

        (system_prompt, converted)
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<AnthropicTool> {
        tools
            .iter()
            .map(|tool| AnthropicTool {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.parameters().clone(),
            })
            .collect()
    }
}

#[async_trait]
impl LlmDriver for AnthropicLlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Note: OTel instrumentation is handled via event listeners.
        // ReasonAtom emits llm.generation events, and OtelEventListener
        // creates gen-ai spans from those events.
        let (system_prompt, anthropic_messages) = Self::convert_messages(&messages);

        let tools = if config.tools.is_empty() {
            None
        } else {
            Some(Self::convert_tools(&config.tools))
        };

        // Build thinking config from reasoning effort
        let thinking = config
            .reasoning_effort
            .as_ref()
            .and_then(|e| AnthropicThinking::from_effort(e));

        tracing::info!(
            model = %config.model,
            reasoning_effort = ?config.reasoning_effort,
            thinking_enabled = thinking.is_some(),
            thinking_budget = ?thinking.as_ref().map(|t| t.budget_tokens),
            "AnthropicDriver: building request with thinking config"
        );

        // Calculate max_tokens - use caller's limit, or model's max output from profile, or 16384 fallback.
        // Anthropic requires max_tokens (can't omit), so we look up the model's native limit.
        let max_tokens_from_profile = config.max_tokens.is_none();
        let base_max_tokens = config.max_tokens.unwrap_or_else(|| {
            everruns_core::get_model_profile(
                &everruns_core::LlmProviderType::Anthropic,
                &config.model,
            )
            .and_then(|p| {
                p.limits.and_then(|l| {
                    u32::try_from(l.output)
                        .ok()
                        .and_then(|v| if v > 0 { Some(v) } else { None })
                })
            })
            .unwrap_or(16_384)
        });
        let max_tokens = if let Some(ref thinking_config) = thinking {
            // max_tokens must be > thinking.budget_tokens per Anthropic requirements
            // Only increase if the caller's limit is too low for the thinking budget
            let min_for_thinking = thinking_config.budget_tokens + 1024; // minimum headroom for response
            base_max_tokens.max(min_for_thinking)
        } else {
            base_max_tokens
        };

        // Check if we need interleaved thinking beta header BEFORE moving values
        let needs_interleaved_thinking = thinking.is_some() && tools.is_some();

        let mut request = AnthropicRequest {
            model: config.model.clone(),
            messages: anthropic_messages,
            max_tokens,
            temperature: config.temperature,
            system: system_prompt,
            stream: true,
            tools,
            thinking,
        };

        // Retry loop for rate limit (429) and transient errors
        let mut retry_metadata = RetryMetadata::default();
        let mut last_error: Option<String> = None;
        let mut max_tokens_fallback_attempted = false;

        let response = loop {
            // Build request with headers (must rebuild each iteration)
            let mut request_builder = self
                .client
                .post(&self.api_url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("Content-Type", "application/json");

            // Add interleaved thinking beta header when thinking is enabled with tools
            // Required for tool use with extended thinking per Anthropic docs
            if needs_interleaved_thinking {
                request_builder =
                    request_builder.header("anthropic-beta", "interleaved-thinking-2025-05-14");
                if retry_metadata.attempts == 0 {
                    tracing::info!(
                        "AnthropicDriver: enabling interleaved thinking beta for tool use"
                    );
                }
            }

            let response = request_builder
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
                    Some(RateLimitInfo::from_anthropic_headers(response.headers()))
                } else {
                    None
                };

                let error_text = response.text().await.unwrap_or_default();

                // Don't retry if this is a request-too-large error (not transient)
                if is_anthropic_request_too_large(status, &error_text) {
                    return Err(AgentLoopError::request_too_large(format!(
                        "Anthropic API error ({}): {}",
                        status, error_text
                    )));
                }

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
                    "AnthropicDriver: rate limit or transient error, retrying"
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
            let error_msg = format!("Anthropic API error ({}): {}", status, error_text);

            // Graceful fallback: if max_tokens derived from profile exceeds model limit (400 error),
            // retry once with a safe fallback value instead of failing immediately.
            if status == reqwest::StatusCode::BAD_REQUEST
                && !max_tokens_fallback_attempted
                && max_tokens_from_profile
                && (error_text.contains("max_tokens")
                    || error_text.contains("maximum output tokens"))
            {
                const FALLBACK_MAX_TOKENS: u32 = 16_384;
                tracing::warn!(
                    attempted = request.max_tokens,
                    fallback = FALLBACK_MAX_TOKENS,
                    model = %config.model,
                    "max_tokens exceeds model limit, retrying with fallback. \
                     Update model profile for {}.",
                    config.model,
                );
                request.max_tokens = FALLBACK_MAX_TOKENS;
                max_tokens_fallback_attempted = true;
                continue;
            }

            // Check if this is a model-not-found error (404 with not_found_error)
            if is_anthropic_model_not_found(status, &error_text) {
                return Err(AgentLoopError::model_not_available(config.model.clone()));
            }

            // Check if this is a request-too-large error
            if is_anthropic_request_too_large(status, &error_text) {
                return Err(AgentLoopError::request_too_large(error_msg));
            }

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
                "AnthropicDriver: request succeeded after retries"
            );
        }

        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream.eventsource();

        let model = config.model.clone();
        let input_tokens = Arc::new(Mutex::new(0u32));
        let output_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let cache_creation_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let current_tool_call = Arc::new(Mutex::new(Option::<ToolCall>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));
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
            let cache_creation_tokens = Arc::clone(&cache_creation_tokens);
            let current_tool_call = Arc::clone(&current_tool_call);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            let retry_metadata_for_done = shared_retry_metadata.clone();

            async move {
                match result {
                    Ok(event) => {
                        // Anthropic uses different event types
                        match event.event.as_str() {
                            "message_start" => {
                                // Parse message_start for input token count and cache tokens
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicMessageStart>(&event.data)
                                    && let Some(usage) = data.message.usage
                                {
                                    *input_tokens.lock().unwrap() = usage.input_tokens;
                                    if let Some(cache_read) = usage.cache_read_input_tokens {
                                        *cache_read_tokens.lock().unwrap() = Some(cache_read);
                                    }
                                    if let Some(cache_creation) = usage.cache_creation_input_tokens
                                    {
                                        *cache_creation_tokens.lock().unwrap() =
                                            Some(cache_creation);
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_start" => {
                                // Check if starting a tool use block
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicContentBlockStart>(&event.data)
                                    && let AnthropicContentBlockDelta::ToolUse { id, name } =
                                        data.content_block
                                {
                                    let mut current = current_tool_call.lock().unwrap();
                                    *current = Some(ToolCall {
                                        id,
                                        name,
                                        arguments: json!(""),
                                    });
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_delta" => {
                                if let Ok(data) = serde_json::from_str::<
                                    AnthropicContentBlockDeltaEvent,
                                >(&event.data)
                                {
                                    match data.delta {
                                        AnthropicDelta::TextDelta { text } => {
                                            *output_tokens.lock().unwrap() += 1;
                                            return Ok(LlmStreamEvent::TextDelta(text));
                                        }
                                        AnthropicDelta::InputJsonDelta { partial_json } => {
                                            // Accumulate tool input JSON
                                            let mut current = current_tool_call.lock().unwrap();
                                            if let Some(ref mut tc) = *current {
                                                let current_args =
                                                    tc.arguments.as_str().unwrap_or("");
                                                let combined =
                                                    format!("{}{}", current_args, partial_json);
                                                tc.arguments = json!(combined);
                                            }
                                            return Ok(LlmStreamEvent::TextDelta(String::new()));
                                        }
                                        AnthropicDelta::ThinkingDelta { thinking } => {
                                            // Stream thinking content from extended thinking models
                                            tracing::debug!(
                                                thinking_len = thinking.len(),
                                                "AnthropicDriver: received thinking_delta from API"
                                            );
                                            return Ok(LlmStreamEvent::ThinkingDelta(thinking));
                                        }
                                        AnthropicDelta::SignatureDelta { signature } => {
                                            // Cryptographic signature for thinking content
                                            // This is required when sending thinking back to Anthropic
                                            tracing::info!(
                                                signature_len = signature.len(),
                                                "AnthropicDriver: received signature_delta from API"
                                            );
                                            return Ok(LlmStreamEvent::ThinkingSignature(signature));
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_stop" => {
                                // Debug: log raw content_block_stop data
                                tracing::debug!(
                                    raw_data = %event.data,
                                    "AnthropicDriver: received content_block_stop event"
                                );

                                // Finalize current tool call if any
                                let mut current = current_tool_call.lock().unwrap();
                                if let Some(mut tc) = current.take() {
                                    // Parse the accumulated JSON string
                                    if let Some(args_str) = tc.arguments.as_str() {
                                        tc.arguments =
                                            serde_json::from_str(args_str).unwrap_or(json!({}));
                                    }
                                    accumulated_tool_calls.lock().unwrap().push(tc);
                                }
                                // Check if this is a thinking block with signature
                                match serde_json::from_str::<AnthropicContentBlockStop>(&event.data)
                                {
                                    Ok(data) => {
                                        tracing::debug!(
                                            has_content_block = data.content_block.is_some(),
                                            "AnthropicDriver: parsed content_block_stop"
                                        );
                                        if let Some(AnthropicCompletedContentBlock::Thinking {
                                            signature,
                                            ..
                                        }) = data.content_block
                                        {
                                            tracing::info!(
                                                signature_len = signature.len(),
                                                "AnthropicDriver: extracted thinking signature from content_block_stop"
                                            );
                                            return Ok(LlmStreamEvent::ThinkingSignature(signature));
                                        }
                                    }
                                    Err(e) => {
                                        tracing::debug!(
                                            error = %e,
                                            raw_data = %event.data,
                                            "AnthropicDriver: failed to parse content_block_stop"
                                        );
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "message_delta" => {
                                // Check for stop_reason and output tokens
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicMessageDelta>(&event.data)
                                {
                                    if let Some(usage) = data.usage {
                                        *output_tokens.lock().unwrap() = usage.output_tokens;
                                        // Cache tokens may also appear in delta
                                        if usage.cache_read_input_tokens.is_some() {
                                            *cache_read_tokens.lock().unwrap() =
                                                usage.cache_read_input_tokens;
                                        }
                                        if usage.cache_creation_input_tokens.is_some() {
                                            *cache_creation_tokens.lock().unwrap() =
                                                usage.cache_creation_input_tokens;
                                        }
                                    }

                                    if let Some(stop_reason) = data.delta.stop_reason
                                        && stop_reason == "tool_use"
                                    {
                                        let tool_calls =
                                            accumulated_tool_calls.lock().unwrap().clone();
                                        if !tool_calls.is_empty() {
                                            return Ok(LlmStreamEvent::ToolCalls(tool_calls));
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "message_stop" => {
                                let in_tokens = *input_tokens.lock().unwrap();
                                let out_tokens = *output_tokens.lock().unwrap();
                                let cache_read = *cache_read_tokens.lock().unwrap();
                                let cache_creation = *cache_creation_tokens.lock().unwrap();

                                Ok(LlmStreamEvent::Done(Box::new(LlmCompletionMetadata {
                                    total_tokens: Some(in_tokens + out_tokens),
                                    prompt_tokens: Some(in_tokens),
                                    completion_tokens: Some(out_tokens),
                                    cache_read_tokens: cache_read,
                                    cache_creation_tokens: cache_creation,
                                    model: Some(model),
                                    finish_reason: Some("stop".to_string()),
                                    retry_metadata: retry_metadata_for_done
                                        .map(|arc| (*arc).clone()),
                                    response_id: None,
                                    phase: None,
                                })))
                            }
                            "error" => Ok(LlmStreamEvent::Error(format!(
                                "Anthropic stream error: {}",
                                event.data
                            ))),
                            "ping" => {
                                // Keep-alive ping, ignore
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            _ => {
                                // Unknown event type, ignore
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                        }
                    }
                    Err(e) => Ok(LlmStreamEvent::Error(format!("Stream error: {}", e))),
                }
            }
        }));

        Ok(converted_stream)
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs (proxies, self-hosted)
        if self.uses_custom_url {
            return Ok(None);
        }

        let response = self
            .client
            .get(ANTHROPIC_MODELS_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .send()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to fetch models: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AgentLoopError::llm(format!(
                "Models API returned {}: {}",
                status, body
            )));
        }

        let models_response: AnthropicModelsResponse = response
            .json()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to parse models response: {}", e)))?;

        // All Anthropic models are chat models, no filtering needed
        let discovered: Vec<DiscoveredModel> = models_response
            .data
            .into_iter()
            .map(|m| {
                let profile = Some(m.to_discovered_profile());
                DiscoveredModel {
                    model_id: m.id,
                    display_name: Some(m.display_name),
                    created_at: m
                        .created_at
                        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc)),
                    owned_by: Some("anthropic".to_string()),
                    discovered_profile: profile,
                }
            })
            .collect();

        Ok(Some(discovered))
    }
}

impl std::fmt::Debug for AnthropicLlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicLlmDriver")
            .field("api_url", &self.api_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the Anthropic driver with the driver registry
///
/// This should be called at application startup to enable Anthropic model support.
///
/// # Example
///
/// ```ignore
/// use everruns_core::DriverRegistry;
/// use everruns_anthropic::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register(ProviderType::Anthropic, |api_key, base_url| {
        let driver = match base_url {
            Some(url) => AnthropicLlmDriver::with_base_url(api_key, url),
            None => AnthropicLlmDriver::new(api_key),
        };
        Box::new(driver) as BoxedLlmDriver
    });
}

// ============================================================================
// Error Detection Helpers
// ============================================================================

fn is_anthropic_model_not_found(status: reqwest::StatusCode, error_text: &str) -> bool {
    if llm_driver_helpers::is_model_not_found(status, error_text, ANTHROPIC_NOT_FOUND_PATTERNS) {
        return true;
    }
    // Compound check: both "model" and "not found" must appear together
    if status == reqwest::StatusCode::NOT_FOUND {
        let lower = error_text.to_lowercase();
        if lower.contains("model") && lower.contains("not found") {
            return true;
        }
    }
    false
}

fn is_anthropic_request_too_large(status: reqwest::StatusCode, error_text: &str) -> bool {
    llm_driver_helpers::is_request_too_large(status, error_text, ANTHROPIC_TOO_LARGE_PATTERNS)
}

// ============================================================================
// Anthropic API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    /// Extended thinking configuration (for Claude models that support it)
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
}

/// Extended thinking configuration for Claude
#[derive(Debug, Serialize)]
struct AnthropicThinking {
    r#type: String,
    /// Budget tokens for thinking (varies by effort level)
    budget_tokens: u32,
}

impl AnthropicThinking {
    /// Create thinking config from reasoning effort level
    fn from_effort(effort: &str) -> Option<Self> {
        llm_driver_helpers::thinking_budget::from_effort(effort).map(|budget| Self {
            r#type: "enabled".to_string(),
            budget_tokens: budget,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        /// Cryptographic signature required when sending thinking back to the API
        signature: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: AnthropicToolResultContent,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

/// Content of a tool_result block - either a simple string or array of content blocks.
/// Anthropic API accepts both forms; we use the array form when images are present.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum AnthropicToolResultContent {
    /// Simple text content
    Text(String),
    /// Array of content blocks (text + images)
    Blocks(Vec<AnthropicToolResultBlock>),
}

/// A content block inside a tool_result (text or image)
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicToolResultBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum AnthropicImageSource {
    #[serde(rename = "base64")]
    Base64 { media_type: String, data: String },
    #[serde(rename = "url")]
    Url { url: String },
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: Value,
}

// Streaming response types

#[derive(Debug, Deserialize)]
struct AnthropicMessageStart {
    message: AnthropicMessageInfo,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // id and model are deserialized but used by event listeners, not directly
struct AnthropicMessageInfo {
    /// Unique identifier for this message
    #[serde(default)]
    id: Option<String>,
    /// Model used for this message
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    /// Tokens read from cache (reduces cost)
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    /// Tokens written to cache
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockStart {
    content_block: AnthropicContentBlockDelta,
}

/// Completed content block from content_block_stop event
/// Includes the cryptographic signature for thinking blocks
#[derive(Debug, Deserialize)]
struct AnthropicContentBlockStop {
    #[serde(default)]
    content_block: Option<AnthropicCompletedContentBlock>,
}

/// Completed content block variants (from content_block_stop)
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)] // Fields used for JSON deserialization
enum AnthropicCompletedContentBlock {
    #[serde(rename = "thinking")]
    Thinking {
        thinking: String,
        /// Cryptographic signature for the thinking content (required to send it back)
        signature: String,
    },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)] // Fields used for deserialization
enum AnthropicContentBlockDelta {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockDeltaEvent {
    delta: AnthropicDelta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(clippy::enum_variant_names)] // Delta suffix matches Anthropic's API naming
enum AnthropicDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
    #[serde(rename = "thinking_delta")]
    ThinkingDelta { thinking: String },
    /// Cryptographic signature for thinking content (sent after thinking_delta completes)
    #[serde(rename = "signature_delta")]
    SignatureDelta { signature: String },
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDelta {
    delta: AnthropicMessageDeltaData,
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageDeltaData {
    stop_reason: Option<String>,
}

// ============================================================================
// Models API Types
// ============================================================================

/// Response from Anthropic's /v1/models endpoint
#[derive(Debug, Deserialize)]
struct AnthropicModelsResponse {
    data: Vec<AnthropicModelInfo>,
}

/// Individual model info from Anthropic's models API
///
/// Includes structured capabilities and token limits returned by the
/// `/v1/models` endpoint, used to build `LlmModelProfile` at discovery time.
#[derive(Debug, Deserialize)]
struct AnthropicModelInfo {
    /// Model identifier (e.g., "claude-opus-4-5-20251101")
    id: String,
    /// Human-readable display name (e.g., "Claude Opus 4.5")
    display_name: String,
    /// ISO 8601 timestamp when the model was created
    #[serde(default)]
    created_at: Option<String>,
    /// Maximum input context window size in tokens
    #[serde(default)]
    max_input_tokens: Option<u32>,
    /// Maximum output tokens
    #[serde(default)]
    max_tokens: Option<u32>,
    /// Model capabilities
    #[serde(default)]
    capabilities: Option<AnthropicModelCapabilities>,
}

/// Capability support flag from Anthropic's models API
#[derive(Debug, Deserialize, Default)]
struct CapabilitySupport {
    #[serde(default)]
    supported: bool,
}

/// Effort capability with per-level support
#[derive(Debug, Deserialize, Default)]
struct EffortCapability {
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    low: Option<CapabilitySupport>,
    #[serde(default)]
    medium: Option<CapabilitySupport>,
    #[serde(default)]
    high: Option<CapabilitySupport>,
    #[serde(default)]
    max: Option<CapabilitySupport>,
}

/// Thinking capability with type configurations
#[derive(Debug, Deserialize, Default)]
struct ThinkingCapability {
    #[serde(default)]
    supported: bool,
    #[serde(default)]
    types: Option<ThinkingTypes>,
}

/// Supported thinking type configurations
#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)] // Fields deserialized from API; stored as metadata
struct ThinkingTypes {
    #[serde(default)]
    enabled: Option<CapabilitySupport>,
    #[serde(default)]
    adaptive: Option<CapabilitySupport>,
}

/// Model capabilities from Anthropic's models API
#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)] // Fields deserialized from API; not all used yet but stored as metadata
struct AnthropicModelCapabilities {
    #[serde(default)]
    image_input: Option<CapabilitySupport>,
    #[serde(default)]
    pdf_input: Option<CapabilitySupport>,
    #[serde(default)]
    structured_outputs: Option<CapabilitySupport>,
    #[serde(default)]
    thinking: Option<ThinkingCapability>,
    #[serde(default)]
    effort: Option<EffortCapability>,
    #[serde(default)]
    citations: Option<CapabilitySupport>,
    #[serde(default)]
    code_execution: Option<CapabilitySupport>,
    #[serde(default)]
    batch: Option<CapabilitySupport>,
}

/// Normalize Anthropic model ID to a family base name by stripping trailing
/// date suffix (e.g., "claude-opus-4-5-20251101" -> "claude-opus-4-5").
fn normalize_anthropic_id(model_id: &str) -> &str {
    // Anthropic date suffixes are always -YYYYMMDD (8 digits after a dash)
    if model_id.len() > 9 {
        let suffix_start = model_id.len() - 9; // "-YYYYMMDD" is 9 chars
        let (base, suffix) = model_id.split_at(suffix_start);
        if suffix.starts_with('-')
            && suffix[1..].len() == 8
            && suffix[1..].chars().all(|c| c.is_ascii_digit())
        {
            return base;
        }
    }
    model_id
}

impl AnthropicModelInfo {
    /// Build an `LlmModelProfile` from the API-provided metadata.
    ///
    /// This profile contains limits and capability flags discovered from the API.
    /// Cost data is NOT available from the API and remains in hardcoded profiles.
    fn to_discovered_profile(&self) -> everruns_core::llm_models::LlmModelProfile {
        use everruns_core::llm_models::*;

        let caps = self.capabilities.as_ref();

        // Build token limits from API fields
        let limits = match (self.max_input_tokens, self.max_tokens) {
            (Some(input), Some(output)) => Some(LlmModelLimits {
                context: input as i32,
                input: None,
                output: output as i32,
                max_media: None,
            }),
            _ => None,
        };

        // Determine if model supports image/PDF input (implies attachment support)
        let image_input = caps
            .and_then(|c| c.image_input.as_ref())
            .is_some_and(|c| c.supported);
        let pdf_input = caps
            .and_then(|c| c.pdf_input.as_ref())
            .is_some_and(|c| c.supported);
        let supports_attachments = image_input || pdf_input;

        // Build modalities from capabilities
        let modalities = {
            let mut input_mods = vec![Modality::Text];
            if image_input {
                input_mods.push(Modality::Image);
            }
            Some(LlmModelModalities {
                input: input_mods,
                output: vec![Modality::Text],
            })
        };

        // Build reasoning effort config from thinking + effort capabilities
        let reasoning = caps
            .and_then(|c| c.thinking.as_ref())
            .is_some_and(|t| t.supported);

        let reasoning_effort = if reasoning {
            self.build_reasoning_effort(caps)
        } else {
            None
        };

        let structured_output = caps
            .and_then(|c| c.structured_outputs.as_ref())
            .is_some_and(|c| c.supported);

        LlmModelProfile {
            name: self.display_name.clone(),
            family: normalize_anthropic_id(&self.id).to_string(),
            description: None,
            release_date: self
                .created_at
                .as_ref()
                .and_then(|s| s.get(..10))
                .map(|s| s.to_string()),
            last_updated: None,
            attachment: supports_attachments,
            reasoning,
            temperature: true, // All Claude models support temperature
            knowledge: None,   // Not available from API
            tool_call: true,   // All Claude models support tool use
            structured_output,
            open_weights: false,
            cost: None, // Not available from API; hardcoded profiles provide this
            limits,
            modalities,
            reasoning_effort,
            tool_search: false,
            supports_phases: false,
        }
    }

    fn build_reasoning_effort(
        &self,
        caps: Option<&AnthropicModelCapabilities>,
    ) -> Option<everruns_core::llm_models::ReasoningEffortConfig> {
        use everruns_core::llm_models::*;

        let thinking = caps?.thinking.as_ref()?;
        if !thinking.supported {
            return None;
        }

        let types = thinking.types.as_ref();
        let supports_adaptive = types
            .and_then(|t| t.adaptive.as_ref())
            .is_some_and(|c| c.supported);

        // Check effort capability for supported levels
        let effort_cap = caps?.effort.as_ref();
        let effort_supported = effort_cap.is_some_and(|e| e.supported);

        if !effort_supported {
            // Thinking supported but no effort levels — basic extended thinking
            return Some(ReasoningEffortConfig {
                values: vec![
                    ReasoningEffortValue {
                        value: ReasoningEffort::Low,
                        name: "Low (1K tokens)".into(),
                    },
                    ReasoningEffortValue {
                        value: ReasoningEffort::Medium,
                        name: "Medium (4K tokens)".into(),
                    },
                    ReasoningEffortValue {
                        value: ReasoningEffort::High,
                        name: "High (16K tokens)".into(),
                    },
                    ReasoningEffortValue {
                        value: ReasoningEffort::Xhigh,
                        name: "Extra High (32K tokens)".into(),
                    },
                ],
                default: ReasoningEffort::Medium,
            });
        }

        // Build effort values from per-level support flags
        let ec = effort_cap.unwrap();
        let mut values = Vec::new();

        if ec.low.as_ref().is_some_and(|c| c.supported) {
            let name = if supports_adaptive {
                "Low"
            } else {
                "Low (1K tokens)"
            };
            values.push(ReasoningEffortValue {
                value: ReasoningEffort::Low,
                name: name.into(),
            });
        }
        if ec.medium.as_ref().is_some_and(|c| c.supported) {
            let name = if supports_adaptive {
                "Medium"
            } else {
                "Medium (4K tokens)"
            };
            values.push(ReasoningEffortValue {
                value: ReasoningEffort::Medium,
                name: name.into(),
            });
        }
        if ec.high.as_ref().is_some_and(|c| c.supported) {
            let name = if supports_adaptive {
                "High"
            } else {
                "High (16K tokens)"
            };
            values.push(ReasoningEffortValue {
                value: ReasoningEffort::High,
                name: name.into(),
            });
        }
        if ec.max.as_ref().is_some_and(|c| c.supported) {
            let name = if supports_adaptive {
                "Max"
            } else {
                "Extra High (32K tokens)"
            };
            values.push(ReasoningEffortValue {
                value: ReasoningEffort::Xhigh,
                name: name.into(),
            });
        }

        if values.is_empty() {
            return None;
        }

        // Default: High for adaptive, Medium for budget-based
        let default = if supports_adaptive {
            ReasoningEffort::High
        } else {
            ReasoningEffort::Medium
        };

        Some(ReasoningEffortConfig { values, default })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // These tests verify that empty text blocks are filtered out to avoid
    // Anthropic API error: "text content blocks must be non-empty"

    #[test]
    fn test_convert_content_filters_empty_text() {
        // Empty text content should produce empty vec
        let content = LlmMessageContent::Text(String::new());
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert!(blocks.is_empty(), "Empty text should be filtered out");
    }

    #[test]
    fn test_convert_content_keeps_non_empty_text() {
        // Non-empty text should be kept
        let content = LlmMessageContent::Text("Hello, world!".to_string());
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert_eq!(blocks.len(), 1, "Non-empty text should be kept");
    }

    #[test]
    fn test_convert_content_filters_empty_text_in_parts() {
        // Empty text parts should be filtered out
        let content = LlmMessageContent::Parts(vec![
            LlmContentPart::Text {
                text: String::new(),
            },
            LlmContentPart::Text {
                text: "Hello".to_string(),
            },
            LlmContentPart::Text {
                text: String::new(),
            },
        ]);
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert_eq!(blocks.len(), 1, "Only non-empty text should be kept");
    }

    #[test]
    fn test_convert_content_keeps_images_with_empty_text() {
        // Images should be kept even when text parts are empty
        let content = LlmMessageContent::Parts(vec![
            LlmContentPart::Text {
                text: String::new(),
            },
            LlmContentPart::Image {
                url: "https://example.com/image.png".to_string(),
            },
        ]);
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert_eq!(blocks.len(), 1, "Image should be kept, empty text filtered");
    }

    #[test]
    fn test_convert_content_all_empty_produces_empty_vec() {
        // All empty content parts should produce empty vec
        let content = LlmMessageContent::Parts(vec![
            LlmContentPart::Text {
                text: String::new(),
            },
            LlmContentPart::Text {
                text: String::new(),
            },
        ]);
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert!(blocks.is_empty(), "All empty text should produce empty vec");
    }

    #[test]
    fn test_convert_messages_assistant_with_empty_text_and_tool_calls() {
        // Assistant message with empty text but tool calls should work
        // This is the specific case that caused the bug
        let messages = vec![LlmMessage {
            role: LlmMessageRole::Assistant,
            content: LlmMessageContent::Text(String::new()),
            tool_calls: Some(vec![everruns_core::tool_types::ToolCall {
                id: "call_123".to_string(),
                name: "get_weather".to_string(),
                arguments: serde_json::json!({"city": "London"}),
            }]),
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        }];

        let (_, converted) = AnthropicLlmDriver::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        // Content should have tool_use block but no empty text block
        assert_eq!(
            converted[0].content.len(),
            1,
            "Should only have tool_use block"
        );
    }

    #[test]
    fn test_convert_content_whitespace_is_kept() {
        // Whitespace-only text is kept (not empty after is_empty() check)
        let content = LlmMessageContent::Text("   ".to_string());
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert_eq!(blocks.len(), 1, "Whitespace-only text is kept");
    }

    #[test]
    fn test_convert_content_base64_image() {
        // Base64 data URL should be parsed correctly
        let content = LlmMessageContent::Parts(vec![LlmContentPart::Image {
            url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
        }]);
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert_eq!(blocks.len(), 1, "Base64 image should be converted");
        match &blocks[0] {
            AnthropicContentBlock::Image { source } => match source {
                AnthropicImageSource::Base64 { media_type, .. } => {
                    assert_eq!(media_type, "image/png");
                }
                _ => panic!("Expected Base64 source"),
            },
            _ => panic!("Expected Image block"),
        }
    }

    #[test]
    fn test_convert_content_http_image() {
        // HTTP URL image should work
        let content = LlmMessageContent::Parts(vec![LlmContentPart::Image {
            url: "https://example.com/photo.jpg".to_string(),
        }]);
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert_eq!(blocks.len(), 1, "HTTP image should be converted");
        match &blocks[0] {
            AnthropicContentBlock::Image { source } => match source {
                AnthropicImageSource::Url { url } => {
                    assert_eq!(url, "https://example.com/photo.jpg");
                }
                _ => panic!("Expected Url source"),
            },
            _ => panic!("Expected Image block"),
        }
    }

    #[test]
    fn test_convert_content_audio_fallback() {
        // Audio should fallback to text note (Anthropic doesn't support audio)
        let content = LlmMessageContent::Parts(vec![LlmContentPart::Audio {
            url: "data:audio/wav;base64,AAAA".to_string(),
        }]);
        let blocks = AnthropicLlmDriver::convert_content(&content);
        assert_eq!(blocks.len(), 1, "Audio should fallback to text note");
        match &blocks[0] {
            AnthropicContentBlock::Text { text } => {
                assert!(text.contains("not supported"));
            }
            _ => panic!("Expected Text block for audio fallback"),
        }
    }

    #[test]
    fn test_convert_messages_system_prompt() {
        // System message should be extracted as system prompt
        let messages = vec![
            LlmMessage {
                role: LlmMessageRole::System,
                content: LlmMessageContent::Text("You are helpful".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
            LlmMessage {
                role: LlmMessageRole::User,
                content: LlmMessageContent::Text("Hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
                phase: None,
                thinking: None,
                thinking_signature: None,
            },
        ];

        let (system, converted) = AnthropicLlmDriver::convert_messages(&messages);

        assert_eq!(system, Some("You are helpful".to_string()));
        assert_eq!(converted.len(), 1); // Only user message
    }

    #[test]
    fn test_convert_messages_tool_result() {
        // Tool result should be converted to user message with tool_result block
        let messages = vec![LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("{\"temp\": 20}".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_123".to_string()),
            phase: None,
            thinking: None,
            thinking_signature: None,
        }];

        let (_, converted) = AnthropicLlmDriver::convert_messages(&messages);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
        assert_eq!(converted[0].content.len(), 1);
        match &converted[0].content[0] {
            AnthropicContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "call_123");
                match content {
                    AnthropicToolResultContent::Text(text) => {
                        assert_eq!(text, "{\"temp\": 20}");
                    }
                    _ => panic!("Expected text content in tool result"),
                }
            }
            _ => panic!("Expected ToolResult block"),
        }
    }

    #[test]
    fn test_tool_result_with_images_conversion() {
        // Tool result with text + image content
        let msg = LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Parts(vec![
                LlmContentPart::Text {
                    text: "{\"status\": \"ok\"}".to_string(),
                },
                LlmContentPart::Image {
                    url: "data:image/png;base64,AAAA".to_string(),
                },
            ]),
            tool_calls: None,
            tool_call_id: Some("call_img".to_string()),
            phase: None,
            thinking: None,
            thinking_signature: None,
        };

        let (_, converted) = AnthropicLlmDriver::convert_messages(&[msg]);

        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
        assert_eq!(converted[0].content.len(), 1);

        match &converted[0].content[0] {
            AnthropicContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "call_img");
                match content {
                    AnthropicToolResultContent::Blocks(blocks) => {
                        assert_eq!(blocks.len(), 2);
                        match &blocks[0] {
                            AnthropicToolResultBlock::Text { text } => {
                                assert_eq!(text, "{\"status\": \"ok\"}");
                            }
                            _ => panic!("Expected text block"),
                        }
                        match &blocks[1] {
                            AnthropicToolResultBlock::Image { source } => match source {
                                AnthropicImageSource::Base64 { media_type, data } => {
                                    assert_eq!(media_type, "image/png");
                                    assert_eq!(data, "AAAA");
                                }
                                _ => panic!("Expected base64 image source"),
                            },
                            _ => panic!("Expected image block"),
                        }
                    }
                    _ => panic!("Expected Blocks content for multimodal tool result"),
                }
            }
            _ => panic!("Expected ToolResult block"),
        }
    }

    #[test]
    fn test_tool_result_text_only_stays_simple() {
        // Tool result with text-only content should use simple Text form
        let msg = LlmMessage {
            role: LlmMessageRole::Tool,
            content: LlmMessageContent::Text("result text".to_string()),
            tool_calls: None,
            tool_call_id: Some("call_txt".to_string()),
            phase: None,
            thinking: None,
            thinking_signature: None,
        };

        let (_, converted) = AnthropicLlmDriver::convert_messages(&[msg]);

        match &converted[0].content[0] {
            AnthropicContentBlock::ToolResult { content, .. } => match content {
                AnthropicToolResultContent::Text(text) => {
                    assert_eq!(text, "result text");
                }
                _ => panic!("Expected simple Text content for text-only tool result"),
            },
            _ => panic!("Expected ToolResult block"),
        }
    }

    // ========================================================================
    // Request-too-large detection tests
    // ========================================================================

    #[test]
    fn test_is_anthropic_request_too_large_413() {
        let error = r#"{"error":{"message":"Request too large"}}"#;
        assert!(is_anthropic_request_too_large(
            reqwest::StatusCode::PAYLOAD_TOO_LARGE,
            error
        ));
    }

    #[test]
    fn test_is_anthropic_request_too_large_prompt_too_long() {
        let error = r#"{"error":{"message":"prompt is too long: 250000 tokens > 200000 maximum"}}"#;
        assert!(is_anthropic_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_anthropic_request_too_large_request_size_exceeded() {
        let error = r#"{"error":{"message":"request size exceeded maximum"}}"#;
        assert!(is_anthropic_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_anthropic_request_too_large_too_many_tokens() {
        let error = r#"{"error":{"message":"too many tokens in request"}}"#;
        assert!(is_anthropic_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_anthropic_request_too_large_false_for_other_errors() {
        // Authentication error
        let error = r#"{"error":{"message":"Invalid API key"}}"#;
        assert!(!is_anthropic_request_too_large(
            reqwest::StatusCode::UNAUTHORIZED,
            error
        ));

        // Rate limit (not token-related)
        let error = r#"{"error":{"message":"Rate limit exceeded"}}"#;
        assert!(!is_anthropic_request_too_large(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            error
        ));

        // Internal server error
        let error = r#"{"error":{"message":"Internal server error"}}"#;
        assert!(!is_anthropic_request_too_large(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error
        ));
    }

    // ========================================================================
    // Model-not-found detection tests
    // ========================================================================

    #[test]
    fn test_is_anthropic_model_not_found_real_error() {
        // Real Anthropic 404 response for nonexistent model
        let error = r#"{"type":"error","error":{"type":"not_found_error","message":"model: claude-sonnet-4-6-20260217"},"request_id":"req_011CYJKSA1AvFr6TL2NYpYEa"}"#;
        assert!(is_anthropic_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_anthropic_model_not_found_generic_not_found() {
        let error = r#"{"error":{"message":"Model not found"}}"#;
        assert!(is_anthropic_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_anthropic_model_not_found_false_for_other_404() {
        // 404 without model-related message
        let error = r#"{"error":{"message":"Endpoint not found"}}"#;
        assert!(!is_anthropic_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_anthropic_model_not_found_false_for_non_404() {
        // not_found_error text but wrong status code
        let error = r#"{"type":"error","error":{"type":"not_found_error","message":"model: x"}}"#;
        assert!(!is_anthropic_model_not_found(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    // ========================================================================
    // Model ID normalization tests
    // ========================================================================

    #[test]
    fn test_normalize_anthropic_id_strips_date_suffix() {
        assert_eq!(
            normalize_anthropic_id("claude-opus-4-5-20251101"),
            "claude-opus-4-5"
        );
        assert_eq!(
            normalize_anthropic_id("claude-sonnet-4-6-20260217"),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn test_normalize_anthropic_id_preserves_base_ids() {
        assert_eq!(normalize_anthropic_id("claude-opus-4-5"), "claude-opus-4-5");
        assert_eq!(
            normalize_anthropic_id("claude-3-5-sonnet"),
            "claude-3-5-sonnet"
        );
    }

    // ========================================================================
    // Discovered profile construction tests
    // ========================================================================

    #[test]
    fn test_to_discovered_profile_basic() {
        let info = AnthropicModelInfo {
            id: "claude-sonnet-4-6-20260217".into(),
            display_name: "Claude Sonnet 4.6".into(),
            created_at: Some("2026-02-17T00:00:00Z".into()),
            max_input_tokens: Some(200_000),
            max_tokens: Some(64_000),
            capabilities: None,
        };

        let profile = info.to_discovered_profile();
        assert_eq!(profile.name, "Claude Sonnet 4.6");
        assert_eq!(profile.family, "claude-sonnet-4-6");
        assert!(profile.limits.is_some());
        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 200_000);
        assert_eq!(limits.output, 64_000);
        assert!(profile.cost.is_none()); // Never from API
    }

    #[test]
    fn test_to_discovered_profile_with_capabilities() {
        let info = AnthropicModelInfo {
            id: "claude-opus-4-6-20260205".into(),
            display_name: "Claude Opus 4.6".into(),
            created_at: None,
            max_input_tokens: Some(1_000_000),
            max_tokens: Some(128_000),
            capabilities: Some(AnthropicModelCapabilities {
                image_input: Some(CapabilitySupport { supported: true }),
                pdf_input: Some(CapabilitySupport { supported: true }),
                structured_outputs: Some(CapabilitySupport { supported: true }),
                thinking: Some(ThinkingCapability {
                    supported: true,
                    types: Some(ThinkingTypes {
                        enabled: Some(CapabilitySupport { supported: true }),
                        adaptive: Some(CapabilitySupport { supported: true }),
                    }),
                }),
                effort: Some(EffortCapability {
                    supported: true,
                    low: Some(CapabilitySupport { supported: true }),
                    medium: Some(CapabilitySupport { supported: true }),
                    high: Some(CapabilitySupport { supported: true }),
                    max: Some(CapabilitySupport { supported: true }),
                }),
                ..Default::default()
            }),
        };

        let profile = info.to_discovered_profile();
        assert!(profile.attachment); // image + PDF
        assert!(profile.reasoning);
        assert!(profile.structured_output);
        assert!(profile.reasoning_effort.is_some());
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.values.len(), 4); // low, medium, high, max
    }

    #[test]
    fn test_to_discovered_profile_pdf_only_is_attachment() {
        let info = AnthropicModelInfo {
            id: "claude-test".into(),
            display_name: "Test".into(),
            created_at: None,
            max_input_tokens: None,
            max_tokens: None,
            capabilities: Some(AnthropicModelCapabilities {
                image_input: Some(CapabilitySupport { supported: false }),
                pdf_input: Some(CapabilitySupport { supported: true }),
                ..Default::default()
            }),
        };

        let profile = info.to_discovered_profile();
        assert!(profile.attachment, "PDF-only should still be attachment");
    }

    #[test]
    fn test_default_max_tokens_from_known_model() {
        // Known Anthropic models should resolve max_tokens from profile
        let profile = everruns_core::get_model_profile(
            &everruns_core::LlmProviderType::Anthropic,
            "claude-sonnet-4-5-20250514",
        );
        assert!(profile.is_some(), "claude-sonnet-4-5 should have a profile");
        let limits = profile.unwrap().limits.expect("profile should have limits");
        assert!(limits.output > 0, "output limit should be positive");
        // Sonnet 4.5 should have a much higher limit than the old 4096 default
        assert!(
            limits.output > 4096,
            "model output limit ({}) should exceed old hardcoded 4096",
            limits.output
        );
    }

    #[test]
    fn test_default_max_tokens_unknown_model_falls_back() {
        // Unknown model should return None (triggering the 16384 fallback)
        let profile = everruns_core::get_model_profile(
            &everruns_core::LlmProviderType::Anthropic,
            "nonexistent-model-xyz",
        );
        assert!(profile.is_none(), "unknown model should not have a profile");
    }
}
