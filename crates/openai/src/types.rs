// OpenAI Protocol Types
//
// These types are kept for backward compatibility with existing code.
// The actual implementation is now in everruns-core/src/openai.rs.

use everruns_core::{ToolCall, ToolDefinition};
use serde::{Deserialize, Serialize};

/// OpenAI API mode - determines which API endpoint to use
///
/// OpenAI offers two API options:
/// - **Completions**: The traditional Chat Completions API (`/v1/chat/completions`)
/// - **Responses**: The Open Responses API (`/v1/responses`), recommended for new projects
///
/// Open Responses (https://www.openresponses.org/) is a vendor-neutral API specification
/// that standardizes LLM interfaces across providers (OpenAI, Anthropic, Gemini, etc.).
///
/// Benefits of Open Responses:
/// - One spec, many providers - same API works across vendors
/// - Agentic loop support with semantic streaming events
/// - Better performance with reasoning models (o1, o3, GPT-5)
/// - 40-80% better cache utilization
/// - Native stateful conversation support
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OpenAIApiMode {
    /// Chat Completions API (for backward compatibility)
    Completions,
    /// Open Responses API (default, recommended for new projects)
    #[default]
    Responses,
}

impl std::fmt::Display for OpenAIApiMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenAIApiMode::Completions => write!(f, "completions"),
            OpenAIApiMode::Responses => write!(f, "responses"),
        }
    }
}

impl std::str::FromStr for OpenAIApiMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "completions" | "chat" | "chat_completions" => Ok(OpenAIApiMode::Completions),
            "responses" => Ok(OpenAIApiMode::Responses),
            _ => Err(format!(
                "Unknown API mode: {}. Use 'completions' or 'responses'",
                s
            )),
        }
    }
}

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
    /// Model identifier (e.g., "gpt-5.2", "gpt-3.5-turbo")
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
// OpenAI API Types (for ChatRequest serialization)
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

// ============================================================================
// OpenAI Models API Types
// ============================================================================

/// Response from OpenAI's /v1/models endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiModelsResponse {
    pub data: Vec<OpenAiModelInfo>,
}

/// Individual model info from OpenAI's models API
#[derive(Debug, Clone, Deserialize)]
pub struct OpenAiModelInfo {
    /// Model identifier (e.g., "gpt-5.2", "gpt-4o")
    pub id: String,
    /// Unix timestamp of model creation
    pub created: i64,
    /// Owner organization (e.g., "openai", "system")
    pub owned_by: String,
}

impl OpenAiModelInfo {
    /// Check if this model is a chat/completion model (not embedding, TTS, etc.)
    ///
    /// Includes: gpt-*, o1*, o3*, o4*, chatgpt-*
    /// Excludes: text-embedding-*, dall-e-*, tts-*, whisper-*, davinci-*, babbage-*, omni-moderation-*, sora-*, codex-*, gpt-image-*
    pub fn is_chat_model(&self) -> bool {
        let id = self.id.as_str();

        // Exclude patterns (check these first)
        if id.starts_with("text-embedding")
            || id.starts_with("dall-e")
            || id.starts_with("tts-")
            || id.starts_with("whisper")
            || id.starts_with("davinci")
            || id.starts_with("babbage")
            || id.starts_with("omni-moderation")
            || id.starts_with("sora-")
            || id.starts_with("gpt-image")
            || id.starts_with("codex-")
            || id.contains("-transcribe")
            || id.contains("-realtime")
            || id.contains("-audio")
            || id.contains("-tts")
        {
            return false;
        }

        // Include patterns
        id.starts_with("gpt-")
            || id.starts_with("o1")
            || id.starts_with("o3")
            || id.starts_with("o4")
            || id.starts_with("chatgpt-")
    }
}
