//! Transport-neutral native compaction request and response contracts.

use crate::driver_registry::{LlmContentPart, LlmMessage, LlmMessageContent, LlmMessageRole};
use serde::{Deserialize, Serialize};

/// Request body for the Open Responses `/v1/responses/compact` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct CompactRequest {
    /// Explicit Responses compaction, required for configuration-update history.
    /// This field is local routing state and never sent to the provider.
    #[serde(skip)]
    pub reasoning_state: Option<crate::reasoning_updates::ReasoningState>,
    /// Model used for compaction.
    pub model: String,
    /// Current conversation items to compact.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<CompactInputItem>,
    /// Previous response identifier, as an alternative to a complete input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Optional system instructions for this compaction request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Transport-neutral input item accepted by native conversation compaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompactInputItem {
    #[serde(rename = "configuration_update")]
    ConfigurationUpdate { reasoning: ConfigurationReasoning },
    /// User, assistant, or developer message.
    #[serde(rename = "message")]
    Message {
        /// Protocol role name.
        role: String,
        /// Message content.
        content: CompactContent,
    },
    /// Function call emitted by the assistant.
    #[serde(rename = "function_call")]
    FunctionCall {
        /// Provider-visible call identifier.
        call_id: String,
        /// Function name.
        name: String,
        /// JSON-encoded arguments.
        arguments: String,
    },
    /// Output corresponding to a function call.
    #[serde(rename = "function_call_output")]
    FunctionCallOutput {
        /// Provider-visible call identifier.
        call_id: String,
        /// String-encoded function output.
        output: String,
    },
    /// Opaque output from an earlier compaction pass.
    #[serde(rename = "compaction")]
    Compaction {
        /// Provider-produced encrypted latent context.
        encrypted_content: String,
    },
    /// Provider-native reasoning or other retained items, replayed verbatim.
    #[serde(untagged)]
    ProviderItem(serde_json::Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationReasoning {
    pub effort: crate::model::ReasoningEffort,
}

impl From<&CompactOutputItem> for CompactInputItem {
    fn from(item: &CompactOutputItem) -> Self {
        match item {
            CompactOutputItem::Message { role, content } => Self::Message {
                role: role.clone(),
                content: content.clone(),
            },
            CompactOutputItem::Compaction { encrypted_content } => Self::Compaction {
                encrypted_content: encrypted_content.clone(),
            },
            CompactOutputItem::ProviderItem(item) => Self::ProviderItem(item.clone()),
        }
    }
}

/// Text or multipart content carried by a compact message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompactContent {
    /// Plain text content.
    Text(String),
    /// Ordered text and image parts.
    Parts(Vec<CompactContentPart>),
}

/// One multipart content item in a compact message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompactContentPart {
    /// Text input.
    #[serde(rename = "input_text")]
    InputText {
        /// Text value.
        text: String,
    },
    /// Image input.
    #[serde(rename = "input_image")]
    InputImage {
        /// Image URL or data URL.
        image_url: String,
    },
}

/// Decoded response from native conversation compaction.
#[derive(Debug, Clone, Deserialize)]
pub struct CompactResponse {
    /// Ordered compacted output items.
    pub output: Vec<CompactOutputItem>,
    /// Optional provider token and cost accounting.
    pub usage: Option<CompactUsage>,
}

/// Output item returned by native conversation compaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum CompactOutputItem {
    /// User message preserved verbatim.
    #[serde(rename = "message")]
    Message {
        /// Protocol role name.
        role: String,
        /// Message content.
        content: CompactContent,
    },
    /// Opaque replacement for earlier assistant/tool context.
    #[serde(rename = "compaction")]
    Compaction {
        /// Provider-produced encrypted latent context.
        encrypted_content: String,
    },
    /// Preserve complete explicit-compaction output, including reasoning and
    /// any retained native items, instead of dropping unknown semantic state.
    #[serde(untagged)]
    ProviderItem(serde_json::Value),
}

/// Provider-reported accounting for one compact request.
#[derive(Debug, Clone, Deserialize)]
pub struct CompactUsage {
    /// Input tokens processed.
    pub input_tokens: Option<u32>,
    /// Output tokens produced.
    pub output_tokens: Option<u32>,
    /// Total tokens billed.
    pub total_tokens: Option<u32>,
    /// Authoritative provider-reported per-request cost in USD, when supplied.
    #[serde(default)]
    pub cost: Option<f64>,
}

