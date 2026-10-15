use super::oi;
use crate::events::TokenUsage;
use opentelemetry::KeyValue;

pub(super) fn cost_usd(usage: &TokenUsage) -> Option<f64> {
    usage
        .effective_cost_usd
        .or(usage.actual_cost_usd)
        .or(usage.estimated_cost_usd)
}

pub(super) fn oi_token_attributes(usage: &TokenUsage) -> Vec<KeyValue> {
    let prompt = prompt_tokens(usage);
    let completion = i64::from(usage.output_tokens);
    let mut attrs = vec![
        KeyValue::new(oi::LLM_TOKEN_COUNT_PROMPT, prompt),
        KeyValue::new(oi::LLM_TOKEN_COUNT_COMPLETION, completion),
        KeyValue::new(oi::LLM_TOKEN_COUNT_TOTAL, prompt + completion),
    ];
    if let Some(read) = usage.cache_read_tokens {
        attrs.push(KeyValue::new(
            oi::LLM_TOKEN_COUNT_PROMPT_CACHE_READ,
            i64::from(read),
        ));
    }
    if let Some(write) = usage.cache_creation_tokens {
        attrs.push(KeyValue::new(
            oi::LLM_TOKEN_COUNT_PROMPT_CACHE_WRITE,
            i64::from(write),
        ));
    }
    attrs
}

// Internal usage buckets are disjoint; both telemetry conventions require
// cache-inclusive prompt totals. Convert only at the exporter boundary.
pub(super) fn prompt_tokens(usage: &TokenUsage) -> i64 {
    i64::from(usage.input_tokens)
        + i64::from(usage.cache_read_tokens.unwrap_or(0))
        + i64::from(usage.cache_creation_tokens.unwrap_or(0))
}
