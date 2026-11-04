//! Explicit customer exporters. No global providers or environment-derived credentials.
use super::{OtelEventListener, TraceConventions};
use crate::{EgressRequest, EgressRequestKind, EgressService};
use async_trait::async_trait;
use bytes::Bytes;
use opentelemetry_http::{HttpClient, HttpError};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::{
    Resource,
    trace::{BatchConfigBuilder, BatchSpanProcessor, Sampler, SdkTracerProvider},
};
use std::{sync::Arc, time::Duration};

struct EgressHttpClient {
    service: Arc<dyn EgressService>,
    runtime: tokio::runtime::Handle,
}
impl std::fmt::Debug for EgressHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TenantTracingHttpClient")
    }
}
#[async_trait]
impl HttpClient for EgressHttpClient {
    async fn send_bytes(
        &self,
        request: http::Request<Bytes>,
    ) -> Result<http::Response<Bytes>, HttpError> {
        let mut outbound = EgressRequest::new(
            "POST",
            request.uri().to_string(),
            EgressRequestKind::Other("tracing".into()),
        )
        .body(request.body().to_vec())
        .timeout_ms(10_000)
        .require_dns_pinning();
        for (k, v) in request.headers() {
            outbound
                .headers
                .insert(k.to_string(), v.to_str()?.to_string());
        }
        let service = self.service.clone();
        // The SDK batcher runs on its own OS thread. Schedule network work on the host runtime.
        let response = self
            .runtime
            .spawn(async move { service.send(outbound).await })
            .await?
            .map_err(|_| std::io::Error::other("Trace export transport failed"))?;
        if !(200..300).contains(&response.status) {
            return Err(
                std::io::Error::other(format!("Trace receiver HTTP {}", response.status)).into(),
            );
        }
        // Do not retain or log receiver response bodies.
        Ok(http::Response::builder()
            .status(response.status)
            .body(Bytes::new())?)
    }
}

pub fn otlp_listener(
    endpoint: &str,
    service: Arc<dyn EgressService>,
    capture: bool,
) -> Result<OtelEventListener, Box<dyn std::error::Error + Send + Sync>> {
    let exporter = SpanExporter::builder()
        .with_http()
        .with_http_client(EgressHttpClient {
            service,
            runtime: tokio::runtime::Handle::current(),
        })
        .with_endpoint(super::telemetry::traces_endpoint(endpoint))
        .with_protocol(Protocol::HttpBinary)
        .with_timeout(Duration::from_secs(10))
        // Override environment headers: customer credentials come exclusively from the scoped boundary.
        .with_headers(Default::default())
        .build()?;
    let processor = BatchSpanProcessor::builder(exporter)
        .with_batch_config(
            BatchConfigBuilder::default()
                .with_max_queue_size(128)
                .with_max_export_batch_size(16)
                .build(),
        )
        .build();
    let provider = SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_sampler(Sampler::AlwaysOn)
        .with_resource(
            Resource::builder_empty()
                .with_service_name("everruns-agent")
                .build(),
        )
        .build();
    Ok(OtelEventListener::with_tracer_provider(
        provider,
        capture,
        TraceConventions::ALL,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{TurnCompletedData, TurnStartedData};
    use crate::{
        EgressResponse, EgressResult, EgressStreamResponse, Event, EventContext, EventListener,
    };
    use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
    use std::sync::Mutex;
    #[derive(Default)]
    struct Collector {
        requests: Mutex<Vec<EgressRequest>>,
    }
    #[async_trait]
    impl EgressService for Collector {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            self.requests.lock().unwrap().push(request);
            Ok(EgressResponse {
                status: 200,
                headers: Default::default(),
                body: vec![],
            })
        }
        async fn send_stream(&self, _: EgressRequest) -> EgressResult<EgressStreamResponse> {
            unreachable!()
        }
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn customer_otlp_exports_through_host_boundary_without_content_or_globals() {
        let collector = Arc::new(Collector::default());
        let listener =
            otlp_listener("https://collector.example", collector.clone(), false).unwrap();
        let turn = TurnId::new();
        let message = MessageId::new();
        let session = SessionId::new();
        let context = EventContext::turn(turn, message);
        listener
            .on_event(&Event::new(
                session,
                context.clone(),
                TurnStartedData {
                    turn_id: turn,
                    input_message_id: message,
                    input_content: Some("PRIVATE_PROMPT_NEVER_EXPORT".into()),
                    agent_id: None,
                    agent_name: Some("Customer test".into()),
                    agent_description: None,
                    origin: None,
                },
            ))
            .await;
        listener
            .on_event(&Event::new(
                session,
                context,
                TurnCompletedData {
                    turn_id: turn,
                    iterations: 1,
                    final_answer_preview: Some("PRIVATE_ANSWER_NEVER_EXPORT".into()),
                    ..Default::default()
                },
            ))
            .await;
        listener.flush().await.unwrap();
        let requests = collector.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.url, "https://collector.example/v1/traces");
        assert!(request.dns_pinning_required);
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("Customer test"));
        assert!(!body.contains("PRIVATE_PROMPT"));
        assert!(!body.contains("PRIVATE_ANSWER"));
    }
}
