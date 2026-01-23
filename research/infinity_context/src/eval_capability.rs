//! Evaluation capability trait
//!
//! Wraps core Capability for eval-specific needs like context preparation
//! and tool execution with excluded messages.

use crate::types::{estimate_tokens, Message, MessageExt, ToolCall, ToolDefinition};
use anyhow::Result;
use std::sync::Arc;

/// Result of context preparation
#[derive(Debug, Clone)]
pub struct PreparedContext {
    /// Messages to include in LLM context
    pub messages: Vec<Message>,
    /// Messages excluded from context (for tool queries)
    pub excluded: Vec<Message>,
    /// System prompt addition (if any)
    pub system_addition: Option<String>,
    /// Tool definitions to provide to LLM
    pub tools: Vec<ToolDefinition>,
    /// Estimated tokens for context
    pub estimated_tokens: usize,
}

/// Trait for capabilities used in evaluation
pub trait EvalCapability: Send + Sync {
    /// Capability name for reporting
    fn name(&self) -> &str;

    /// Capability description
    fn description(&self) -> &str;

    /// Prepare context from message history
    fn prepare_context(&self, messages: &[Message], budget_tokens: usize) -> PreparedContext;

    /// Execute a tool call with access to excluded messages
    fn execute_tool(&self, tool_call: &ToolCall, excluded: &[Message]) -> Result<String>;

    /// Whether this capability has tools
    fn has_tools(&self) -> bool {
        false
    }
}

/// No-op baseline - passes all messages through
pub struct BaselineCapability;

impl EvalCapability for BaselineCapability {
    fn name(&self) -> &str {
        "baseline"
    }

    fn description(&self) -> &str {
        "No context management - passes all messages through"
    }

    fn prepare_context(&self, messages: &[Message], _budget_tokens: usize) -> PreparedContext {
        let estimated_tokens: usize = messages.iter().map(estimate_tokens).sum();
        PreparedContext {
            messages: messages.to_vec(),
            excluded: vec![],
            system_addition: None,
            tools: vec![],
            estimated_tokens,
        }
    }

    fn execute_tool(&self, _tool_call: &ToolCall, _excluded: &[Message]) -> Result<String> {
        Ok("Tool not supported".to_string())
    }
}

/// Wrapper to use core Capability + NaiveTrim as EvalCapability
pub struct NaiveTrimEval {
    pub budget_tokens: usize,
    pub min_recent: usize,
}

impl Default for NaiveTrimEval {
    fn default() -> Self {
        Self {
            budget_tokens: 100_000,
            min_recent: 10,
        }
    }
}

impl EvalCapability for NaiveTrimEval {
    fn name(&self) -> &str {
        "naive_trim"
    }

    fn description(&self) -> &str {
        "Drops oldest messages when context exceeds budget"
    }

    fn prepare_context(&self, messages: &[Message], budget_tokens: usize) -> PreparedContext {
        let budget = budget_tokens.min(self.budget_tokens);
        let result =
            crate::capabilities::naive_trim::trim_messages_to_budget(messages.to_vec(), budget, self.min_recent);

        let estimated_tokens: usize = result.kept.iter().map(estimate_tokens).sum();

        PreparedContext {
            messages: result.kept,
            excluded: result.excluded,
            system_addition: None,
            tools: vec![],
            estimated_tokens,
        }
    }

    fn execute_tool(&self, _tool_call: &ToolCall, _excluded: &[Message]) -> Result<String> {
        Ok("Tool not supported".to_string())
    }
}

/// Infinity context - trims + provides query_history tool
pub struct InfinityContextEval {
    pub budget_tokens: usize,
    pub min_recent: usize,
    pub boost_recency: bool,
}

impl Default for InfinityContextEval {
    fn default() -> Self {
        Self {
            budget_tokens: 100_000,
            min_recent: 10,
            boost_recency: true,
        }
    }
}

impl EvalCapability for InfinityContextEval {
    fn name(&self) -> &str {
        "infinity_context"
    }

    fn description(&self) -> &str {
        "Trims context + provides query_history tool for excluded messages"
    }

    fn prepare_context(&self, messages: &[Message], budget_tokens: usize) -> PreparedContext {
        let budget = budget_tokens.min(self.budget_tokens);
        let result =
            crate::capabilities::naive_trim::trim_messages_to_budget(messages.to_vec(), budget, self.min_recent);

        let estimated_tokens: usize = result.kept.iter().map(estimate_tokens).sum();

        let system_addition = if !result.excluded.is_empty() {
            Some(format!(
                "[Context Notice: {} earlier message(s) not shown. Use `query_history` tool to search history.]",
                result.excluded.len()
            ))
        } else {
            None
        };

        let tools = vec![ToolDefinition {
            name: "query_history".to_string(),
            description: "Search or retrieve messages from earlier in this conversation that are not currently visible.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search term to find relevant messages"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 20,
                        "description": "Maximum messages to return"
                    }
                }
            }),
        }];

        PreparedContext {
            messages: result.kept,
            excluded: result.excluded,
            system_addition,
            tools,
            estimated_tokens,
        }
    }

    fn execute_tool(&self, tool_call: &ToolCall, excluded: &[Message]) -> Result<String> {
        if tool_call.name != "query_history" {
            return Ok(format!("Unknown tool: {}", tool_call.name));
        }

        let query = tool_call
            .arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let limit = tool_call
            .arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        if query.is_empty() {
            // Return most recent excluded messages
            let recent: Vec<_> = excluded.iter().rev().take(limit).collect();
            let mut output = format!("Most recent {} excluded messages:\n\n", recent.len());
            for msg in recent {
                let content = msg.text_content();
                let truncated = if content.len() > 500 {
                    format!("{}...", &content[..500])
                } else {
                    content
                };
                output.push_str(&format!("--- {:?} ---\n{}\n\n", msg.role, truncated));
            }
            return Ok(output);
        }

        // Search by keyword
        let query_lower = query.to_lowercase();
        let mut results: Vec<(usize, &Message, f64)> = Vec::new();

        for (idx, msg) in excluded.iter().enumerate() {
            let content = msg.text_content().to_lowercase();
            if content.contains(&query_lower) {
                let mut score = 1.0;
                // Recency boost
                if self.boost_recency && !excluded.is_empty() {
                    score += (idx as f64 / excluded.len() as f64) * 0.3;
                }
                results.push((idx, msg, score));
            }
        }

        results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        if results.is_empty() {
            return Ok("No matching messages found.".to_string());
        }

        let mut output = format!("Found {} relevant message(s):\n\n", results.len());
        for (idx, msg, score) in results {
            let content = msg.text_content();
            let truncated = if content.len() > 500 {
                format!("{}...", &content[..500])
            } else {
                content
            };
            output.push_str(&format!(
                "--- Message {} ({:?}, score: {:.2}) ---\n{}\n\n",
                idx + 1,
                msg.role,
                score,
                truncated
            ));
        }

        Ok(output)
    }

    fn has_tools(&self) -> bool {
        true
    }
}

/// Get all eval capabilities
pub fn all_capabilities() -> Vec<Arc<dyn EvalCapability>> {
    vec![
        Arc::new(BaselineCapability),
        Arc::new(NaiveTrimEval::default()),
        Arc::new(InfinityContextEval::default()),
    ]
}
