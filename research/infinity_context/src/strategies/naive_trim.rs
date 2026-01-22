//! Naive trim strategy: drop oldest messages to fit context
//!
//! Uses the NaiveTrimCapability from the local capabilities module.

use super::ContextStrategy;
use crate::capabilities::naive_trim::trim_messages_to_budget;
use crate::types::{estimate_tokens, Message, PreparedContext};

pub struct NaiveTrimStrategy {
    min_recent: usize,
}

impl Default for NaiveTrimStrategy {
    fn default() -> Self {
        Self { min_recent: 10 }
    }
}

impl NaiveTrimStrategy {
    #[allow(dead_code)]
    pub fn with_min_recent(min_recent: usize) -> Self {
        Self { min_recent }
    }
}

impl ContextStrategy for NaiveTrimStrategy {
    fn name(&self) -> &str {
        "naive_trim"
    }

    fn description(&self) -> &str {
        "Drop oldest messages to fit context (loses information permanently)"
    }

    fn prepare_context(&self, messages: &[Message], budget_tokens: usize) -> PreparedContext {
        if messages.is_empty() {
            return PreparedContext {
                messages: vec![],
                excluded_messages: vec![],
                system_additions: vec![],
                additional_tools: vec![],
                estimated_tokens: 0,
            };
        }

        // Use the capability's trim function
        let result = trim_messages_to_budget(messages.to_vec(), budget_tokens, self.min_recent);

        let estimated_tokens: usize = result.kept.iter().map(|m| estimate_tokens(m)).sum();

        PreparedContext {
            messages: result.kept,
            excluded_messages: result.excluded,
            system_additions: vec![],
            additional_tools: vec![],
            estimated_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::message_helpers;

    #[test]
    fn test_no_trimming_when_under_budget() {
        let strategy = NaiveTrimStrategy::with_min_recent(5);
        let messages: Vec<Message> = (0..10)
            .map(|i| message_helpers::user(format!("Message {}", i)))
            .collect();

        let result = strategy.prepare_context(&messages, 100000);

        assert_eq!(result.messages.len(), 10);
        assert!(result.excluded_messages.is_empty());
    }

    #[test]
    fn test_empty_messages() {
        let strategy = NaiveTrimStrategy::default();
        let result = strategy.prepare_context(&[], 100000);

        assert!(result.messages.is_empty());
        assert!(result.excluded_messages.is_empty());
    }
}
