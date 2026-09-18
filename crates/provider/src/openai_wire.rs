//! The OpenAI chat-completions JSON shapes, as values Everruns can read and write.
//!
//! The `/chat/completions` message and tool JSON is the lingua franca an
//! embedder already holds: it arrives from that embedder's own HTTP API, sits
//! in its stored transcripts, or comes out of another SDK. Everruns' drivers
//! speak [`LlmMessage`] and [`ToolDefinition`] instead, and translating between
//! the two by hand goes wrong in the same places every time — the two assistant
//! tool-call shapes in circulation, a `tool` message missing its
//! `tool_call_id`, and multimodal content quietly flattened to its JSON
//! rendering because the parts array looked like it had nowhere to go.
//!
//! This module is that translation, both ways, in one tested place. It converts
//! the wire shapes only; it never sends anything.
//!
//! ```
//! use everruns_provider::openai_wire;
//! use serde_json::json;
//!
//! let message = openai_wire::message_from_openai(&json!({
//!     "role": "user",
//!     "content": [
//!         {"type": "text", "text": "what is in this image?"},
//!         {"type": "image_url", "image_url": {"url": "https://example.com/cat.png"}}
//!     ],
//! }))
//! .expect("a well-formed user message");
//! assert!(message.is_parts(), "the image survives the conversion");
//! ```

use serde_json::{Map, Value, json};
use thiserror::Error;

use crate::driver_registry::{LlmContentPart, LlmMessage, LlmMessageContent, LlmMessageRole};
use crate::tool_types::{ToolCall, ToolDefinition};

/// Why a piece of OpenAI-shaped JSON could not be read.
///
/// Every variant names the field at fault, because the caller is usually
/// holding a transcript and needs to know which entry to fix.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum OpenAiWireError {
    /// A required field is absent.
    #[error("missing `{field}`")]
    Missing {
        /// The field that was expected.
        field: &'static str,
    },
    /// A field is present but not the shape it must be.
    #[error("`{field}` is not {expected}")]
    Invalid {
        /// The field at fault.
        field: &'static str,
        /// What that field has to be.
        expected: &'static str,
    },
    /// The `role` is not one the provider contract has.
    #[error("unsupported message role: {role}")]
    UnsupportedRole {
        /// The role as it appeared on the wire.
        role: String,
    },
}

impl From<OpenAiWireError> for crate::error::AgentLoopError {
    fn from(error: OpenAiWireError) -> Self {
        crate::error::AgentLoopError::config(error.to_string())
    }
}

type Result<T> = std::result::Result<T, OpenAiWireError>;

/// Read one OpenAI chat message into an [`LlmMessage`].
///
/// Accepts the shapes the wire actually carries:
///
/// - `content` as a string, as a parts array (text, `image_url`,
///   `input_audio`, `file`), or `null` on an assistant message that only calls
///   tools;
/// - assistant `tool_calls` in the nested `{"type":"function","function":{…}}`
///   form, and in the flat `{"id","name","arguments"}` form transcripts often
///   store;
/// - `arguments` as the JSON-encoded string the API sends, or as an already
///   parsed object.
///
/// # Errors
///
/// When the role is missing or unknown, when a `tool` message has no
/// `tool_call_id` (the provider will reject it), or when a tool call has no
/// name.
pub fn message_from_openai(value: &Value) -> Result<LlmMessage> {
    let object = value.as_object().ok_or(OpenAiWireError::Invalid {
        field: "message",
        expected: "an object",
    })?;
    let role = object
        .get("role")
        .and_then(Value::as_str)
        .ok_or(OpenAiWireError::Missing { field: "role" })?;
    let role = match role {
        "system" | "developer" => LlmMessageRole::System,
        "user" => LlmMessageRole::User,
        "assistant" => LlmMessageRole::Assistant,
        "tool" | "function" => LlmMessageRole::Tool,
        other => {
            return Err(OpenAiWireError::UnsupportedRole {
                role: other.to_owned(),
            });
        }
    };

    let content = match object.get("content") {
        None | Some(Value::Null) => LlmMessageContent::Text(String::new()),
        Some(Value::String(text)) => LlmMessageContent::Text(text.clone()),
        Some(Value::Array(parts)) => {
            LlmMessageContent::Parts(parts.iter().map(content_part_from_openai).collect())
        }
        Some(_) => {
            return Err(OpenAiWireError::Invalid {
                field: "content",
                expected: "a string, an array of parts, or null",
            });
        }
    };

    let mut message = LlmMessage {
        content,
        ..LlmMessage::text(role, "")
    };

    if let Some(calls) = object.get("tool_calls") {
        let calls = calls.as_array().ok_or(OpenAiWireError::Invalid {
            field: "tool_calls",
            expected: "an array",
        })?;
        let parsed = calls
            .iter()
            .map(tool_call_from_openai)
            .collect::<Result<Vec<_>>>()?;
        if !parsed.is_empty() {
            message.tool_calls = Some(parsed);
        }
    }

    match object.get("tool_call_id").and_then(Value::as_str) {
        Some(id) => message.tool_call_id = Some(id.to_owned()),
        // The provider rejects a tool result it cannot correlate, so this is
        // caught here rather than one round trip later.
        None if message.role == LlmMessageRole::Tool => {
            return Err(OpenAiWireError::Missing {
                field: "tool_call_id",
            });
        }
        None => {}
    }

    Ok(message)
}

