// OpenAI Protocol LLM Driver
//
// Base implementation of the OpenAI chat completion protocol.
// This driver can be used with any OpenAI-compatible API endpoint.
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff, respecting x-ratelimit-reset-* and retry-after headers.
// Retry metadata is included in the response for observability.
//
// This is the base protocol implementation used in examples.
// For production use with OpenAI-specific features, use OpenAILlmDriver from everruns-openai.
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// llm.generation events are emitted by ReasonAtom, and OtelEventListener
// creates the appropriate gen-ai spans. No direct tracing in drivers.

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::{Client, RequestBuilder, Url};
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
use crate::tool_types::{ToolCall, ToolDefinition};

const DEFAULT_API_URL: &str = "https://api.openai.com/v1/chat/completions";

pub(crate) fn apply_openai_api_auth(
    request: RequestBuilder,
    api_url: &str,
    api_key: &str,
) -> RequestBuilder {
    if is_azure_openai_api_url(api_url) {
        request.header("api-key", api_key)
    } else {
        request.header("Authorization", format!("Bearer {}", api_key))
    }
}

pub fn is_azure_openai_api_url(api_url: &str) -> bool {
    Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| {
            host.ends_with(".openai.azure.com") || host.ends_with(".services.ai.azure.com")
        })
}

/// Whether `api_url` points at OpenAI's hosted API (`api.openai.com`).
///
/// Host-based (not prefix-based) so it tolerates ports and trailing paths.
pub fn is_openai_api_url(api_url: &str) -> bool {
    Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(|host| host.to_ascii_lowercase()))
        .is_some_and(|host| host == "api.openai.com")
}

/// OpenAI Protocol LLM Driver
///
/// Base implementation of `LlmDriver` for OpenAI-compatible APIs.
/// Supports streaming responses and tool calls.
///
/// Rate limit handling: On 429 errors, automatically retries with exponential
/// backoff, respecting `x-ratelimit-reset-*` and `retry-after` headers.
///
/// This is the base protocol driver used in examples and for OpenAI-compatible endpoints.
/// For production use with OpenAI, consider using `OpenAILlmDriver` from the `everruns-openai` crate.
///
/// # Example
///
/// ```ignore
/// use everruns_core::OpenAIProtocolLlmDriver;
///
/// let driver = OpenAIProtocolLlmDriver::from_env()?;
/// // or
/// let driver = OpenAIProtocolLlmDriver::new("your-api-key");
/// // or with custom endpoint
/// let driver = OpenAIProtocolLlmDriver::with_base_url("your-api-key", "https://api.example.com/v1/chat/completions");
/// // or with custom retry config
/// let driver = OpenAIProtocolLlmDriver::new("your-api-key")
///     .with_retry_config(LlmRetryConfig::aggressive());
/// ```
#[derive(Clone)]
pub struct OpenAIProtocolLlmDriver {
    client: Client,
    api_key: String,
    api_url: String,
    /// Retry configuration for rate limit errors
    retry_config: LlmRetryConfig,
}

impl OpenAIProtocolLlmDriver {
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

    /// Create a new driver with a custom API URL (for OpenAI-compatible APIs)
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
            LlmMessageRole::System => "system",
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        }
    }

    fn convert_message(msg: &LlmMessage) -> OpenAiMessage {
        let content = match &msg.content {
            LlmMessageContent::Text(text) => OpenAiContent::Text(text.clone()),
            LlmMessageContent::Parts(parts) => {
                let openai_parts: Vec<OpenAiContentPart> = parts
                    .iter()
                    .map(|part| match part {
                        LlmContentPart::Text { text } => OpenAiContentPart::Text {
                            r#type: "text".to_string(),
                            text: text.clone(),
                        },
                        LlmContentPart::Image { url } => OpenAiContentPart::ImageUrl {
                            r#type: "image_url".to_string(),
                            image_url: OpenAiImageUrl { url: url.clone() },
                        },
                        LlmContentPart::Audio { url } => OpenAiContentPart::InputAudio {
                            r#type: "input_audio".to_string(),
                            input_audio: OpenAiInputAudio {
                                data: url.clone(),
                                format: "wav".to_string(),
                            },
                        },
                    })
                    .collect();
                OpenAiContent::Parts(openai_parts)
            }
        };

        // OpenAI only accepts tool_calls on assistant messages
        let tool_calls = if msg.role == LlmMessageRole::Assistant {
            msg.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .map(|tc| OpenAiToolCall {
                        id: tc.id.clone(),
                        r#type: "function".to_string(),
                        function: OpenAiFunctionCall {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                        },
                    })
                    .collect()
            })
        } else {
            None
        };

        OpenAiMessage {
            role: Self::convert_role(&msg.role).to_string(),
            content: Some(content),
            tool_calls,
            tool_call_id: msg.tool_call_id.clone(),
        }
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<OpenAiTool> {
        tools
            .iter()
            .map(|tool| OpenAiTool {
                r#type: "function".to_string(),
                function: OpenAiFunction {
                    name: tool.name().to_string(),
                    description: tool.description().to_string(),
                    parameters: tool.parameters().clone(),
                },
            })
            .collect()
    }
}

