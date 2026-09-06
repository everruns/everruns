// OpenResponses API Types
//
// Type definitions matching the OpenResponses OpenAPI specification v2.3.0
// https://github.com/openresponses/openresponses/blob/main/public/openapi/openapi.json
//
// The OpenResponses spec is a vendor-neutral API standard for LLM interfaces.
// See https://www.openresponses.org/ for the full specification.
//
// Types are organized by category:
// - Request types (CreateResponseBody, input items, tools)
// - Response types (ResponseResource, output items, usage)
// - Streaming event types (24 distinct SSE event types)
// - Error types (Error, ErrorPayload)
// - Enums (roles, statuses, tool choice modes)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// ============================================================================
// Enums
// ============================================================================

/// Message role in the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Developer,
}

/// Status for items (messages, function calls).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    InProgress,
    Completed,
    Incomplete,
}

/// Reasoning effort level for o-series and gpt-5 models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

/// Reasoning summary verbosity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    Concise,
    Detailed,
    Auto,
}

/// Tool choice mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolChoiceMode {
    None,
    Auto,
    Required,
}

/// Service tier for request priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceTier {
    Auto,
    Default,
    Flex,
    Priority,
}

/// Truncation mode for long inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Truncation {
    Auto,
    Disabled,
}

/// Image detail level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageDetail {
    Low,
    High,
    Auto,
}

/// Verbosity level for text output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    Low,
    Medium,
    High,
}

/// Response status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseStatus {
    Completed,
    Failed,
    Cancelled,
    InProgress,
    Queued,
    Incomplete,
    #[serde(other)]
    Unknown,
}

// ============================================================================
// Input Content Types
// ============================================================================

/// Text content input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputTextContent {
    #[serde(rename = "type")]
    pub type_: String,
    /// Text input (max 10MB).
    pub text: String,
}

impl InputTextContent {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            type_: "input_text".to_string(),
            text: text.into(),
        }
    }
}

/// Image content input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputImageContent {
    #[serde(rename = "type")]
    pub type_: String,
    /// URL or base64 data URL (max 20MB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    /// Detail level (default: auto).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<ImageDetail>,
}

impl InputImageContent {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            type_: "input_image".to_string(),
            image_url: Some(url.into()),
            detail: None,
        }
    }
}

/// Audio content input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAudioContent {
    #[serde(rename = "type")]
    pub type_: String,
    /// Audio data.
    pub input_audio: InputAudioData,
}

/// Audio data payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAudioData {
    /// Base64-encoded audio data.
    pub data: String,
    /// Audio format (e.g., "wav", "mp3").
    pub format: String,
}

/// File content input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputFileContent {
    #[serde(rename = "type")]
    pub type_: String,
    /// Filename.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Base64-encoded file data (max 32MB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_data: Option<String>,
    /// File URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_url: Option<String>,
}

/// Video content input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputVideoContent {
    #[serde(rename = "type")]
    pub type_: String,
    /// Base64 or remote URL to video file.
    pub video_url: String,
}

/// Content parts in a message (polymorphic).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPart {
    #[serde(rename = "input_text")]
    InputText { text: String },
    #[serde(rename = "input_image")]
    InputImage {
        #[serde(skip_serializing_if = "Option::is_none")]
        image_url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<ImageDetail>,
    },
    #[serde(rename = "input_audio")]
    InputAudio { input_audio: InputAudioData },
    #[serde(rename = "input_file")]
    InputFile {
        #[serde(skip_serializing_if = "Option::is_none")]
        filename: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        file_url: Option<String>,
    },
    #[serde(rename = "input_video")]
    InputVideo { video_url: String },
    #[serde(rename = "output_text")]
    OutputText {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        annotations: Vec<UrlCitation>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        logprobs: Vec<LogProb>,
    },
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "summary_text")]
    SummaryText { text: String },
    #[serde(rename = "reasoning_text")]
    ReasoningText { text: String },
    #[serde(rename = "refusal")]
    Refusal { refusal: String },
}

/// Message content (string or parts).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<ContentPart>),
}