/// Read a whole OpenAI `messages` array.
///
/// # Errors
///
/// The first message that cannot be read, so a bad transcript fails on the
/// entry at fault rather than silently dropping it.
pub fn messages_from_openai(values: &[Value]) -> Result<Vec<LlmMessage>> {
    values.iter().map(message_from_openai).collect()
}

/// Write an [`LlmMessage`] back out as OpenAI chat JSON.
///
/// The inverse of [`message_from_openai`] for everything that has an OpenAI
/// equivalent: tool calls go out in the nested `function` form the API
/// expects, and `arguments` as the JSON-encoded string. Everruns' own
/// additions with no place on this wire — reasoning artifacts, execution
/// phase, native calls — are left out rather than invented.
pub fn message_to_openai(message: &LlmMessage) -> Value {
    let mut object = Map::new();
    object.insert(
        "role".to_owned(),
        Value::String(
            match message.role {
                LlmMessageRole::System => "system",
                LlmMessageRole::User => "user",
                LlmMessageRole::Assistant => "assistant",
                LlmMessageRole::Tool => "tool",
            }
            .to_owned(),
        ),
    );
    let content = match &message.content {
        LlmMessageContent::Text(text) => Value::String(text.clone()),
        LlmMessageContent::Parts(parts) => {
            Value::Array(parts.iter().map(content_part_to_openai).collect())
        }
    };
    object.insert("content".to_owned(), content);
    if let Some(calls) = &message.tool_calls {
        object.insert(
            "tool_calls".to_owned(),
            Value::Array(calls.iter().map(tool_call_to_openai).collect()),
        );
    }
    if let Some(id) = &message.tool_call_id {
        object.insert("tool_call_id".to_owned(), Value::String(id.clone()));
    }
    Value::Object(object)
}

/// Read one OpenAI tool definition (`{"type":"function","function":{…}}`, or
/// the bare function object) into a [`ToolDefinition`].
///
/// The result is a [`ToolDefinition::ClientSide`]: a schema Everruns passes to
/// the model and whose calls come back to the caller, which is what a tool
/// arriving as JSON always is — the runtime has no implementation to run for
/// it.
///
/// # Errors
///
/// When the function has no `name`.
pub fn tool_from_openai(value: &Value) -> Result<ToolDefinition> {
    let function = value.get("function").unwrap_or(value);
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or(OpenAiWireError::Missing {
            field: "function.name",
        })?;
    let description = function
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let parameters = function
        .get("parameters")
        .cloned()
        .unwrap_or_else(empty_parameters);
    Ok(ToolDefinition::function(name, description, parameters))
}

/// Read a whole OpenAI `tools` array.
///
/// # Errors
///
/// The first entry that cannot be read.
pub fn tools_from_openai(values: &[Value]) -> Result<Vec<ToolDefinition>> {
    values.iter().map(tool_from_openai).collect()
}

/// Write a [`ToolDefinition`] out as an OpenAI tool entry.
pub fn tool_to_openai(tool: &ToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name(),
            "description": tool.description(),
            "parameters": tool.parameters().clone(),
        },
    })
}

