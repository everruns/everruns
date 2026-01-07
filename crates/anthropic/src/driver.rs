// Anthropic Claude LLM Driver
//
// Implementation of LlmDriver for Anthropic's Claude API.
// Uses the Messages API with streaming support.
//
// Note: OTel instrumentation is handled via the event-listener pattern.
// llm.generation events are emitted by ReasonAtom, and OtelEventListener
// creates the appropriate gen-ai spans. No direct tracing in drivers.

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DriverRegistry, LlmCallConfig, LlmCompletionMetadata, LlmContentPart,
    LlmDriver, LlmMessage, LlmMessageContent, LlmMessageRole, LlmResponseStream, LlmStreamEvent,
    ProviderType,
};
use everruns_core::tool_types::{ToolCall, ToolDefinition};

const DEFAULT_API_URL: &str = "https://api.anthropic.com/v1/messages";
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
}

impl AnthropicLlmDriver {
    /// Create a new provider with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            api_key: api_key.into(),
            api_url: DEFAULT_API_URL.to_string(),
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
        }
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
                vec![AnthropicContentBlock::Text { text: text.clone() }]
            }
            LlmMessageContent::Parts(parts) => parts
                .iter()
                .map(|part| match part {
                    LlmContentPart::Text { text } => {
                        AnthropicContentBlock::Text { text: text.clone() }
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
                            AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Base64 { media_type, data },
                            }
                        } else {
                            // HTTP URL
                            AnthropicContentBlock::Image {
                                source: AnthropicImageSource::Url { url: url.clone() },
                            }
                        }
                    }
                    LlmContentPart::Audio { .. } => {
                        // Anthropic doesn't support audio input yet, convert to text note
                        AnthropicContentBlock::Text {
                            text: "[Audio content not supported]".to_string(),
                        }
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
            .map(|tool| {
                let (name, description, parameters) = match tool {
                    ToolDefinition::Builtin(builtin) => {
                        (&builtin.name, &builtin.description, &builtin.parameters)
                    }
                };

                AnthropicTool {
                    name: name.clone(),
                    description: description.clone(),
                    input_schema: parameters.clone(),
                }
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

        let mut request = AnthropicRequest {
            model: config.model.clone(),
            messages: anthropic_messages,
            max_tokens: config.max_tokens.unwrap_or(4096),
            temperature: config.temperature,
            system: system_prompt,
            stream: true,
            tools,
            thinking,
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
            return Err(AgentLoopError::llm(format!(
                "Anthropic API error ({}): {}",
                status, error_text
            )));
        }

        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream.eventsource();

        let model = config.model.clone();
        let input_tokens = Arc::new(Mutex::new(0u32));
        let output_tokens = Arc::new(Mutex::new(0u32));
        let current_tool_call = Arc::new(Mutex::new(Option::<ToolCall>::None));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));

        let converted_stream: LlmResponseStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let input_tokens = Arc::clone(&input_tokens);
            let output_tokens = Arc::clone(&output_tokens);
            let current_tool_call = Arc::clone(&current_tool_call);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);

            async move {
                match result {
                    Ok(event) => {
                        // Anthropic uses different event types
                        match event.event.as_str() {
                            "message_start" => {
                                // Parse message_start for input token count
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicMessageStart>(&event.data)
                                {
                                    if let Some(usage) = data.message.usage {
                                        *input_tokens.lock().unwrap() = usage.input_tokens;
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "content_block_start" => {
                                // Check if starting a tool use block
                                if let Ok(data) =
                                    serde_json::from_str::<AnthropicContentBlockStart>(&event.data)
                                {
                                    if let AnthropicContentBlockDelta::ToolUse { id, name } =
                                        data.content_block
                                    {
                                        let mut current = current_tool_call.lock().unwrap();
                                        *current = Some(ToolCall {
                                            id,
                                            name,
                                            arguments: json!(""),
                                        });
                                    }
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
                                    }

                                    if let Some(stop_reason) = data.delta.stop_reason {
                                        if stop_reason == "tool_use" {
                                            let tool_calls =
                                                accumulated_tool_calls.lock().unwrap().clone();
                                            if !tool_calls.is_empty() {
                                                return Ok(LlmStreamEvent::ToolCalls(tool_calls));
                                            }
                                        }
                                    }
                                }
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            "message_stop" => {
                                let in_tokens = *input_tokens.lock().unwrap();
                                let out_tokens = *output_tokens.lock().unwrap();

                                Ok(LlmStreamEvent::Done(LlmCompletionMetadata {
                                    total_tokens: Some(in_tokens + out_tokens),
                                    prompt_tokens: Some(in_tokens),
                                    completion_tokens: Some(out_tokens),
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
