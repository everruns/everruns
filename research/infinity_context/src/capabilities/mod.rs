//! Context Strategy Capabilities for Evaluation
//!
//! These capabilities implement context management strategies using the
//! core infrastructure (Capability trait, MessageFilterProvider, etc.)
//! but live in the evals crate for research purposes.

mod infinity_context;
pub mod naive_trim;

pub use infinity_context::InfinityContextCapability;
pub use naive_trim::NaiveTrimCapability;

// Re-export useful traits from core
pub use everruns_core::capabilities::{Capability, CapabilityStatus};
pub use everruns_core::message_filter::{
    InjectedMessage, InjectionPosition, MessageFilter, MessageFilterProvider, MessageQuery,
};

use serde::{Deserialize, Serialize};

/// Configuration for context strategy capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextStrategyConfig {
    /// Maximum tokens to send to LLM (default: 100000)
    #[serde(default = "default_budget")]
    pub context_budget_tokens: usize,

    /// Minimum recent messages to always keep (default: 10)
    #[serde(default = "default_min_recent")]
    pub min_recent_messages: usize,
}

fn default_budget() -> usize {
    100_000
}

fn default_min_recent() -> usize {
    10
}

impl Default for ContextStrategyConfig {
    fn default() -> Self {
        Self {
            context_budget_tokens: default_budget(),
            min_recent_messages: default_min_recent(),
        }
    }
}
