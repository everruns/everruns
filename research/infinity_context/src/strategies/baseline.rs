//! Baseline strategy: send all messages without trimming
//!
//! This represents the "naive" approach of sending everything.
//! It will fail when context exceeds the model's limit.

use super::ContextStrategy;
use crate::types::{estimate_tokens, Message, PreparedContext};

pub struct BaselineStrategy;

impl ContextStrategy for BaselineStrategy {
    fn name(&self) -> &str {
        "baseline"
    }

    fn description(&self) -> &str {
        "Send all messages without trimming (fails on long conversations)"
    }

    fn prepare_context(&self, messages: &[Message], _budget_tokens: usize) -> PreparedContext {
        let estimated_tokens: usize = messages.iter().map(|m| estimate_tokens(m)).sum();

        PreparedContext {
            messages: messages.to_vec(),
            excluded_messages: vec![],
            system_additions: vec![],
            additional_tools: vec![],
            estimated_tokens,
        }
    }
}