impl CompactInputItem {
    /// Convert one provider-neutral message to its ordered compact input items.
    ///
    /// Assistant tool calls expand to function-call items and tool messages
    /// become function-call outputs.
    pub fn from_llm_message(msg: &LlmMessage) -> Vec<Self> {
        let mut items = Vec::new();
        if let Some(effort) = msg.configuration_update {
            items.push(Self::ConfigurationUpdate {
                reasoning: ConfigurationReasoning { effort },
            });
        }
        for part in &msg.reasoning {
            if part.provider == "openai"
                && let (Some(id), Some(encrypted)) = (&part.item_id, &part.encrypted)
            {
                let summary = match &part.text {
                    Some(crate::reasoning::ReasoningText::Summary { parts }) => parts
                        .iter()
                        .map(|text| serde_json::json!({"type": "summary_text", "text": text}))
                        .collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                items.push(Self::ProviderItem(serde_json::json!({
                    "type": "reasoning", "id": id, "encrypted_content": encrypted,
                    "summary": summary,
                })));
            }
        }
        let role = match msg.role {
            LlmMessageRole::System => "developer",
            LlmMessageRole::User => "user",
            LlmMessageRole::Assistant => "assistant",
            LlmMessageRole::Tool => "tool",
        };

        if msg.role == LlmMessageRole::Tool
            && let Some(tool_call_id) = &msg.tool_call_id
        {
            let output = match &msg.content {
                LlmMessageContent::Text(text) => text.clone(),
                LlmMessageContent::Parts(parts) => parts
                    .iter()
                    .filter_map(|part| match part {
                        LlmContentPart::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(""),
            };
            items.push(Self::FunctionCallOutput {
                call_id: tool_call_id.clone(),
                output,
            });
            return items;
        }

        let content = Self::content_from_llm_message(msg);
        let has_content = match &content {
            CompactContent::Text(text) => !text.is_empty(),
            CompactContent::Parts(parts) => !parts.is_empty(),
        };
        if has_content || msg.tool_calls.is_none() {
            let message = Self::Message {
                role: role.to_string(),
                content,
            };
            if msg.role == LlmMessageRole::Assistant
                && let Some(phase) = msg.phase
            {
                let mut value = serde_json::json!(message);
                value["phase"] = serde_json::json!(phase.as_provider_str());
                items.push(Self::ProviderItem(value));
            } else {
                items.push(message);
            }
        }

        if msg.role == LlmMessageRole::Assistant
            && let Some(tool_calls) = &msg.tool_calls
        {
            items.extend(tool_calls.iter().map(|call| Self::FunctionCall {
                call_id: call.id.clone(),
                name: call.name.clone(),
                arguments: call.arguments.to_string(),
            }));
        }
        items
    }

    pub fn is_assistant_item(&self) -> bool {
        match self {
            Self::FunctionCall { .. } => true,
            Self::Message { role, .. } => role == "assistant",
            Self::ProviderItem(value) => {
                value["role"] == "assistant"
                    || value["type"] == "reasoning"
                    || value["type"] == "function_call"
            }
            _ => false,
        }
    }

    fn content_from_llm_message(msg: &LlmMessage) -> CompactContent {
        match &msg.content {
            LlmMessageContent::Text(text) => CompactContent::Text(text.clone()),
            LlmMessageContent::Parts(parts) => {
                let compact_parts = parts
                    .iter()
                    .filter_map(|part| match part {
                        LlmContentPart::Text { text } => {
                            Some(CompactContentPart::InputText { text: text.clone() })
                        }
                        LlmContentPart::Image { url } => Some(CompactContentPart::InputImage {
                            image_url: url.clone(),
                        }),
                        LlmContentPart::Audio { .. } => None,
                    })
                    .collect::<Vec<_>>();
                if compact_parts.len() == 1
                    && let CompactContentPart::InputText { text } = &compact_parts[0]
                {
                    return CompactContent::Text(text.clone());
                }
                CompactContent::Parts(compact_parts)
            }
        }
    }
}

/// Convert provider-neutral messages into ordered native compact input items.
pub fn messages_to_compact_input(messages: &[LlmMessage]) -> Vec<CompactInputItem> {
    messages
        .iter()
        .flat_map(CompactInputItem::from_llm_message)
        .collect()
}