// ============================================================================
// Input Items
// ============================================================================

/// Message item in conversation input.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageItem {
    #[serde(rename = "type")]
    pub type_: String,
    pub role: String,
    pub content: MessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

impl MessageItem {
    pub fn new(role: &str, content: MessageContent) -> Self {
        Self {
            type_: "message".to_string(),
            role: role.to_string(),
            content,
            id: None,
            status: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::new("user", MessageContent::Text(text.into()))
    }

    pub fn developer(text: impl Into<String>) -> Self {
        Self::new("developer", MessageContent::Text(text.into()))
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self::new("assistant", MessageContent::Text(text.into()))
    }
}

/// Function call item (from model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallItem {
    #[serde(rename = "type")]
    pub type_: String,
    /// Unique ID for this function call (1-64 chars).
    pub call_id: String,
    /// Function name (1-64 chars, alphanumeric + underscore/dash).
    pub name: String,
    /// JSON arguments string.
    pub arguments: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ItemStatus>,
}

impl FunctionCallItem {
    pub fn new(
        call_id: impl Into<String>,
        name: impl Into<String>,
        arguments: impl Into<String>,
    ) -> Self {
        Self {
            type_: "function_call".to_string(),
            call_id: call_id.into(),
            name: name.into(),
            arguments: arguments.into(),
            id: None,
            status: None,
        }
    }
}

/// Function call output item (tool result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallOutputItem {
    #[serde(rename = "type")]
    pub type_: String,
    /// ID of the function call this is responding to.
    pub call_id: String,
    /// Output (string or content parts).
    pub output: FunctionCallOutput,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ItemStatus>,
}

/// Function call output (string or structured).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FunctionCallOutput {
    Text(String),
    Parts(Vec<ContentPart>),
}

impl FunctionCallOutputItem {
    pub fn new(call_id: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            type_: "function_call_output".to_string(),
            call_id: call_id.into(),
            output: FunctionCallOutput::Text(output.into()),
            id: None,
            status: None,
        }
    }
}

/// Reasoning item for o-series models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningItem {
    #[serde(rename = "type")]
    pub type_: String,
    pub id: String,
    /// Summary of reasoning.
    #[serde(default)]
    pub summary: Vec<ContentPart>,
    /// Full reasoning content (if included).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ContentPart>>,
    /// Encrypted reasoning content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
}

/// Item reference for conversation chaining.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemReference {
    #[serde(rename = "type")]
    pub type_: String,
    pub id: String,
}

/// Input item (polymorphic union).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum InputItem {
    Message(MessageItem),
    FunctionCall(FunctionCallItem),
    FunctionCallOutput(FunctionCallOutputItem),
    Reasoning(ReasoningItem),
    Reference(ItemReference),
}

impl<'de> Deserialize<'de> for InputItem {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        // Shapes overlap: an item reference also satisfies the reasoning fields.
        // Dispatch on the protocol discriminator before decoding the payload.
        match value.get("type").and_then(Value::as_str) {
            Some("message") => serde_json::from_value(value).map(Self::Message),
            Some("function_call") => serde_json::from_value(value).map(Self::FunctionCall),
            Some("function_call_output") => {
                serde_json::from_value(value).map(Self::FunctionCallOutput)
            }
            Some("reasoning") => serde_json::from_value(value).map(Self::Reasoning),
            Some("item_reference") => serde_json::from_value(value).map(Self::Reference),
            _ => {
                return Err(serde::de::Error::custom(
                    "unsupported or missing input item type",
                ));
            }
        }
        .map_err(serde::de::Error::custom)
    }
}

// ============================================================================
// Tools
// ============================================================================

/// Function tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionTool {
    #[serde(rename = "type")]
    pub type_: String,
    /// Function name (1-64 chars, pattern: ^[a-zA-Z0-9_-]+$).
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema for parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    /// Enable strict schema validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

impl FunctionTool {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            type_: "function".to_string(),
            name: name.into(),
            description: None,
            parameters: None,
            strict: None,
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn with_parameters(mut self, parameters: Value) -> Self {
        self.parameters = Some(parameters);
        self
    }
}

