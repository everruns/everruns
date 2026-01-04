// OpenTelemetry Telemetry Module
//
// This module provides OpenTelemetry integration for Everruns, including:
// - Gen-AI semantic conventions for LLM operations
// - Initialization helpers for OTLP exporters
// - Span creation helpers with proper attribute naming

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
    Resource,
};
use std::time::Duration;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

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

    // Server attributes
    /// GenAI server address
    pub const SERVER_ADDRESS: &str = "server.address";
    /// GenAI server port
    pub const SERVER_PORT: &str = "server.port";

    /// Operation names as per semantic conventions
    pub mod operation {
        pub const CHAT: &str = "chat";
        pub const EMBEDDINGS: &str = "embeddings";
        pub const TEXT_COMPLETION: &str = "text_completion";
        pub const GENERATE_CONTENT: &str = "generate_content";
        pub const EXECUTE_TOOL: &str = "execute_tool";
        pub const CREATE_AGENT: &str = "create_agent";
        pub const INVOKE_AGENT: &str = "invoke_agent";
    }

    /// Provider names as per semantic conventions
    pub mod provider {
        pub const OPENAI: &str = "openai";
        pub const ANTHROPIC: &str = "anthropic";
        pub const AZURE_OPENAI: &str = "azure.ai.openai";
        pub const AWS_BEDROCK: &str = "aws.bedrock";
        pub const GCP_VERTEX_AI: &str = "gcp.vertex_ai";
    }

    /// Tool types as per semantic conventions
    pub mod tool_type {
        pub const FUNCTION: &str = "function";
        pub const EXTENSION: &str = "extension";
        pub const DATASTORE: &str = "datastore";
    }
}

// ============================================================================
// Telemetry Configuration
// ============================================================================

/// Configuration for OpenTelemetry
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Service name for traces
    pub service_name: String,
    /// Service version
    pub service_version: Option<String>,
    /// OTLP endpoint (e.g., "http://localhost:4317")
    pub otlp_endpoint: Option<String>,
    /// Environment (e.g., "development", "production")
    pub environment: Option<String>,
    /// Whether to enable console logging
    pub enable_console: bool,
    /// Log filter (e.g., "info", "debug", "everruns=debug")
    pub log_filter: Option<String>,
    /// Whether to enable content recording (input/output messages)
    /// Disabled by default for privacy/security
    pub record_content: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "everruns".to_string(),
            service_version: None,
            otlp_endpoint: None,
            environment: None,
            enable_console: true,
            log_filter: None,
            record_content: false,
        }
    }
}

impl TelemetryConfig {
    /// Create configuration from environment variables
    ///
    /// Environment variables:
    /// - `OTEL_SERVICE_NAME`: Service name (default: "everruns")
    /// - `OTEL_SERVICE_VERSION`: Service version
    /// - `OTEL_EXPORTER_OTLP_ENDPOINT`: OTLP endpoint (e.g., "http://localhost:4317")
    /// - `OTEL_ENVIRONMENT`: Deployment environment
    /// - `RUST_LOG` or `LOG_LEVEL`: Log filter
    /// - `OTEL_RECORD_CONTENT`: Whether to record input/output content ("true" to enable)
    pub fn from_env() -> Self {
        Self {
            service_name: std::env::var("OTEL_SERVICE_NAME")
                .unwrap_or_else(|_| "everruns".to_string()),
            service_version: std::env::var("OTEL_SERVICE_VERSION").ok(),
            otlp_endpoint: std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok(),
            environment: std::env::var("OTEL_ENVIRONMENT").ok(),
            enable_console: true,
            log_filter: std::env::var("RUST_LOG")
                .ok()
                .or_else(|| std::env::var("LOG_LEVEL").ok()),
            record_content: std::env::var("OTEL_RECORD_CONTENT")
                .map(|v| v.to_lowercase() == "true")
                .unwrap_or(false),
        }
    }
}

// ============================================================================
// Initialization
// ============================================================================

