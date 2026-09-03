//! OpenInference semantic conventions (Arize Phoenix and compatible backends).
//!
//! Attribute vocabulary and value builders for the OpenInference spec
//! (<https://arize-ai.github.io/openinference/spec/semantic_conventions.html>).
//! The `OtelEventListener` emits these next to the OpenTelemetry Gen-AI
//! attributes on the same spans, so one OTLP stream renders in a Gen-AI-aware
//! backend (Tempo, Jaeger, Datadog, Langfuse) and in Phoenix alike.
//!
//! OpenInference flattens lists into indexed keys
//! (`llm.input_messages.0.message.role`), which is why message builders here
//! return `KeyValue` lists rather than JSON.

use everruns_core::message::{ContentPart, Message};
use everruns_core::telemetry::content;
use everruns_provider::tool_types::ToolCall;
use opentelemetry::KeyValue;

/// The OpenInference span kind attribute.
pub const SPAN_KIND: &str = "openinference.span.kind";

/// `openinference.span.kind` values used by the listener.
pub mod span_kind {
    /// A span that encompasses calls to LLMs and tools.
    pub const AGENT: &str = "AGENT";
    /// A call to a large language model.
    pub const LLM: &str = "LLM";
    /// A call to an external tool.
    pub const TOOL: &str = "TOOL";
    /// A link between application steps.
    pub const CHAIN: &str = "CHAIN";
}

/// The input value of an operation.
pub const INPUT_VALUE: &str = "input.value";
/// MIME type of `input.value`.
pub const INPUT_MIME_TYPE: &str = "input.mime_type";
/// The output value of an operation.
pub const OUTPUT_VALUE: &str = "output.value";
/// MIME type of `output.value`.
pub const OUTPUT_MIME_TYPE: &str = "output.mime_type";

/// MIME types OpenInference recognizes for input and output values.
pub mod mime {
    pub const TEXT: &str = "text/plain";
    pub const JSON: &str = "application/json";
}

/// Model name used for the call.
pub const LLM_MODEL_NAME: &str = "llm.model_name";
/// Hosting provider of the model.
pub const LLM_PROVIDER: &str = "llm.provider";
/// AI product family behind the model.
pub const LLM_SYSTEM: &str = "llm.system";
/// JSON of the parameters used to invoke the model.
pub const LLM_INVOCATION_PARAMETERS: &str = "llm.invocation_parameters";
/// Prefix for flattened input messages.
pub const LLM_INPUT_MESSAGES: &str = "llm.input_messages";
/// Prefix for flattened output messages.
pub const LLM_OUTPUT_MESSAGES: &str = "llm.output_messages";
/// Prefix for flattened tool definitions advertised to the model.
pub const LLM_TOOLS: &str = "llm.tools";
/// Prompt token count.
pub const LLM_TOKEN_COUNT_PROMPT: &str = "llm.token_count.prompt";
/// Completion token count.
pub const LLM_TOKEN_COUNT_COMPLETION: &str = "llm.token_count.completion";
/// Total token count.
pub const LLM_TOKEN_COUNT_TOTAL: &str = "llm.token_count.total";
/// Prompt tokens served from cache.
pub const LLM_TOKEN_COUNT_PROMPT_CACHE_READ: &str = "llm.token_count.prompt_details.cache_read";
/// Prompt tokens written to cache.
pub const LLM_TOKEN_COUNT_PROMPT_CACHE_WRITE: &str = "llm.token_count.prompt_details.cache_write";
/// Total cost of the call in USD.
pub const LLM_COST_TOTAL: &str = "llm.cost.total";

/// Message role within a flattened message.
pub const MESSAGE_ROLE: &str = "message.role";
/// Message text within a flattened message.
pub const MESSAGE_CONTENT: &str = "message.content";
/// Tool call id a tool-role message answers.
pub const MESSAGE_TOOL_CALL_ID: &str = "message.tool_call_id";
/// Prefix for flattened tool calls within a message.
pub const MESSAGE_TOOL_CALLS: &str = "message.tool_calls";
/// Tool call id within a flattened tool call.
pub const TOOL_CALL_ID: &str = "tool_call.id";
/// Function name within a flattened tool call.
pub const TOOL_CALL_FUNCTION_NAME: &str = "tool_call.function.name";
/// JSON arguments within a flattened tool call.
pub const TOOL_CALL_FUNCTION_ARGUMENTS: &str = "tool_call.function.arguments";
/// JSON schema of a tool definition within a flattened tool list.
pub const TOOL_JSON_SCHEMA: &str = "tool.json_schema";

