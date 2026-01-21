// Open Responses Protocol Driver
//
// Implementation of the Open Responses specification (https://www.openresponses.org/)
// an open-source, vendor-neutral API standard for multi-provider LLM interfaces.
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
use crate::tool_types::{ToolCall, ToolDefinition};

const DEFAULT_API_URL: &str = "https://api.openai.com/v1/responses";

/// Open Responses Protocol Driver (OpenAI implementation)
///
/// Implements `LlmDriver` using the Open Responses specification
/// (https://www.openresponses.org/). This driver targets OpenAI's API
/// but follows the vendor-neutral Open Responses standard.
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
/// ```
#[derive(Clone)]
pub struct OpenResponsesProtocolLlmDriver {
    client: Client,
    api_key: String,
    api_url: String,
}

impl OpenResponsesProtocolLlmDriver {
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

    /// Create a new driver with a custom API URL
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
            LlmMessageRole::System => "developer", // Responses API uses "developer" for system
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        }
    }

    fn convert_message(msg: &LlmMessage) -> ResponsesInputItem {
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
                "OpenAI Responses API error ({}): {}",
                status, error_text
            )));
        }

        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream.eventsource();

        let model = config.model.clone();
        let input_tokens = Arc::new(Mutex::new(0u32));
        let output_tokens = Arc::new(Mutex::new(0u32));
        let cache_read_tokens = Arc::new(Mutex::new(Option::<u32>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCallAccumulator>::new()));
        let finish_reason = Arc::new(Mutex::new(Option::<String>::None));

        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let input_tokens = Arc::clone(&input_tokens);
            let output_tokens = Arc::clone(&output_tokens);
            let cache_read_tokens = Arc::clone(&cache_read_tokens);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            let finish_reason = Arc::clone(&finish_reason);

            async move {
                match result {
                    Ok(event) => {
                        // Parse the event type and data
                        let event_data = &event.data;

                        // Parse as generic JSON to get the event type
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

                                    // Reasoning/thinking content from reasoning models (o1, o3, GPT-5.2+)
                                    Some("response.reasoning_summary.delta")
                                    | Some("response.reasoning.delta") => {
                                        // Reasoning delta - emit as ThinkingDelta
                                        if let Some(delta) =
                                            json.get("delta").and_then(|d| d.as_str())
                                        {
                                            Ok(LlmStreamEvent::ThinkingDelta(delta.to_string()))
                                        } else if let Some(text) =
                                            json.get("text").and_then(|t| t.as_str())
                                        {
                                            // Some versions use "text" instead of "delta"
                                            Ok(LlmStreamEvent::ThinkingDelta(text.to_string()))
                                        } else {
                                            Ok(LlmStreamEvent::TextDelta(String::new()))
                                        }
                                    }

                                    Some("response.function_call_arguments.delta") => {
                                        // Function call arguments delta
                                        if let (Some(call_id), Some(delta)) = (
                                            json.get("call_id").and_then(|c| c.as_str()),
                                            json.get("delta").and_then(|d| d.as_str()),
                                        ) {
                                            let mut acc = accumulated_tool_calls.lock().unwrap();
                                            // Find or create accumulator for this call_id
                                            if let Some(tc) =
                                                acc.iter_mut().find(|t| t.id == call_id)
                                            {
                                                tc.arguments.push_str(delta);
                                            } else {
                                                acc.push(ToolCallAccumulator {
                                                    id: call_id.to_string(),
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
                                            if let Some(tc) =
                                                acc.iter_mut().find(|t| t.id == call_id)
                                            {
                                                tc.name = name;
                                            } else {
                                                acc.push(ToolCallAccumulator {
                                                    id: call_id,
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
                                                            id: tc.id.clone(),
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
    id: String,
    name: String,
    arguments: String,
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
            },
            LlmMessage {
                role: LlmMessageRole::Tool,
                content: LlmMessageContent::Text("2025-01-19T10:30:00Z".to_string()),
                tool_calls: None,
                tool_call_id: Some("call_xyz789".to_string()),
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
}