/// Guard that shuts down the tracer provider when dropped
pub struct TelemetryGuard {
    _provider: Option<SdkTracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(provider) = self._provider.take() {
            if let Err(e) = provider.shutdown() {
                eprintln!("Failed to shutdown tracer provider: {:?}", e);
            }
        }
    }
}

/// Initialize OpenTelemetry with the given configuration
///
/// Returns a guard that will shut down the tracer provider when dropped.
/// Keep this guard alive for the lifetime of your application.
///
/// # Example
///
/// ```ignore
/// use everruns_core::telemetry::{init_telemetry, TelemetryConfig};
///
/// #[tokio::main]
/// async fn main() {
///     let config = TelemetryConfig::from_env();
///     let _guard = init_telemetry(config);
///     // ... your application code
/// }
/// ```
pub fn init_telemetry(config: TelemetryConfig) -> TelemetryGuard {
    // Build resource with service info
    let mut resource_attrs = vec![KeyValue::new("service.name", config.service_name.clone())];

    if let Some(version) = &config.service_version {
        resource_attrs.push(KeyValue::new("service.version", version.clone()));
    }

    if let Some(env) = &config.environment {
        resource_attrs.push(KeyValue::new("deployment.environment", env.clone()));
    }

    let resource = Resource::builder().with_attributes(resource_attrs).build();

    // Build log filter
    let filter = config
        .log_filter
        .as_ref()
        .and_then(|f| EnvFilter::try_new(f).ok())
        .unwrap_or_else(|| EnvFilter::new("info"));

    // Build console layer if enabled
    let console_layer = if config.enable_console {
        Some(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_filter(filter),
        )
    } else {
        None
    };

    // Build OTLP tracer if endpoint is configured
    let (tracer_provider, otel_layer, otel_status) = if let Some(endpoint) = &config.otlp_endpoint {
        match build_otlp_tracer(endpoint, resource) {
            Ok((provider, tracer)) => {
                let layer = tracing_opentelemetry::layer().with_tracer(tracer);
                (Some(provider), Some(layer), Some(Ok(endpoint.clone())))
            }
            Err(e) => (None, None, Some(Err(e.to_string()))),
        }
    } else {
        (None, None, None)
    };

    // Initialize the subscriber
    tracing_subscriber::registry()
        .with(console_layer)
        .with(otel_layer)
        .init();

    // Log OTEL status after subscriber is initialized
    match otel_status {
        Some(Ok(endpoint)) => {
            tracing::info!(endpoint = %endpoint, "OpenTelemetry tracing enabled");
        }
        Some(Err(e)) => {
            tracing::warn!(error = %e, "Failed to initialize OTLP tracer, continuing without tracing");
        }
        None => {
            tracing::debug!("OpenTelemetry tracing disabled: OTEL_EXPORTER_OTLP_ENDPOINT not set");
        }
    }

    TelemetryGuard {
        _provider: tracer_provider,
    }
}

fn build_otlp_tracer(
    endpoint: &str,
    resource: Resource,
) -> Result<(SdkTracerProvider, opentelemetry_sdk::trace::Tracer), opentelemetry::trace::TraceError>
{
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .with_timeout(Duration::from_secs(10))
        .build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_sampler(Sampler::AlwaysOn)
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource)
        .build();

    let tracer = provider.tracer("everruns");

    Ok((provider, tracer))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_span_name() {
        assert_eq!(chat_span_name("gpt-4"), "chat gpt-4");
        assert_eq!(chat_span_name("claude-3-opus"), "chat claude-3-opus");
    }

    #[test]
    fn test_tool_span_name() {
        assert_eq!(tool_span_name("read_file"), "execute_tool read_file");
        assert_eq!(tool_span_name("web_search"), "execute_tool web_search");
    }

    #[test]
    fn test_config_defaults() {
        let config = TelemetryConfig::default();
        assert_eq!(config.service_name, "everruns");
        assert!(config.otlp_endpoint.is_none());
        assert!(config.enable_console);
        assert!(!config.record_content);
    }
}