/// Name of the tool on a TOOL span.
pub const TOOL_NAME: &str = "tool.name";
/// Description of the tool on a TOOL span.
pub const TOOL_DESCRIPTION: &str = "tool.description";

/// Session identifier.
pub const SESSION_ID: &str = "session.id";
/// Name of the agent an AGENT span represents.
pub const AGENT_NAME: &str = "agent.name";
/// JSON metadata for a span.
pub const METADATA: &str = "metadata";

/// Exception class recorded on an `exception` span event.
pub const EXCEPTION_TYPE: &str = "exception.type";
/// Exception message recorded on an `exception` span event.
pub const EXCEPTION_MESSAGE: &str = "exception.message";

/// OpenInference `llm.provider` and `llm.system` values for an Everruns driver
/// id. `llm.system` names the API family, so it is only set where the driver
/// speaks a vendor's own API; aggregators such as OpenRouter keep the driver id
/// as the provider and leave the system unset.
pub fn provider_and_system(driver_id: &str) -> (&str, Option<&'static str>) {
    match driver_id {
        "openai" | "openai_completions" => ("openai", Some("openai")),
        "azure_openai" => ("azure", Some("openai")),
        "anthropic" => ("anthropic", Some("anthropic")),
        "gemini" => ("google", Some("vertexai")),
        "bedrock" => ("aws", None),
        "mistral" | "mistral_ai" => ("mistralai", Some("mistralai")),
        "cohere" => ("cohere", Some("cohere")),
        "xai" | "x_ai" => ("xai", None),
        "deepseek" => ("deepseek", None),
        other => (other, None),
    }
}

/// Flattened attributes for one input message at `llm.input_messages.{index}`.
pub fn input_message_attributes(index: usize, message: &Message) -> Vec<KeyValue> {
    let prefix = format!("{LLM_INPUT_MESSAGES}.{index}");
    let mut attrs = vec![KeyValue::new(
        format!("{prefix}.{MESSAGE_ROLE}"),
        content::role_name(&message.role),
    )];
    let text = message_text(message);
    if !text.is_empty() {
        attrs.push(KeyValue::new(format!("{prefix}.{MESSAGE_CONTENT}"), text));
    }
    if let Some(id) = message.tool_call_id() {
        attrs.push(KeyValue::new(
            format!("{prefix}.{MESSAGE_TOOL_CALL_ID}"),
            id.to_string(),
        ));
    }
    let tool_calls: Vec<(&str, &str, &serde_json::Value)> = message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolCall(tc) => Some((tc.id.as_str(), tc.name.as_str(), &tc.arguments)),
            _ => None,
        })
        .collect();
    for (j, (id, name, arguments)) in tool_calls.into_iter().enumerate() {
        attrs.extend(tool_call_attributes(&prefix, j, id, name, arguments));
    }
    attrs
}

/// Flattened attributes for the model's output at `llm.output_messages.0`.
pub fn output_message_attributes(text: Option<&str>, tool_calls: &[ToolCall]) -> Vec<KeyValue> {
    let prefix = format!("{LLM_OUTPUT_MESSAGES}.0");
    let mut attrs = vec![KeyValue::new(
        format!("{prefix}.{MESSAGE_ROLE}"),
        "assistant",
    )];
    if let Some(text) = text.filter(|t| !t.is_empty()) {
        attrs.push(KeyValue::new(
            format!("{prefix}.{MESSAGE_CONTENT}"),
            text.to_string(),
        ));
    }
    for (j, call) in tool_calls.iter().enumerate() {
        attrs.extend(tool_call_attributes(
            &prefix,
            j,
            &call.id,
            &call.name,
            &call.arguments,
        ));
    }
    attrs
}

