//! Model metadata and token accounting carried on generation events.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Metadata about the model used for generation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelMetadata {
    /// Model name (e.g., "gpt-5.6-sol", "claude-sonnet-5")
    pub model: String,

    /// Model ID (internal identifier)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,

    /// Provider ID (internal identifier)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<Uuid>,
}

/// Token usage statistics
///
/// Tracks token consumption per LLM call including cache tokens for cost
/// optimization.
///
/// # Disjoint bucket convention
///
/// Prompt token buckets are **disjoint** (non-overlapping). Drivers normalize
/// provider wire formats at the boundary so this holds for every provider:
///
/// ```text
/// total_prompt = input_tokens + cache_read_tokens + cache_creation_tokens
/// ```
///
/// - `input_tokens` — non-cached prompt tokens only.
/// - `cache_read_tokens` — tokens served from cache, never counted in
///   `input_tokens`.
/// - `cache_creation_tokens` — tokens written to cache, never counted in
///   `input_tokens`.
///
/// Inclusive providers (OpenAI Responses / Chat Completions, Gemini) report a
/// prompt count that *includes* cached reads; their drivers subtract the cached
/// subset so the value stored here is the non-cached remainder. Anthropic /
/// Bedrock already report disjoint buckets. Cost is therefore uniform across
/// providers (`input·in + cache_read·cr + cache_creation·cw + output·out`);
/// consumers must not re-derive a non-cached input by subtracting cache reads.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TokenUsage {
    /// Number of non-cached prompt tokens (cached reads/writes are tracked
    /// separately; see the disjoint bucket convention on the struct)
    pub input_tokens: u32,
    /// Number of output/completion tokens
    pub output_tokens: u32,
    /// Number of tokens read from cache (reduces cost), disjoint from `input_tokens`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    /// Number of tokens written to cache, disjoint from `input_tokens`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_tokens: Option<u32>,

    /// Actual cost of this generation in USD, as reported by the provider inline
    /// (e.g. OpenRouter's `usage.cost`, which reflects real post-routing/BYOK/cache
    /// pricing). `None` for providers that do not return a cost.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_cost_usd: Option<f64>,

    /// Estimated cost of this generation in USD, derived from the model's static
    /// price-table profile. Computed whenever a profile with cost data exists,
    /// independently of `actual_cost_usd`, so estimate-vs-actual drift can be
    /// reconciled. `None` when there is no profile cost data for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,

    /// Best-effort USD cost when it cannot be derived from actual/estimated
    /// alone: already-aggregated usage, or a generation carrying a cost that
    /// belongs to neither slot. Per-generation usage normally leaves this unset
    /// and derives the effective cost from actual-else-estimated; the exception
    /// is a turn whose compaction cost is folded in, where the combined total
    /// has to live here precisely so the generation's own actual-vs-estimated
    /// distinction survives (EVE-895).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_cost_usd: Option<f64>,
}

impl TokenUsage {
    /// Create a new TokenUsage with just input and output tokens
    pub fn new(input_tokens: u32, output_tokens: u32) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            actual_cost_usd: None,
            estimated_cost_usd: None,
            effective_cost_usd: None,
        }
    }

    /// Create a TokenUsage with cache tokens
    pub fn with_cache(
        input_tokens: u32,
        output_tokens: u32,
        cache_read_tokens: Option<u32>,
        cache_creation_tokens: Option<u32>,
    ) -> Self {
        Self {
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            actual_cost_usd: None,
            estimated_cost_usd: None,
            effective_cost_usd: None,
        }
    }

    /// Set the actual (provider-reported) and estimated (price-table) USD costs,
    /// returning `self` for chaining. The two are tracked independently so an
    /// authoritative charge stays distinguishable from an estimate.
    pub fn with_cost(
        mut self,
        actual_cost_usd: Option<f64>,
        estimated_cost_usd: Option<f64>,
    ) -> Self {
        self.actual_cost_usd = actual_cost_usd;
        self.estimated_cost_usd = estimated_cost_usd;
        self
    }

    /// Set the precomputed best-effort USD cost, returning `self` for chaining.
    /// Generation usage should leave this unset so the best-effort cost remains
    /// actual-else-estimated for that generation, unless a cost outside those
    /// two slots is folded in (see the field's documentation).
    pub fn with_effective_cost(mut self, effective_cost_usd: Option<f64>) -> Self {
        self.effective_cost_usd = effective_cost_usd;
        self
    }

    /// Best-effort cost in USD. Aggregate usage can carry a precomputed total
    /// that sums actual-else-estimated per generation; otherwise this falls back
    /// to the generation-level actual-else-estimated behavior.
    pub fn effective_cost_usd(&self) -> Option<f64> {
        self.effective_cost_usd
            .or(self.actual_cost_usd.or(self.estimated_cost_usd))
    }

    /// Get total tokens (input + output)
    pub fn total_tokens(&self) -> u32 {
        self.input_tokens.saturating_add(self.output_tokens)
    }

    /// Add another TokenUsage to this one (for aggregation)
    pub fn add(&mut self, other: &TokenUsage) {
        // Capture the accumulator's effective cost BEFORE mutating the
        // actual/estimated slots below: `effective_cost_usd()` falls back to
        // those slots when no explicit total is set, so reading it afterward
        // would fold in `other`'s just-added actual/estimated and double-count.
        let current_cost = self.effective_cost_usd();
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        if let Some(cache) = other.cache_read_tokens {
            let total = self.cache_read_tokens.get_or_insert(0);
            *total = total.saturating_add(cache);
        }
        if let Some(cache) = other.cache_creation_tokens {
            let total = self.cache_creation_tokens.get_or_insert(0);
            *total = total.saturating_add(cache);
        }
        if let Some(cost) = other.actual_cost_usd {
            *self.actual_cost_usd.get_or_insert(0.0) += cost;
        }
        if let Some(cost) = other.estimated_cost_usd {
            *self.estimated_cost_usd.get_or_insert(0.0) += cost;
        }
        if let Some(cost) = other.effective_cost_usd() {
            *self
                .effective_cost_usd
                .get_or_insert(current_cost.unwrap_or(0.0)) += cost;
        }
    }
}
