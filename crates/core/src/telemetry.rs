// Telemetry Conventions
//
// Vendor-neutral gen-AI span metadata contracts shared by execution and
// exporters:
// - OpenTelemetry GenAI semantic-convention attribute names and well-known
//   values (https://github.com/open-telemetry/semantic-conventions-genai)
// - The spec's JSON shapes for captured instructions, inputs, outputs, and
//   tool definitions, built from Everruns messages
// - Span-name helpers
//
// OpenTelemetry initialization (OTLP exporter wiring, tracing-subscriber
// layers, TelemetryConfig/TelemetryGuard) and the span-producing listener live
// behind `everruns-host/observability` (EVE-876) so core carries no OTel SDK,
// exporter, or subscriber dependencies. Guard:
// scripts/lib/check-observability-isolation.sh.

use crate::events::ToolDefinitionSummary;
use crate::message::{ContentPart, Message, MessageRole};
use crate::tool_types::ToolCall;
use everruns_provider::reasoning::ReasoningText;
use serde_json::{Value, json};

// ============================================================================
// Gen-AI Semantic Conventions
// See: https://opentelemetry.io/docs/specs/semconv/gen-ai/
// ============================================================================

/// Gen-AI semantic convention attribute names.
///
/// Names track the `semantic-conventions-genai` registry. Everything here is
/// spec vocabulary; Everruns-specific span attributes live under the
/// `everruns.*` namespace in the host listener, never here.
pub mod gen_ai {
    // Operation and provider attributes
    /// The name of the operation being performed (e.g., "chat", "execute_tool")
    pub const OPERATION_NAME: &str = "gen_ai.operation.name";
    /// The GenAI provider as identified by the instrumentation (e.g., "openai")
    pub const PROVIDER_NAME: &str = "gen_ai.provider.name";
    /// Deprecated spelling of the provider attribute. Still emitted next to
    /// `gen_ai.provider.name` because several backends key their GenAI views on
    /// it; remove once the major backends read `gen_ai.provider.name`.
    pub const SYSTEM: &str = "gen_ai.system";

    // Request attributes
    /// The name of the model requested
    pub const REQUEST_MODEL: &str = "gen_ai.request.model";
    /// Maximum number of tokens in the response
    pub const REQUEST_MAX_TOKENS: &str = "gen_ai.request.max_tokens";
    /// Sampling temperature
    pub const REQUEST_TEMPERATURE: &str = "gen_ai.request.temperature";
    /// Top-P sampling parameter
    pub const REQUEST_TOP_P: &str = "gen_ai.request.top_p";
    /// Top-K sampling parameter
    pub const REQUEST_TOP_K: &str = "gen_ai.request.top_k";
    /// Frequency penalty
    pub const REQUEST_FREQUENCY_PENALTY: &str = "gen_ai.request.frequency_penalty";
    /// Presence penalty
    pub const REQUEST_PRESENCE_PENALTY: &str = "gen_ai.request.presence_penalty";
    /// Stop sequences
    pub const REQUEST_STOP_SEQUENCES: &str = "gen_ai.request.stop_sequences";
    /// Random seed for reproducibility
    pub const REQUEST_SEED: &str = "gen_ai.request.seed";
    /// Number of response candidates to generate
    pub const REQUEST_CHOICE_COUNT: &str = "gen_ai.request.choice.count";
    /// Whether the request was made in streaming mode
    pub const REQUEST_STREAM: &str = "gen_ai.request.stream";
    /// Reasoning / thinking effort level requested (exact string sent to the provider)
    pub const REQUEST_REASONING_LEVEL: &str = "gen_ai.request.reasoning.level";
    /// Requested encoding formats (embeddings)
    pub const REQUEST_ENCODING_FORMATS: &str = "gen_ai.request.encoding_formats";

    // Response attributes
    /// Unique identifier for the completion
    pub const RESPONSE_ID: &str = "gen_ai.response.id";
    /// The actual model used (may differ from requested)
    pub const RESPONSE_MODEL: &str = "gen_ai.response.model";
    /// Reasons why generation stopped (string array, one per candidate)
    pub const RESPONSE_FINISH_REASONS: &str = "gen_ai.response.finish_reasons";
    /// Time to first streamed chunk, in seconds (double)
    pub const RESPONSE_TIME_TO_FIRST_CHUNK: &str = "gen_ai.response.time_to_first_chunk";

