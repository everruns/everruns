//! Infinity Context Capability
//!
//! Enables unlimited conversation length by trimming context and providing
//! a history search tool for the LLM to query excluded messages.

use super::{
    BatchTransformResult, Capability, CapabilityStatus, InjectedMessage, InjectionPosition,
    MessageFilter, MessageFilterProvider, MessageQuery,
};
use crate::capabilities::naive_trim::trim_messages_to_budget;
use crate::types::ContextStrategyConfig;
use async_trait::async_trait;
use everruns_core::message::{ContentPart, Message, MessageRole};
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

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
        vec![Box::new(QueryHistoryTool::new())]
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

        let budget = config.context_budget_tokens;
        let min_recent = config.min_recent_messages;

        // Add batch transform filter that trims messages
        query.filters.push(MessageFilter::BatchTransform(Arc::new(
            move |messages: Vec<Message>| -> BatchTransformResult {
                trim_messages_to_budget(messages, budget, min_recent)
            },
        )));

        // Inject a notice about excluded messages at the start
        let notice = Message::system(
            "[Context Notice: Earlier messages may not be shown. Use `query_history` tool to search history.]",
        );
        query.injections.push(InjectedMessage {
            position: InjectionPosition::Start,
            message: notice,
        });
    }

    fn priority(&self) -> i32 {
        100
    }
}

// ============================================================================
// Query History Tool
// ============================================================================

/// Tool for querying conversation history
///
/// Allows the LLM to search and retrieve messages excluded from current context.
pub struct QueryHistoryTool {
    excluded_messages: Arc<RwLock<Vec<Message>>>,
    config: Arc<RwLock<ContextStrategyConfig>>,
}

impl QueryHistoryTool {
    pub fn new() -> Self {
        Self {
            excluded_messages: Arc::new(RwLock::new(Vec::new())),
            config: Arc::new(RwLock::new(ContextStrategyConfig::default())),
        }
    }

    /// Set the excluded messages for querying
    pub fn set_excluded_messages(&self, messages: Vec<Message>) {
        *self.excluded_messages.write().unwrap() = messages;
    }

    /// Set the configuration
    pub fn set_config(&self, config: ContextStrategyConfig) {
        *self.config.write().unwrap() = config;
    }

    fn search_messages(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let messages = self.excluded_messages.read().unwrap();
        let config = self.config.read().unwrap();
        let query_lower = query.to_lowercase();
        let mut results: Vec<SearchResult> = Vec::new();

        for (idx, msg) in messages.iter().enumerate() {
            let content_text = extract_text_content(msg);
            let content_lower = content_text.to_lowercase();

            let mut score = 0.0;

            if content_lower.contains(&query_lower) {
                score += 1.0;
                if content_lower.split_whitespace().any(|w| w == query_lower) {
                    score += 0.5;
                }
            }

            if score > 0.0 {
                if config.boost_recency && !messages.is_empty() {
                    let recency_factor = (idx as f64 + 1.0) / messages.len() as f64;
                    score += recency_factor * 0.3;
                }

                if config.boost_conversation {
                    match msg.role {
                        MessageRole::User | MessageRole::Assistant => score += 0.2,
                        MessageRole::ToolResult => {}
                        MessageRole::System => score += 0.1,
                    }
                }

                results.push(SearchResult {
                    index: idx,
                    message: msg.clone(),
                    score,
                });
            }
        }

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }

    fn get_range(&self, from: usize, to: usize, limit: usize) -> Vec<Message> {
        let messages = self.excluded_messages.read().unwrap();
        let from = from.min(messages.len());
        let to = to.min(messages.len()).max(from);
        messages[from..to].iter().take(limit).cloned().collect()
    }
}

impl Default for QueryHistoryTool {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct SearchResult {
    index: usize,
    message: Message,
    score: f64,
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

#[derive(Debug, Deserialize)]
struct QueryHistoryParams {
    query: Option<String>,
    message_range: Option<MessageRange>,
    #[allow(dead_code)]
    message_types: Option<Vec<String>>,
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

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let params: QueryHistoryParams = match serde_json::from_value(arguments) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(format!("Invalid parameters: {}", e)),
        };

        let messages = self.excluded_messages.read().unwrap();
        if messages.is_empty() {
            return ToolExecutionResult::success(json!({
                "message": "No excluded messages available. All conversation history is visible.",
                "count": 0
            }));
        }

        let limit = params.limit.min(50);
        let total = messages.len();

        if let Some(range) = params.message_range {
            drop(messages);
            let range_messages = self.get_range(range.from, range.to, limit);
            return format_range_result(&range_messages, range.from, total);
        }

        if let Some(ref search_query) = params.query {
            drop(messages);
            let results = self.search_messages(search_query, limit);
            return format_search_result(&results, total);
        }

        let recent: Vec<_> = messages.iter().rev().take(limit).cloned().collect();
        format_recent_result(&recent, total)
    }
}

fn format_range_result(messages: &[Message], start_idx: usize, total: usize) -> ToolExecutionResult {
    if messages.is_empty() {
        return ToolExecutionResult::success(json!({
            "message": "No messages in the specified range.",
            "count": 0
        }));
    }

    let formatted: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(offset, msg)| {
            json!({
                "index": start_idx + offset,
                "role": format!("{:?}", msg.role),
                "content": truncate_content(&extract_text_content(msg), 500)
            })
        })
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": messages.len(),
        "total_excluded": total
    }))
}

fn format_search_result(results: &[SearchResult], total: usize) -> ToolExecutionResult {
    if results.is_empty() {
        return ToolExecutionResult::success(json!({
            "message": "No matching messages found.",
            "count": 0,
            "total_excluded": total
        }));
    }

    let formatted: Vec<Value> = results
        .iter()
        .map(|r| {
            json!({
                "index": r.index,
                "role": format!("{:?}", r.message.role),
                "content": truncate_content(&extract_text_content(&r.message), 500),
                "relevance": r.score
            })
        })
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": results.len(),
        "total_excluded": total
    }))
}

fn format_recent_result(messages: &[Message], total: usize) -> ToolExecutionResult {
    let formatted: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(idx, msg)| {
            json!({
                "index": total - messages.len() + idx,
                "role": format!("{:?}", msg.role),
                "content": truncate_content(&extract_text_content(msg), 500)
            })
        })
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": messages.len(),
        "total_excluded": total
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::message_helpers;

    #[test]
    fn test_capability_properties() {
        let cap = InfinityContextCapability;
        assert_eq!(cap.id(), "infinity_context");
        assert!(cap.message_filter_provider().is_some());
        assert_eq!(cap.tools().len(), 1);
        assert!(cap.system_prompt_addition().is_some());
    }

    #[test]
    fn test_query_history_tool_search() {
        let tool = QueryHistoryTool::new();

        let messages = vec![
            message_helpers::user("Let's discuss the API design"),
            message_helpers::assistant("Sure, what about authentication?"),
            message_helpers::user("We should use JWT tokens"),
            message_helpers::assistant("JWT sounds good for the API"),
        ];

        tool.set_excluded_messages(messages);

        let results = tool.search_messages("API", 10);
        assert!(!results.is_empty());
    }
}