/// Read one assistant tool call, in either shape the wire carries.
///
/// # Errors
///
/// When the call has no name under either shape.
pub fn tool_call_from_openai(value: &Value) -> Result<ToolCall> {
    // The nested `function` object wins when present; otherwise the fields are
    // read flat, which is how transcripts and hand-built calls store them.
    let function = value.get("function").unwrap_or(value);
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| value.get("name").and_then(Value::as_str))
        .ok_or(OpenAiWireError::Missing { field: "name" })?;
    let arguments = function
        .get("arguments")
        .or_else(|| value.get("arguments"))
        .map(parse_arguments)
        .unwrap_or_else(|| json!({}));
    Ok(ToolCall {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        name: name.to_owned(),
        arguments,
    })
}

/// Write a [`ToolCall`] out in the nested OpenAI form, with `arguments`
/// JSON-encoded as the API expects.
pub fn tool_call_to_openai(call: &ToolCall) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {
            "name": call.name,
            "arguments": serde_json::to_string(&call.arguments).unwrap_or_else(|_| "{}".to_owned()),
        },
    })
}

/// Arguments arrive JSON-encoded from the API and already parsed from callers
/// that decoded them; a string that is not JSON is kept as a string rather
/// than discarded.
fn parse_arguments(value: &Value) -> Value {
    match value {
        Value::String(raw) => serde_json::from_str(raw).unwrap_or_else(|_| value.clone()),
        other => other.clone(),
    }
}

