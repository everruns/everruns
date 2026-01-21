// OpenAI Protocol LLM Driver
//
// Base implementation of the OpenAI chat completion protocol.
// This driver can be used with any OpenAI-compatible API endpoint.
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
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

use crate::error::{AgentLoopError, Result};
use crate::llm_driver_registry::{
    LlmCallConfig, LlmCompletionMetadata, LlmContentPart, LlmDriver, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmResponseStream, LlmStreamEvent,
};
use crate::tool_types::{ToolCall, ToolDefinition};

const DEFAULT_API_URL: &str = "https://api.openai.com/v1/chat/completions";

/// OpenAI Protocol LLM Driver
///
/// Base implementation of `LlmDriver` for OpenAI-compatible APIs.
/// Supports streaming responses and tool calls.
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
/// ```
#[derive(Clone)]
pub struct OpenAIProtocolLlmDriver {
    client: Client,
    api_key: String,
    api_url: String,
}

impl OpenAIProtocolLlmDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_url: DEFAULT_API_URL.to_string(),
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
        }
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
            reasoning_effort: config.reasoning_effort.clone(),
            metadata,
        };

        let response = self
            .client
            .post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to send request: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            let error_msg = format!("OpenAI API error ({}): {}", status, error_text);

            // Check if this is a request-too-large error
            if is_openai_request_too_large(status, &error_text) {
                return Err(AgentLoopError::request_too_large(error_msg));
            }

            return Err(AgentLoopError::llm(error_msg));
        }

        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream.eventsource();

        let model = config.model.clone();
        let total_tokens = Arc::new(Mutex::new(0u32));
        let prompt_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));

        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let total_tokens = Arc::clone(&total_tokens);
            let prompt_tokens = Arc::clone(&prompt_tokens);
            let cache_read_tokens = Arc::clone(&cache_read_tokens);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            let finish_reason = Arc::clone(&finish_reason);

            async move {
                match result {
                    Ok(event) => {
                        if event.data == "[DONE]" {
                            let output_tokens = *total_tokens.lock().unwrap();
                            let input_tokens = *prompt_tokens.lock().unwrap();
                            let cached = *cache_read_tokens.lock().unwrap();
                            let reason = finish_reason.lock().unwrap().clone();

                            return Ok(LlmStreamEvent::Done(LlmCompletionMetadata {
                                total_tokens: Some(input_tokens + output_tokens),
                                prompt_tokens: Some(input_tokens),
                                completion_tokens: Some(output_tokens),
                                cache_read_tokens: cached,
                                cache_creation_tokens: None,
                                model: Some(model),
                                finish_reason: reason.or_else(|| Some("stop".to_string())),
                            }));
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
                                }

                                if let Some(choice) = chunk.choices.first() {
                                    // Handle tool calls
                                    if let Some(tool_calls) = &choice.delta.tool_calls {
                                        let mut acc = accumulated_tool_calls.lock().unwrap();

                                        for tc in tool_calls {
                                            let idx = tc.index as usize;
                                            while acc.len() <= idx {
                                                acc.push(ToolCall {
                                                    id: String::new(),
                                                    name: String::new(),
                                                    arguments: json!(""),
                                                });
                                            }

                                            if let Some(id) = &tc.id {
                                                acc[idx].id = id.clone();
                                            }
                                            if let Some(function) = &tc.function {
                                                if let Some(name) = &function.name {
                                                    acc[idx].name = name.clone();
                                                }
                                                if let Some(args) = &function.arguments {
                                                    let current =
                                                        acc[idx].arguments.as_str().unwrap_or("");
                                                    let combined = format!("{}{}", current, args);
                                                    acc[idx].arguments = json!(combined);
                                                }
                                            }
                                        }
                                        return Ok(LlmStreamEvent::TextDelta(String::new()));
                                    }

                                    // Handle content delta
                                    if let Some(content) = &choice.delta.content {
                                        *total_tokens.lock().unwrap() += 1;
                                        return Ok(LlmStreamEvent::TextDelta(content.clone()));
                                    }

                                    // Handle finish reason
                                    // Note: Don't return Done here - wait for [DONE] marker
                                    // OpenAI sends usage in a separate chunk AFTER finish_reason
                                    if let Some(fr) = &choice.finish_reason {
                                        // Store finish_reason for the [DONE] handler
                                        *finish_reason.lock().unwrap() = Some(fr.clone());

                                        // For tool_calls, emit ToolCalls event immediately
                                        // so the agent can start processing tools
                                        if fr == "tool_calls" {
                                            let tool_calls =
                                                accumulated_tool_calls.lock().unwrap().clone();
                                            if !tool_calls.is_empty() {
                                                let parsed_calls: Vec<ToolCall> = tool_calls
                                                    .into_iter()
                                                    .map(|mut tc| {
                                                        if let Some(args_str) =
                                                            tc.arguments.as_str()
                                                        {
                                                            tc.arguments =
                                                                serde_json::from_str(args_str)
                                                                    .unwrap_or(json!({}));
                                                        }
                                                        tc
                                                    })
                                                    .collect();
                                                return Ok(LlmStreamEvent::ToolCalls(parsed_calls));
                                            }
                                        }
                                        // For other finish reasons (stop, length, etc.),
                                        // just continue - [DONE] will return Done with usage
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            Err(e) => Ok(LlmStreamEvent::Error(format!(
                                "Failed to parse chunk: {}",
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
}
