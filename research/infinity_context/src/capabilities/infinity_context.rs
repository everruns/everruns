//! Infinity Context Capability
//!
//! Enables unlimited conversation length by trimming context and providing
//! a history search tool for the LLM to query excluded messages.
//!
//! Uses DB-level LIMIT for scalability. The query_history tool queries
//! the MessageRetriever directly for excluded messages.

use super::{
    Capability, CapabilityStatus, MessageFilterProvider, MessageQuery,
};
use super::naive_trim::calculate_message_limit;
use crate::types::ContextStrategyConfig;
use async_trait::async_trait;
use everruns_core::message::{ContentPart, Message, MessageRole};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// Capability that enables unlimited conversation length via history querying.
pub struct InfinityContextCapability;

impl Capability for InfinityContextCapability {
    fn id(&self) -> &str {
        "infinity_context"
    }

    fn name(&self) -> &str {
        "Infinity Context"
    }

    fn description(&self) -> &str {
        "Enables unlimited conversation length by trimming context and providing history search."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("infinity")
    }

    fn category(&self) -> Option<&str> {
        Some("Context Management")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(INFINITY_CONTEXT_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(QueryHistoryTool)]
    }

    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        Some(Arc::new(InfinityContextFilterProvider))
    }
}

const INFINITY_CONTEXT_SYSTEM_PROMPT: &str = r#"## Conversation History

This conversation may have additional history not shown in the current context.
If you need information from earlier in the conversation that is not visible,
use the `query_history` tool to search or retrieve previous messages.

The tool supports:
- Keyword search: Find messages containing specific terms
- Range retrieval: Get messages by position (oldest = 0)
- Type filtering: Filter by user, assistant, or tool_result messages"#;

/// Message filter provider for infinity context
struct InfinityContextFilterProvider;

impl MessageFilterProvider for InfinityContextFilterProvider {
    fn apply_filters(&self, query: &mut MessageQuery, config: &Value) {
        let config: ContextStrategyConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();

        // Calculate message limit based on token budget
        let limit = calculate_message_limit(config.context_budget_tokens, config.min_recent_messages);
        query.limit = Some(limit as i64);
    }

    fn priority(&self) -> i32 {
        100
    }
}

// ============================================================================
// Query History Tool
// ============================================================================

/// Tool for querying conversation history via MessageRetriever.
///
/// Uses `execute_with_context()` to access messages through the retriever.
pub struct QueryHistoryTool;

#[derive(Debug, Deserialize)]
struct QueryHistoryParams {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    message_range: Option<MessageRange>,
    #[serde(default = "default_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
struct MessageRange {
    from: usize,
    to: usize,
}

fn default_limit() -> usize {
    20
}

#[async_trait]
impl Tool for QueryHistoryTool {
    fn name(&self) -> &str {
        "query_history"
    }

    fn description(&self) -> &str {
        "Search or retrieve messages from earlier in this conversation that are not currently visible."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search term to find relevant messages"
                },
                "message_range": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "integer", "description": "Start index (0-based)" },
                        "to": { "type": "integer", "description": "End index (exclusive)" }
                    },
                    "description": "Retrieve messages by position range"
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "description": "Maximum messages to return"
                }
            }
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        // Without context, we can't access messages
        ToolExecutionResult::tool_error(
            "query_history requires ToolContext with MessageRetriever. Use execute_with_context()."
        )
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let params: QueryHistoryParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(format!("Invalid parameters: {}", e)),
        };

        // Get messages from MessageRetriever
        let Some(retriever) = &context.message_retriever else {
            return ToolExecutionResult::tool_error("No message retriever available");
        };

        let messages = match retriever.load(context.session_id).await {
            Ok(m) => m,
            Err(e) => return ToolExecutionResult::tool_error(format!("Failed to load messages: {}", e)),
        };

        if messages.is_empty() {
            return ToolExecutionResult::success(json!({
                "message": "No messages available in history.",
                "count": 0
            }));
        }

        let limit = params.limit.min(50);
        let total = messages.len();

        // Handle range query
        if let Some(range) = params.message_range {
            let from = range.from.min(total);
            let to = range.to.min(total).max(from);
            let range_messages: Vec<_> = messages[from..to].iter().take(limit).collect();
            return format_range_result(&range_messages, from, total);
        }

        // Handle search query
        if let Some(ref search_query) = params.query {
            let results = search_messages(&messages, search_query, limit);
            return format_search_result(&results, total);
        }

        // Default: return most recent messages
        let recent: Vec<_> = messages.iter().rev().take(limit).collect();
        format_recent_result(&recent, total)
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