/// Tool definition (currently only function tools).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Tool {
    #[serde(rename = "function")]
    Function {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        parameters: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
}

/// Specific function to call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecificFunction {
    #[serde(rename = "type")]
    pub type_: String,
    pub name: String,
}

/// Tool choice configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolChoice {
    Mode(ToolChoiceMode),
    Specific(SpecificFunction),
    AllowedTools {
        #[serde(rename = "type")]
        type_: String,
        tools: Vec<SpecificFunction>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mode: Option<ToolChoiceMode>,
    },
}

// ============================================================================
// Reasoning Configuration
// ============================================================================

/// Reasoning configuration for o-series and gpt-5 models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReasoningSummary>,
}

// ============================================================================
// Text Configuration
// ============================================================================

/// Text format configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TextFormat {
    #[serde(rename = "text")]
    Text,
    #[serde(rename = "json_object")]
    JsonObject,
    #[serde(rename = "json_schema")]
    JsonSchema {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
    },
}

/// Text output configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<TextFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<Verbosity>,
}

// ============================================================================
// Request Body
// ============================================================================

/// Request body for creating a response.
/// See: <https://www.openresponses.org/specification>
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResponseBody {
    /// Model to use (e.g., "gpt-5.2", "o3").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Input context (string for simple user message, or array of items).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Input>,
    /// Previous response ID for conversation chaining.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Additional instructions (developer/system message).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Available tools for model to call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    /// Tool choice configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    /// Metadata key-value pairs (max 16, keys max 64 chars, values max 512 chars).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    /// Sampling temperature (0-2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// Nucleus sampling parameter (0-1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Presence penalty for token diversity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    /// Frequency penalty for token diversity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    /// Maximum output tokens (min 16).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    /// Maximum tool calls allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u32>,
    /// Enable streaming SSE response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    /// Run in background.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    /// Store response for retrieval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    /// Allow parallel tool calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    /// Reasoning configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
    /// Text output configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextConfig>,
    /// Input truncation mode.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<Truncation>,
    /// Service tier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<ServiceTier>,
    /// Include options (e.g., "reasoning.encrypted_content").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<String>>,
    /// Number of top logprobs to return (0-20).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u8>,
    /// Safety identifier for abuse detection.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    /// Prompt cache key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

/// Input (string or array of items).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Input {
    Text(String),
    Items(Vec<InputItem>),
}

// ============================================================================
// Response Types
// ============================================================================

/// URL citation annotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UrlCitation {
    #[serde(rename = "type")]
    pub type_: String,
    pub url: String,
    pub title: String,
    pub start_index: u32,
    pub end_index: u32,
}

/// Log probability entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogProb {
    pub token: String,
    pub logprob: f64,
    pub bytes: Vec<u8>,
    #[serde(default)]
    pub top_logprobs: Vec<TopLogProb>,
}

/// Top log probability.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopLogProb {
    pub token: String,
    pub logprob: f64,
    pub bytes: Vec<u8>,
}

/// Token usage details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens_details: Option<InputTokensDetails>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens_details: Option<OutputTokensDetails>,
    /// Authoritative per-request cost in USD credits, returned by
    /// OpenAI-compatible gateways such as OpenRouter. Absent for direct OpenAI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// Input token breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputTokensDetails {
    /// Tokens served from cache.
    pub cached_tokens: u32,
}

/// Output token breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputTokensDetails {
    /// Tokens used for reasoning.
    pub reasoning_tokens: u32,
}

/// Incomplete response details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncompleteDetails {
    pub reason: String,
}

