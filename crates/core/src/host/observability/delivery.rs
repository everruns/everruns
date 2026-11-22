//! Opt-in OpenTelemetry export for the channel host's delivery observer.

use std::time::SystemTime;

use opentelemetry::global::{self, BoxedTracer};
use opentelemetry::trace::{Span, SpanKind, Status, TraceContextExt, Tracer};
use opentelemetry::{Context, KeyValue};

use crate::channel_runtime::{DeliveryKind, DeliveryObserver, DeliveryOutcome};

use super::otel::causal::committed_turn_span_context;

/// Records outbound Slack delivery under the committed turn span.
#[derive(Default)]
pub struct OtelDeliveryObserver {
    #[cfg(test)]
    tracer: Option<BoxedTracer>,
}

impl DeliveryObserver for OtelDeliveryObserver {
    fn observe(
        &self,
        platform: &str,
        traceparent: &str,
        kind: DeliveryKind,
        outcome: DeliveryOutcome,
        start: SystemTime,
    ) {
        if platform != "slack" {
            return;
        }
        // THREAT[TM-OBS-015]: only a validated, committed host anchor is a parent.
        let Some(anchor) = committed_turn_span_context(traceparent) else {
            return;
        };
        let parent = Context::new().with_remote_span_context(anchor);
        #[cfg(test)]
        let tracer = self.tracer.as_ref();
        #[cfg(not(test))]
        let tracer: Option<&BoxedTracer> = None;
        let fallback;
        let tracer = match tracer {
            Some(tracer) => tracer,
            None => {
                fallback = global::tracer("everruns.channel");
                &fallback
            }
        };
        let kind = match kind {
            DeliveryKind::Approval => "approval",
            DeliveryKind::Reply => "reply",
        };
        let builder = tracer
            .span_builder("agent.channel.deliver")
            .with_kind(SpanKind::Producer)
            .with_start_time(start)
            .with_attributes(vec![
                KeyValue::new("agent.channel", "slack"),
                KeyValue::new("agent.channel.delivery.kind", kind),
            ]);
        let mut span = tracer.build_with_context(builder, &parent);
        // THREAT[TM-OBS-010]: classifications only; no adapter error strings.
        let status = if let Some(error_type) = outcome.error_type() {
            span.set_attribute(KeyValue::new("error.type", error_type));
            span.set_status(Status::error(error_type));
            "failed"
        } else {
            span.set_status(Status::Ok);
            "succeeded"
        };
        span.set_attribute(KeyValue::new("agent.channel.delivery.outcome", status));
        span.end();
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use std::sync::Arc;

    pub(crate) struct DeliveryTraceFixture {
        exporter: InMemorySpanExporter,
        provider: SdkTracerProvider,
    }

    impl DeliveryTraceFixture {
        pub(crate) fn new() -> Self {
            let exporter = InMemorySpanExporter::default();
            let provider = SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            Self { exporter, provider }
        }

        pub(crate) fn observer(&self) -> Arc<dyn DeliveryObserver> {
            Arc::new(OtelDeliveryObserver {
                tracer: Some(BoxedTracer::new(Box::new(self.provider.tracer("test")))),
            })
        }

        pub(crate) fn completed_turn_carrier(&self) -> String {
            let mut turn = self.provider.tracer("test").start("durable.turn");
            let carrier = super::super::otel::causal::traceparent(turn.span_context());
            turn.end();
            carrier
        }

        pub(crate) fn assert_deliveries(&self, carrier: &str, failed: bool) {
            let anchor = committed_turn_span_context(carrier).unwrap();
            let spans = self.exporter.get_finished_spans().unwrap();
            let mut deliveries = spans.iter().filter(|span| {
                span.span_context.trace_id() == anchor.trace_id()
                    && span.name == "agent.channel.deliver"
            });
            for (kind, error) in [
                ("approval", "transient_delivery"),
                ("reply", "permanent_delivery"),
            ] {
                let span = deliveries.next().unwrap();
                let attr = |key| {
                    span.attributes
                        .iter()
                        .find(|kv| kv.key.as_str() == key)
                        .map(|kv| kv.value.to_string())
                };
                assert_eq!(span.parent_span_id, anchor.span_id());
                assert_eq!(attr("agent.channel.delivery.kind").as_deref(), Some(kind));
                let outcome = if failed { "failed" } else { "succeeded" };
                assert_eq!(
                    attr("agent.channel.delivery.outcome").as_deref(),
                    Some(outcome)
                );
                assert_eq!(attr("error.type").as_deref(), failed.then_some(error));
                assert!(!format!("{span:?}").contains("PRIVATE_"));
                assert!(!format!("{span:?}").contains("https://slack.example"));
            }
            assert!(
                deliveries.next().is_none(),
                "do not count the fallback notice as the reply"
            );
        }
    }
}
