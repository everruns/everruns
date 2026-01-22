//! Naive Trim Capability
//!
//! Drops oldest messages to fit context budget. Simple but loses historical
//! information permanently.

use super::{
    BatchTransformResult, Capability, CapabilityStatus, MessageFilter, MessageFilterProvider,
    MessageQuery,
};
use crate::types::{estimate_tokens, ContextStrategyConfig};
use everruns_core::message::Message;
use serde_json::Value;
use std::sync::Arc;

/// Capability that drops oldest messages to fit context budget.
///
/// This is the simplest context management strategy. When the conversation
/// exceeds the budget, oldest messages are removed.
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

        let budget = config.context_budget_tokens;
        let min_recent = config.min_recent_messages;

        query.filters.push(MessageFilter::BatchTransform(Arc::new(
            move |messages: Vec<Message>| -> BatchTransformResult {
                trim_messages_to_budget(messages, budget, min_recent)
            },
        )));
    }

    fn priority(&self) -> i32 {
        // Run after other filters so we trim the final set
        100
    }
}

/// Trim messages to fit within token budget, keeping most recent
pub fn trim_messages_to_budget(
    messages: Vec<Message>,
    budget_tokens: usize,
    min_recent: usize,
) -> BatchTransformResult {
    if messages.is_empty() {
        return BatchTransformResult {
            kept: vec![],
            excluded: vec![],
        };
    }

    let mut selected: Vec<Message> = Vec::new();
    let mut current_tokens = 0usize;

    // Always include the most recent messages (up to min_recent)
    let recent_start = messages.len().saturating_sub(min_recent);
    let recent_messages = &messages[recent_start..];

    for msg in recent_messages {
        selected.push(msg.clone());
        current_tokens += estimate_tokens(msg);
    }

    // Add older messages (newest first) while we have budget
    let older_messages = &messages[..recent_start];
    let mut cutoff_idx = 0;
    for (idx, msg) in older_messages.iter().rev().enumerate() {
        let msg_tokens = estimate_tokens(msg);
        if current_tokens + msg_tokens > budget_tokens {
            cutoff_idx = older_messages.len() - idx;
            break;
        }
        selected.insert(0, msg.clone());
        current_tokens += msg_tokens;
    }

    // Messages before cutoff_idx are excluded
    let excluded: Vec<Message> = messages[..cutoff_idx].to_vec();

    BatchTransformResult {
        kept: selected,
        excluded,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::message_helpers;

    #[test]
    fn test_no_trimming_when_under_budget() {
        let messages: Vec<Message> = (0..10)
            .map(|i| message_helpers::user(format!("Message {}", i)))
            .collect();

        let result = trim_messages_to_budget(messages, 100000, 5);

        assert_eq!(result.kept.len(), 10);
        assert!(result.excluded.is_empty());
    }

    #[test]
    fn test_trimming_oldest_messages() {
        let messages: Vec<Message> = (0..100)
            .map(|i| message_helpers::user(format!("Message number {}", i)))
            .collect();

        let result = trim_messages_to_budget(messages, 100, 5);

        // Should keep at least min_recent
        assert!(result.kept.len() >= 5);
        // Should have excluded some
        assert!(!result.excluded.is_empty());
    }

    #[test]
    fn test_capability_properties() {
        let cap = NaiveTrimCapability;
        assert_eq!(cap.id(), "naive_trim");
        assert!(cap.message_filter_provider().is_some());
        assert!(cap.tools().is_empty());
    }
}
