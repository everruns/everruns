//! Naive Trim Capability
//!
//! Drops oldest messages to fit context budget. Simple but loses historical
//! information permanently.
//!
//! Uses DB-level LIMIT instead of in-memory BatchTransform for scalability.

use super::{Capability, CapabilityStatus, MessageFilterProvider, MessageQuery};
use crate::types::ContextStrategyConfig;
use serde_json::Value;
use std::sync::Arc;

/// Capability that drops oldest messages to fit context budget.
///
/// This is the simplest context management strategy. When the conversation
/// exceeds the budget, oldest messages are removed via query limit.
pub struct NaiveTrimCapability;

impl Capability for NaiveTrimCapability {
    fn id(&self) -> &str {
        "naive_trim"
    }

    fn name(&self) -> &str {
        "Naive Context Trim"
    }

    fn description(&self) -> &str {
        "Drops oldest messages when context exceeds budget. Simple but loses historical information."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("scissors")
    }

    fn category(&self) -> Option<&str> {
        Some("Context Management")
    }

    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        Some(Arc::new(NaiveTrimFilterProvider))
    }
}

/// Message filter provider for naive trimming
struct NaiveTrimFilterProvider;

impl MessageFilterProvider for NaiveTrimFilterProvider {
    fn apply_filters(&self, query: &mut MessageQuery, config: &Value) {
        let config: ContextStrategyConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();

        // Calculate message limit based on token budget and estimated tokens per message
        let limit =
            calculate_message_limit(config.context_budget_tokens, config.min_recent_messages);
        query.limit = Some(limit as i64);
    }

    fn priority(&self) -> i32 {
        // Run after other filters
        100
    }
}

/// Estimate message limit based on token budget
///
/// Uses ~250 tokens per message as heuristic, accounting for longer
/// messages in realistic conversations (questions, explanations,
/// code snippets, tool outputs).
pub fn calculate_message_limit(budget_tokens: usize, min_recent: usize) -> usize {
    const AVG_TOKENS_PER_MESSAGE: usize = 250;
    let estimated_limit = budget_tokens / AVG_TOKENS_PER_MESSAGE;
    estimated_limit.max(min_recent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_message_limit() {
        // 10000 tokens / 250 avg = 40 messages
        assert_eq!(calculate_message_limit(10000, 5), 40);

        // 35000 tokens / 250 avg = 140 messages
        assert_eq!(calculate_message_limit(35000, 10), 140);

        // Respects min_recent
        assert_eq!(calculate_message_limit(100, 50), 50);
    }

    #[test]
    fn test_capability_properties() {
        let cap = NaiveTrimCapability;
        assert_eq!(cap.id(), "naive_trim");
        assert!(cap.message_filter_provider().is_some());
        assert!(cap.tools().is_empty());
    }
}
