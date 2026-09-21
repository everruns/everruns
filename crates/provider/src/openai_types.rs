//! The OpenAI `/chat/completions` request and response types.
//!
//! The wire shapes [`OpenAIProtocolChatDriver`](super::openai_protocol) serializes
//! and parses. Crate-private: callers exchange the driver-facing
//! [`Message`](crate::driver_registry::Message) and friends, and convert
//! with [`openai_wire`](crate::openai_wire) when they hold the JSON itself.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize)]
pub(crate) struct OpenAiRequest {
    pub(crate) model: String,
    pub(crate) messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_tokens: Option<u32>,
    pub(crate) stream: bool,
    /// Request usage info in streaming response (required for token counts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream_options: Option<OpenAiStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<OpenAiTool>>,
    /// Request-level control over parallel tool calls. Omitted when unset so the
    /// provider default applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning_effort: Option<String>,
    /// Speed selector: OpenAI service tier ("flex", "default", "priority").
    /// Omitted when `None` so the provider keeps its default ("auto") routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service_tier: Option<String>,
    /// Verbosity selector ("low", "medium", "high"). Top-level field on the
    /// Chat Completions API. Omitted when `None` so the provider keeps its
    /// default ("medium") output length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) verbosity: Option<String>,
    /// Metadata for tracking API usage (up to 16 key-value pairs).
    /// Useful for correlating requests with session_id, agent_id, org_id, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct OpenAiStreamOptions {
    pub(crate) include_usage: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum OpenAiContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum OpenAiContentPart {
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
    File {
        r#type: String,
        file: OpenAiFile,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAiFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) filename: Option<String>,
    pub(crate) file_data: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAiImageUrl {
    pub(crate) url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAiInputAudio {
    pub(crate) data: String,
    pub(crate) format: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAiMessage {
    pub(crate) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<OpenAiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
}

/// Non-streaming `chat/completions` response (`stream: false`).
#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiChatCompletionResponse {
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) choices: Vec<OpenAiChatChoice>,
    #[serde(default)]
    pub(crate) usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiChatChoice {
    pub(crate) message: OpenAiChatMessage,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiChatMessage {
    #[serde(default)]
    pub(crate) content: Option<OpenAiContent>,
    #[serde(default)]
    pub(crate) tool_calls: Vec<OpenAiToolCall>,
    /// DeepSeek-style non-streamed reasoning payload.
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAiTool {
    pub(crate) r#type: String,
    pub(crate) function: OpenAiFunction,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAiFunction {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) strict: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAiToolCall {
    pub(crate) id: String,
    pub(crate) r#type: String,
    pub(crate) function: OpenAiFunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct OpenAiFunctionCall {
    pub(crate) name: String,
    pub(crate) arguments: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // id and model are deserialized but used by event listeners, not directly
pub(crate) struct OpenAiStreamChunk {
    /// Unique identifier for this completion
    #[serde(default)]
    pub(crate) id: Option<String>,
    /// Model used for completion (may differ from requested)
    #[serde(default)]
    pub(crate) model: Option<String>,
    pub(crate) choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    pub(crate) usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiUsage {
    pub(crate) prompt_tokens: Option<u32>,
    pub(crate) completion_tokens: Option<u32>,
    /// Detailed breakdown of prompt tokens (includes cached tokens)
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<OpenAiPromptTokensDetails>,
    /// Detailed breakdown of completion tokens (includes reasoning tokens)
    #[serde(default)]
    pub(crate) completion_tokens_details: Option<OpenAiCompletionTokensDetails>,
    /// Authoritative per-request cost in USD credits, returned by
    /// OpenAI-compatible gateways such as OpenRouter. Absent for direct OpenAI.
    #[serde(default)]
    pub(crate) cost: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpenAiPromptTokensDetails {
    /// Number of tokens retrieved from cache
    #[serde(default)]
    pub(crate) cached_tokens: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct OpenAiCompletionTokensDetails {
    /// Reasoning tokens billed inside `completion_tokens`
    #[serde(default)]
    pub(crate) reasoning_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiStreamChoice {
    pub(crate) delta: OpenAiDelta,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiDelta {
    #[serde(default)]
    pub(crate) content: Option<String>,
    /// Reasoning text on the Chat Completions wire. Reasoning models reached
    /// over this protocol (DeepSeek-R1, Qwen, Groq, Fireworks) stream it here;
    /// vendors split between two field names for the same thing.
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
    #[serde(default)]
    pub(crate) reasoning: Option<String>,
    #[serde(default)]
    pub(crate) tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

impl OpenAiDelta {
    pub(crate) fn reasoning_text(&self) -> Option<&str> {
        self.reasoning_content
            .as_deref()
            .or(self.reasoning.as_deref())
            .filter(|text| !text.is_empty())
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiStreamToolCall {
    pub(crate) index: u32,
    pub(crate) id: Option<String>,
    pub(crate) function: Option<OpenAiStreamFunction>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiStreamFunction {
    pub(crate) name: Option<String>,
    pub(crate) arguments: Option<String>,
}
