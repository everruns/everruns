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
    /// Output cap for providers that still take the original field name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_tokens: Option<u32>,
    /// Output cap for OpenAI-family hosts, which reject `max_tokens` on current
    /// models: `gpt-6-luna` answers HTTP 400 "Unsupported parameter:
    /// 'max_tokens' is not supported with this model. Use
    /// 'max_completion_tokens' instead." Exactly one of the two is ever set,
    /// chosen by [`max_output_fields`](super::openai_protocol::max_output_fields).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) max_completion_tokens: Option<u32>,
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
    /// Structured output: `{type: "json_schema", json_schema: {name, schema, strict}}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) response_format: Option<Value>,
}

impl OpenAiRequest {
    /// `reasoning_effort` for the call. An explicit "no reasoning" omits the
    /// field: sending it to a non-thinking model is an API error.
    pub(crate) fn reasoning_effort_for(
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Option<String> {
        let effort = config.reasoning_effort;
        effort
            .filter(crate::model::ReasoningEffort::requests_reasoning)
            .map(|e| e.as_str().to_string())
    }

    /// `response_format` for the call, from the provider-neutral structured output.
    pub(crate) fn response_format_for(
        config: &crate::driver_registry::LlmCallConfig,
    ) -> Option<Value> {
        config
            .response_format
            .as_ref()
            .map(|f| f.chat_completions_response_format())
    }
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
    /// Error envelope some gateways return with a `200` status.
    #[serde(default)]
    pub(crate) error: Option<OpenAiErrorEnvelope>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiChatChoice {
    pub(crate) message: OpenAiChatMessage,
    #[serde(default)]
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OpenAiChatMessage {
    /// A string, or (Mistral with reasoning on) an array of typed chunks;
    /// read through [`content_chunks_text`] in [`Self::text_and_thinking`].
    #[serde(default)]
    pub(crate) content: Option<Value>,
    /// Mistral sends `"tool_calls": null` on a plain answer.
    #[serde(default, deserialize_with = "null_as_empty")]
    pub(crate) tool_calls: Vec<OpenAiToolCall>,
    /// DeepSeek-style non-streamed reasoning payload.
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
}

fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

impl OpenAiChatMessage {
    /// The answer text and any reasoning, from either wire shape. A dedicated
    /// `reasoning_content` field wins over thinking chunks.
    pub(crate) fn text_and_thinking(&mut self) -> (String, Option<String>) {
        let (text, thinking) = match self.content.take() {
            Some(Value::String(text)) => (Some(text), None),
            Some(Value::Array(chunks)) => content_chunks_text(&chunks),
            _ => (None, None),
        };
        (
            text.unwrap_or_default(),
            self.reasoning_content.take().or(thinking),
        )
    }
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
    #[serde(deserialize_with = "arguments_as_json_text")]
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
    // Defaulted so an error chunk that carries no `choices` still parses and
    // its message reaches the caller instead of "missing field `choices`".
    #[serde(default)]
    pub(crate) choices: Vec<OpenAiStreamChoice>,
    #[serde(default)]
    pub(crate) usage: Option<OpenAiUsage>,
    /// In-stream error envelope. OpenAI-compatible gateways (OpenRouter,
    /// LiteLLM, vLLM) report an upstream failure after the `200` as a chunk
    /// carrying `error`, sometimes beside a `finish_reason: "error"` choice.
    #[serde(default)]
    pub(crate) error: Option<OpenAiErrorEnvelope>,
}

/// The `error` object of an OpenAI-shaped error envelope.
///
/// Every field is optional and loosely typed: gateways disagree on whether
/// `code` is a string (`"server_error"`) or an HTTP status (`502`), and the
/// envelope is only useful if it parses whatever shape arrives.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct OpenAiErrorEnvelope {
    #[serde(default)]
    pub(crate) message: Option<String>,
    #[serde(default)]
    pub(crate) code: Option<Value>,
    #[serde(default, rename = "type")]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<u16>,
}

impl OpenAiErrorEnvelope {
    /// Convert to a stream error, keeping the vendor's message, code and any
    /// HTTP status the envelope carries so hosts match on structure.
    pub(crate) fn into_stream_error(self) -> crate::stream_error::LlmStreamError {
        let (code, status) = match self.code {
            Some(Value::String(code)) => (Some(code), self.status),
            Some(Value::Number(number)) => {
                let status = number.as_u64().and_then(|n| u16::try_from(n).ok());
                (None, self.status.or(status))
            }
            _ => (None, self.status),
        };
        let code = code.or(self.kind);
        let message = self
            .message
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| "provider reported an error in the response stream".to_string());
        crate::stream_error::LlmStreamError::provider(code, status, message)
    }
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
#[serde(try_from = "RawOpenAiDelta")]
pub(crate) struct OpenAiDelta {
    /// Text for this chunk.
    ///
    /// Read leniently because Cloudflare's AI REST API serializes a token that
    /// is entirely numeric as a JSON *number* rather than a string
    /// (`"delta":{"content":1}` between two ordinary `"content":", 2"`
    /// chunks). With a plain `Option<String>` that field fails to deserialize,
    /// which fails the whole chunk, and the token is dropped from the stream
    /// with no error: "1, 2, 3" arrives as ", 2, 3". Accepting a number loses
    /// nothing for a conformant provider, which never sends one.
    ///
    /// Mistral sends an array of typed chunks instead once reasoning is on
    /// (`[{"type":"thinking","thinking":[{"type":"text","text":"…"}]},
    /// {"type":"text","text":"391"}]`). The text chunks land here and the
    /// thinking chunks in `reasoning_content`; see [`content_chunks_text`].
    pub(crate) content: Option<String>,
    /// Reasoning text on the Chat Completions wire. Reasoning models reached
    /// over this protocol (DeepSeek-R1, Qwen, Groq, Fireworks) stream it here;
    /// vendors split between two field names for the same thing.
    pub(crate) reasoning_content: Option<String>,
    pub(crate) reasoning: Option<String>,
    pub(crate) tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

/// [`OpenAiDelta`] as it arrives, before typed content chunks are split into
/// text and reasoning.
#[derive(Deserialize)]
struct RawOpenAiDelta {
    #[serde(default)]
    content: Option<Value>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiStreamToolCall>>,
}

impl TryFrom<RawOpenAiDelta> for OpenAiDelta {
    type Error = String;

    fn try_from(raw: RawOpenAiDelta) -> Result<Self, Self::Error> {
        let (content, thinking) = match raw.content {
            None | Some(Value::Null) => (None, None),
            Some(Value::String(text)) => (Some(text), None),
            Some(Value::Number(number)) => (Some(number.to_string()), None),
            Some(Value::Array(chunks)) => content_chunks_text(&chunks),
            // A shape nobody sends stays an error: silently accepting an
            // object would hide a real protocol change behind an empty delta.
            Some(other) => {
                return Err(format!(
                    "expected a string, number or chunk array for streamed text, got {other}"
                ));
            }
        };
        Ok(Self {
            content,
            reasoning_content: raw.reasoning_content.or(thinking),
            reasoning: raw.reasoning,
            tool_calls: raw.tool_calls,
        })
    }
}

/// Split Mistral-style typed content chunks into `(text, thinking)`.
///
/// `text` chunks carry `text`; `thinking` chunks carry a nested list of text
/// chunks. Other chunk types (references, images) carry no streamed text and
/// are skipped rather than failing the chunk, which would drop the text riding
/// beside them. Each side is `None` when it carried no text.
pub(crate) fn content_chunks_text(chunks: &[Value]) -> (Option<String>, Option<String>) {
    fn texts<'a>(chunks: impl IntoIterator<Item = &'a Value>) -> String {
        chunks
            .into_iter()
            .filter(|chunk| chunk.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|chunk| chunk.get("text").and_then(Value::as_str))
            .collect()
    }
    let text = texts(chunks);
    let thinking: String = chunks
        .iter()
        .filter(|chunk| chunk.get("type").and_then(Value::as_str) == Some("thinking"))
        .filter_map(|chunk| chunk.get("thinking").and_then(Value::as_array))
        .map(texts)
        .collect();
    let non_empty = |text: String| (!text.is_empty()).then_some(text);
    (non_empty(text), non_empty(thinking))
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
    #[serde(default, deserialize_with = "optional_arguments_as_json_text")]
    pub(crate) arguments: Option<String>,
}

/// Tool-call `arguments` as JSON text.
///
/// The OpenAI wire sends arguments as a JSON-encoded string, but some
/// compatible servers send the object itself. Serializing a non-string value
/// back to text keeps those calls intact instead of failing the chunk.
fn arguments_as_json_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Value::deserialize(deserializer)? {
        Value::String(text) => text,
        Value::Null => String::new(),
        other => other.to_string(),
    })
}