fn tool_call_attributes(
    message_prefix: &str,
    index: usize,
    id: &str,
    name: &str,
    arguments: &serde_json::Value,
) -> Vec<KeyValue> {
    let prefix = format!("{message_prefix}.{MESSAGE_TOOL_CALLS}.{index}");
    vec![
        KeyValue::new(format!("{prefix}.{TOOL_CALL_ID}"), id.to_string()),
        KeyValue::new(
            format!("{prefix}.{TOOL_CALL_FUNCTION_NAME}"),
            name.to_string(),
        ),
        KeyValue::new(
            format!("{prefix}.{TOOL_CALL_FUNCTION_ARGUMENTS}"),
            arguments.to_string(),
        ),
    ]
}

/// The text a message carries, as OpenInference's single `message.content`:
/// text parts joined, tool results rendered as their JSON or error.
fn message_text(message: &Message) -> String {
    let mut chunks: Vec<String> = Vec::new();
    for part in &message.content {
        match part {
            ContentPart::Text(t) => chunks.push(t.text.clone()),
            ContentPart::ToolResult(tr) => match (&tr.error, &tr.result) {
                (Some(error), _) => chunks.push(format!("Error: {error}")),
                (None, Some(serde_json::Value::String(s))) => chunks.push(s.clone()),
                (None, Some(value)) => chunks.push(value.to_string()),
                (None, None) => {}
            },
            _ => {}
        }
    }
    chunks.join("\n")
}

/// Flattened attributes for the tools advertised to the model at
/// `llm.tools.{index}.tool.json_schema`.
pub fn tool_attributes(index: usize, name: &str, description: &str) -> KeyValue {
    let schema = serde_json::json!({
        "type": "function",
        "function": { "name": name, "description": description },
    });
    KeyValue::new(
        format!("{LLM_TOOLS}.{index}.{TOOL_JSON_SCHEMA}"),
        schema.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn attr(attrs: &[KeyValue], key: &str) -> Option<String> {
        attrs
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| kv.value.to_string())
    }

    #[test]
    fn input_messages_flatten_role_content_and_tool_calls() {
        let call = ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "a.txt" }),
        };
        let attrs =
            input_message_attributes(2, &Message::assistant_with_tools("Reading", vec![call]));
        assert_eq!(
            attr(&attrs, "llm.input_messages.2.message.role").as_deref(),
            Some("assistant")
        );
        assert_eq!(
            attr(&attrs, "llm.input_messages.2.message.content").as_deref(),
            Some("Reading")
        );
        assert_eq!(
            attr(
                &attrs,
                "llm.input_messages.2.message.tool_calls.0.tool_call.function.name"
            )
            .as_deref(),
            Some("read_file")
        );
        assert_eq!(
            attr(
                &attrs,
                "llm.input_messages.2.message.tool_calls.0.tool_call.function.arguments"
            )
            .as_deref(),
            Some(r#"{"path":"a.txt"}"#)
        );
    }

    #[test]
    fn tool_result_messages_carry_their_call_id() {
        let attrs = input_message_attributes(
            0,
            &Message::tool_result("call_9", Some(json!({ "ok": true })), None),
        );
        assert_eq!(
            attr(&attrs, "llm.input_messages.0.message.role").as_deref(),
            Some("tool")
        );
        assert_eq!(
            attr(&attrs, "llm.input_messages.0.message.tool_call_id").as_deref(),
            Some("call_9")
        );
        assert_eq!(
            attr(&attrs, "llm.input_messages.0.message.content").as_deref(),
            Some(r#"{"ok":true}"#)
        );
    }

    #[test]
    fn providers_map_to_openinference_vocabulary() {
        assert_eq!(provider_and_system("openai"), ("openai", Some("openai")));
        assert_eq!(
            provider_and_system("azure_openai"),
            ("azure", Some("openai"))
        );
        assert_eq!(provider_and_system("gemini"), ("google", Some("vertexai")));
        assert_eq!(provider_and_system("openrouter"), ("openrouter", None));
    }
}
