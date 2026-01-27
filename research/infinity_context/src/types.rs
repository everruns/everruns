//! Core types for the evaluation framework
//!
//! Uses types from everruns-core where possible, with thin wrappers
//! for eval-specific functionality.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export core types
pub use everruns_core::message::{Message, MessageRole};

// ============================================================================
// Context Strategy Configuration
// ============================================================================

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

/// Helper to create messages for eval scenarios
pub mod message_helpers {
    use super::*;

    pub fn user(content: impl Into<String>) -> Message {
        Message::user(content)
    }

    pub fn assistant(content: impl Into<String>) -> Message {
        Message::assistant(content)
    }
}

/// Result of running a single scenario with a strategy
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub scenario_name: String,
    pub strategy_name: String,
    /// Aggregate score (0.0-1.0)
    pub score: f64,
    /// Individual scorer results
    pub scores: Vec<crate::scorer::Score>,
    pub response: String,
    pub metrics: EvalMetrics,
    pub error: Option<String>,
}

impl ScenarioResult {
    /// Returns true if aggregate score >= 0.5
    pub fn passed(&self) -> bool {
        self.score >= 0.5
    }
}

/// Metrics collected during evaluation
#[derive(Debug, Clone, Default, Serialize)]
pub struct EvalMetrics {
    /// Total tokens used (input + output)
    pub total_tokens: usize,
    /// Input tokens
    pub input_tokens: usize,
    /// Output tokens
    pub output_tokens: usize,
    /// Number of LLM calls made
    pub llm_calls: usize,
    /// Number of history queries made (for infinity strategy)
    pub history_queries: usize,
    /// Time to complete in milliseconds
    pub latency_ms: u64,
    /// Whether context was exceeded
    pub context_exceeded: bool,
    /// Messages included in context
    pub messages_in_context: usize,
    /// Messages excluded from context
    pub messages_excluded: usize,
}

/// Aggregated results across all scenarios for a strategy
#[derive(Debug, Clone, Serialize)]
pub struct StrategyResults {
    pub strategy_name: String,
    pub total_scenarios: usize,
    /// Number of scenarios with score >= 0.5
    pub passed: usize,
    /// Number of scenarios with score < 0.5
    pub failed: usize,
    pub context_exceeded: usize,
    /// Average score across all scenarios (0.0-1.0)
    pub avg_score: f64,
    pub avg_tokens: f64,
    pub avg_latency_ms: f64,
    pub avg_history_queries: f64,
    pub scenario_results: Vec<ScenarioResult>,
}

impl StrategyResults {
    /// Average score as percentage (0-100)
    pub fn accuracy(&self) -> f64 {
        self.avg_score * 100.0
    }
}

/// Full evaluation results
#[derive(Debug, Clone, Serialize)]
pub struct EvaluationResults {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub config: HashMap<String, String>,
    pub strategy_results: Vec<StrategyResults>,
}