    // Token usage attributes
    /// Number of tokens in the input/prompt (includes cached tokens)
    pub const USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
    /// Number of tokens in the output/completion
    pub const USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
    /// Number of input tokens served from a provider-managed cache
    pub const USAGE_CACHE_READ_INPUT_TOKENS: &str = "gen_ai.usage.cache_read.input_tokens";
    /// Number of input tokens written to a provider-managed cache
    pub const USAGE_CACHE_WRITE_INPUT_TOKENS: &str = "gen_ai.usage.cache_write.input_tokens";

    // Content attributes (opt-in, may contain sensitive data)
    /// Chat history sent to the model, JSON per the spec's input-messages schema
    pub const INPUT_MESSAGES: &str = "gen_ai.input.messages";
    /// Messages returned by the model, JSON per the spec's output-messages schema
    pub const OUTPUT_MESSAGES: &str = "gen_ai.output.messages";
    /// System instructions provided separately from the chat history
    pub const SYSTEM_INSTRUCTIONS: &str = "gen_ai.system_instructions";
    /// Tool definitions available to the model or agent
    pub const TOOL_DEFINITIONS: &str = "gen_ai.tool.definitions";

    // Tool execution attributes
    /// Name of the tool being executed
    pub const TOOL_NAME: &str = "gen_ai.tool.name";
    /// Type of tool (function, extension, datastore)
    pub const TOOL_TYPE: &str = "gen_ai.tool.type";
    /// Tool description
    pub const TOOL_DESCRIPTION: &str = "gen_ai.tool.description";
    /// Tool call identifier
    pub const TOOL_CALL_ID: &str = "gen_ai.tool.call.id";
    /// Tool call arguments (opt-in, may contain sensitive data)
    pub const TOOL_CALL_ARGUMENTS: &str = "gen_ai.tool.call.arguments";
    /// Tool call result (opt-in, may contain sensitive data)
    pub const TOOL_CALL_RESULT: &str = "gen_ai.tool.call.result";

    // Conversation tracking
    /// Conversation or session identifier
    pub const CONVERSATION_ID: &str = "gen_ai.conversation.id";
    /// Whether the conversation context was compacted before this operation
    pub const CONVERSATION_COMPACTED: &str = "gen_ai.conversation.compacted";

    // Embeddings attributes
    /// Number of dimensions in output embeddings
    pub const EMBEDDINGS_DIMENSION_COUNT: &str = "gen_ai.embeddings.dimension.count";

    // Output attributes
    /// Output modality requested by the client (text, image, json, speech)
    pub const OUTPUT_TYPE: &str = "gen_ai.output.type";

    // Agent attributes
    /// Agent identifier
    pub const AGENT_ID: &str = "gen_ai.agent.id";
    /// Agent name
    pub const AGENT_NAME: &str = "gen_ai.agent.name";
    /// Agent description
    pub const AGENT_DESCRIPTION: &str = "gen_ai.agent.description";
    /// Agent version
    pub const AGENT_VERSION: &str = "gen_ai.agent.version";

    // Server attributes
    /// GenAI server address
    pub const SERVER_ADDRESS: &str = "server.address";
    /// GenAI server port
    pub const SERVER_PORT: &str = "server.port";

    // Error attributes
    /// Low-cardinality class of the error the operation ended with
    pub const ERROR_TYPE: &str = "error.type";
    /// The `error.type` fallback when no more specific class is known
    pub const ERROR_TYPE_OTHER: &str = "_OTHER";

    /// Operation names as per semantic conventions
    pub mod operation {
        pub const CHAT: &str = "chat";
        pub const EMBEDDINGS: &str = "embeddings";
        pub const TEXT_COMPLETION: &str = "text_completion";
        pub const GENERATE_CONTENT: &str = "generate_content";
        pub const EXECUTE_TOOL: &str = "execute_tool";
        pub const CREATE_AGENT: &str = "create_agent";
        pub const INVOKE_AGENT: &str = "invoke_agent";
        pub const INVOKE_WORKFLOW: &str = "invoke_workflow";
        pub const PLAN: &str = "plan";
    }