struct SearchResult<'a> {
    index: usize,
    message: &'a Message,
    score: f64,
}

fn search_messages<'a>(messages: &'a [Message], query: &str, limit: usize) -> Vec<SearchResult<'a>> {
    let query_lower = query.to_lowercase();
    let mut results: Vec<SearchResult<'a>> = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        let content = extract_text_content(msg).to_lowercase();

        if content.contains(&query_lower) {
            let mut score = 1.0;

            // Exact word match bonus
            if content.split_whitespace().any(|w| w == query_lower) {
                score += 0.5;
            }

            // Recency boost
            if !messages.is_empty() {
                score += (idx as f64 / messages.len() as f64) * 0.3;
            }

            // Message type boost
            match msg.role {
                MessageRole::User | MessageRole::Assistant => score += 0.2,
                MessageRole::System => score += 0.1,
                MessageRole::ToolResult => {}
            }

            results.push(SearchResult { index: idx, message: msg, score });
        }
    }

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(limit);
    results
}

fn extract_text_content(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(t) => Some(t.text.clone()),
            ContentPart::ToolResult(tr) => tr.result.as_ref().map(|v| v.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_content(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        content.to_string()
    } else {
        format!("{}...", &content[..max_len])
    }
}

fn format_message(msg: &Message, idx: usize, total: usize) -> Value {
    json!({
        "index": idx,
        "position": format!("{}/{}", idx + 1, total),
        "role": format!("{:?}", msg.role),
        "content": truncate_content(&extract_text_content(msg), 500)
    })
}

fn format_range_result(messages: &[&Message], start_idx: usize, total: usize) -> ToolExecutionResult {
    if messages.is_empty() {
        return ToolExecutionResult::success(json!({
            "message": "No messages in the specified range.",
            "count": 0
        }));
    }

    let formatted: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(i, msg)| format_message(msg, start_idx + i, total))
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": messages.len(),
        "total_in_history": total,
        "range": format!("{}-{}", start_idx + 1, start_idx + messages.len())
    }))
}

fn format_search_result(results: &[SearchResult], total: usize) -> ToolExecutionResult {
    if results.is_empty() {
        return ToolExecutionResult::success(json!({
            "message": "No matching messages found.",
            "count": 0
        }));
    }

    let formatted: Vec<Value> = results
        .iter()
        .map(|r| {
            let mut obj = format_message(r.message, r.index, total);
            obj["relevance_score"] = json!(format!("{:.2}", r.score));
            obj
        })
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": results.len(),
        "total_in_history": total
    }))
}

fn format_recent_result(messages: &[&Message], total: usize) -> ToolExecutionResult {
    let formatted: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(i, msg)| format_message(msg, total - messages.len() + i, total))
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": messages.len(),
        "total_in_history": total,
        "note": "Showing most recent messages. Use 'query' parameter to search."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::message_helpers;

    #[test]
    fn test_search_messages() {
        let messages = vec![
            message_helpers::user("Let's discuss the API design"),
            message_helpers::assistant("Sure, what about authentication?"),
            message_helpers::user("We should use JWT tokens"),
            message_helpers::assistant("JWT sounds good for the API"),
        ];

        let results = search_messages(&messages, "API", 10);
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| extract_text_content(r.message).contains("API")));
    }

    #[test]
    fn test_extract_text_content() {
        let msg = message_helpers::user("Hello world");
        assert_eq!(extract_text_content(&msg), "Hello world");
    }
}
