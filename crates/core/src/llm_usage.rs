//! Shared conversion of provider usage into canonical, priced generation usage.

use crate::events::TokenUsage;
use everruns_contracts::openai_hosted_tools::hosted_calls_cost_usd;
use everruns_contracts::{DriverId, driver_registry::LlmCompletionMetadata};

/// Usage for one generation. Cost is tracked as two values: the provider's
/// authoritative inline cost when present (OpenRouter `usage.cost`), and a
/// price-table estimate from the model profile, plus hosted tool calls, which
/// bill per call on top of tokens. Keeping both lets consumers prefer the
/// actual charge while reconciling estimate-vs-actual drift.
pub(crate) fn completion_usage(
    meta: &LlmCompletionMetadata,
    provider_type: &DriverId,
    model: &str,
) -> Option<TokenUsage> {
    let (Some(input), Some(output)) = (meta.prompt_tokens, meta.completion_tokens) else {
        return meta.provider_cost_usd.map(|cost| {
            TokenUsage::with_cache(
                meta.prompt_tokens.unwrap_or(0),
                meta.completion_tokens.unwrap_or(0),
                meta.cache_read_tokens,
                meta.cache_creation_tokens,
            )
            .with_cost(Some(cost), None)
        });
    };
    // Priced at the tier that served the call, not the one requested.
    let tokens = everruns_contracts::model_profiles::estimate_cost_usd_for_speed(
        provider_type,
        model,
        input,
        output,
        meta.cache_read_tokens.unwrap_or(0),
        meta.cache_creation_tokens.unwrap_or(0),
        meta.service_tier.as_deref(),
    );
    let prompt = input
        .saturating_add(meta.cache_read_tokens.unwrap_or(0))
        .saturating_add(meta.cache_creation_tokens.unwrap_or(0));
    let tokens = match everruns_contracts::model_profiles::cache_write_1h_surcharge_usd(
        provider_type,
        model,
        prompt,
        meta.cache_creation_1h_tokens.unwrap_or(0),
    ) {
        Some(surcharge) => tokens.map(|tokens| tokens + surcharge),
        None => tokens,
    };
    let tokens = match everruns_contracts::model_profiles::cache_write_12h_surcharge_usd(
        provider_type,
        model,
        prompt,
        meta.cache_creation_12h_tokens.unwrap_or(0),
        meta.service_tier.as_deref(),
    ) {
        Some(surcharge) => tokens.map(|tokens| tokens + surcharge),
        None => tokens,
    };
    let hosted = hosted_calls_cost_usd(&meta.hosted_tool_calls, model);
    let estimated = match (tokens, hosted) {
        (Some(tokens), Some(hosted)) => Some(tokens + hosted),
        (tokens, hosted) => tokens.or(hosted),
    };
    Some(
        TokenUsage::with_cache(
            input,
            output,
            meta.cache_read_tokens,
            meta.cache_creation_tokens,
        )
        .with_cost(meta.provider_cost_usd, estimated),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_cost_survives_missing_token_counts_without_inventing_an_estimate() {
        let mut metadata = LlmCompletionMetadata::default();
        assert!(completion_usage(&metadata, &DriverId::OpenAI, "gpt-6.2-sol").is_none());
        metadata.provider_cost_usd = Some(0.012);
        let usage = completion_usage(&metadata, &DriverId::OpenAI, "gpt-6.2-sol").unwrap();
        assert_eq!(usage.actual_cost_usd, Some(0.012));
        assert_eq!(usage.estimated_cost_usd, None);
        assert_eq!((usage.input_tokens, usage.output_tokens), (0, 0));
    }
}
