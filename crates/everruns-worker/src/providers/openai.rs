// OpenAI provider implementation using direct HTTP API

use super::{
    ChatCompletionResult, ChatMessage, CompletionMetadata, LlmConfig, LlmProvider, LlmStream,
    LlmStreamEvent, MessageRole,
};
use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use everruns_contracts::tools::{ToolCall, ToolDefinition};
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{Arc, Mutex};

const OPENAI_API_URL: &str = "https://api.openai.com/v1/chat/completions";

/// OpenAI LLM provider
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
}

impl OpenAiProvider {
    /// Create a new OpenAI provider
    /// Requires OPENAI_API_KEY environment variable
    pub fn new() -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .context("OPENAI_API_KEY environment variable not set")?;
        let client = Client::new();
        Ok(Self { client, api_key })
    }

    /// Convert our ChatMessage to OpenAI's API format
    fn convert_message(msg: &ChatMessage) -> OpenAiMessage {
        let role = match msg.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        };

        OpenAiMessage {
            role: role.to_string(),
            content: Some(msg.content.clone()),
            tool_calls: msg.tool_calls.as_ref().map(|calls| {
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
            }),
            tool_call_id: msg.tool_call_id.clone(),
        }
    }

    /// Convert tool definitions to OpenAI's format
    fn convert_tools(tools: &[ToolDefinition]) -> Vec<OpenAiTool> {
        tools
            .iter()
            .map(|tool| {
                let (name, description, parameters) = match tool {
                    ToolDefinition::Webhook(webhook) => {
                        (&webhook.name, &webhook.description, &webhook.parameters)
                    }
                    ToolDefinition::Builtin(builtin) => {
                        (&builtin.name, &builtin.description, &builtin.parameters)
                    }
                };

                OpenAiTool {
                    r#type: "function".to_string(),
                    function: OpenAiFunction {
                        name: name.clone(),
                        description: description.clone(),
                        parameters: parameters.clone(),
                    },
                }
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenAiProvider {
    /// Non-streaming chat completion
    async fn chat_completion(
        &self,
        messages: Vec<ChatMessage>,
        config: &LlmConfig,
    ) -> Result<ChatCompletionResult> {
        // Convert messages to OpenAI format
        let mut openai_messages: Vec<OpenAiMessage> =
            messages.iter().map(Self::convert_message).collect();

        // Add system prompt if provided in config and not in messages
        if let Some(system_prompt) = &config.system_prompt {
            if !messages
                .iter()
                .any(|m| matches!(m.role, MessageRole::System))
            {
                openai_messages.insert(
                    0,
                    OpenAiMessage {
                        role: "system".to_string(),
                        content: Some(system_prompt.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                    },
                );
            }
        }

        // Build request body (non-streaming)
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
            stream: false,
            tools,
        };

        // Make request
        let response = self
            .client
            .post(OPENAI_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send OpenAI request")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "OpenAI API request failed with status {}: {}",
                status,
                error_text
            );
        }

        // Parse response
        let response_json: OpenAiResponse = response
            .json()
            .await
            .context("Failed to parse OpenAI response")?;

        // Extract content from the first choice
        let choice = response_json
            .choices
            .first()
            .ok_or_else(|| anyhow::anyhow!("No choices in OpenAI response"))?;

        let text = choice.message.content.clone().unwrap_or_default();

        // Extract tool calls if any
        let tool_calls = choice.message.tool_calls.as_ref().map(|calls| {
            calls
                .iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments).unwrap_or(json!({})),
                })
                .collect()
        });

        let metadata = CompletionMetadata {
            total_tokens: response_json.usage.as_ref().map(|u| u.total_tokens),
            prompt_tokens: response_json.usage.as_ref().map(|u| u.prompt_tokens),
            completion_tokens: response_json.usage.as_ref().map(|u| u.completion_tokens),
            model: response_json.model,
            finish_reason: choice.finish_reason.clone(),
        };

        Ok(ChatCompletionResult {
            text,
            tool_calls,
            metadata,
        })
    }

    /// Streaming chat completion (kept for future use)
    async fn chat_completion_stream(
        &self,
        messages: Vec<ChatMessage>,
        config: &LlmConfig,
    ) -> Result<LlmStream> {
        // Convert messages to OpenAI format
        let mut openai_messages: Vec<OpenAiMessage> =
            messages.iter().map(Self::convert_message).collect();

        // Add system prompt if provided in config and not in messages
        if let Some(system_prompt) = &config.system_prompt {
            if !messages
                .iter()
                .any(|m| matches!(m.role, MessageRole::System))
            {
                openai_messages.insert(
                    0,
                    OpenAiMessage {
                        role: "system".to_string(),
                        content: Some(system_prompt.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                    },
                );
            }
        }

        // Build request body
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
            tools,
        };

        // Make streaming request
        let response = self
            .client
            .post(OPENAI_API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .context("Failed to send OpenAI request")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "OpenAI API request failed with status {}: {}",
                status,
                error_text
            );
        }

        // Convert response stream to SSE events
        let byte_stream = response.bytes_stream();
        let event_stream = byte_stream.eventsource();

        // Parse SSE events into our format
        let model = config.model.clone();
        let total_tokens = Arc::new(Mutex::new(0u32));
        let accumulated_tool_calls = Arc::new(Mutex::new(Vec::<ToolCall>::new()));

        let converted_stream: LlmStream = Box::pin(event_stream.then(move |result| {
            let model = model.clone();
            let total_tokens = Arc::clone(&total_tokens);
            let accumulated_tool_calls = Arc::clone(&accumulated_tool_calls);
            async move {
                match result {
                    Ok(event) => {
                        // OpenAI sends [DONE] to signal completion
                        if event.data == "[DONE]" {
                            let tokens = *total_tokens.lock().unwrap();
                            return Ok(LlmStreamEvent::Done(CompletionMetadata {
                                total_tokens: Some(tokens),
                                prompt_tokens: None,
                                completion_tokens: Some(tokens),
                                model: model.clone(),
                                finish_reason: Some("stop".to_string()),
                            }));
                        }

                        // Parse the JSON chunk
                        match serde_json::from_str::<OpenAiStreamChunk>(&event.data) {
                            Ok(chunk) => {
                                if let Some(choice) = chunk.choices.first() {
                                    // Handle tool calls (incremental streaming)
                                    if let Some(tool_calls) = &choice.delta.tool_calls {
                                        let mut acc = accumulated_tool_calls.lock().unwrap();

                                        for tc in tool_calls {
                                            let idx = tc.index as usize;

                                            // Ensure we have enough slots
                                            while acc.len() <= idx {
                                                acc.push(ToolCall {
                                                    id: String::new(),
                                                    name: String::new(),
                                                    arguments: json!(""),
                                                });
                                            }

                                            // Update tool call fields incrementally
                                            if let Some(id) = &tc.id {
                                                acc[idx].id = id.clone();
                                            }
                                            if let Some(function) = &tc.function {
                                                if let Some(name) = &function.name {
                                                    acc[idx].name = name.clone();
                                                }
                                                if let Some(args) = &function.arguments {
                                                    // Arguments are streamed as strings, accumulate
                                                    let current =
                                                        acc[idx].arguments.as_str().unwrap_or("");
                                                    let combined = format!("{}{}", current, args);
                                                    acc[idx].arguments = json!(combined);
                                                }
                                            }
                                        }

                                        // Return empty delta to continue stream
                                        return Ok(LlmStreamEvent::TextDelta(String::new()));
                                    }

                                    // Handle content delta
                                    if let Some(content) = &choice.delta.content {
                                        *total_tokens.lock().unwrap() += 1; // Rough approximation
                                        return Ok(LlmStreamEvent::TextDelta(content.clone()));
                                    }

                                    // Handle finish reason
                                    if let Some(finish_reason) = &choice.finish_reason {
                                        let tokens = *total_tokens.lock().unwrap();

                                        // If finish_reason is tool_calls, emit the accumulated tool calls
                                        if finish_reason == "tool_calls" {
                                            let tool_calls =
                                                accumulated_tool_calls.lock().unwrap().clone();
                                            if !tool_calls.is_empty() {
                                                // Parse accumulated JSON argument strings
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

                                        return Ok(LlmStreamEvent::Done(CompletionMetadata {
                                            total_tokens: Some(tokens),
                                            prompt_tokens: None,
                                            completion_tokens: Some(tokens),
                                            model: model.clone(),
                                            finish_reason: Some(finish_reason.clone()),
                                        }));
                                    }
                                }

                                // No meaningful content, return empty delta
                                Ok(LlmStreamEvent::TextDelta(String::new()))
                            }
                            Err(e) => Ok(LlmStreamEvent::Error(format!(
                                "Failed to parse OpenAI chunk: {}",
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

impl Default for OpenAiProvider {
    fn default() -> Self {
        Self::new().expect("Failed to create OpenAI provider")
    }
}

// OpenAI API types

#[derive(Debug, Clone, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiTool {
    r#type: String,
    function: OpenAiFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiToolCall {
    id: String,
    r#type: String,
    function: OpenAiFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenAiFunctionCall {
    name: String,
    arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiStreamChunk {
    choices: Vec<OpenAiStreamChoice>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiStreamChoice {
    delta: OpenAiDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiStreamToolCall {
    index: u32,
    id: Option<String>,
    #[allow(dead_code)]
    r#type: Option<String>,
    function: Option<OpenAiStreamFunction>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiStreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}

// Non-streaming response types

#[derive(Debug, Clone, Deserialize)]
struct OpenAiResponse {
    model: String,
    choices: Vec<OpenAiChoice>,
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}
