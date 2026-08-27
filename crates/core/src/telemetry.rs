// Telemetry Conventions
//
// Vendor-neutral gen-AI span metadata contracts shared by execution and
// exporters:
// - Gen-AI semantic convention attribute names for LLM operations
// - Span-name helpers with proper attribute naming
//
// OpenTelemetry initialization (OTLP exporter wiring, tracing-subscriber
// layers, TelemetryConfig/TelemetryGuard) lives behind `everruns-host/observability`
// (EVE-876) so core carries no OTel SDK, exporter, or subscriber dependencies.
// Guard: scripts/lib/check-observability-isolation.sh.

// ============================================================================
// Gen-AI Semantic Conventions
// See: https://opentelemetry.io/docs/specs/semconv/gen-ai/
// ============================================================================

/// Gen-AI semantic convention attribute names
pub mod gen_ai {
    // Operation and provider attributes
    /// The name of the operation being performed (e.g., "chat", "embeddings")
    pub const OPERATION_NAME: &str = "gen_ai.operation.name";
    /// The name of the GenAI provider (e.g., "openai", "anthropic")
    pub const PROVIDER_NAME: &str = "gen_ai.provider.name";

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

    // Response attributes
    /// Unique identifier for the completion
    pub const RESPONSE_ID: &str = "gen_ai.response.id";
    /// The actual model used (may differ from requested)
    pub const RESPONSE_MODEL: &str = "gen_ai.response.model";
    /// Reasons why generation stopped
    pub const RESPONSE_FINISH_REASONS: &str = "gen_ai.response.finish_reasons";

    // Token usage attributes
    /// Number of tokens in the input/prompt
    pub const USAGE_INPUT_TOKENS: &str = "gen_ai.usage.input_tokens";
    /// Number of tokens in the output/completion
    pub const USAGE_OUTPUT_TOKENS: &str = "gen_ai.usage.output_tokens";
    /// Number of tokens read from cache (reduces cost)
    pub const USAGE_CACHE_READ_TOKENS: &str = "gen_ai.usage.cache_read_tokens";
    /// Number of tokens written to cache (Anthropic-specific)
    pub const USAGE_CACHE_CREATION_TOKENS: &str = "gen_ai.usage.cache_creation_tokens";

    // Content attributes (opt-in, may contain sensitive data)
    /// Input messages/prompts
    pub const INPUT_MESSAGES: &str = "gen_ai.input.messages";
    /// Output messages/completions
    pub const OUTPUT_MESSAGES: &str = "gen_ai.output.messages";
    /// System instructions/prompts
    pub const SYSTEM_INSTRUCTIONS: &str = "gen_ai.system_instructions";
    /// Tool definitions available
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

    // Embeddings attributes
    /// Number of dimensions in output embeddings
    pub const EMBEDDINGS_DIMENSION_COUNT: &str = "gen_ai.embeddings.dimension.count";
    /// Requested encoding formats
    pub const REQUEST_ENCODING_FORMATS: &str = "gen_ai.request.encoding_formats";

    // Additional request attributes
    /// Number of response candidates to generate
    pub const REQUEST_CHOICE_COUNT: &str = "gen_ai.request.choice.count";

    // Output attributes
    /// Output modality type (text, image, json, speech)
    pub const OUTPUT_TYPE: &str = "gen_ai.output.type";

    // Agent attributes (extension for agent frameworks)
    /// Agent identifier
    pub const AGENT_ID: &str = "gen_ai.agent.id";
    /// Agent name
    pub const AGENT_NAME: &str = "gen_ai.agent.name";
    /// Agent description
    pub const AGENT_DESCRIPTION: &str = "gen_ai.agent.description";

    // Server attributes
    /// GenAI server address
    pub const SERVER_ADDRESS: &str = "server.address";
    /// GenAI server port
    pub const SERVER_PORT: &str = "server.port";

    // System attribute (gen_ai.system — provider identifier for older convention usage)
    /// Provider system identifier (e.g., "openai", "anthropic")
    pub const SYSTEM: &str = "gen_ai.system";

    /// Operation names as per semantic conventions
    pub mod operation {
        pub const CHAT: &str = "chat";
        pub const EMBEDDINGS: &str = "embeddings";
        pub const TEXT_COMPLETION: &str = "text_completion";
        pub const GENERATE_CONTENT: &str = "generate_content";
        pub const EXECUTE_TOOL: &str = "execute_tool";
        pub const CREATE_AGENT: &str = "create_agent";
        pub const INVOKE_AGENT: &str = "invoke_agent";
        // Phase operations (agentic loop specific)
        pub const REASON: &str = "reason";
        pub const ACT: &str = "act";
        pub const THINKING: &str = "thinking";
    }

    /// Provider names as per semantic conventions
    pub mod provider {
        pub const OPENAI: &str = "openai";
        pub const ANTHROPIC: &str = "anthropic";
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
/// Format: `invoke_agent {agent_name}`
/// Example: "invoke_agent customer_support_agent"
pub fn invoke_agent_span_name(agent_name: &str) -> String {
    format!("{} {}", gen_ai::operation::INVOKE_AGENT, agent_name)
}

/// Create a span name for embeddings following gen-ai conventions
///
/// Format: `embeddings {model_name}`
/// Example: "embeddings text-embedding-ada-002"
pub fn embeddings_span_name(model: &str) -> String {
    format!("{} {}", gen_ai::operation::EMBEDDINGS, model)
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
            invoke_agent_span_name("customer_support"),
            "invoke_agent customer_support"
        );
    }

    #[test]
    fn test_embeddings_span_name() {
        assert_eq!(
            embeddings_span_name("text-embedding-ada-002"),
            "embeddings text-embedding-ada-002"
        );
    }
}
