// OpenAI Protocol Types
//
// These types represent the OpenAI API protocol format.
// They serve as the base protocol for LLM providers in the system.

use everruns_contracts::tools::{ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};

/// Provider-agnostic chat message following OpenAI's format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    /// Tool call results (for assistant messages with tool calls)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// Tool call ID (for tool result messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Message role in conversation (OpenAI format)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// LLM configuration following OpenAI's API parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Model identifier (e.g., "gpt-4", "gpt-3.5-turbo")
    pub model: String,
    /// Sampling temperature (0.0 - 2.0)
    pub temperature: Option<f32>,
    /// Maximum tokens to generate
    pub max_tokens: Option<u32>,
    /// System prompt (if not in messages)
    pub system_prompt: Option<String>,
    /// Available tools (for function calling)
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

/// Events emitted during LLM streaming
#[derive(Debug, Clone)]
pub enum LlmStreamEvent {
    /// Text delta (incremental content)
    TextDelta(String),
    /// Tool calls from the LLM
    ToolCalls(Vec<ToolCall>),
    /// Streaming completed successfully
    Done(CompletionMetadata),
    /// Error occurred during streaming
    Error(String),
}

/// Completion metadata returned on stream completion
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompletionMetadata {
    /// Total tokens used (if available)
    pub total_tokens: Option<u32>,
    /// Input tokens used (if available)
    pub prompt_tokens: Option<u32>,
    /// Output tokens generated (if available)
    pub completion_tokens: Option<u32>,
    /// Model used
    pub model: String,
    /// Finish reason
    pub finish_reason: Option<String>,
}

/// OpenAI chat completion request format
#[derive(Debug, Clone, Serialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenAiTool>>,
}

// ============================================================================
// OpenAI API Types (Internal)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiTool {
    pub r#type: String,
    pub function: OpenAiFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiFunction {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiToolCall {
    pub id: String,
    pub r#type: String,
    pub function: OpenAiFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiFunctionCall {
    pub name: String,
    pub arguments: String,
}

// Streaming types
#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiStreamChunk {
    pub choices: Vec<OpenAiStreamChoice>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiStreamChoice {
    pub delta: OpenAiDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiStreamToolCall {
    pub index: u32,
    pub id: Option<String>,
    #[allow(dead_code)]
    pub r#type: Option<String>,
    pub function: Option<OpenAiStreamFunction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiStreamFunction {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

// Non-streaming response types
#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiResponse {
    pub model: String,
    pub choices: Vec<OpenAiChoice>,
    pub usage: Option<OpenAiUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiChoice {
    pub message: OpenAiMessage,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

// ============================================================================
// Conversions
// ============================================================================

impl ChatMessage {
    /// Convert to OpenAI API message format
    pub fn to_openai(&self) -> OpenAiMessage {
        let role = match self.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::Tool => "tool",
        };

        OpenAiMessage {
            role: role.to_string(),
            content: Some(self.content.clone()),
            tool_calls: self.tool_calls.as_ref().map(|calls| {
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
            tool_call_id: self.tool_call_id.clone(),
        }
    }
}

impl LlmConfig {
    /// Convert tool definitions to OpenAI's format
    pub fn tools_to_openai(&self) -> Vec<OpenAiTool> {
        self.tools
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
