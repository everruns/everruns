//! How a generation, and the turn it ended, finished, for the exporters.
//!
//! `llm.generation` carries the provider's raw stop reason and the tool calls a
//! driver discarded or ran from a truncated response; `turn.completed` carries
//! the turn's stop reason. The Gen-AI conventions cover only the normalized
//! `gen_ai.response.finish_reasons`, so the rest go out as `everruns.*`
//! attributes (OTel, and so OpenInference spans too) and Braintrust metadata.
//!
//! Split out of `otel.rs` and `braintrust.rs` (file-size ratchet).

use crate::events::{Event, EventData, LlmGenerationMetadata};

/// Normalized finish reason of the generation, as a scalar: arrays are awkward
/// to filter on in most trace backends.
#[cfg(feature = "otel")]
pub(crate) const LLM_FINISH_REASON: &str = "everruns.llm.finish_reason";
/// The provider's own stop reason, verbatim.
#[cfg(feature = "otel")]
pub(crate) const LLM_PROVIDER_FINISH_REASON: &str = "everruns.llm.provider_finish_reason";
/// Tool calls the driver discarded because the response was cut off.
#[cfg(feature = "otel")]
pub(crate) const LLM_TOOL_CALLS_DROPPED: &str = "everruns.llm.tool_calls_dropped";
/// Tool calls run from a truncated response; their own arguments are complete.
#[cfg(feature = "otel")]
pub(crate) const LLM_TOOL_CALLS_TRUNCATED_EXECUTED: &str =
    "everruns.llm.tool_calls_truncated_executed";
/// What the output-truncation gate did: `retried` or `failed`.
#[cfg(feature = "otel")]
pub(crate) const LLM_TRUNCATION_GATE: &str = "everruns.llm.truncation_gate";
/// Stop reason of the turn's final generation.
#[cfg(feature = "otel")]
pub(crate) const TURN_STOP_REASON: &str = "everruns.turn.stop_reason";

fn first_finish_reason(meta: &LlmGenerationMetadata) -> Option<&str> {
    meta.finish_reasons
        .as_ref()
        .and_then(|reasons| reasons.first())
        .map(String::as_str)
}

/// The `everruns.*` stop attributes of a chat span, plus its unknown-cost
/// annotation. Zero counts are left off.
#[cfg(feature = "otel")]
pub(crate) fn otel_chat_attributes(meta: &LlmGenerationMetadata) -> Vec<opentelemetry::KeyValue> {
    use opentelemetry::KeyValue;
    let mut attrs = super::provider_attrs::otel_unknown_cost_attributes(meta);
    if let Some(reason) = first_finish_reason(meta) {
        attrs.push(KeyValue::new(LLM_FINISH_REASON, reason.to_string()));
    }
    if let Some(raw) = &meta.provider_finish_reason {
        attrs.push(KeyValue::new(LLM_PROVIDER_FINISH_REASON, raw.clone()));
    }
    if meta.tool_calls_dropped > 0 {
        attrs.push(KeyValue::new(
            LLM_TOOL_CALLS_DROPPED,
            i64::from(meta.tool_calls_dropped),
        ));
    }
    if meta.tool_calls_truncated_executed > 0 {
        attrs.push(KeyValue::new(
            LLM_TOOL_CALLS_TRUNCATED_EXECUTED,
            i64::from(meta.tool_calls_truncated_executed),
        ));
    }
    if let Some(action) = &meta.truncation_gate {
        attrs.push(KeyValue::new(LLM_TRUNCATION_GATE, action.clone()));
    }
    attrs
}

/// `everruns.turn.stop_reason` on the `invoke_agent` span, when known.
#[cfg(feature = "otel")]
pub(crate) fn otel_turn_attributes(
    data: &crate::events::TurnCompletedData,
) -> Vec<opentelemetry::KeyValue> {
    data.stop_reason
        .iter()
        .map(|reason| opentelemetry::KeyValue::new(TURN_STOP_REASON, reason.clone()))
        .collect()
}

/// The same facts as Braintrust span metadata, on the generation and turn
/// spans. Braintrust metadata is free-form, so the keys drop the prefix.
#[cfg_attr(not(feature = "braintrust"), allow(dead_code))]
pub(crate) fn annotate_braintrust(event: &Event, metadata: &mut serde_json::Value) {
    match &event.data {
        EventData::LlmGeneration(data) => {
            let meta = &data.metadata;
            if let Some(reason) = first_finish_reason(meta) {
                metadata["finish_reason"] = serde_json::json!(reason);
            }
            if let Some(raw) = &meta.provider_finish_reason {
                metadata["provider_finish_reason"] = serde_json::json!(raw);
            }
            if let Some(usage) = &meta.usage {
                metadata["output_tokens"] = serde_json::json!(usage.output_tokens);
            }
            if meta.tool_calls_dropped > 0 {
                metadata["tool_calls_dropped"] = serde_json::json!(meta.tool_calls_dropped);
            }
            if meta.tool_calls_truncated_executed > 0 {
                metadata["tool_calls_truncated_executed"] =
                    serde_json::json!(meta.tool_calls_truncated_executed);
            }
            if let Some(action) = &meta.truncation_gate {
                metadata["truncation_gate"] = serde_json::json!(action);
            }
        }
        EventData::TurnCompleted(data) => {
            if let Some(reason) = &data.stop_reason {
                metadata["stop_reason"] = serde_json::json!(reason);
            }
        }
        _ => {}
    }
}
