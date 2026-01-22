//! Naive trim strategy: drop oldest messages to fit context
//!
//! Simple approach that removes old messages when context is exceeded.
//! Lost messages cannot be retrieved - context is permanently lost.
//!
//! This strategy mirrors the NaiveTrimCapability from core, adapted
//! for the evaluation framework.

use super::ContextStrategy;
use crate::types::{estimate_tokens, Message, MessageExt, PreparedContext};

pub struct NaiveTrimStrategy {
    /// Minimum number of recent messages to always keep
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

        let mut selected: Vec<Message> = Vec::new();
        let mut current_tokens = 0usize;

        // Always include the most recent messages (up to min_recent)
        let recent_start = messages.len().saturating_sub(self.min_recent);
        let recent_messages = &messages[recent_start..];

        for msg in recent_messages {
            selected.push(msg.clone());
            current_tokens += estimate_tokens(msg);
        }

        // Add older messages (newest first) while we have budget
        let older_messages = &messages[..recent_start];
        for msg in older_messages.iter().rev() {
            let msg_tokens = estimate_tokens(msg);
            if current_tokens + msg_tokens > budget_tokens {
                break;
            }
            selected.insert(0, msg.clone());
            current_tokens += msg_tokens;
        }

        // Determine excluded messages (those we couldn't fit)
        let selected_len = selected.len();
        let excluded: Vec<Message> = messages[..messages.len() - selected_len].to_vec();

        PreparedContext {
            messages: selected,
            excluded_messages: excluded,
            system_additions: vec![],
            additional_tools: vec![],
            estimated_tokens: current_tokens,
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
    fn test_trimming_oldest_messages() {
        let strategy = NaiveTrimStrategy::with_min_recent(3);

        // Create messages with known token counts
        let messages: Vec<Message> = (0..10)
            .map(|i| message_helpers::user(format!("Message number {} with some content", i)))
            .collect();

        // Set a small budget that can't fit all messages
        // Each message is roughly 10-12 tokens
        let result = strategy.prepare_context(&messages, 50);

        // Should keep at least min_recent
        assert!(result.messages.len() >= 3);
        // Should have excluded some
        assert!(!result.excluded_messages.is_empty());
        // Most recent messages should be included
        let last_text = result.messages.last().unwrap().text_content();
        assert!(last_text.contains("9"));
    }

    #[test]
    fn test_empty_messages() {
        let strategy = NaiveTrimStrategy::default();
        let result = strategy.prepare_context(&[], 100000);

        assert!(result.messages.is_empty());
        assert!(result.excluded_messages.is_empty());
    }
}
