//! Curated Framework configuration values for portable capabilities.

use everruns_capability::{CapabilityRef, CapabilitySpec, IntoCapability};

/// Strategy used when a conversation outgrows the model context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CompactionStrategy {
    /// Cascade through masking, provider-native compaction, and summarization.
    #[default]
    Auto,
    /// Use the provider's native compaction operation.
    Native,
    /// Replace older tool outputs with compact summaries.
    ObservationMasking,
    /// Ask the configured model to summarize older turns.
    Summarization,
}

impl CompactionStrategy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Native => "native",
            Self::ObservationMasking => "observation_masking",
            Self::Summarization => "summarization",
        }
    }
}

/// Application-facing context-compaction policy.
///
/// The default proactively compacts at 85% of the model's context budget and
/// lets the runtime select the best available strategy. Durable checkpoint
/// storage remains a host concern. This is the canonical `CompactionConfig`
/// exported from `everruns-builtins` and `everruns`; the expanded runtime
/// representation is named `RuntimeCompactionConfig`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactionConfig {
    strategy: CompactionStrategy,
    proactive: bool,
    budget_percent: f32,
}

impl CompactionConfig {
    /// Start with the safe automatic policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Choose the compaction strategy.
    pub fn strategy(mut self, strategy: CompactionStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Enable or disable compaction before a provider rejects an oversized request.
    pub fn proactive(mut self, enabled: bool) -> Self {
        self.proactive = enabled;
        self
    }

    /// Set the proactive trigger as a fraction of the model context budget.
    ///
    /// The capability validator accepts values from 0.1 through 1.0.
    pub fn budget_percent(mut self, budget_percent: f32) -> Self {
        self.budget_percent = budget_percent;
        self
    }
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            strategy: CompactionStrategy::Auto,
            proactive: true,
            budget_percent: 0.85,
        }
    }
}

impl IntoCapability for CompactionConfig {
    fn into_capability(self) -> CapabilitySpec {
        CapabilityRef::new(super::COMPACTION_CAPABILITY_ID)
            .config(serde_json::json!({
                "strategy": self.strategy.as_str(),
                "proactive": self.proactive,
                "budget_percent": self.budget_percent,
            }))
            .into()
    }
}

/// Model-adaptive deferred tool-loading configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolSearch {
    threshold: Option<usize>,
    never_defer: Vec<String>,
}

impl ToolSearch {
    /// Select hosted tool search when the model supports it and the generic
    /// client-side implementation otherwise.
    pub fn automatic() -> Self {
        Self::default()
    }

    /// Override the minimum total tool count at which schemas are deferred.
    pub fn threshold(mut self, threshold: usize) -> Self {
        self.threshold = Some(threshold);
        self
    }

    /// Keep these tools' full schemas visible under the generic fallback.
    pub fn never_defer<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.never_defer.extend(names.into_iter().map(Into::into));
        self
    }
}

impl IntoCapability for ToolSearch {
    fn into_capability(self) -> CapabilitySpec {
        let mut config = serde_json::Map::new();
        if let Some(threshold) = self.threshold {
            config.insert("threshold".to_string(), threshold.into());
        }
        if !self.never_defer.is_empty() {
            config.insert("never_defer".to_string(), self.never_defer.into());
        }
        CapabilityRef::new(super::AUTO_TOOL_SEARCH_CAPABILITY_ID)
            .config(serde_json::Value::Object(config))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_compaction_uses_the_stable_id_and_schema() {
        let spec = CompactionConfig::new()
            .strategy(CompactionStrategy::ObservationMasking)
            .budget_percent(0.9)
            .into_capability();
        assert_eq!(spec.capability_ref().id(), "compaction");
        let budget = spec.capability_ref().config_value()["budget_percent"]
            .as_f64()
            .unwrap();
        assert!((budget - 0.9).abs() < 1e-6);
    }

    #[test]
    fn typed_tool_search_uses_the_model_adaptive_capability() {
        let spec = ToolSearch::automatic()
            .threshold(12)
            .never_defer(["read_file"])
            .into_capability();
        assert_eq!(spec.capability_ref().id(), "auto_tool_search");
        assert_eq!(spec.capability_ref().config_value()["threshold"], 12);
    }
}