    /// Well-known `gen_ai.provider.name` values
    pub mod provider {
        pub const OPENAI: &str = "openai";
        pub const ANTHROPIC: &str = "anthropic";
        pub const AZURE_OPENAI: &str = "azure.ai.openai";
        pub const GEMINI: &str = "gcp.gemini";
        pub const VERTEX_AI: &str = "gcp.vertex_ai";
        pub const BEDROCK: &str = "aws.bedrock";
        pub const MISTRAL_AI: &str = "mistral_ai";
        pub const GROQ: &str = "groq";
        pub const COHERE: &str = "cohere";
        pub const DEEPSEEK: &str = "deepseek";
        pub const PERPLEXITY: &str = "perplexity";
        pub const X_AI: &str = "x_ai";

        /// Map an Everruns driver id (`DriverId::as_str`) to the spec's
        /// well-known provider name. Drivers without a spec entry keep their
        /// Everruns id, which the spec permits as a custom value.
        pub fn from_driver_id(driver_id: &str) -> &str {
            match driver_id {
                "openai" | "openai_completions" => OPENAI,
                "anthropic" => ANTHROPIC,
                "azure_openai" => AZURE_OPENAI,
                "gemini" => GEMINI,
                "vertex_ai" | "vertexai" => VERTEX_AI,
                "bedrock" => BEDROCK,
                "mistral" | "mistral_ai" => MISTRAL_AI,
                "groq" => GROQ,
                "cohere" => COHERE,
                "deepseek" => DEEPSEEK,
                "perplexity" => PERPLEXITY,
                "xai" | "x_ai" => X_AI,
                other => other,
            }
        }
    }

    /// Tool types as per semantic conventions
    pub mod tool_type {
        pub const FUNCTION: &str = "function";
        pub const EXTENSION: &str = "extension";
        pub const DATASTORE: &str = "datastore";
    }

    /// Output types as per semantic conventions
    pub mod output_type {
        pub const TEXT: &str = "text";
        pub const IMAGE: &str = "image";
        pub const JSON: &str = "json";
        pub const SPEECH: &str = "speech";
    }

    /// Message roles used in captured input/output messages
    pub mod role {
        pub const SYSTEM: &str = "system";
        pub const USER: &str = "user";
        pub const ASSISTANT: &str = "assistant";
        pub const TOOL: &str = "tool";
    }

    /// Message part types used in captured input/output messages
    pub mod part_type {
        pub const TEXT: &str = "text";
        pub const TOOL_CALL: &str = "tool_call";
        pub const TOOL_CALL_RESPONSE: &str = "tool_call_response";
        pub const REASONING: &str = "reasoning";
        pub const URI: &str = "uri";
        pub const BLOB: &str = "blob";
    }
}

// ============================================================================
// Captured content shapes
// ============================================================================

/// Builders for the JSON the spec defines for `gen_ai.system_instructions`,
/// `gen_ai.input.messages`, `gen_ai.output.messages`, and
/// `gen_ai.tool.definitions`.
///
/// Everruns keeps the agent's instructions as a distinct concept from the
/// chat history (drivers send them through the provider's dedicated
/// instruction channel where one exists), so system-role messages are recorded
/// under `gen_ai.system_instructions` and left out of `gen_ai.input.messages`.
pub mod content {
    use super::*;

    /// Parts of every system-role message, in order, or `None` when the
    /// prompt carried no instructions.
    pub fn system_instructions(messages: &[Message]) -> Option<Value> {
        let parts: Vec<Value> = messages
            .iter()
            .filter(|m| m.role == MessageRole::System)
            .flat_map(|m| m.content.iter().filter_map(part_json))
            .collect();
        if parts.is_empty() {
            None
        } else {
            Some(Value::Array(parts))
        }
    }

    /// The chat history sent to the model, excluding system instructions.
    pub fn input_messages(messages: &[Message]) -> Value {
        Value::Array(
            messages
                .iter()
                .filter(|m| m.role != MessageRole::System)
                .map(message_json)
                .collect(),
        )
    }

