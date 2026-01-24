//! Core types for the evaluation framework
//!
//! Uses types from everruns-core where possible, with thin wrappers
//! for eval-specific functionality.

use chrono::{DateTime, Utc};
use everruns_core::message::ContentPart;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export core types
pub use everruns_core::message::{Message, MessageRole};

// ============================================================================
// Token Estimation
// ============================================================================

/// Estimate token count from message content
/// Uses character-based approximation: ~4 chars per token for English
pub fn estimate_tokens(message: &Message) -> usize {
    let content_len: usize = message
        .content
        .iter()
        .map(|part| match part {
            ContentPart::Text(t) => t.text.len(),
            ContentPart::ToolCall(tc) => tc.name.len() + tc.arguments.to_string().len(),
            ContentPart::ToolResult(tr) => {
                tr.result.as_ref().map(|v| v.to_string().len()).unwrap_or(0)
                    + tr.error.as_ref().map(|e| e.len()).unwrap_or(0)
            }
            ContentPart::Image(_) | ContentPart::ImageFile(_) => 1000, // Images ~1k tokens
        })
        .sum();

    // Rough approximation: 1 token ≈ 4 characters
    content_len.div_ceil(4)
}

/// Estimate total tokens for a slice of messages
pub fn estimate_total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_tokens).sum()
}

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

    /// Boost more recent messages in search results (default: true)
    #[serde(default = "default_true")]
    pub boost_recency: bool,

    /// Boost user/assistant messages over tool results (default: true)
    #[serde(default = "default_true")]
    pub boost_conversation: bool,
}

fn default_budget() -> usize {
    100_000
}

fn default_min_recent() -> usize {
    10
}

fn default_true() -> bool {
    true
}

impl Default for ContextStrategyConfig {
    fn default() -> Self {
        Self {
            context_budget_tokens: default_budget(),
            min_recent_messages: default_min_recent(),
            boost_recency: true,
            boost_conversation: true,
        }
    }
}

// ============================================================================
// Message Extension Trait
// ============================================================================

/// Extension trait for Message with eval-specific helpers
pub trait MessageExt {
    /// Get text content as a single string
    fn text_content(&self) -> String;
    /// Estimate token count for this message
    fn estimated_tokens(&self) -> usize;
}

impl MessageExt for Message {
    fn text_content(&self) -> String {
        self.content
            .iter()
            .filter_map(|part| part.as_text())
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn estimated_tokens(&self) -> usize {
        estimate_tokens(self)
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

    pub fn system(content: impl Into<String>) -> Message {
        Message::system(content)
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Message {
        Message::tool_result(
            tool_call_id,
            Some(serde_json::json!(content.into())),
            None,
        )
    }
}

/// A test scenario definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    /// Unique name for the scenario
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Type of scenario
    pub scenario_type: ScenarioType,
    /// The conversation history
    pub messages: Vec<Message>,
    /// The task/question to answer
    pub task: String,
    /// Expected answer or criteria for success
    pub expected: ExpectedResult,
    /// Metadata about planted information
    #[serde(default)]
    pub planted_info: Vec<PlantedInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioType {
    /// Find specific info planted early in conversation
    NeedleInHaystack,
    /// Synthesize info from multiple points
    MultiHop,
    /// Track cumulative changes
    Cumulative,
    /// General long-context QA
    LongContextQa,
    /// Decision changes multiple times, task asks for final state
    FinalDecision,
    /// Decision changes multiple times, task asks for timeline
    DecisionTimeline,
    /// Same tool called multiple times with different results, must not mix
    ToolResultDisambiguation,
}

impl std::fmt::Display for ScenarioType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScenarioType::NeedleInHaystack => write!(f, "needle_in_haystack"),
            ScenarioType::MultiHop => write!(f, "multi_hop"),
            ScenarioType::Cumulative => write!(f, "cumulative"),
            ScenarioType::LongContextQa => write!(f, "long_context_qa"),
            ScenarioType::FinalDecision => write!(f, "final_decision"),
            ScenarioType::DecisionTimeline => write!(f, "decision_timeline"),
            ScenarioType::ToolResultDisambiguation => write!(f, "tool_result_disambiguation"),
        }
    }
}

/// Information planted in the conversation for retrieval tests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlantedInfo {
    /// Position in message list (0-indexed)
    pub message_index: usize,
    /// The key information to find
    pub key: String,
    /// The value/content to retrieve
    pub value: String,
}

/// Expected result for evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExpectedResult {
    /// Exact string match
    Exact(String),
    /// Must contain all of these strings
    Contains { contains: Vec<String> },
    /// Regex pattern match
    Pattern { pattern: String },
    /// Custom evaluation (LLM-as-judge)
    LlmJudge { criteria: String },
}

/// Result of running a single scenario with a strategy
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    pub scenario_name: String,
    pub strategy_name: String,
    pub success: bool,
    pub response: String,
    pub metrics: EvalMetrics,
    pub error: Option<String>,
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
    pub successful: usize,
    pub failed: usize,
    pub context_exceeded: usize,
    pub avg_tokens: f64,
    pub avg_latency_ms: f64,
    pub avg_history_queries: f64,
    pub scenario_results: Vec<ScenarioResult>,
}

impl StrategyResults {
    pub fn accuracy(&self) -> f64 {
        if self.total_scenarios == 0 {
            0.0
        } else {
            self.successful as f64 / self.total_scenarios as f64 * 100.0
        }
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