#[async_trait]
impl LlmDriver for OpenAIProtocolLlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Note: OTel instrumentation is handled via event listeners.
        // ReasonAtom emits llm.generation events, and OtelEventListener
        // creates gen-ai spans from those events.
        let openai_messages: Vec<OpenAiMessage> =
            messages.iter().map(Self::convert_message).collect();

        let tools = if config.tools.is_empty() {
            None
        } else {
            Some(Self::convert_tools(&config.tools))
        };

        // Build metadata for request tracking
        let metadata = if config.metadata.is_empty() {
            None
        } else {
            Some(config.metadata.clone())
        };

        let request = OpenAiRequest {
            model: config.model.clone(),
            messages: openai_messages,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            stream: true,
            stream_options: Some(OpenAiStreamOptions {
                include_usage: true,
            }),
            tools,
            // Skip "none" — sending reasoning_effort to non-thinking models causes API errors
            reasoning_effort: config
                .reasoning_effort
                .as_ref()
                .filter(|e| !e.eq_ignore_ascii_case("none"))
                .cloned(),
            metadata,
        };

        // Retry loop for rate limit (429) and transient errors
        let mut retry_metadata = RetryMetadata::default();
        let mut last_error: Option<String> = None;

        let response = loop {
            let response = apply_openai_api_auth(
                self.client.post(&self.api_url),
                &self.api_url,
                &self.api_key,
            )
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

                // Don't retry if this is a request-too-large error (not transient)
                if is_openai_request_too_large(status, &error_text) {
                    return Err(AgentLoopError::request_too_large(format!(
                        "OpenAI API error ({}): {}",
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
                    "OpenAIProtocolDriver: rate limit or transient error, retrying"
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
            let error_msg = format!("OpenAI API error ({}): {}", status, error_text);

            // Check if this is a model-not-found error
            if is_openai_model_not_found(status, &error_text) {
                return Err(AgentLoopError::model_not_available(config.model.clone()));
            }

            // Check if this is a request-too-large error
            if is_openai_request_too_large(status, &error_text) {
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
                "OpenAIProtocolDriver: request succeeded after retries"
            );
        }

        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream.eventsource();

        let model = config.model.clone();
        let total_tokens = Arc::new(Mutex::new(0u32));
        let prompt_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        // OpenAI-compatible gateways (e.g. OpenRouter) report an authoritative
        // per-request cost in `usage.cost`; direct OpenAI leaves it absent.
        let provider_cost_usd = Arc::new(Mutex::new(Option::<f64>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));
        // Share retry metadata with stream closure (only set if retries occurred)
        let shared_retry_metadata = if retry_metadata.had_retries() {
            Some(Arc::new(retry_metadata))
        } else {
            None
        };

        // Each SSE event maps to zero-or-more stream events (the [DONE] marker can
        // emit a flushed ToolCalls plus Done), so the closure yields a Vec that is
        // flattened back into the stream.
        let converted_stream: LlmResponseStream = Box::pin(
            event_stream
                .then(move |result| {
                    let model = model.clone();
                    let total_tokens = Arc::clone(&total_tokens);
                    let prompt_tokens = Arc::clone(&prompt_tokens);
                    let cache_read_tokens = Arc::clone(&cache_read_tokens);
                    let provider_cost_usd = Arc::clone(&provider_cost_usd);
                    let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
                    let finish_reason = Arc::clone(&finish_reason);
                    let retry_metadata_for_done = shared_retry_metadata.clone();

                    async move {
                        let event = match result {
                            Ok(event) => event,
                            Err(e) => {
                                return vec![Ok(LlmStreamEvent::Error(format!(
                                    "Stream error: {}",
                                    e
                                )))];
                            }
                        };

                        if event.data == "[DONE]" {
                            let output_tokens = *total_tokens.lock().unwrap();
                            let input_tokens = *prompt_tokens.lock().unwrap();
                            let cached = *cache_read_tokens.lock().unwrap();
                            let cost = *provider_cost_usd.lock().unwrap();
                            let mut reason = finish_reason.lock().unwrap().clone();

                            let mut events = Vec::new();

                            // Defense in depth (EVE-522): flush any tool calls that
                            // were accumulated but never emitted before Done, so they
                            // are never silently dropped. The normal path drains the
                            // accumulator at the finish chunk, so this only fires as a
                            // fallback — e.g. a provider that ends the stream with
                            // [DONE] without a tool_calls finish chunk reaching the
                            // handler. When it fires, reflect the tool-call completion
                            // in the reported finish_reason.
                            {
                                let mut acc = accumulated_tool_calls.lock().unwrap();
                                if let Some(event) = take_pending_tool_calls(&mut acc) {
                                    events.push(Ok(event));
                                    reason.get_or_insert_with(|| "tool_calls".to_string());
                                }
                            }

                            events.push(Ok(LlmStreamEvent::Done(Box::new(
                                LlmCompletionMetadata {
                                    total_tokens: Some(input_tokens + output_tokens),
                                    prompt_tokens: Some(input_tokens),
                                    completion_tokens: Some(output_tokens),
                                    cache_read_tokens: cached,
                                    cache_creation_tokens: None,
                                    provider_cost_usd: cost,
                                    model: Some(model),
                                    finish_reason: reason.or_else(|| Some("stop".to_string())),
                                    retry_metadata: retry_metadata_for_done
                                        .map(|arc| (*arc).clone()),
                                    response_id: None,
                                    phase: None,
                                },
                            ))));

                            return events;
                        }

                        match serde_json::from_str::<OpenAiStreamChunk>(&event.data) {
                            Ok(chunk) => {
                                // Capture usage from chunk if available
                                if let Some(usage) = &chunk.usage {
                                    if let Some(pt) = usage.prompt_tokens {
                                        *prompt_tokens.lock().unwrap() = pt;
                                    }
                                    if let Some(ct) = usage.completion_tokens {
                                        *total_tokens.lock().unwrap() = ct;
                                    }
                                    // Capture cached tokens from prompt_tokens_details
                                    if let Some(details) = &usage.prompt_tokens_details
                                        && details.cached_tokens.is_some()
                                    {
                                        *cache_read_tokens.lock().unwrap() = details.cached_tokens;
                                    }
                                    // Authoritative cost from OpenAI-compatible gateways
                                    // (e.g. OpenRouter `usage.cost`, in USD credits).
                                    if usage.cost.is_some() {
                                        *provider_cost_usd.lock().unwrap() = usage.cost;
                                    }
                                }

                                if let Some(choice) = chunk.choices.first() {
                                    let mut tt = total_tokens.lock().unwrap();
                                    let mut acc = accumulated_tool_calls.lock().unwrap();
                                    let mut fr = finish_reason.lock().unwrap();
                                    let stream_event =
                                        process_stream_choice(choice, &mut tt, &mut acc, &mut fr);
                                    return vec![Ok(stream_event)];
                                }
                                vec![Ok(LlmStreamEvent::TextDelta(String::new()))]
                            }
                            Err(e) => vec![Ok(LlmStreamEvent::Error(format!(
                                "Failed to parse chunk: {}",
                                e
                            )))],
                        }
                    }
                })
                .flat_map(futures::stream::iter),
        );

        Ok(converted_stream)
    }
}

impl std::fmt::Debug for OpenAIProtocolLlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIProtocolLlmDriver")
            .field("api_url", &self.api_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Error Detection Helpers
// ============================================================================

/// Check if the error indicates the model was not found.
///
/// OpenAI returns 404 or 400 with `"model_not_found"` code or `"does not exist"` message.
/// Also handles Gemini/OpenAI-compatible endpoints with similar patterns.
pub fn is_openai_model_not_found(status: reqwest::StatusCode, error_text: &str) -> bool {
    let error_lower = error_text.to_lowercase();

    // OpenAI can return either 404 or 400 for nonexistent models
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::BAD_REQUEST {
        // OpenAI: {"error":{"code":"model_not_found","message":"The model 'x' does not exist"}}
        if error_lower.contains("model_not_found") {
            return true;
        }
    }

    // 404 with generic model-not-found patterns
    if status == reqwest::StatusCode::NOT_FOUND {
        if error_lower.contains("does not exist") {
            return true;
        }
        if error_lower.contains("model") && error_lower.contains("not found") {
            return true;
        }
    }

    false
}

/// Check if an OpenAI API error indicates the request is too large.
///
/// Detects:
/// - 429 with "Request too large" or token limit messages
/// - 400 with "context_length_exceeded" code
/// - Any message about maximum context length being exceeded
pub fn is_openai_request_too_large(status: reqwest::StatusCode, error_text: &str) -> bool {
    let error_lower = error_text.to_lowercase();

    // HTTP 429 with token-related errors
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // "Request too large for gpt-4" pattern
        if error_lower.contains("request too large") {
            return true;
        }
        // Token limit errors: "tokens per min (TPM): Limit X, Requested Y"
        if error_lower.contains("tokens") && error_lower.contains("limit") {
            return true;
        }
    }

    // HTTP 400 with context length errors
    if status == reqwest::StatusCode::BAD_REQUEST {
        // "context_length_exceeded" error code
        if error_lower.contains("context_length_exceeded") {
            return true;
        }
        // "maximum context length" message
        if error_lower.contains("maximum context length") {
            return true;
        }
    }

    // Generic patterns that could appear with various status codes
    if error_lower.contains("tokens must be reduced")
        || error_lower.contains("reduce the length")
        || error_lower.contains("input is too long")
    {
        return true;
    }

    false
}

// ============================================================================
// OpenAI API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    /// Request usage info in streaming response (required for token counts)
    #[serde(skip_serializing_if = "Option::is_none")]
    stream_options: Option<OpenAiStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<String>,
    /// Metadata for tracking API usage (up to 16 key-value pairs).
    /// Useful for correlating requests with session_id, agent_id, org_id, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
struct OpenAiStreamOptions {
    include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum OpenAiContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum OpenAiContentPart {
    Text {
        r#type: String,
        text: String,
    },
    ImageUrl {
        r#type: String,
        image_url: OpenAiImageUrl,
    },
    InputAudio {
        r#type: String,
        input_audio: OpenAiInputAudio,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiImageUrl {
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiInputAudio {
    data: String,
    format: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OpenAiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiTool {
    r#type: String,
    function: OpenAiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    r#type: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // id and model are deserialized but used by event listeners, not directly
struct OpenAiStreamChunk {
    /// Unique identifier for this completion
    #[serde(default)]
    id: Option<String>,
    /// Model used for completion (may differ from requested)
    #[serde(default)]
    model: Option<String>,
    choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    /// Detailed breakdown of prompt tokens (includes cached tokens)
    #[serde(default)]
    prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
    /// Authoritative per-request cost in USD credits, returned by
    /// OpenAI-compatible gateways such as OpenRouter. Absent for direct OpenAI.
    #[serde(default)]
    cost: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenAiPromptTokensDetails {
    /// Number of tokens retrieved from cache
    #[serde(default)]
    cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamToolCall {
    index: u32,
    id: Option<String>,
    function: Option<OpenAiStreamFunction>,
}

#[derive(Debug, Deserialize)]
struct OpenAiStreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

/// Parses each accumulated tool call's argument string (assembled from streamed
/// fragments) into JSON, falling back to an empty object on parse failure.
fn finalize_tool_calls(tool_calls: Vec<ToolCall>) -> Vec<ToolCall> {
    tool_calls
        .into_iter()
        .map(|mut tc| {
            if let Some(args_str) = tc.arguments.as_str() {
                tc.arguments = serde_json::from_str(args_str).unwrap_or(json!({}));
            }
            tc
        })
        .collect()
}

/// Drains tool calls that were accumulated but not yet emitted, returning a
/// final `ToolCalls` event for the `[DONE]` handler. Returns `None` when nothing
/// is pending (the common case, since the finish chunk normally drains them).
fn take_pending_tool_calls(accumulated_tool_calls: &mut Vec<ToolCall>) -> Option<LlmStreamEvent> {
    if accumulated_tool_calls.is_empty() {
        return None;
    }
    let calls = std::mem::take(accumulated_tool_calls);
    Some(LlmStreamEvent::ToolCalls(finalize_tool_calls(calls)))
}

/// Processes a single chat-completion stream choice, updating the running
/// accumulators and returning the event to emit.
///
/// EVE-522: some OpenAI-compatible providers (OpenRouter/DeepInfra) send an
/// empty `content: ""` delta in the *same* chunk that carries
/// `finish_reason: "tool_calls"`. The content branch must therefore ignore
/// empty content, otherwise it short-circuits before the finish handler and the
/// accumulated tool calls are silently dropped. Emitting drains the accumulator
/// so a repeated finish chunk does not re-emit the same calls.
fn process_stream_choice(
    choice: &OpenAiStreamChoice,
    total_tokens: &mut u32,
    accumulated_tool_calls: &mut Vec<ToolCall>,
    finish_reason: &mut Option<String>,
) -> LlmStreamEvent {
    // Accumulate streamed tool-call fragments.
    if let Some(tool_calls) = &choice.delta.tool_calls {
        for tc in tool_calls {
            let idx = tc.index as usize;
            while accumulated_tool_calls.len() <= idx {
                accumulated_tool_calls.push(ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: json!(""),
                });
            }

            if let Some(id) = &tc.id {
                accumulated_tool_calls[idx].id = id.clone();
            }
            if let Some(function) = &tc.function {
                if let Some(name) = &function.name {
                    accumulated_tool_calls[idx].name = name.clone();
                }
                if let Some(args) = &function.arguments {
                    let current = accumulated_tool_calls[idx].arguments.as_str().unwrap_or("");
                    let combined = format!("{}{}", current, args);
                    accumulated_tool_calls[idx].arguments = json!(combined);
                }
            }
        }
        return LlmStreamEvent::TextDelta(String::new());
    }

    // Content delta. Guard on non-empty: an empty-content delta that rides along
    // with finish_reason must not short-circuit the finish handler below.
    if let Some(content) = &choice.delta.content
        && !content.is_empty()
    {
        *total_tokens += 1;
        return LlmStreamEvent::TextDelta(content.clone());
    }

    // Finish reason. Store it for the [DONE] handler; for tool_calls, emit the
    // accumulated calls immediately so the agent can start working. Draining the
    // accumulator prevents a second finish chunk from re-emitting the calls.
    if let Some(fr) = &choice.finish_reason {
        *finish_reason = Some(fr.clone());

        if fr == "tool_calls" && !accumulated_tool_calls.is_empty() {
            let calls = std::mem::take(accumulated_tool_calls);
            return LlmStreamEvent::ToolCalls(finalize_tool_calls(calls));
        }
    }

    LlmStreamEvent::TextDelta(String::new())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_with_api_key() {
        let driver = OpenAIProtocolLlmDriver::new("test-key");
        assert!(format!("{:?}", driver).contains("OpenAIProtocolLlmDriver"));
    }

    #[test]
    fn test_driver_with_base_url() {
        let driver = OpenAIProtocolLlmDriver::with_base_url(
            "test-key",
            "https://custom.api.com/v1/completions",
        );
        assert!(format!("{:?}", driver).contains("OpenAIProtocolLlmDriver"));
        assert_eq!(driver.api_url(), "https://custom.api.com/v1/completions");
    }

    #[test]
    fn test_is_azure_openai_api_url() {
        assert!(is_azure_openai_api_url(
            "https://example.openai.azure.com/openai/v1/chat/completions"
        ));
        assert!(is_azure_openai_api_url(
            "https://example.services.ai.azure.com/openai/v1/responses"
        ));
        assert!(!is_azure_openai_api_url(
            "https://api.openai.com/v1/chat/completions"
        ));
    }

    #[test]
    fn test_request_includes_stream_options_for_usage() {
        // OpenAI streaming API requires stream_options.include_usage=true
        // to return token usage in the response
        let request = OpenAiRequest {
            model: "gpt-4o".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(OpenAiContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: true,
            stream_options: Some(OpenAiStreamOptions {
                include_usage: true,
            }),
            tools: None,
            reasoning_effort: None,
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["stream"], true);
        assert_eq!(json["stream_options"]["include_usage"], true);
    }

    #[test]
    fn test_request_includes_metadata() {
        // Metadata should be included when provided
        let mut metadata = std::collections::HashMap::new();
        metadata.insert("session_id".to_string(), "session_abc123".to_string());
        metadata.insert("agent_id".to_string(), "agent_xyz789".to_string());

        let request = OpenAiRequest {
            model: "gpt-4o".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(OpenAiContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: true,
            stream_options: None,
            tools: None,
            reasoning_effort: None,
            metadata: Some(metadata),
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["metadata"]["session_id"], "session_abc123");
        assert_eq!(json["metadata"]["agent_id"], "agent_xyz789");
    }

    #[test]
    fn test_usage_chunk_parsing() {
        // OpenAI sends usage in a separate chunk after finish_reason
        // This test verifies we can parse it correctly
        let usage_chunk = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion.chunk",
            "created": 1234567890,
            "model": "gpt-4o",
            "choices": [],
            "usage": {
                "prompt_tokens": 150,
                "completion_tokens": 42,
                "total_tokens": 192
            }
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(usage_chunk).unwrap();
        assert!(chunk.usage.is_some());
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(150));
        assert_eq!(usage.completion_tokens, Some(42));
    }

    #[test]
    fn test_usage_chunk_with_cached_tokens() {
        // OpenAI includes cached_tokens in prompt_tokens_details
        let usage_chunk = r#"{
            "id": "chatcmpl-123",
            "choices": [],
            "usage": {
                "prompt_tokens": 150,
                "completion_tokens": 42,
                "prompt_tokens_details": {
                    "cached_tokens": 100
                }
            }
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(usage_chunk).unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, Some(150));
        assert_eq!(usage.completion_tokens, Some(42));
        assert!(usage.prompt_tokens_details.is_some());
        assert_eq!(
            usage.prompt_tokens_details.unwrap().cached_tokens,
            Some(100)
        );
    }

    #[test]
    fn test_usage_chunk_with_openrouter_cost() {
        // OpenAI-compatible gateways like OpenRouter add `usage.cost` (USD credits).
        let usage_chunk = r#"{
            "id": "gen-123",
            "choices": [],
            "usage": {
                "prompt_tokens": 194,
                "completion_tokens": 2,
                "total_tokens": 196,
                "cost": 0.00095
            }
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(usage_chunk).unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.cost, Some(0.00095));
    }

    #[test]
    fn test_usage_chunk_without_cost_defaults_none() {
        // Direct OpenAI omits `cost`; it must deserialize to None, not error.
        let usage_chunk = r#"{
            "id": "chatcmpl-123",
            "choices": [],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 }
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(usage_chunk).unwrap();
        assert_eq!(chunk.usage.unwrap().cost, None);
    }

    #[test]
    fn test_finish_reason_chunk_parsing() {
        // Finish reason comes in a chunk BEFORE the usage chunk
        let finish_chunk = r#"{
            "id": "chatcmpl-123",
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop"
            }]
        }"#;

        let chunk: OpenAiStreamChunk = serde_json::from_str(finish_chunk).unwrap();
        assert!(chunk.usage.is_none()); // No usage in finish_reason chunk
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].finish_reason, Some("stop".to_string()));
    }

    // ========================================================================
    // Request-too-large detection tests
    // ========================================================================

    #[test]
    fn test_is_openai_request_too_large_429_request_too_large() {
        let error = r#"{"error":{"message":"Request too large for gpt-4o in organization org-xxx on tokens per min (TPM): Limit 500000, Requested 538772."}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_429_token_limit() {
        let error =
            r#"{"error":{"message":"tokens per min (TPM): Limit 500000, Requested 600000"}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_400_context_length() {
        let error = r#"{"error":{"code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens."}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_400_max_context() {
        let error =
            r#"{"error":{"message":"This model's maximum context length is 128000 tokens"}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_tokens_must_be_reduced() {
        let error = r#"{"error":{"message":"The input or output tokens must be reduced"}}"#;
        assert!(is_openai_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_request_too_large_false_for_other_errors() {
        // Regular rate limit (not token-related)
        let error = r#"{"error":{"message":"Rate limit exceeded: too many requests per minute"}}"#;
        assert!(!is_openai_request_too_large(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            error
        ));

        // Internal server error
        let error = r#"{"error":{"message":"Internal server error"}}"#;
        assert!(!is_openai_request_too_large(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            error
        ));

        // Generic 400 error
        let error = r#"{"error":{"message":"Invalid request"}}"#;
        assert!(!is_openai_request_too_large(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    // ========================================================================
    // Model-not-found detection tests
    // ========================================================================

    #[test]
    fn test_is_openai_model_not_found_real_error() {
        // Real OpenAI 404 response for nonexistent model
        let error = r#"{"error":{"code":"model_not_found","message":"The model 'gpt-99' does not exist or you do not have access to it.","type":"invalid_request_error","param":null}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_does_not_exist() {
        let error = r#"{"error":{"message":"The model 'fake-model' does not exist"}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_generic_not_found() {
        let error = r#"{"error":{"message":"Model not found"}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_400_with_model_not_found_code() {
        // OpenAI Responses API returns 400 (not 404) for nonexistent models
        let error = r#"{"error":{"code":"model_not_found","message":"The requested model 'gpt-99' does not exist.","type":"invalid_request_error","param":"model"}}"#;
        assert!(is_openai_model_not_found(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_false_for_non_model_error() {
        // 400 without model_not_found code should not match
        let error = r#"{"error":{"code":"invalid_request","message":"Some other error"}}"#;
        assert!(!is_openai_model_not_found(
            reqwest::StatusCode::BAD_REQUEST,
            error
        ));
    }

    #[test]
    fn test_is_openai_model_not_found_false_for_other_404() {
        // 404 without model-related message
        let error = r#"{"error":{"message":"Endpoint not found"}}"#;
        assert!(!is_openai_model_not_found(
            reqwest::StatusCode::NOT_FOUND,
            error
        ));
    }

    // ========================================================================
    // Reasoning effort guard tests
    // ========================================================================

    #[test]
    fn test_reasoning_effort_none_is_omitted() {
        // When reasoning_effort is "none", it should be filtered out
        // to avoid "Unrecognized request argument" errors on non-thinking models
        let request = OpenAiRequest {
            model: "gpt-4o-mini".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(OpenAiContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: true,
            stream_options: None,
            tools: None,
            reasoning_effort: Some("none".to_string())
                .as_ref()
                .filter(|e| !e.eq_ignore_ascii_case("none"))
                .cloned(),
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert!(
            json.get("reasoning_effort").is_none(),
            "reasoning_effort should be omitted when effort is 'none'"
        );
    }

    #[test]
    fn test_reasoning_effort_high_is_included() {
        let request = OpenAiRequest {
            model: "o3-mini".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: Some(OpenAiContent::Text("Hello".to_string())),
                tool_calls: None,
                tool_call_id: None,
            }],
            temperature: None,
            max_tokens: None,
            stream: true,
            stream_options: None,
            tools: None,
            reasoning_effort: Some("high".to_string())
                .as_ref()
                .filter(|e| !e.eq_ignore_ascii_case("none"))
                .cloned(),
            metadata: None,
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["reasoning_effort"], "high");
    }

    // ------------------------------------------------------------------
    // EVE-522: streaming chunk handling (process_stream_choice)
    // ------------------------------------------------------------------

    fn choice(json_str: &str) -> OpenAiStreamChoice {
        serde_json::from_str(json_str).unwrap()
    }

    /// EVE-522 regression: providers such as OpenRouter/DeepInfra send an empty
    /// `content: ""` in the same chunk that carries `finish_reason: "tool_calls"`.
    /// The accumulated tool calls must still be emitted exactly once.
    #[test]
    fn test_empty_content_finish_chunk_still_emits_tool_calls() {
        let mut total_tokens = 0u32;
        let mut acc: Vec<ToolCall> = Vec::new();
        let mut finish_reason: Option<String> = None;

        // Chunk 2: tool_calls delta opens the call (id + name).
        let e = process_stream_choice(
            &choice(
                r#"{"delta":{"content":null,"tool_calls":[{"index":0,"id":"call_1","function":{"name":"read_file","arguments":""}}]},"finish_reason":null}"#,
            ),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        assert!(matches!(e, LlmStreamEvent::TextDelta(s) if s.is_empty()));

        // Chunk 3: tool_calls delta streams the arguments.
        let e = process_stream_choice(
            &choice(
                r#"{"delta":{"content":null,"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"Cargo.toml\"}"}}]},"finish_reason":null}"#,
            ),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        assert!(matches!(e, LlmStreamEvent::TextDelta(s) if s.is_empty()));

        // Chunk 4: content:"" alongside finish_reason:"tool_calls" — must NOT
        // short-circuit; emits the accumulated call with parsed JSON arguments.
        let e = process_stream_choice(
            &choice(r#"{"delta":{"content":""},"finish_reason":"tool_calls"}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        match e {
            LlmStreamEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].id, "call_1");
                assert_eq!(calls[0].name, "read_file");
                assert_eq!(calls[0].arguments, json!({"path": "Cargo.toml"}));
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
        assert_eq!(finish_reason.as_deref(), Some("tool_calls"));

        // Chunk 5: second finish chunk with content:"" — the accumulator was
        // drained, so the same call must not be emitted again.
        let e = process_stream_choice(
            &choice(r#"{"delta":{"content":""},"finish_reason":"tool_calls"}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        assert!(
            matches!(e, LlmStreamEvent::TextDelta(s) if s.is_empty()),
            "tool calls must only be emitted once"
        );
    }

    /// Non-empty content deltas are still emitted and counted as output tokens.
    #[test]
    fn test_non_empty_content_is_emitted() {
        let mut total_tokens = 0u32;
        let mut acc: Vec<ToolCall> = Vec::new();
        let mut finish_reason: Option<String> = None;

        let e = process_stream_choice(
            &choice(r#"{"delta":{"content":"hello"},"finish_reason":null}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        assert!(matches!(e, LlmStreamEvent::TextDelta(s) if s == "hello"));
        assert_eq!(total_tokens, 1);
    }

    /// OpenAI's native path sends `delta: {}` (no content key) in the finish
    /// chunk; the existing behavior of emitting tool calls there is preserved.
    #[test]
    fn test_finish_chunk_without_content_emits_tool_calls() {
        let mut total_tokens = 0u32;
        let mut acc: Vec<ToolCall> = Vec::new();
        let mut finish_reason: Option<String> = None;

        process_stream_choice(
            &choice(
                r#"{"delta":{"tool_calls":[{"index":0,"id":"call_9","function":{"name":"list_dir","arguments":"{}"}}]},"finish_reason":null}"#,
            ),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );

        let e = process_stream_choice(
            &choice(r#"{"delta":{},"finish_reason":"tool_calls"}"#),
            &mut total_tokens,
            &mut acc,
            &mut finish_reason,
        );
        match e {
            LlmStreamEvent::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "list_dir");
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
    }

    /// The [DONE] fallback flushes accumulated-but-unemitted tool calls and
    /// drains the accumulator; once drained it returns None.
    #[test]
    fn test_take_pending_tool_calls_flushes_then_drains() {
        let mut acc = vec![ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!(r#"{"path":"Cargo.toml"}"#),
        }];

        match take_pending_tool_calls(&mut acc) {
            Some(LlmStreamEvent::ToolCalls(calls)) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "read_file");
                assert_eq!(calls[0].arguments, json!({"path": "Cargo.toml"}));
            }
            other => panic!("expected ToolCalls, got {:?}", other),
        }
        assert!(acc.is_empty(), "accumulator must be drained after flush");
        assert!(take_pending_tool_calls(&mut acc).is_none());
    }

    #[test]
    fn test_finalize_tool_calls_parses_arguments() {
        let calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!(r#"{"path":"src/main.rs"}"#),
        }];
        let finalized = finalize_tool_calls(calls);
        assert_eq!(finalized[0].arguments, json!({"path": "src/main.rs"}));
    }
}
