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
            return Err(AgentLoopError::llm(format!(
                "OpenAI API error ({}): {}",
                status, error_text
            )));
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
        };

        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["stream"], true);
        assert_eq!(json["stream_options"]["include_usage"], true);
    }
}
