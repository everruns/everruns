//! Provider correlation and unknown-cost annotations shared by the
//! exporters (EVE-1125).
//!
//! A runtime backend that hands the loop to a provider (the OpenAI Agents
//! API) records the provider's ids in event metadata beside the local ids,
//! and a generation can name billable components whose amount is unknown.
//! Both reach spans so an operator can follow a span to the provider's own
//! records and see spend that is unknown rather than zero.

use everruns_core::events::Event;
use everruns_core::events::LlmGenerationMetadata;
use everruns_core::events::correlation::provider_correlation;

/// Prefix of provider correlation attributes (`everruns.provider_session_id`).
#[cfg(feature = "otel")]
const ATTRIBUTE_PREFIX: &str = "everruns.";
/// Billable components of a generation whose amount is unknown.
#[cfg(feature = "otel")]
pub(crate) const USAGE_COST_UNKNOWN_COMPONENTS: &str = "everruns.usage.cost_unknown_components";

/// `kind:name` of every billable component without a known amount.
fn unpriced_components(meta: &LlmGenerationMetadata) -> Vec<String> {
    meta.unpriced_cost_components()
        .map(|component| format!("{}:{}", component.kind, component.name))
        .collect()
}

/// Provider ids as `everruns.*` span attributes.
#[cfg(feature = "otel")]
pub(crate) fn otel_provider_attributes(event: &Event) -> Vec<opentelemetry::KeyValue> {
    provider_correlation(event.metadata.as_ref())
        .into_iter()
        .map(|(key, value)| opentelemetry::KeyValue::new(format!("{ATTRIBUTE_PREFIX}{key}"), value))
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

/// Provider ids and, on a generation, its cost components in Braintrust
/// span metadata. `cost_usd: null` marks an amount nobody could price.
#[cfg(feature = "braintrust")]
pub(crate) fn annotate_braintrust(event: &Event, metadata: &mut serde_json::Value) {
    for (key, value) in provider_correlation(event.metadata.as_ref()) {
        metadata[key] = serde_json::json!(value);
    }
    if let everruns_core::events::EventData::LlmGeneration(data) = &event.data
        && !data.metadata.cost_components.is_empty()
    {
        metadata["cost_components"] = serde_json::json!(data.metadata.cost_components);
        metadata["cost_unknown_components"] =
            serde_json::json!(unpriced_components(&data.metadata));
    }
}