/// An unknown part is carried as its text rendering rather than dropped: a
/// message that loses content silently is worse than one that carries it
/// plainly.
fn content_part_from_openai(value: &Value) -> LlmContentPart {
    let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        "text" | "input_text" | "output_text" => LlmContentPart::text(
            value
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ),
        "image_url" | "input_image" => {
            let url = value
                .pointer("/image_url/url")
                .or_else(|| value.get("image_url"))
                .or_else(|| value.get("url"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            LlmContentPart::image(url)
        }
        "input_audio" | "audio" => {
            let url = value
                .pointer("/input_audio/data")
                .or_else(|| value.pointer("/audio/url"))
                .or_else(|| value.get("url"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            LlmContentPart::audio(url)
        }
        "file" | "input_file" => {
            let file = value.get("file").or_else(|| value.get("input_file"));
            let url = file
                .and_then(|file| file.get("file_data").or_else(|| file.get("file_url")))
                .or_else(|| value.get("url"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let filename = file
                .and_then(|file| file.get("filename"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            LlmContentPart::file(url, filename)
        }
        _ => LlmContentPart::text(
            value
                .get("text")
                .and_then(Value::as_str)
                .map_or_else(|| value.to_string(), str::to_owned),
        ),
    }
}

fn content_part_to_openai(part: &LlmContentPart) -> Value {
    match part {
        LlmContentPart::Text { text } => json!({"type": "text", "text": text}),
        LlmContentPart::Image { url } => json!({"type": "image_url", "image_url": {"url": url}}),
        LlmContentPart::Audio { url } => {
            json!({"type": "input_audio", "input_audio": {"data": url}})
        }
        LlmContentPart::File { url, filename } => {
            let mut file = Map::new();
            file.insert("file_data".to_owned(), Value::String(url.clone()));
            if let Some(filename) = filename {
                file.insert("filename".to_owned(), Value::String(filename.clone()));
            }
            json!({"type": "file", "file": Value::Object(file)})
        }
    }
}

/// The schema for a tool that takes no arguments.
pub(crate) fn empty_parameters() -> Value {
    json!({"type": "object", "properties": {}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_exchange_round_trips() {
        let wire = json!([
            {"role": "system", "content": "be brief"},
            {"role": "user", "content": "hi"},
        ]);
        let messages = messages_from_openai(wire.as_array().unwrap()).unwrap();
        assert_eq!(messages[0].role, LlmMessageRole::System);
        assert_eq!(messages[1].content.to_text(), "hi");
        let back: Vec<Value> = messages.iter().map(message_to_openai).collect();
        assert_eq!(Value::Array(back), wire);
    }

    #[test]
    fn developer_is_read_as_a_system_message() {
        let message = message_from_openai(&json!({"role": "developer", "content": "rules"}))
            .expect("the newer system role name is still a system message");
        assert_eq!(message.role, LlmMessageRole::System);
    }

    #[test]
    fn both_assistant_tool_call_shapes_are_read() {
        let nested = message_from_openai(&json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {"name": "search", "arguments": "{\"q\":\"rust\"}"},
            }],
        }))
        .unwrap();
        let flat = message_from_openai(&json!({
            "role": "assistant",
            "tool_calls": [{"id": "call_1", "name": "search", "arguments": {"q": "rust"}}],
        }))
        .unwrap();
        for message in [&nested, &flat] {
            let calls = message.tool_calls.as_ref().expect("a tool call");
            assert_eq!(calls[0].id, "call_1");
            assert_eq!(calls[0].name, "search");
            assert_eq!(calls[0].arguments, json!({"q": "rust"}));
        }
    }

    #[test]
    fn tool_call_arguments_go_back_out_json_encoded() {
        let call = ToolCall {
            id: "call_1".into(),
            name: "search".into(),
            arguments: json!({"q": "rust"}),
        };
        let wire = tool_call_to_openai(&call);
        assert_eq!(
            wire.pointer("/function/arguments").unwrap(),
            &json!("{\"q\":\"rust\"}"),
            "the API takes arguments as a string, not an object"
        );
        assert_eq!(tool_call_from_openai(&wire).unwrap(), call);
    }

    #[test]
    fn arguments_that_are_not_json_survive_as_text() {
        let call =
            tool_call_from_openai(&json!({"name": "note", "arguments": "not json"})).unwrap();
        assert_eq!(call.arguments, json!("not json"));
    }

    #[test]
    fn a_tool_message_without_its_correlation_id_is_refused() {
        let error = message_from_openai(&json!({"role": "tool", "content": "42"}))
            .expect_err("the provider would reject this");
        assert_eq!(
            error,
            OpenAiWireError::Missing {
                field: "tool_call_id"
            }
        );
        let ok = message_from_openai(&json!({
            "role": "tool", "content": "42", "tool_call_id": "call_1",
        }))
        .unwrap();
        assert_eq!(ok.tool_call_id.as_deref(), Some("call_1"));
    }

    #[test]
    fn an_unknown_role_names_itself() {
        let error = message_from_openai(&json!({"role": "narrator", "content": "…"}))
            .expect_err("there is no such role");
        assert_eq!(
            error,
            OpenAiWireError::UnsupportedRole {
                role: "narrator".into()
            }
        );
    }

    #[test]
    fn multimodal_parts_convert_instead_of_flattening_to_json_text() {
        let wire = json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "what is this?"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
                {"type": "input_audio", "input_audio": {"data": "data:audio/wav;base64,BBBB"}},
                {"type": "file", "file": {"file_data": "data:application/pdf;base64,CCCC",
                                          "filename": "report.pdf"}},
            ],
        });
        let message = message_from_openai(&wire).unwrap();
        let LlmMessageContent::Parts(parts) = &message.content else {
            panic!("the parts array must stay parts");
        };
        assert_eq!(parts.len(), 4);
        assert_eq!(
            parts[1],
            LlmContentPart::image("data:image/png;base64,AAAA")
        );
        assert_eq!(
            parts[3],
            LlmContentPart::file(
                "data:application/pdf;base64,CCCC",
                Some("report.pdf".into())
            )
        );
        assert_eq!(message_to_openai(&message), wire);
    }

    #[test]
    fn an_unrecognized_part_keeps_its_content_rather_than_vanishing() {
        let message = message_from_openai(&json!({
            "role": "user",
            "content": [{"type": "video", "url": "https://example.com/clip.mp4"}],
        }))
        .unwrap();
        assert!(
            message.content.to_text().contains("clip.mp4"),
            "an unknown part must not silently drop its content"
        );
    }

    #[test]
    fn tool_definitions_read_both_wrapped_and_bare() {
        let wrapped = json!({
            "type": "function",
            "function": {
                "name": "search",
                "description": "look things up",
                "parameters": {"type": "object", "properties": {"q": {"type": "string"}}},
            },
        });
        let tool = tool_from_openai(&wrapped).unwrap();
        assert_eq!(tool.name(), "search");
        assert_eq!(tool.description(), "look things up");
        assert_eq!(tool_to_openai(&tool), wrapped);

        let bare = tool_from_openai(&json!({"name": "ping"})).unwrap();
        assert_eq!(bare.name(), "ping");
        assert_eq!(bare.parameters(), &empty_parameters());
    }

    #[test]
    fn a_nameless_tool_is_refused() {
        assert_eq!(
            tool_from_openai(&json!({"description": "no name"})).unwrap_err(),
            OpenAiWireError::Missing {
                field: "function.name"
            }
        );
    }
}
