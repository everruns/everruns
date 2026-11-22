//! Content-free delivery outcomes under the authoritative turn trace.
//!
//! Delivery is leased and can run on a different process from the producer.
//! The committed host-generated trace carrier, not an ambient tracing span,
//! lets the resulting span stay with the turn that generated the reply.

use super::DeliveryResult;

#[derive(Clone, Copy)]
pub(super) enum DeliveryOutcome {
    Succeeded,
    Transient,
    Permanent,
}

impl DeliveryOutcome {
    pub(super) fn error_type(self) -> Option<&'static str> {
        match self {
            Self::Succeeded => None,
            Self::Transient => Some("transient_delivery"),
            Self::Permanent => Some("permanent_delivery"),
        }
    }

    pub(super) fn or(self, next: Self) -> Self {
        if self.error_type().is_some() {
            self
        } else {
            next
        }
    }
}

impl From<&DeliveryResult> for DeliveryOutcome {
    fn from(result: &DeliveryResult) -> Self {
        match result {
            DeliveryResult::Ok => Self::Succeeded,
            DeliveryResult::TransientError(_) => Self::Transient,
            DeliveryResult::PermanentError(_) => Self::Permanent,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum DeliveryKind {
    Approval,
    Reply,
}

#[cfg(feature = "otel")]
#[derive(Default)]
pub(super) struct DeliveryTelemetry {
    parent: Option<opentelemetry::Context>,
    #[cfg(test)]
    pub(super) tracer: Option<opentelemetry::global::BoxedTracer>,
}

#[cfg(feature = "otel")]
impl DeliveryTelemetry {
    pub(super) fn anchor(&mut self, carrier: &str) {
        use opentelemetry::trace::TraceContextExt;
        // THREAT[TM-OBS-015]: only the committed host anchor can be a parent.
        use crate::host::observability::otel::causal::committed_turn_span_context;
        if self.parent.is_none() {
            self.parent = committed_turn_span_context(carrier)
                .map(|span| opentelemetry::Context::new().with_remote_span_context(span));
        }
    }

    pub(super) fn start(&self, kind: DeliveryKind) -> DeliverySpan {
        use opentelemetry::trace::{SpanKind, Tracer};
        let Some(parent) = &self.parent else {
            return DeliverySpan(None);
        };
        let kind = match kind {
            DeliveryKind::Approval => "approval",
            DeliveryKind::Reply => "reply",
        };
        let builder = opentelemetry::trace::SpanBuilder::from_name("agent.channel.deliver")
            .with_kind(SpanKind::Producer)
            .with_attributes(vec![
                opentelemetry::KeyValue::new("agent.channel", "slack"),
                opentelemetry::KeyValue::new("agent.channel.delivery.kind", kind),
            ]);
        #[cfg(test)]
        if let Some(tracer) = &self.tracer {
            return DeliverySpan(Some(tracer.build_with_context(builder, parent)));
        }
        let tracer = opentelemetry::global::tracer("everruns.channel");
        DeliverySpan(Some(tracer.build_with_context(builder, parent)))
    }
}

#[cfg(feature = "otel")]
pub(super) struct DeliverySpan(Option<opentelemetry::global::BoxedSpan>);

#[cfg(feature = "otel")]
impl DeliverySpan {
    pub(super) fn finish(self, outcome: DeliveryOutcome) {
        use opentelemetry::KeyValue;
        use opentelemetry::trace::{Span, Status};
        let Some(mut span) = self.0 else { return };
        // THREAT[TM-OBS-010]: classification only; no adapter error strings.
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

#[cfg(not(feature = "otel"))]
#[derive(Default)]
pub(super) struct DeliveryTelemetry {}

#[cfg(not(feature = "otel"))]
impl DeliveryTelemetry {
    pub(super) fn anchor(&mut self, _: &str) {}

    pub(super) fn start(&self, _: DeliveryKind) -> DeliverySpan {
        DeliverySpan
    }
}

#[cfg(not(feature = "otel"))]
pub(super) struct DeliverySpan;

#[cfg(not(feature = "otel"))]
impl DeliverySpan {
    pub(super) fn finish(self, _: DeliveryOutcome) {}
}
