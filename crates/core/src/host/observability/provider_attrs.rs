//! Unknown-cost annotations shared by the exporters.
//!
//! A generation can name billable components whose amount is unknown. They
//! reach spans so an operator sees spend that is unknown rather than zero.

use crate::events::Event;
use crate::events::LlmGenerationMetadata;

/// Billable components of a generation whose amount is unknown.
#[cfg(feature = "otel")]
pub(crate) const USAGE_COST_UNKNOWN_COMPONENTS: &str = "everruns.usage.cost_unknown_components";

/// `kind:name` of every billable component without a known amount.
fn unpriced_components(meta: &LlmGenerationMetadata) -> Vec<String> {
    meta.unpriced_cost_components()
        .map(|component| format!("{}:{}", component.kind, component.name))
        .collect()
}

/// The unknown-cost attribute of a chat span, when any component is unpriced.
#[cfg(feature = "otel")]
pub(crate) fn otel_unknown_cost_attributes(
    meta: &LlmGenerationMetadata,
) -> Vec<opentelemetry::KeyValue> {
    let unpriced: Vec<opentelemetry::StringValue> = unpriced_components(meta)
        .into_iter()
        .map(Into::into)
        .collect();
    if unpriced.is_empty() {
        return Vec::new();
    }
    vec![opentelemetry::KeyValue::new(
        USAGE_COST_UNKNOWN_COMPONENTS,
        opentelemetry::Value::Array(unpriced.into()),
    )]
}

/// A generation's cost components in Braintrust span metadata. `cost_usd: null` marks an amount nobody could price.
#[cfg(feature = "braintrust")]
pub(crate) fn annotate_braintrust(event: &Event, metadata: &mut serde_json::Value) {
    super::generation_attrs::annotate_braintrust(event, metadata);
    if let crate::events::EventData::LlmGeneration(data) = &event.data
        && !data.metadata.cost_components.is_empty()
    {
        metadata["cost_components"] = serde_json::json!(data.metadata.cost_components);
        metadata["cost_unknown_components"] =
            serde_json::json!(unpriced_components(&data.metadata));
    }
}
