//! High-level context-compaction configuration.

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
    pub(crate) fn as_str(self) -> &'static str {
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
/// storage remains a host concern; applications configure behavior without
/// supplying a checkpoint store.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactionConfig {
    pub(crate) strategy: CompactionStrategy,
    pub(crate) proactive: bool,
    pub(crate) budget_percent: f32,
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
    /// Values must be at least 0.1 and at most one. Validation happens in
    /// [`AgentBuilder::build`](crate::AgentBuilder::build).
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

impl crate::IntoCapability for CompactionConfig {
    fn into_capability(self) -> crate::CapabilitySpec {
        crate::CapabilityRef::new("compaction")
            .config(serde_json::json!({
                "strategy": self.strategy.as_str(),
                "proactive": self.proactive,
                "budget_percent": self.budget_percent,
            }))
            .into()
    }
}