    /// One output message (Everruns requests a single candidate), carrying the
    /// model's reasoning, text, and tool calls as ordered parts.
    pub fn output_messages(
        text: Option<&str>,
        tool_calls: &[ToolCall],
        reasoning: Option<&str>,
        finish_reason: Option<&str>,
    ) -> Value {
        let mut parts = Vec::new();
        if let Some(reasoning) = reasoning.filter(|r| !r.is_empty()) {
            parts.push(json!({
                "type": gen_ai::part_type::REASONING,
                "content": reasoning,
            }));
        }
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            parts.push(json!({ "type": gen_ai::part_type::TEXT, "content": text }));
        }
        for call in tool_calls {
            parts.push(json!({
                "type": gen_ai::part_type::TOOL_CALL,
                "id": call.id,
                "name": call.name,
                "arguments": call.arguments,
            }));
        }
        let mut message = json!({ "role": gen_ai::role::ASSISTANT, "parts": parts });
        if let Some(reason) = finish_reason {
            message["finish_reason"] = Value::String(reason.to_string());
        }
        Value::Array(vec![message])
    }

    /// Tool definitions in the spec's `{type, name, description, parameters}`
    /// shape. Everruns events carry tool summaries without parameter schemas,
    /// so `parameters` is omitted.
    pub fn tool_definitions(tools: &[ToolDefinitionSummary]) -> Value {
        Value::Array(
            tools
                .iter()
                .map(|t| {
                    json!({
                        "type": gen_ai::tool_type::FUNCTION,
                        "name": t.name,
                        "description": t.description,
                    })
                })
                .collect(),
        )
    }

    /// A single message in the spec's `{role, parts}` shape.
    pub fn message_json(message: &Message) -> Value {
        let parts: Vec<Value> = message.content.iter().filter_map(part_json).collect();
        json!({ "role": role_name(&message.role), "parts": parts })
    }

    /// The spec role for an Everruns message role.
    pub fn role_name(role: &MessageRole) -> &'static str {
        match role {
            MessageRole::System => gen_ai::role::SYSTEM,
            MessageRole::User => gen_ai::role::USER,
            MessageRole::Agent => gen_ai::role::ASSISTANT,
            MessageRole::ToolResult => gen_ai::role::TOOL,
        }
    }

    /// One content part in the spec's part shape. Image bytes are never
    /// copied into telemetry: base64 images become a `blob` part that names
    /// the media type only.
    pub fn part_json(part: &ContentPart) -> Option<Value> {
        match part {
            ContentPart::Text(t) => Some(json!({
                "type": gen_ai::part_type::TEXT,
                "content": t.text,
            })),
            ContentPart::ToolCall(tc) => Some(json!({
                "type": gen_ai::part_type::TOOL_CALL,
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            })),
            ContentPart::ToolResult(tr) => {
                let response = match (&tr.error, &tr.result) {
                    (Some(error), _) => json!({ "error": error }),
                    (None, Some(result)) => result.clone(),
                    (None, None) => Value::Null,
                };
                Some(json!({
                    "type": gen_ai::part_type::TOOL_CALL_RESPONSE,
                    "id": tr.tool_call_id,
                    "response": response,
                }))
            }
            ContentPart::Image(img) => {
                if let Some(url) = &img.url {
                    Some(json!({
                        "type": gen_ai::part_type::URI,
                        "modality": "image",
                        "uri": url,
                    }))
                } else if img.base64.is_some() {
                    Some(json!({
                        "type": gen_ai::part_type::BLOB,
                        "modality": "image",
                        "mime_type": img.media_type.as_deref().unwrap_or("image/png"),
                    }))
                } else {
                    None
                }
            }
            ContentPart::ImageFile(file) => Some(json!({
                "type": gen_ai::part_type::URI,
                "modality": "image",
                "uri": format!("image_file:{}", file.image_id),
            })),
            ContentPart::Reasoning(reasoning) => {
                let text = match reasoning.text.as_ref()? {
                    ReasoningText::Plain { text } => text.clone(),
                    ReasoningText::Summary { parts } => parts.join("\n"),
                    ReasoningText::Redacted => return None,
                };
                Some(json!({
                    "type": gen_ai::part_type::REASONING,
                    "content": text,
                }))
            }
        }
    }
}

// ============================================================================
// Span Helpers
// ============================================================================

/// Create a span name for LLM chat operations following gen-ai conventions
///
/// Format: `{operation_name} {model_name}`
/// Example: "chat gpt-4"
pub fn chat_span_name(model: &str) -> String {
    format!("{} {}", gen_ai::operation::CHAT, model)
}

/// Create a span name for tool execution following gen-ai conventions
///
/// Format: `execute_tool {tool_name}`
/// Example: "execute_tool read_file"
pub fn tool_span_name(tool_name: &str) -> String {
    format!("{} {}", gen_ai::operation::EXECUTE_TOOL, tool_name)
}

/// Create a span name for text completion following gen-ai conventions
///
/// Format: `text_completion {model_name}`
/// Example: "text_completion gpt-3.5-turbo-instruct"
pub fn text_completion_span_name(model: &str) -> String {
    format!("{} {}", gen_ai::operation::TEXT_COMPLETION, model)
}

