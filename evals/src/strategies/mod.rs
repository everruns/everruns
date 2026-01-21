//! Context management strategies
//!
//! Each strategy defines how to prepare messages for LLM context.

mod baseline;
mod infinity;
mod naive_trim;

pub use baseline::BaselineStrategy;
pub use infinity::InfinityContextStrategy;
pub use naive_trim::NaiveTrimStrategy;

use crate::types::{Message, PreparedContext, ToolCall};
use anyhow::Result;
use std::sync::Arc;

/// Strategy for managing conversation context
pub trait ContextStrategy: Send + Sync {
    /// Name of the strategy for reporting
    fn name(&self) -> &str;

    /// Description of the strategy
    fn description(&self) -> &str;

    /// Prepare context from full message history
    ///
    /// - `messages`: Full conversation history
    /// - `budget_tokens`: Maximum tokens to use for context
    fn prepare_context(&self, messages: &[Message], budget_tokens: usize) -> PreparedContext;

    /// Handle a tool call from the LLM (for strategies that provide tools)
    /// Returns the tool result content
    fn handle_tool_call(
        &self,
        _tool_call: &ToolCall,
        _excluded_messages: &[Message],
    ) -> Result<String> {
        Ok("Tool not supported by this strategy".to_string())
    }

    /// Whether this strategy supports iterative tool calls
    fn supports_tools(&self) -> bool {
        false
    }
}

/// Get all available strategies
pub fn all_strategies() -> Vec<Arc<dyn ContextStrategy>> {
    vec![
        Arc::new(BaselineStrategy),
        Arc::new(NaiveTrimStrategy::default()),
        Arc::new(InfinityContextStrategy::default()),
    ]
}

/// Get a strategy by name
pub fn get_strategy(name: &str) -> Result<Arc<dyn ContextStrategy>> {
    match name.to_lowercase().as_str() {
        "baseline" | "none" => Ok(Arc::new(BaselineStrategy)),
        "naive" | "naive_trim" | "trim" => Ok(Arc::new(NaiveTrimStrategy::default())),
        "infinity" | "infinity_context" => Ok(Arc::new(InfinityContextStrategy::default())),
        _ => anyhow::bail!("Unknown strategy: {}", name),
    }
}
