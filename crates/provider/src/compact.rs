//! Transport-neutral native compaction request and response contracts.

use crate::driver_registry::{LlmContentPart, LlmMessage, LlmMessageContent, LlmMessageRole};
use serde::{Deserialize, Serialize};

/// Request body for the Open Responses `/v1/responses/compact` endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct CompactRequest {
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
            items.push(Self::Message {
                role: role.to_string(),
                content,
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_wire_covers_every_item_and_omits_absent_continuation_fields() {
        let request = CompactRequest {
            model: "model".into(),
            input: vec![
                CompactInputItem::Message {
                    role: "user".into(),
                    content: CompactContent::Text("hello".into()),
                },
                CompactInputItem::Message {
                    role: "assistant".into(),
                    content: CompactContent::Parts(vec![
                        CompactContentPart::InputText {
                            text: "image".into(),
                        },
                        CompactContentPart::InputImage {
                            image_url: "data:image/png;base64,abc".into(),
                        },
                    ]),
                },
                CompactInputItem::FunctionCall {
                    call_id: "call-1".into(),
                    name: "lookup".into(),
                    arguments: r#"{"city":"NYC"}"#.into(),
                },
                CompactInputItem::FunctionCallOutput {
                    call_id: "call-1".into(),
                    output: "result".into(),
                },
                CompactInputItem::Compaction {
                    encrypted_content: "opaque".into(),
                },
            ],
            previous_response_id: None,
            instructions: Some("rules".into()),
        };
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({"model":"model","instructions":"rules","input":[
                {"type":"message","role":"user","content":"hello"},
                {"type":"message","role":"assistant","content":[{"type":"input_text","text":"image"},{"type":"input_image","image_url":"data:image/png;base64,abc"}]},
                {"type":"function_call","call_id":"call-1","name":"lookup","arguments":"{\"city\":\"NYC\"}"},
                {"type":"function_call_output","call_id":"call-1","output":"result"},
                {"type":"compaction","encrypted_content":"opaque"}
            ]})
        );
        assert_eq!(
            serde_json::to_value(CompactRequest {
                model: "model".into(),
                input: vec![],
                previous_response_id: Some("resp-previous".into()),
                instructions: None
            })
            .unwrap(),
            json!({"model":"model","previous_response_id":"resp-previous"})
        );
    }

    #[test]
    fn response_decoding_preserves_opaque_and_multipart_replay_with_optional_usage() {
        let output = json!([
            {"type":"message","role":"user","content":"hello"},
            {"type":"message","role":"user","content":[{"type":"input_text","text":"see"},{"type":"input_image","image_url":"https://images.example/a.png"}]},
            {"type":"compaction","encrypted_content":"opaque-secret"}
        ]);
        let response: CompactResponse = serde_json::from_value(json!({"output":output,"usage":{"input_tokens":100,"output_tokens":50,"total_tokens":150,"cost":0.04}})).unwrap();
        assert_eq!(serde_json::to_value(&response.output).unwrap(), output);
        let replay: Vec<_> = response.output.iter().map(CompactInputItem::from).collect();
        assert_eq!(serde_json::to_value(replay).unwrap(), output);
        let usage = response.usage.unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens,
                usage.cost
            ),
            (Some(100), Some(50), Some(150), Some(0.04))
        );
        let minimal: CompactResponse = serde_json::from_value(json!({"output":[]})).unwrap();
        assert!(minimal.output.is_empty());
        assert!(minimal.usage.is_none());
        let sparse: CompactResponse =
            serde_json::from_value(json!({"output":[],"usage":{"input_tokens":9}})).unwrap();
        let usage = sparse.usage.unwrap();
        assert_eq!(
            (
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens,
                usage.cost
            ),
            (Some(9), None, None, None)
        );
        for invalid in [
            json!({"type":"compaction"}),
            json!({"type":"unknown","encrypted_content":"x"}),
            json!({"type":"message","role":"user"}),
        ] {
            assert!(
                serde_json::from_value::<CompactOutputItem>(invalid.clone()).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn message_conversion_keeps_roles_call_order_and_supported_content() {
        let mut assistant = LlmMessage::text(LlmMessageRole::Assistant, "checking");
        assistant.tool_calls = Some(vec![crate::tool_types::ToolCall {
            id: "call-1".into(),
            name: "lookup".into(),
            arguments: json!({"q":1}),
        }]);
        let mut result = LlmMessage::parts(
            LlmMessageRole::Tool,
            vec![
                LlmContentPart::text("do"),
                LlmContentPart::image("https://images.example/ignored.png"),
                LlmContentPart::text("ne"),
            ],
        );
        result.tool_call_id = Some("call-1".into());
        let input = messages_to_compact_input(&[
            LlmMessage::text(LlmMessageRole::System, "rules"),
            LlmMessage::parts(
                LlmMessageRole::User,
                vec![
                    LlmContentPart::text("see"),
                    LlmContentPart::image("https://images.example/a.png"),
                    LlmContentPart::Audio {
                        url: "data:audio/wav;base64,aA==".into(),
                    },
                ],
            ),
            assistant,
            result,
            LlmMessage::parts(
                LlmMessageRole::User,
                vec![LlmContentPart::text("only text")],
            ),
        ]);
        assert_eq!(
            serde_json::to_value(input).unwrap(),
            json!([
                {"type":"message","role":"developer","content":"rules"},
                {"type":"message","role":"user","content":[{"type":"input_text","text":"see"},{"type":"input_image","image_url":"https://images.example/a.png"}]},
                {"type":"message","role":"assistant","content":"checking"},
                {"type":"function_call","call_id":"call-1","name":"lookup","arguments":"{\"q\":1}"},
                {"type":"function_call_output","call_id":"call-1","output":"done"},
                {"type":"message","role":"user","content":"only text"}
            ])
        );
        let mut calls_only = LlmMessage::text(LlmMessageRole::Assistant, "");
        calls_only.tool_calls = Some(vec![crate::tool_types::ToolCall {
            id: "call-2".into(),
            name: "clock".into(),
            arguments: json!({}),
        }]);
        assert_eq!(
            serde_json::to_value(CompactInputItem::from_llm_message(&calls_only)).unwrap(),
            json!([{"type":"function_call","call_id":"call-2","name":"clock","arguments":"{}"}])
        );
    }
}