/// Output item (polymorphic).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutputItem {
    #[serde(rename = "message")]
    Message {
        id: String,
        status: ItemStatus,
        role: MessageRole,
        content: Vec<ContentPart>,
        /// Execution phase assigned by the model (e.g., "commentary", "final_answer").
        /// Must be preserved and sent back when replaying conversation history.
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        namespace: Option<String>,
        arguments: String,
        status: ItemStatus,
    },
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        id: String,
        call_id: String,
        output: FunctionCallOutput,
        status: ItemStatus,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        id: String,
        summary: Vec<ContentPart>,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ContentPart>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        encrypted_content: Option<String>,
    },
    #[serde(rename = "tool_search_call")]
    ToolSearchCall {
        execution: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        status: ItemStatus,
        arguments: Value,
    },
    #[serde(rename = "tool_search_output")]
    ToolSearchOutput {
        execution: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        call_id: Option<String>,
        status: ItemStatus,
        tools: Vec<Value>,
    },
}

/// Complete response resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseResource {
    pub id: String,
    pub object: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    pub status: ResponseStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<IncompleteDetails>,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default)]
    pub output: Vec<OutputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<Error>,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation: Option<Truncation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency_penalty: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_logprobs: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<Reasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

// ============================================================================
// Error Types
// ============================================================================

/// API error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

/// Streaming error payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

// ============================================================================
// Streaming Events
// ============================================================================

/// Streaming event wrapper (SSE data).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamingEvent {
    // Response lifecycle events
    #[serde(rename = "response.created")]
    ResponseCreated {
        sequence_number: u32,
        response: ResponseResource,
    },
    #[serde(rename = "response.queued")]
    ResponseQueued {
        sequence_number: u32,
        response: ResponseResource,
    },
    #[serde(rename = "response.in_progress")]
    ResponseInProgress {
        sequence_number: u32,
        response: ResponseResource,
    },
    #[serde(rename = "response.completed")]
    ResponseCompleted {
        sequence_number: u32,
        response: ResponseResource,
    },
    #[serde(rename = "response.failed")]
    ResponseFailed {
        sequence_number: u32,
        response: ResponseResource,
    },
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete {
        sequence_number: u32,
        response: ResponseResource,
    },

    // Output item events
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded {
        sequence_number: u32,
        output_index: u32,
        item: Option<OutputItem>,
    },
    #[serde(rename = "response.output_item.done")]
    OutputItemDone {
        sequence_number: u32,
        output_index: u32,
        item: Option<OutputItem>,
    },

    // Content part events
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        part: ContentPart,
    },
    #[serde(rename = "response.content_part.done")]
    ContentPartDone {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        part: ContentPart,
    },

    // Text delta events
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        delta: String,
        #[serde(default)]
        logprobs: Vec<LogProb>,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.output_text.done")]
    OutputTextDone {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        text: String,
        #[serde(default)]
        logprobs: Vec<LogProb>,
    },
    #[serde(rename = "response.output_text.annotation.added")]
    OutputTextAnnotationAdded {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        annotation_index: u32,
        annotation: Option<UrlCitation>,
    },

    // Refusal events
    #[serde(rename = "response.refusal.delta")]
    RefusalDelta {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        delta: String,
    },
    #[serde(rename = "response.refusal.done")]
    RefusalDone {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        refusal: String,
    },

    // Reasoning events
    #[serde(rename = "response.reasoning.delta")]
    ReasoningDelta {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.reasoning.done")]
    ReasoningDone {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        content_index: u32,
        text: String,
    },

    // Plaintext reasoning deltas as emitted by OpenAI-compatible gateways
    // (e.g. OpenRouter) for open reasoning models like NVIDIA Nemotron. These
    // carry the model's chain-of-thought directly (no encrypted artifact), so
    // they map to streaming thinking. Fields beyond `delta` are optional to stay
    // tolerant of gateway-to-gateway shape differences.
    #[serde(rename = "response.reasoning_text.delta")]
    ReasoningTextDelta {
        #[serde(default)]
        sequence_number: u32,
        #[serde(default)]
        item_id: String,
        #[serde(default)]
        output_index: u32,
        #[serde(default)]
        content_index: u32,
        delta: String,
    },

    // Reasoning summary events
    #[serde(rename = "response.reasoning_summary_part.added")]
    ReasoningSummaryPartAdded {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        summary_index: u32,
        part: ContentPart,
    },
    #[serde(rename = "response.reasoning_summary_part.done")]
    ReasoningSummaryPartDone {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        summary_index: u32,
        part: ContentPart,
    },
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryDelta {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        summary_index: u32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.reasoning_summary_text.done")]
    ReasoningSummaryDone {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        summary_index: u32,
        text: String,
    },

    // Function call events
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        delta: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        obfuscation: Option<String>,
    },
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone {
        sequence_number: u32,
        item_id: String,
        output_index: u32,
        arguments: String,
    },

    // Error event
    #[serde(rename = "error")]
    Error {
        sequence_number: u32,
        error: ErrorPayload,
    },
}

