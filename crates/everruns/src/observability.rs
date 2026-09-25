//! Built-in Engine observability integrations.
//!
//! Stability: alpha — may change without a major bump; see [`crate::stability`].

use std::sync::Arc;

use everruns_core::EventListener as CoreEventListener;

/// A built-in integration accepted by [`crate::EngineBuilder::observe`].
///
/// This trait is sealed. Applications can use [`OpenTelemetry`] and
/// [`Braintrust`] when their matching Cargo features are enabled.
pub trait Observation: private::Sealed {}

impl<T: private::Sealed> Observation for T {}

pub(crate) fn into_listener(observation: impl Observation) -> Arc<dyn CoreEventListener> {
    private::Sealed::into_listener(observation)
}

mod private {
    use std::sync::Arc;

    use everruns_core::EventListener as CoreEventListener;

    pub trait Sealed {
        fn into_listener(self) -> Arc<dyn CoreEventListener>;
    }
}

/// OpenTelemetry SDK tracer provider accepted by
/// [`OpenTelemetry::with_tracer_provider`].
#[cfg(feature = "otel")]
pub use everruns_host::observability::SdkTracerProvider;
/// Guard that flushes and shuts down the provider installed by
/// [`install_otlp_from_env`].
#[cfg(feature = "otel")]
pub use everruns_host::observability::TelemetryGuard;

#[cfg(feature = "otel")]
enum OpenTelemetrySource {
    Global,
    Provider(SdkTracerProvider),
}

/// OpenTelemetry event integration for an [`crate::Engine`].
///
/// Constructing this value never installs global tracer or subscriber state.
/// Use [`install_otlp_from_env`] only when the application wants that
/// process-global convenience.
#[cfg(feature = "otel")]
pub struct OpenTelemetry {
    source: OpenTelemetrySource,
    record_content: bool,
    conventions: everruns_host::observability::TraceConventions,
}

#[cfg(feature = "otel")]
impl OpenTelemetry {
    /// Use the tracer provider already installed by the application.
    ///
    /// Message content capture is disabled.
    pub fn global() -> Self {
        Self {
            source: OpenTelemetrySource::Global,
            record_content: false,
            conventions: everruns_host::observability::TraceConventions::ALL,
        }
    }

    /// Use an application-owned SDK tracer provider.
    ///
    /// Message content capture is disabled. The provider is retained so
    /// [`crate::Engine::shutdown`] can force-flush completed spans.
    pub fn with_tracer_provider(provider: SdkTracerProvider) -> Self {
        Self {
            source: OpenTelemetrySource::Provider(provider),
            record_content: false,
            conventions: everruns_host::observability::TraceConventions::ALL,
        }
    }

    /// Use the global tracer and read content capture and convention settings
    /// from the OpenTelemetry environment variables.
    pub fn from_env() -> Self {
        let listener = everruns_host::observability::OtelEventListener::new();
        Self {
            source: OpenTelemetrySource::Global,
            record_content: listener.record_content(),
            conventions: listener.conventions(),
        }
    }

    /// Enable or disable prompt, message, reasoning, and tool payload capture.
    pub fn record_content(mut self, record_content: bool) -> Self {
        self.record_content = record_content;
        self
    }
}

#[cfg(feature = "otel")]
impl private::Sealed for OpenTelemetry {
    fn into_listener(self) -> Arc<dyn CoreEventListener> {
        let listener = match self.source {
            OpenTelemetrySource::Global => {
                everruns_host::observability::OtelEventListener::with_global(
                    self.record_content,
                    self.conventions,
                )
            }
            OpenTelemetrySource::Provider(provider) => {
                everruns_host::observability::OtelEventListener::with_tracer_provider(
                    provider,
                    self.record_content,
                    self.conventions,
                )
            }
        };
        Arc::new(listener)
    }
}

/// Install an OTLP exporter and tracing subscriber from environment settings.
///
/// This is the only Framework observability helper that installs process-global
/// tracer and subscriber state. Keep the returned guard alive until after
/// [`crate::Engine::shutdown`].
#[cfg(feature = "otel")]
pub fn install_otlp_from_env() -> TelemetryGuard {
    everruns_host::observability::init_telemetry(
        everruns_host::observability::TelemetryConfig::from_env(),
    )
}

