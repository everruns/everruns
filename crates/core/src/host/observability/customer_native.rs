//! Receiver-specific aliases, applied only to already privacy-filtered customer spans.
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::{
    Resource,
    error::OTelSdkResult,
    trace::{SpanData, SpanExporter},
};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Default)]
pub enum NativeTraceProvider {
    #[default]
    Generic,
    Langfuse,
    Langsmith,
}

#[derive(Debug)]
pub(super) struct NativeExporter<E> {
    pub inner: E,
    pub provider: NativeTraceProvider,
}
impl<E: SpanExporter> SpanExporter for NativeExporter<E> {
    async fn export(&self, mut batch: Vec<SpanData>) -> OTelSdkResult {
        for span in &mut batch {
            annotate(&mut span.attributes, self.provider);
        }
        self.inner.export(batch).await
    }
    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        self.inner.shutdown_with_timeout(timeout)
    }
    fn force_flush(&self) -> OTelSdkResult {
        self.inner.force_flush()
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

fn annotate(attrs: &mut Vec<KeyValue>, provider: NativeTraceProvider) {
    let original = attrs.clone();
    let get = |key: &str| {
        original
            .iter()
            .find(|kv| kv.key.as_str() == key)
            .map(|kv| kv.value.clone())
    };
    let kind = get("openinference.span.kind").map(|v| v.to_string());
    match provider {
        NativeTraceProvider::Generic => {}
        NativeTraceProvider::Langfuse => {
            let kind = match kind.as_deref() {
                Some("LLM") => "generation",
                Some("TOOL") => "tool",
                Some("AGENT") => "agent",
                Some("CHAIN") => "chain",
                _ => "span",
            };
            attrs.push(KeyValue::new("langfuse.observation.type", kind));
            for (from, to) in [
                ("gen_ai.conversation.id", "langfuse.session.id"),
                ("gen_ai.request.model", "langfuse.observation.model.name"),
                ("input.value", "langfuse.observation.input"),
                ("output.value", "langfuse.observation.output"),
                (
                    "everruns.turn.id",
                    "langfuse.observation.metadata.everruns_turn_id",
                ),
                (
                    "everruns.exec.id",
                    "langfuse.observation.metadata.everruns_exec_id",
                ),
                (
                    "everruns.input_message.id",
                    "langfuse.observation.metadata.everruns_input_message_id",
                ),
            ] {
                if let Some(v) = get(from) {
                    attrs.push(KeyValue::new(to, v));
                }
            }
            // Keep GenAI/OpenInference usage fields intact, including cache details.
            // A partial native usage_details object would override receiver mapping.
            if kind == "generation"
                && let Some(Value::F64(cost)) = get("llm.cost.total")
                && cost.is_finite()
                && cost >= 0.0
            {
                attrs.push(KeyValue::new(
                    "langfuse.observation.cost_details",
                    serde_json::json!({"total":cost}).to_string(),
                ));
            }
        }
        NativeTraceProvider::Langsmith => {
            let kind = match kind.as_deref() {
                Some("LLM") => "llm",
                Some("TOOL") => "tool",
                _ => "chain",
            };
            attrs.push(KeyValue::new("langsmith.span.kind", kind));
            for (from, to) in [("gen_ai.conversation.id", "langsmith.metadata.thread_id")] {
                if let Some(v) = get(from) {
                    attrs.push(KeyValue::new(to, v));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_mapping_preserves_usage_and_never_invents_content() {
        for provider in [
            NativeTraceProvider::Langfuse,
            NativeTraceProvider::Langsmith,
        ] {
            let mut attrs = vec![
                KeyValue::new("openinference.span.kind", "LLM"),
                KeyValue::new("gen_ai.conversation.id", "session"),
                KeyValue::new("llm.token_count.prompt", 12_i64),
                KeyValue::new("llm.cost.total", 0.25),
                KeyValue::new("everruns.turn.id", "turn"),
            ];
            annotate(&mut attrs, provider);
            assert!(
                !attrs
                    .iter()
                    .any(|kv| kv.key.as_str().contains("input")
                        || kv.key.as_str().contains("output"))
            );
            assert!(attrs.iter().any(|kv| kv.value.to_string() == "session"));
            if matches!(provider, NativeTraceProvider::Langfuse) {
                assert!(attrs.iter().any(|kv| kv.key.as_str()
                    == "langfuse.observation.metadata.everruns_turn_id"
                    && kv.value.to_string() == "turn"));
                assert!(
                    attrs
                        .iter()
                        .any(|kv| kv.key.as_str() == "llm.token_count.prompt"
                            && kv.value.to_string().contains("12"))
                );
            }
        }
    }
    #[test]
    fn native_mapping_copies_only_present_content() {
        let mut attrs = vec![
            KeyValue::new("input.value", "approved"),
            KeyValue::new("openinference.span.kind", "TOOL"),
        ];
        annotate(&mut attrs, NativeTraceProvider::Langfuse);
        assert!(
            attrs
                .iter()
                .any(|kv| kv.key.as_str() == "langfuse.observation.input"
                    && kv.value.to_string() == "approved")
        );
        assert!(
            attrs
                .iter()
                .any(|kv| kv.key.as_str() == "langfuse.observation.type"
                    && kv.value.to_string() == "tool")
        );
    }
}