// ============================================================================
// Validation Constants
// ============================================================================

/// Maximum text content length (10MB).
pub const MAX_TEXT_LENGTH: usize = 10 * 1024 * 1024;
/// Maximum image URL length (20MB).
pub const MAX_IMAGE_URL_LENGTH: usize = 20 * 1024 * 1024;
/// Maximum file data length (32MB).
pub const MAX_FILE_DATA_LENGTH: usize = 32 * 1024 * 1024;
/// Maximum function name length.
pub const MAX_FUNCTION_NAME_LENGTH: usize = 64;
/// Minimum function name length.
pub const MIN_FUNCTION_NAME_LENGTH: usize = 1;
/// Maximum metadata keys.
pub const MAX_METADATA_KEYS: usize = 16;
/// Maximum metadata key length.
pub const MAX_METADATA_KEY_LENGTH: usize = 64;
/// Maximum metadata value length.
pub const MAX_METADATA_VALUE_LENGTH: usize = 512;
/// Minimum max_output_tokens.
pub const MIN_MAX_OUTPUT_TOKENS: u32 = 16;
/// Maximum top_logprobs.
pub const MAX_TOP_LOGPROBS: u8 = 20;

/// Validates a function name matches spec pattern: ^[a-zA-Z0-9_-]+$
pub fn validate_function_name(name: &str) -> bool {
    if name.len() < MIN_FUNCTION_NAME_LENGTH || name.len() > MAX_FUNCTION_NAME_LENGTH {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Validates metadata according to spec constraints.
pub fn validate_metadata(metadata: &HashMap<String, String>) -> bool {
    if metadata.len() > MAX_METADATA_KEYS {
        return false;
    }
    metadata.iter().all(|(k, v)| {
        k.chars().take(MAX_METADATA_KEY_LENGTH + 1).count() <= MAX_METADATA_KEY_LENGTH
            && v.chars().take(MAX_METADATA_VALUE_LENGTH + 1).count() <= MAX_METADATA_VALUE_LENGTH
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn input_constructors_emit_complete_literal_payloads() {
        for (item, expected) in [
            (
                InputItem::Message(MessageItem::user("Hello")),
                json!({"type":"message","role":"user","content":"Hello"}),
            ),
            (
                InputItem::Message(MessageItem::developer("Rules")),
                json!({"type":"message","role":"developer","content":"Rules"}),
            ),
            (
                InputItem::Message(MessageItem::assistant("Answer")),
                json!({"type":"message","role":"assistant","content":"Answer"}),
            ),
            (
                InputItem::FunctionCall(FunctionCallItem::new(
                    "call_123",
                    "get_weather",
                    r#"{"location":"NYC"}"#,
                )),
                json!({"type":"function_call","call_id":"call_123","name":"get_weather","arguments":"{\"location\":\"NYC\"}"}),
            ),
            (
                InputItem::FunctionCallOutput(FunctionCallOutputItem::new(
                    "call_123",
                    r#"{"temp": 72}"#,
                )),
                json!({"type":"function_call_output","call_id":"call_123","output":"{\"temp\": 72}"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(item).unwrap(), expected);
        }
    }

    #[test]
    fn request_items_use_discriminators_and_preserve_complete_payloads() {
        let items = json!([
            {"type":"message","id":"msg","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Answer"}]},
            {"type":"function_call","id":"fc","status":"in_progress","call_id":"call","name":"weather","arguments":"{\"city\":\"NYC\"}"},
            {"type":"function_call_output","id":"out","status":"completed","call_id":"call","output":[{"type":"input_text","text":"72"}]},
            {"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"Summary"}],"content":[{"type":"reasoning_text","text":"Reasoning"}],"encrypted_content":"opaque"},
            {"type":"item_reference","id":"msg_123"}
        ]);
        let request = json!({"model":"model","input":items});
        let parsed: CreateResponseBody = serde_json::from_value(request.clone()).unwrap();
        let Some(Input::Items(items)) = &parsed.input else {
            panic!("expected item array")
        };
        assert!(matches!(
            items.as_slice(),
            [
                InputItem::Message(_),
                InputItem::FunctionCall(_),
                InputItem::FunctionCallOutput(_),
                InputItem::Reasoning(_),
                InputItem::Reference(_)
            ]
        ));
        assert_eq!(serde_json::to_value(parsed).unwrap(), request);
        for invalid in [
            json!({"type":"function_call","id":"fc"}),
            json!({"type":"unknown","id":"id"}),
            json!({"id":"id"}),
            json!({"type":"message","id":"msg","content":"missing role"}),
        ] {
            assert!(
                serde_json::from_value::<InputItem>(invalid.clone()).is_err(),
                "{invalid}"
            );
        }
        let simple = json!({"input":"Hello"});
        assert_eq!(
            serde_json::to_value(
                serde_json::from_value::<CreateResponseBody>(simple.clone()).unwrap()
            )
            .unwrap(),
            simple
        );
    }

    #[test]
    fn lifecycle_statuses_decode_from_response_snapshots_and_emit_protocol_values() {
        for (wire, expected) in [
            ("in_progress", ResponseStatus::InProgress),
            ("completed", ResponseStatus::Completed),
            ("failed", ResponseStatus::Failed),
            ("cancelled", ResponseStatus::Cancelled),
            ("queued", ResponseStatus::Queued),
            ("incomplete", ResponseStatus::Incomplete),
            ("provider_extension", ResponseStatus::Unknown),
        ] {
            let value = json!({"type":"response.in_progress","sequence_number":7,"response":{"id":"resp","object":"response","created_at":1,"model":"model","status":wire,"output":[],"tools":[]}});
            let event: StreamingEvent = serde_json::from_value(value.clone()).unwrap();
            let StreamingEvent::ResponseInProgress {
                sequence_number,
                response,
            } = &event
            else {
                panic!("wrong event")
            };
            assert_eq!(*sequence_number, 7);
            assert_eq!(response.status, expected);
            let mut encoded = value;
            if expected == ResponseStatus::Unknown {
                encoded["response"]["status"] = json!("unknown");
            }
            assert_eq!(serde_json::to_value(event).unwrap(), encoded);
        }
    }

    #[test]
    fn streaming_deltas_preserve_routing_indexes_text_and_optional_fields() {
        let logprobs = json!([{"token":"hi","logprob":-0.25,"bytes":[104,105],"top_logprobs":[{"token":"hey","logprob":-0.5,"bytes":[104,101,121]}]}]);
        for wire in [
            json!({"type":"response.output_text.delta","sequence_number":5,"item_id":"msg_123","output_index":2,"content_index":3,"delta":"Hello","logprobs":logprobs,"obfuscation":"opaque"}),
            json!({"type":"response.reasoning_text.delta","sequence_number":3,"item_id":"rs_tmp","output_index":4,"content_index":2,"delta":"User asks"}),
        ] {
            let parsed: StreamingEvent = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
        }
        let minimal: StreamingEvent =
            serde_json::from_value(json!({"type":"response.reasoning_text.delta","delta":"text"}))
                .unwrap();
        assert_eq!(
            serde_json::to_value(minimal).unwrap(),
            json!({"type":"response.reasoning_text.delta","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"delta":"text"})
        );
        let no_logprobs: StreamingEvent = serde_json::from_value(json!({"type":"response.output_text.delta","sequence_number":5,"item_id":"msg","output_index":0,"content_index":0,"delta":"text"})).unwrap();
        assert_eq!(
            serde_json::to_value(no_logprobs).unwrap(),
            json!({"type":"response.output_text.delta","sequence_number":5,"item_id":"msg","output_index":0,"content_index":0,"delta":"text","logprobs":[]})
        );
    }

    #[test]
    fn errors_preserve_full_payload_and_omit_absent_options() {
        let error = ErrorPayload {
            type_: "invalid_request_error".into(),
            code: Some("model_not_found".into()),
            message: "Model not found".into(),
            param: Some("model".into()),
            headers: Some(HashMap::from([("retry-after".into(), "5".into())])),
        };
        let expected = json!({"type":"invalid_request_error","code":"model_not_found","message":"Model not found","param":"model","headers":{"retry-after":"5"}});
        assert_eq!(serde_json::to_value(error).unwrap(), expected);
        let wire = json!({"type":"error","sequence_number":8,"error":expected});
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<StreamingEvent>(wire.clone()).unwrap())
                .unwrap(),
            wire
        );
        let minimal = json!({"type":"server_error","message":"Failed"});
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<ErrorPayload>(minimal.clone()).unwrap())
                .unwrap(),
            minimal
        );
        assert_eq!(
            Error {
                code: "failed".into(),
                message: "Problem".into()
            }
            .to_string(),
            "failed: Problem"
        );
    }

    #[test]
    fn function_names_enforce_literal_length_and_ascii_boundaries() {
        for name in [
            "a".into(),
            "a".repeat(64),
            "get_weather".into(),
            "get-weather".into(),
            "getWeather123".into(),
            "_-09AZaz".into(),
        ] {
            assert!(validate_function_name(&name), "{name}");
        }
        for name in [
            "".into(),
            "a".repeat(65),
            "get weather".into(),
            "get.weather".into(),
            "é".into(),
            "a\n".into(),
            "a/b".into(),
        ] {
            assert!(!validate_function_name(&name), "{name:?}");
        }
    }

    #[test]
    fn metadata_enforces_pair_and_unicode_character_boundaries() {
        for count in [0, 1, 16, 17] {
            let values = (0..count)
                .map(|i| (format!("key{i}"), "value".into()))
                .collect();
            assert_eq!(validate_metadata(&values), count <= 16, "{count}");
        }
        for unit in ["a", "é", "🦀"] {
            for (key_len, value_len, valid) in [
                (0, 0, true),
                (64, 512, true),
                (65, 512, false),
                (64, 513, false),
            ] {
                assert_eq!(
                    validate_metadata(&HashMap::from([(
                        unit.repeat(key_len),
                        unit.repeat(value_len)
                    )])),
                    valid,
                    "{unit}: {key_len}/{value_len}"
                );
            }
        }
    }

    #[test]
    fn content_variants_preserve_full_multimodal_payloads() {
        for wire in [
            json!({"type":"input_text","text":"Hello"}),
            json!({"type":"input_image","image_url":"data:image/png;base64,aGk=","detail":"high"}),
            json!({"type":"input_image"}),
            json!({"type":"input_audio","input_audio":{"data":"aGk=","format":"wav"}}),
            json!({"type":"input_file","filename":"note.txt","file_data":"aGk=","file_url":"https://example.com/file"}),
            json!({"type":"input_video","video_url":"https://example.com/video"}),
            json!({"type":"output_text","text":"answer","annotations":[{"type":"url_citation","url":"https://example.com","title":"Source","start_index":0,"end_index":6}],"logprobs":[{"token":"answer","logprob":-0.5,"bytes":[97],"top_logprobs":[]}]}),
            json!({"type":"text","text":"plain"}),
            json!({"type":"reasoning_text","text":"thought"}),
            json!({"type":"summary_text","text":"summary"}),
            json!({"type":"refusal","refusal":"Cannot"}),
        ] {
            assert_eq!(
                serde_json::to_value(serde_json::from_value::<ContentPart>(wire.clone()).unwrap())
                    .unwrap(),
                wire
            );
        }
        assert_eq!(
            serde_json::to_value(InputTextContent::new("Hello")).unwrap(),
            json!({"type":"input_text","text":"Hello"})
        );
        assert_eq!(
            serde_json::to_value(InputImageContent::new("https://example.com/image")).unwrap(),
            json!({"type":"input_image","image_url":"https://example.com/image"})
        );
    }

    #[test]
    fn function_tools_and_choices_preserve_schema_strictness_and_omission() {
        let schema = json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false});
        let tool = Tool::Function {
            name: "get_weather".into(),
            description: Some("Get weather".into()),
            parameters: Some(schema.clone()),
            strict: Some(true),
        };
        let wire = json!({"type":"function","name":"get_weather","description":"Get weather","parameters":schema,"strict":true});
        assert_eq!(serde_json::to_value(tool).unwrap(), wire);
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<Tool>(wire.clone()).unwrap()).unwrap(),
            wire
        );
        let built = FunctionTool::new("get_weather")
            .with_description("Get weather")
            .with_parameters(schema);
        let mut expected = wire;
        expected.as_object_mut().unwrap().remove("strict");
        assert_eq!(serde_json::to_value(built).unwrap(), expected);
        assert_eq!(
            serde_json::to_value(FunctionTool::new("minimal")).unwrap(),
            json!({"type":"function","name":"minimal"})
        );
        for wire in [
            json!("auto"),
            json!("none"),
            json!("required"),
            json!({"type":"function","name":"get_weather"}),
            json!({"type":"allowed_tools","mode":"required","tools":[{"type":"function","name":"get_weather"}]}),
        ] {
            assert_eq!(
                serde_json::to_value(serde_json::from_value::<ToolChoice>(wire.clone()).unwrap())
                    .unwrap(),
                wire
            );
        }
    }

    #[test]
    fn usage_preserves_complete_counts_details_and_optional_gateway_cost() {
        for wire in [
            json!({"input_tokens":194,"output_tokens":2,"total_tokens":196,"cost":0.00095,"input_tokens_details":{"cached_tokens":180},"output_tokens_details":{"reasoning_tokens":1}}),
            json!({"input_tokens":10,"output_tokens":5,"total_tokens":15}),
            json!({"input_tokens":0,"output_tokens":0,"total_tokens":0,"cost":0.0}),
        ] {
            let usage: Usage = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(usage.cost, wire.get("cost").and_then(Value::as_f64));
            assert_eq!(serde_json::to_value(usage).unwrap(), wire);
        }
    }
    #[test]
    fn hosted_tool_search_preserves_complete_namespace_and_call_payloads() {
        let json = r#"{
            "id": "resp_123",
            "object": "response",
            "created_at": 1780000000,
            "status": "completed",
            "model": "gpt-5.5",
            "output": [
                {
                    "type": "tool_search_call",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "arguments": { "paths": ["Math"] }
                },
                {
                    "type": "tool_search_output",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "tools": [
                        {
                            "type": "namespace",
                            "name": "Math",
                            "description": "Tools for Math",
                            "tools": [
                                {
                                    "type": "function",
                                    "name": "add",
                                    "description": "Add numbers.",
                                    "defer_loading": true,
                                    "parameters": {
                                        "type": "object",
                                        "properties": {
                                            "a": { "type": "number" },
                                            "b": { "type": "number" }
                                        },
                                        "required": ["a", "b"],
                                        "additionalProperties": false
                                    }
                                }
                            ]
                        }
                    ]
                },
                {
                    "type": "function_call",
                    "id": "fc_123",
                    "call_id": "call_123",
                    "name": "add",
                    "namespace": "Math",
                    "arguments": "{\"a\":7,\"b\":3}",
                    "status": "completed"
                }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "total_tokens": 15
            }
        }"#;

        let response: ResponseResource = serde_json::from_str(json).unwrap();
        let mut expected: Value = serde_json::from_str(json).unwrap();
        expected["output"][0]
            .as_object_mut()
            .unwrap()
            .remove("call_id");
        expected["output"][1]
            .as_object_mut()
            .unwrap()
            .remove("call_id");
        expected["tools"] = json!([]);
        assert_eq!(serde_json::to_value(response).unwrap(), expected);
    }
}
