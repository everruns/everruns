// OpenTelemetry Initialization (EVE-876)
//
// OTLP exporter wiring, tracing-subscriber layers, and the telemetry
// configuration/guard moved here from `everruns_core::telemetry` so the
// neutral kernel carries no OpenTelemetry SDK, exporter, or subscriber
// dependencies. Core keeps only the vendor-neutral gen-AI span conventions
// (`everruns_core::telemetry::gen_ai`) and span-name helpers that execution
// and exporters share.

use opentelemetry::KeyValue;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    trace::{RandomIdGenerator, Sampler, SdkTracerProvider},
};
use std::time::Duration;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

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
    /// - `OTEL_SDK_DISABLED`: If "true", disables OpenTelemetry tracing entirely
    /// - `OTEL_SERVICE_NAME`: Service name (default: "everruns")
    /// - `OTEL_SERVICE_VERSION`: Service version
    /// - `OTEL_EXPORTER_OTLP_ENDPOINT`: OTLP endpoint (e.g., "http://localhost:4317")
    /// - `OTEL_ENVIRONMENT`: Deployment environment
    /// - `RUST_LOG` or `LOG_LEVEL`: Log filter
    /// - `OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT`: Record input/output content (standard OTel)
    /// - `OTEL_RECORD_CONTENT`: Legacy alias for content recording
    pub fn from_env() -> Self {
        use everruns_core::config::{env_bool, env_opt_string, env_string};

        let sdk_disabled = env_bool("OTEL_SDK_DISABLED", false);

        Self {
            service_name: env_string("OTEL_SERVICE_NAME", "everruns"),
            service_version: env_opt_string("OTEL_SERVICE_VERSION"),
            otlp_endpoint: if sdk_disabled {
                None
            } else {
                env_opt_string("OTEL_EXPORTER_OTLP_ENDPOINT")
            },
            environment: env_opt_string("OTEL_ENVIRONMENT"),
            enable_console: true,
            log_filter: env_opt_string("RUST_LOG").or_else(|| env_opt_string("LOG_LEVEL")),
            // Standard OTel env var, with legacy alias fallback
            record_content: std::env::var("OTEL_INSTRUMENTATION_GENAI_CAPTURE_MESSAGE_CONTENT")
                .or_else(|_| std::env::var("OTEL_RECORD_CONTENT"))
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
        if let Some(provider) = self._provider.take()
            && let Err(e) = provider.shutdown()
        {
            eprintln!("Failed to shutdown tracer provider: {:?}", e);
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
/// use everruns_host::observability::{init_telemetry, TelemetryConfig};
///
/// #[tokio::main]
/// async fn main() {
///     let config = TelemetryConfig::from_env();
///     let _guard = init_telemetry(config);
///     // ... your application code
/// }
/// ```
pub fn init_telemetry(config: TelemetryConfig) -> TelemetryGuard {
    everruns_provider::install_default_crypto_provider();

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
                // `OtelEventListener` builds its Gen-AI spans on the global
                // tracer so they can carry event timestamps; `tracing` spans
                // (HTTP requests) keep flowing through the layer above.
                opentelemetry::global::set_tracer_provider(provider.clone());
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
) -> Result<
    (SdkTracerProvider, opentelemetry_sdk::trace::Tracer),
    Box<dyn std::error::Error + Send + Sync>,
> {
    // Use HTTP OTLP instead of gRPC - more reliable with Docker DNS
    let exporter = SpanExporter::builder()
        .with_http()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_defaults() {
        let config = TelemetryConfig::default();
        assert_eq!(config.service_name, "everruns");
        assert!(config.otlp_endpoint.is_none());
        assert!(config.enable_console);
        assert!(!config.record_content);
    }
}