/// Create a span name for agent creation following gen-ai conventions
///
/// Format: `create_agent {agent_name}`
/// Example: "create_agent customer_support_agent"
pub fn create_agent_span_name(agent_name: &str) -> String {
    format!("{} {}", gen_ai::operation::CREATE_AGENT, agent_name)
}

/// Create a span name for agent invocation following gen-ai conventions
///
/// Format: `invoke_agent {agent_name}` when the agent name is known, plain
/// `invoke_agent` otherwise.
pub fn invoke_agent_span_name(agent_name: Option<&str>) -> String {
    match agent_name.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => format!("{} {}", gen_ai::operation::INVOKE_AGENT, name),
        None => gen_ai::operation::INVOKE_AGENT.to_string(),
    }
}

/// Create a span name for embeddings following gen-ai conventions
///
/// Format: `embeddings {model_name}`
/// Example: "embeddings text-embedding-ada-002"
pub fn embeddings_span_name(model: &str) -> String {
    format!("{} {}", gen_ai::operation::EMBEDDINGS, model)
}

/// Classify an error into the low-cardinality `error.type` the conventions
/// ask for. An explicit code wins; otherwise a provider HTTP status embedded in
/// the message (`503`) or a timeout is recognized; everything else is
/// `_OTHER`, with the full message kept in the span status description.
pub fn error_type(code: Option<&str>, message: &str) -> String {
    if let Some(code) = code.map(str::trim).filter(|c| !c.is_empty()) {
        return code.to_string();
    }
    let lowered = message.to_ascii_lowercase();
    if lowered.contains("timed out") || lowered.contains("timeout") {
        return "timeout".to_string();
    }
    if let Some(status) = http_status_in(message) {
        return status.to_string();
    }
    gen_ai::ERROR_TYPE_OTHER.to_string()
}

