//! Serde shapes for the OpenResponses request and its input items.

// Open Responses Protocol Driver
//
// Implementation of the Open Responses specification (https://www.openresponses.org/)
// an open-source, vendor-neutral API standard for multi-provider LLM interfaces.
//
// Rate limit handling: On 429 errors, the driver automatically retries with
// exponential backoff, respecting x-ratelimit-reset-* and retry-after headers.
// Retry metadata is included in the response for observability.
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

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::compact::{CompactContent, CompactContentPart, CompactOutputItem};
use crate::openresponses_types::{self as types};

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesRequest {
    pub(crate) model: String,
    pub(crate) input: Vec<ResponsesInputItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) previous_response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_output_tokens: Option<u32>,
    pub(crate) stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<ResponsesTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reasoning: Option<ResponsesReasoning>,
    /// Metadata for tracking API usage (up to 16 key-value pairs).
    /// Useful for correlating requests with session_id, agent_id, org_id, etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<std::collections::HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_cache_key: Option<String>,
    /// Request-level parallel tool calling preference (EVE-598). Omitted when
    /// `None` to preserve the provider default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parallel_tool_calls: Option<bool>,
    /// Speed selector: OpenAI service tier ("flex", "default", "priority").
    /// Omitted when `None` so the provider keeps its default ("auto") routing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) service_tier: Option<String>,
    /// Text output controls, currently just `verbosity`. Omitted when there is
    /// nothing to configure so the provider keeps its default output length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<ResponsesText>,
    /// Opt-in response fields. `reasoning.encrypted_content` is what makes
    /// reasoning replayable without server-side state: without it the API
    /// returns reasoning items carrying no payload, so a stateless follow-up
    /// (after compaction, a model switch, or router failover) silently loses
    /// the reasoning chain. Omitted when there is nothing to include.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) include: Option<Vec<String>>,
}

/// `text` request block for the Responses API. Verbosity ("low"/"medium"/"high")
/// controls output length independently of reasoning effort.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesText {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) verbosity: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ResponsesReasoning {
    pub(crate) effort: String,
    /// Request reasoning summary to get thinking tokens streamed back.
    /// Without this, reasoning happens internally but tokens are not exposed.
    pub(crate) summary: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum ResponsesInputItem {
    ConfigurationUpdate {
        r#type: String,
        reasoning: crate::compact::ConfigurationReasoning,
    },
    ProviderItem(Value),
    Message {
        r#type: String,
        role: String,
        content: ResponsesContent,
        /// Execution phase for assistant messages (e.g., "in_progress", "completed").
        /// Helps GPT-5.x distinguish intermediate working commentary from final answers.
        /// Only set on assistant messages; must be preserved when replaying history.
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<String>,
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
    /// Reasoning item for o-series and GPT-5 models
    /// Contains encrypted reasoning content that preserves reasoning context across turns
    /// (similar to Anthropic's thinking signature).
    ///
    /// Stateless requests must re-send prior `Reasoning` items in `input` so the model can
    /// continue from them. Stateful continuations (those carrying `previous_response_id`)
    /// rely on OpenAI to hold the prior reasoning chain server-side, so [`compute_delta_input_items`]
    /// intentionally drops `Reasoning` items that belong to a prior assistant turn — re-sending
    /// them alongside `previous_response_id` would violate the no-mixing invariant.
    Reasoning {
        r#type: String,
        /// Unique ID for this reasoning item
        id: String,
        /// Encrypted reasoning content (required for multi-turn conversations)
        encrypted_content: String,
        /// Provider-curated summary segments. The API rejects a reasoning input
        /// item without this key (`400 … missing required field \`summary\``),
        /// so it is always serialized — an empty list when the artifact carried
        /// no summary, which is the common case since summaries arrive only
        /// when the request asked for them.
        summary: Vec<types::ContentPart>,
    },
    /// Opaque native context returned by `/responses/compact`.
    Compaction {
        r#type: String,
        encrypted_content: String,
    },
}

impl From<&CompactOutputItem> for ResponsesInputItem {
    fn from(item: &CompactOutputItem) -> Self {
        match item {
            CompactOutputItem::ProviderItem(item) => Self::ProviderItem(item.clone()),
            CompactOutputItem::Message { role, content } => Self::Message {
                r#type: "message".to_string(),
                role: role.clone(),
                content: match content {
                    CompactContent::Text(text) => ResponsesContent::Text(text.clone()),
                    CompactContent::Parts(parts) => ResponsesContent::Parts(
                        parts
                            .iter()
                            .map(|part| match part {
                                CompactContentPart::InputText { text } => {
                                    ResponsesContentPart::InputText {
                                        r#type: "input_text".to_string(),
                                        text: text.clone(),
                                    }
                                }
                                CompactContentPart::InputImage { image_url } => {
                                    ResponsesContentPart::InputImage {
                                        r#type: "input_image".to_string(),
                                        image_url: image_url.clone(),
                                    }
                                }
                                CompactContentPart::InputFile {
                                    file_data,
                                    filename,
                                } => ResponsesContentPart::InputFile {
                                    r#type: "input_file".to_string(),
                                    input_file: ResponsesInputFile {
                                        file_data: Some(file_data.clone()),
                                        file_url: None,
                                        filename: filename.clone(),
                                    },
                                },
                            })
                            .collect(),
                    ),
                },
                phase: None,
            },
            CompactOutputItem::Compaction { encrypted_content } => Self::Compaction {
                r#type: "compaction".to_string(),
                encrypted_content: encrypted_content.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(crate) enum ResponsesContent {
    Text(String),
    Parts(Vec<ResponsesContentPart>),
}

// The "Input" prefix matches OpenAI's Responses API naming convention
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum ResponsesContentPart {
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
    InputFile {
        r#type: String,
        input_file: ResponsesInputFile,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ResponsesInputAudio {
    pub(crate) data: String,
    pub(crate) format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ResponsesInputFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) file_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) file_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) filename: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub(crate) enum ResponsesTool {
    /// Standard function tool (or deferred function with defer_loading)
    Function {
        r#type: String,
        name: String,
        description: String,
        parameters: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        strict: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        defer_loading: Option<bool>,
    },
    /// Namespace grouping for tool_search (groups related deferred tools)
    Namespace {
        r#type: String,
        name: String,
        description: String,
        tools: Vec<ResponsesTool>,
    },
    /// Activates tool_search on the request
    ToolSearch { r#type: String },
}

// ============================================================================
// Tests
// ============================================================================
