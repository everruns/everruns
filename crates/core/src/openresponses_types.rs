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

// ----------------------------------------------------------------------------
// Enums
// ----------------------------------------------------------------------------

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
#[serde(rename_all = "lowercase")]
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

// ----------------------------------------------------------------------------
// Input Content Types
// ----------------------------------------------------------------------------

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

// ----------------------------------------------------------------------------
// Input Items
// ----------------------------------------------------------------------------

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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InputItem {
    Message(MessageItem),
    FunctionCall(FunctionCallItem),
    FunctionCallOutput(FunctionCallOutputItem),
    Reasoning(ReasoningItem),
    Reference(ItemReference),
}

// ----------------------------------------------------------------------------
// Tools
// ----------------------------------------------------------------------------

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

// ----------------------------------------------------------------------------
// Reasoning Configuration
// ----------------------------------------------------------------------------

/// Reasoning configuration for o-series and gpt-5 models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReasoningSummary>,
}

// ----------------------------------------------------------------------------
// Text Configuration
// ----------------------------------------------------------------------------

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

// ----------------------------------------------------------------------------
// Request Body
// ----------------------------------------------------------------------------

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

// ----------------------------------------------------------------------------
// Response Types
// ----------------------------------------------------------------------------

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
    },
    #[serde(rename = "function_call")]
    FunctionCall {
        id: String,
        call_id: String,
        name: String,
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

// ----------------------------------------------------------------------------
// Error Types
// ----------------------------------------------------------------------------

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

// ----------------------------------------------------------------------------
// Streaming Events
// ----------------------------------------------------------------------------

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

// ----------------------------------------------------------------------------
// Validation Constants
// ----------------------------------------------------------------------------

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
    metadata
        .iter()
        .all(|(k, v)| k.len() <= MAX_METADATA_KEY_LENGTH && v.len() <= MAX_METADATA_VALUE_LENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_item_serialization() {
        let msg = MessageItem::user("Hello");
        let json = serde_json::to_value(&msg).unwrap();
        assert_eq!(json["type"], "message");
        assert_eq!(json["role"], "user");
        assert_eq!(json["content"], "Hello");
    }

    #[test]
    fn test_function_call_item_serialization() {
        let call = FunctionCallItem::new("call_123", "get_weather", r#"{"location":"NYC"}"#);
        let json = serde_json::to_value(&call).unwrap();
        assert_eq!(json["type"], "function_call");
        assert_eq!(json["call_id"], "call_123");
        assert_eq!(json["name"], "get_weather");
    }

    #[test]
    fn test_function_call_output_serialization() {
        let output = FunctionCallOutputItem::new("call_123", r#"{"temp": 72}"#);
        let json = serde_json::to_value(&output).unwrap();
        assert_eq!(json["type"], "function_call_output");
        assert_eq!(json["call_id"], "call_123");
    }

    #[test]
    fn test_streaming_event_deserialization() {
        let json = r#"{"type":"response.output_text.delta","sequence_number":5,"item_id":"msg_123","output_index":0,"content_index":0,"delta":"Hello","logprobs":[]}"#;
        let event: StreamingEvent = serde_json::from_str(json).unwrap();
        match event {
            StreamingEvent::OutputTextDelta { delta, .. } => assert_eq!(delta, "Hello"),
            _ => panic!("Wrong event type"),
        }
    }

    #[test]
    fn test_error_payload_serialization() {
        let error = ErrorPayload {
            type_: "invalid_request_error".to_string(),
            code: Some("model_not_found".to_string()),
            message: "Model not found".to_string(),
            param: Some("model".to_string()),
            headers: None,
        };
        let json = serde_json::to_value(&error).unwrap();
        assert_eq!(json["type"], "invalid_request_error");
    }

    #[test]
    fn test_validate_function_name() {
        assert!(validate_function_name("get_weather"));
        assert!(validate_function_name("get-weather"));
        assert!(validate_function_name("getWeather123"));
        assert!(!validate_function_name("")); // too short
        assert!(!validate_function_name("a".repeat(65).as_str())); // too long
        assert!(!validate_function_name("get weather")); // has space
        assert!(!validate_function_name("get.weather")); // has dot
    }

    #[test]
    fn test_validate_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());
        assert!(validate_metadata(&metadata));

        // Too many keys
        for i in 0..20 {
            metadata.insert(format!("key{}", i), "value".to_string());
        }
        assert!(!validate_metadata(&metadata));
    }

    #[test]
    fn test_content_part_serialization() {
        let part = ContentPart::InputText {
            text: "Hello".to_string(),
        };
        let json = serde_json::to_value(&part).unwrap();
        assert_eq!(json["type"], "input_text");
        assert_eq!(json["text"], "Hello");
    }

    #[test]
    fn test_tool_serialization() {
        let tool = Tool::Function {
            name: "get_weather".to_string(),
            description: Some("Get weather".to_string()),
            parameters: Some(serde_json::json!({"type": "object"})),
            strict: Some(true),
        };
        let json = serde_json::to_value(&tool).unwrap();
        assert_eq!(json["type"], "function");
        assert_eq!(json["name"], "get_weather");
    }
}