/// First standalone 4xx/5xx status code in the text, if any.
fn http_status_in(text: &str) -> Option<u16> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let window = &bytes[i..i + 3];
        let standalone_before = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let standalone_after = i + 3 == bytes.len() || !bytes[i + 3].is_ascii_alphanumeric();
        if standalone_before
            && standalone_after
            && window.iter().all(u8::is_ascii_digit)
            && matches!(window[0], b'4' | b'5')
        {
            return std::str::from_utf8(window).ok()?.parse().ok();
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_span_name() {
        assert_eq!(chat_span_name("gpt-4"), "chat gpt-4");
        assert_eq!(chat_span_name("claude-opus-5"), "chat claude-opus-5");
    }

    #[test]
    fn test_tool_span_name() {
        assert_eq!(tool_span_name("read_file"), "execute_tool read_file");
        assert_eq!(tool_span_name("web_search"), "execute_tool web_search");
    }

    #[test]
    fn test_text_completion_span_name() {
        assert_eq!(
            text_completion_span_name("gpt-3.5-turbo-instruct"),
            "text_completion gpt-3.5-turbo-instruct"
        );
    }

    #[test]
    fn test_create_agent_span_name() {
        assert_eq!(
            create_agent_span_name("customer_support"),
            "create_agent customer_support"
        );
    }

    #[test]
    fn test_invoke_agent_span_name() {
        assert_eq!(
            invoke_agent_span_name(Some("customer_support")),
            "invoke_agent customer_support"
        );
        assert_eq!(invoke_agent_span_name(None), "invoke_agent");
        assert_eq!(invoke_agent_span_name(Some("  ")), "invoke_agent");
    }

    #[test]
    fn test_embeddings_span_name() {
        assert_eq!(
            embeddings_span_name("text-embedding-ada-002"),
            "embeddings text-embedding-ada-002"
        );
    }

    #[test]
    fn provider_names_follow_the_registry() {
        assert_eq!(gen_ai::provider::from_driver_id("openai"), "openai");
        assert_eq!(gen_ai::provider::from_driver_id("anthropic"), "anthropic");
        assert_eq!(gen_ai::provider::from_driver_id("gemini"), "gcp.gemini");
        assert_eq!(gen_ai::provider::from_driver_id("bedrock"), "aws.bedrock");
        assert_eq!(
            gen_ai::provider::from_driver_id("azure_openai"),
            "azure.ai.openai"
        );
        assert_eq!(gen_ai::provider::from_driver_id("openrouter"), "openrouter");
        assert_eq!(gen_ai::provider::from_driver_id("llmsim"), "llmsim");
    }

    #[test]
    fn error_type_prefers_codes_then_status_then_other() {
        assert_eq!(
            error_type(Some("budget_exhausted"), "anything"),
            "budget_exhausted"
        );
        assert_eq!(error_type(None, "provider returned 503"), "503");
        assert_eq!(error_type(None, "request timed out after 30s"), "timeout");
        assert_eq!(error_type(None, "HTTP 429 Too Many Requests"), "429");
        assert_eq!(error_type(None, "id 12345 not found"), "_OTHER");
        assert_eq!(error_type(None, "something broke"), "_OTHER");
    }

    #[test]
    fn system_messages_go_to_instructions_not_history() {
        let messages = vec![
            Message::system("Be terse."),
            Message::user("Hi"),
            Message::assistant("Hello"),
        ];
        let instructions = content::system_instructions(&messages).unwrap();
        assert_eq!(
            instructions,
            json!([{ "type": "text", "content": "Be terse." }])
        );
        let history = content::input_messages(&messages);
        assert_eq!(
            history,
            json!([
                { "role": "user", "parts": [{ "type": "text", "content": "Hi" }] },
                { "role": "assistant", "parts": [{ "type": "text", "content": "Hello" }] },
            ])
        );
        assert!(content::system_instructions(&[Message::user("x")]).is_none());
    }

    #[test]
    fn tool_calls_and_results_use_spec_part_types() {
        let call = ToolCall {
            id: "call_1".to_string(),
            name: "get_weather".to_string(),
            arguments: json!({ "city": "Paris" }),
        };
        let messages = vec![
            Message::assistant_with_tools("Checking", vec![call.clone()]),
            Message::tool_result("call_1", Some(json!({ "temp": 21 })), None),
        ];
        let history = content::input_messages(&messages);
        assert_eq!(
            history,
            json!([
                {
                    "role": "assistant",
                    "parts": [
                        { "type": "text", "content": "Checking" },
                        { "type": "tool_call", "id": "call_1", "name": "get_weather",
                          "arguments": { "city": "Paris" } },
                    ]
                },
                {
                    "role": "tool",
                    "parts": [
                        { "type": "tool_call_response", "id": "call_1", "response": { "temp": 21 } },
                    ]
                },
            ])
        );

        let output = content::output_messages(
            Some("Sunny"),
            &[call],
            Some("thinking..."),
            Some("tool_calls"),
        );
        assert_eq!(
            output,
            json!([{
                "role": "assistant",
                "finish_reason": "tool_calls",
                "parts": [
                    { "type": "reasoning", "content": "thinking..." },
                    { "type": "text", "content": "Sunny" },
                    { "type": "tool_call", "id": "call_1", "name": "get_weather",
                      "arguments": { "city": "Paris" } },
                ]
            }])
        );
    }

    #[test]
    fn tool_errors_become_error_responses() {
        let messages = vec![Message::tool_result(
            "call_2",
            None,
            Some("boom".to_string()),
        )];
        let history = content::input_messages(&messages);
        assert_eq!(
            history[0]["parts"][0],
            json!({ "type": "tool_call_response", "id": "call_2", "response": { "error": "boom" } })
        );
    }

    #[test]
    fn image_bytes_never_reach_telemetry() {
        let mut message = Message::user("");
        message.content = vec![
            ContentPart::image_url("https://example.com/a.png"),
            ContentPart::Image(crate::message::ImageContentPart::from_base64(
                "AAAA",
                "image/jpeg",
            )),
        ];
        let history = content::input_messages(&[message]);
        assert_eq!(
            history[0]["parts"],
            json!([
                { "type": "uri", "modality": "image", "uri": "https://example.com/a.png" },
                { "type": "blob", "modality": "image", "mime_type": "image/jpeg" },
            ])
        );
    }

    #[test]
    fn tool_definitions_are_function_typed() {
        let tools = vec![ToolDefinitionSummary {
            name: "read_file".to_string(),
            display_name: None,
            category: None,
            capability_id: None,
            capability_name: None,
            description: "Read a file".to_string(),
        }];
        assert_eq!(
            content::tool_definitions(&tools),
            json!([{ "type": "function", "name": "read_file", "description": "Read a file" }])
        );
    }
}