/// Braintrust event integration for an [`crate::Engine`].
#[cfg(feature = "braintrust")]
pub struct Braintrust {
    listener: everruns_host::observability::BraintrustListener,
}

#[cfg(feature = "braintrust")]
impl Braintrust {
    /// Create an integration from Braintrust environment settings.
    ///
    /// Returns `None` when Braintrust is disabled, credentials are absent, or
    /// the HTTP client cannot be initialized.
    pub fn from_env() -> Option<Self> {
        everruns_host::observability::BraintrustListener::from_env()
            .map(|listener| Self { listener })
    }
}

#[cfg(feature = "braintrust")]
impl private::Sealed for Braintrust {
    fn into_listener(self) -> Arc<dyn CoreEventListener> {
        Arc::new(self.listener)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use everruns_provider::tool_types::ToolCall;
    use serde_json::json;

    use crate::{Agent, Engine, FunctionTool, Model};

    fn tool_agent() -> Agent {
        let tool = FunctionTool::new(
            "lookup",
            "Look up a value.",
            json!({
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"]
            }),
            |arguments: serde_json::Value| async move {
                Ok::<_, String>(json!({ "value": arguments["key"] }))
            },
        );
        Agent::builder()
            .instructions("Use the lookup tool.")
            .model(Model::simulated_scripted(
                "done",
                vec![
                    vec![ToolCall {
                        id: "call_lookup_1".to_string(),
                        name: "lookup".to_string(),
                        arguments: json!({ "key": "answer" }),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent")
    }

    #[cfg(feature = "otel")]
    #[tokio::test]
    async fn framework_run_exports_agent_chat_and_tool_spans() {
        use everruns_host::observability::InMemorySpanExporter;

        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let engine = Engine::builder()
            .observe(OpenTelemetry::with_tracer_provider(provider))
            .build();

        engine
            .create(tool_agent())
            .run("look it up")
            .await
            .expect("tool turn runs");
        let report = engine.shutdown(Duration::from_secs(5)).await;

        assert!(!report.timed_out);
        let names = exporter
            .get_finished_spans()
            .expect("in-memory exporter returns spans")
            .into_iter()
            .map(|span| span.name.to_string())
            .collect::<Vec<_>>();
        for expected in ["invoke_agent", "chat", "execute_tool"] {
            assert!(
                names.iter().any(|name| name.starts_with(expected)),
                "missing {expected} span: {names:?}"
            );
        }
    }

    #[cfg(feature = "braintrust")]
    #[tokio::test]
    async fn framework_shutdown_waits_for_final_braintrust_batch() {
        use everruns_core::DeploymentGrade;
        use everruns_host::observability::braintrust::{
            BraintrustConfig, BraintrustContentConfig, BraintrustDeliveryConfig, BraintrustListener,
        };
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/project_logs/test-project/insert"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let listener = BraintrustListener::new(BraintrustConfig {
            api_key: "test-key".to_string(),
            project_id: "test-project".to_string(),
            api_url: server.uri(),
            delivery: BraintrustDeliveryConfig {
                flush_interval: Duration::from_secs(3600),
                max_batch_size: 100,
                ..BraintrustDeliveryConfig::default()
            },
            content: BraintrustContentConfig::default(),
            deployment_grade: DeploymentGrade::Dev,
        })
        .expect("mock Braintrust client builds");
        let engine = Engine::builder().observe(Braintrust { listener }).build();

        engine
            .create(tool_agent())
            .run("look it up")
            .await
            .expect("tool turn runs");
        let report = engine.shutdown(Duration::from_secs(5)).await;

        assert!(!report.timed_out);
        let requests = server
            .received_requests()
            .await
            .expect("mock server records requests");
        assert_eq!(requests.len(), 1);
        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("valid Braintrust request");
        assert!(
            body["events"]
                .as_array()
                .is_some_and(|events| !events.is_empty())
        );
    }
}