fn optional_arguments_as_json_text<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(match Value::deserialize(deserializer)? {
        Value::Null => None,
        Value::String(text) => Some(text),
        other => Some(other.to_string()),
    })
}

#[cfg(test)]
mod delta_text_tests {
    use super::OpenAiDelta;

    /// The ordinary wire shape.
    #[test]
    fn a_string_delta_reads_as_text() {
        let delta: OpenAiDelta = serde_json::from_value(serde_json::json!({"content": ", 2"}))
            .expect("a string delta should deserialize");
        assert_eq!(delta.content.as_deref(), Some(", 2"));
    }

    /// Cloudflare's AI REST API sends a numeric token as a JSON number. Before
    /// this was read leniently the field failed, which failed the whole chunk,
    /// and "1, 2, 3" reached the caller as ", 2, 3".
    #[test]
    fn a_numeric_delta_is_not_dropped() {
        let delta: OpenAiDelta = serde_json::from_value(serde_json::json!({"content": 1}))
            .expect("a numeric delta should deserialize");
        assert_eq!(delta.content.as_deref(), Some("1"));
    }

    #[test]
    fn an_absent_or_null_delta_is_none() {
        let absent: OpenAiDelta =
            serde_json::from_value(serde_json::json!({})).expect("absent content is valid");
        assert_eq!(absent.content, None);
        let null: OpenAiDelta = serde_json::from_value(serde_json::json!({"content": null}))
            .expect("null content is valid");
        assert_eq!(null.content, None);
    }

