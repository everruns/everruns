// Anthropic Claude LLM Driver
//
// Implementation of LlmDriver for Anthropic's Claude API.
// Uses the Messages API with streaming support.
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
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverRegistry, LlmCallConfig, LlmCompletionMetadata,
    LlmContentPart, LlmDriver, LlmMessage, LlmMessageContent, LlmMessageRole, LlmResponseStream,
    LlmStreamEvent, ProviderType,
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
/// ```
#[derive(Clone)]
pub struct AnthropicLlmDriver {
    client: Client,
    api_key: String,
    api_url: String,
    /// Whether using a custom base URL (not Anthropic's API)
    uses_custom_url: bool,
}

impl AnthropicLlmDriver {
    /// Create a new provider with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_url: DEFAULT_API_URL.to_string(),
            uses_custom_url: false,
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
        }
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
                        // Parse data URL or use as-is
                        if url.starts_with("data:") {
                            // Parse data URL: data:image/jpeg;base64,/9j/4AAQ...
                            let parts: Vec<&str> = url.splitn(2, ',').collect();
                            let (media_type, data) = if parts.len() == 2 {
                                let type_part = parts[0]
                                    .trim_start_matches("data:")
                                    .trim_end_matches(";base64");
                                (type_part.to_string(), parts[1].to_string())
                            } else {
                                ("image/jpeg".to_string(), url.clone())
                            };
                            Some(AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Base64 { media_type, data },
                            })
                        } else {
                            // HTTP URL
                            Some(AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Url { url: url.clone() },
                            })
                        }
                    }
                    LlmContentPart::Audio { .. } => {
                        // Anthropic doesn't support audio input yet, convert to text note
                        Some(AnthropicContentBlock::Text {
                            text: "[Audio content not supported]".to_string(),
                        })
                    }
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
                    // Tool results in Anthropic are user messages with tool_result content blocks
                    if let Some(tool_call_id) = &msg.tool_call_id {
                        converted.push(AnthropicMessage {
                            role: "user".to_string(),
                            content: vec![AnthropicContentBlock::ToolResult {
                                tool_use_id: tool_call_id.clone(),
                                content: msg.content.to_text(),
                                is_error: None,
                            }],
                        });
                    }
                }
                LlmMessageRole::Assistant => {
                    let mut content = Self::convert_content(&msg.content);

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

        // Build metadata for request tracking
        // Use session_id as the user_id for Anthropic's abuse detection
        let metadata = config
            .metadata
            .get("session_id")
            .map(|session_id| AnthropicMetadata {
                user_id: Some(session_id.clone()),
            });

        let mut request = AnthropicRequest {
            model: config.model.clone(),
            messages: anthropic_messages,
            max_tokens: config.max_tokens.unwrap_or(4096),
            temperature: config.temperature,
            system: system_prompt,
            stream: true,
            tools,
            thinking,
            metadata,
        };

        // Ensure max_tokens is set (required by Anthropic)
        if request.max_tokens == 0 {
            request.max_tokens = 4096;
        }

        let response = self
            .client
            .post(&self.api_url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            let error_msg = format!("Anthropic API error ({}): {}", status, error_text);

            // Check if this is a request-too-large error
            if is_anthropic_request_too_large(status, &error_text) {
                return Err(AgentLoopError::request_too_large(error_msg));
            }

            return Err(AgentLoopError::llm(error_msg));
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

        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let input_tokens = Arc::clone(&input_tokens);
            let output_tokens = Arc::clone(&output_tokens);
            let cache_read_tokens = Arc::clone(&cache_read_tokens);
            let cache_creation_tokens = Arc::clone(&cache_creation_tokens);
            let current_tool_call = Arc::clone(&current_tool_call);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);

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
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_stop" => {
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

                                Ok(LlmStreamEvent::Done(LlmCompletionMetadata {
                                    total_tokens: Some(in_tokens + out_tokens),
                                    prompt_tokens: Some(in_tokens),
                                    completion_tokens: Some(out_tokens),
                                    cache_read_tokens: cache_read,
                                    cache_creation_tokens: cache_creation,
                                    model: Some(model),
                                    finish_reason: Some("stop".to_string()),
                                }))
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
            .map(|m| DiscoveredModel {
                model_id: m.id,
                display_name: Some(m.display_name),
                created_at: m
                    .created_at
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc)),
                owned_by: Some("anthropic".to_string()),
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

/// Check if an Anthropic API error indicates the request is too large.
///
/// Detects:
/// - 413 Request Entity Too Large
/// - 400 with "prompt is too long" message
/// - "request size exceeded" message
fn is_anthropic_request_too_large(status: reqwest::StatusCode, error_text: &str) -> bool {
    let error_lower = error_text.to_lowercase();

    // HTTP 413 Request Entity Too Large
    if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
        return true;
    }

    // HTTP 400 with prompt/context length errors
    if status == reqwest::StatusCode::BAD_REQUEST {
        // "prompt is too long: X tokens > Y maximum"
        if error_lower.contains("prompt is too long") {
            return true;
        }
        // "request size exceeded maximum"
        if error_lower.contains("request size exceeded") {
            return true;
        }
        // Generic context length error
        if error_lower.contains("context length") || error_lower.contains("too many tokens") {
            return true;
        }
    }

    // Generic patterns that could appear with various status codes
    if error_lower.contains("input is too long")
        || error_lower.contains("exceeds the maximum")
        || error_lower.contains("maximum context")
    {
        return true;
    }

    false
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
    /// Metadata for tracking API usage.
    /// Contains user_id for abuse detection and any additional tracking fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<AnthropicMetadata>,
}

/// Metadata for Anthropic API requests
#[derive(Debug, Serialize)]
struct AnthropicMetadata {
    /// End-user identifier for abuse detection and rate limiting.
    /// Anthropic recommends setting this to help with monitoring.
    #[serde(skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
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
        let budget = match effort.to_lowercase().as_str() {
            "low" => 1024,
            "medium" => 4096,
            "high" => 16384,
            "xhigh" => 32768,
            _ => return None,
        };
        Some(Self {
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
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
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

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)] // Fields used for deserialization
enum AnthropicContentBlockDelta {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse { id: String, name: String },
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlockDeltaEvent {
    delta: AnthropicDelta,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicDelta {
    #[serde(rename = "text_delta")]
    TextDelta { text: String },
    #[serde(rename = "input_json_delta")]
    InputJsonDelta { partial_json: String },
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
#[derive(Debug, Deserialize)]
struct AnthropicModelInfo {
    /// Model identifier (e.g., "claude-opus-4-5-20251101")
    id: String,
    /// Human-readable display name (e.g., "Claude Opus 4.5")
    display_name: String,
    /// ISO 8601 timestamp when the model was created
    #[serde(default)]
    created_at: Option<String>,
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
            },
            LlmMessage {
                role: LlmMessageRole::User,
                content: LlmMessageContent::Text("Hello".to_string()),
                tool_calls: None,
                tool_call_id: None,
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
                assert_eq!(content, "{\"temp\": 20}");
            }
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

    #[test]
    fn test_request_includes_metadata() {
        // Test that metadata with user_id is correctly serialized
        let metadata = AnthropicMetadata {
            user_id: Some("session_abc123".to_string()),
        };

        let json = serde_json::to_value(&metadata).unwrap();
        assert_eq!(json["user_id"], "session_abc123");
    }

    #[test]
    fn test_request_metadata_skips_none() {
        // Test that None user_id is skipped in serialization
        let metadata = AnthropicMetadata { user_id: None };

        let json = serde_json::to_value(&metadata).unwrap();
        assert!(json.get("user_id").is_none());
    }
}