    /// Mistral's typed chunks: text joins the content, thinking the reasoning.
    #[test]
    fn a_chunk_array_splits_text_from_thinking() {
        let delta: OpenAiDelta = serde_json::from_value(serde_json::json!({"content": [
            {"type": "thinking", "thinking": [{"type": "text", "text": "plan"}]},
            {"type": "text", "text": "an"}, {"type": "text", "text": "swer"}
        ]}))
        .expect("a chunk array should deserialize");
        assert_eq!(delta.content.as_deref(), Some("answer"));
        assert_eq!(delta.reasoning_text(), Some("plan"));
        let empty: OpenAiDelta = serde_json::from_value(
            serde_json::json!({"content": [{"type": "thinking", "thinking": []}]}),
        )
        .expect("an empty thinking chunk is valid");
        assert_eq!(
            (empty.content.as_deref(), empty.reasoning_text()),
            (None, None)
        );
    }

    /// A shape nobody sends stays an error: quietly accepting an object would
    /// hide a real protocol change behind an empty delta.
    #[test]
    fn a_structured_delta_is_still_an_error() {
        let result: Result<OpenAiDelta, _> =
            serde_json::from_value(serde_json::json!({"content": {"text": "hi"}}));
        assert!(result.is_err(), "an object is not streamed text");
    }
}
