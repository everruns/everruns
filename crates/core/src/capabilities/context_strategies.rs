//! Context Strategy Capabilities
//!
//! These capabilities control how conversation history is managed when sending
//! messages to the LLM. They implement the "infinity context" approach for
//! handling conversations that exceed model context limits.
//!
//! ## Available Strategies
//!
//! - **NaiveTrimCapability**: Drops oldest messages to fit context budget.
//!   Simple but loses historical information permanently.
//!
//! - **InfinityContextCapability**: Trims old messages but provides a
//!   `query_history` tool allowing the LLM to search/retrieve excluded messages.
//!   Enables arbitrarily long conversations without losing access to history.
//!
//! ## Configuration
//!
//! Both capabilities accept configuration via `AgentCapabilityConfig`:
//!
//! ```json
//! {
//!   "context_budget_tokens": 100000,  // Max tokens to send to LLM
//!   "min_recent_messages": 10,        // Always keep this many recent messages
//!   "boost_recency": true,            // Prefer recent messages in search (infinity only)
//!   "boost_conversation": true        // Prefer user/assistant over tools (infinity only)
//! }
//! ```

use super::{Capability, CapabilityStatus};
use crate::message::{ContentPart, Message, MessageRole};
use crate::message_filter::{
    BatchTransformResult, InjectedMessage, InjectionPosition, MessageFilter, MessageFilterProvider,
    MessageQuery,
};
// Unused import for now, but may be used in future:
// use crate::tool_types::{BuiltinTool, ToolDefinition, ToolPolicy};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

// ============================================================================
// Configuration
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
            ContentPart::Image(_) | ContentPart::ImageFile(_) => 1000, // Images cost ~1k tokens
        })
        .sum();

    // Rough approximation: 1 token ≈ 4 characters
    (content_len + 3) / 4
}

/// Estimate total tokens for a slice of messages
pub fn estimate_total_tokens(messages: &[Message]) -> usize {
    messages.iter().map(estimate_tokens).sum()
}

// ============================================================================
// Naive Trim Capability
// ============================================================================

/// Capability that drops oldest messages to fit context budget.
///
/// This is the simplest context management strategy. When the conversation
/// exceeds the budget, oldest messages are removed. The information in
/// those messages is permanently lost for that LLM call.
///
/// Use this for conversations where:
/// - Historical context is not critical
/// - You want minimal overhead
/// - You're fine with the LLM only seeing recent context
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

        // Add a batch transform filter that trims to budget
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
/// Returns both kept and excluded messages for tracking
fn trim_messages_to_budget(
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

// ============================================================================
// Infinity Context Capability
// ============================================================================

/// Capability that enables unlimited conversation length via history querying.
///
/// This strategy:
/// 1. Trims old messages to fit within the context budget
/// 2. Injects a notice about excluded messages
/// 3. Provides a `query_history` tool for the LLM to search/retrieve history
///
/// The LLM can "pull" relevant historical context on-demand, enabling
/// arbitrarily long conversations without losing access to information.
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

        // Add batch transform filter that trims and tracks excluded messages
        query.filters.push(MessageFilter::BatchTransform(Arc::new(
            move |messages: Vec<Message>| -> BatchTransformResult {
                trim_messages_to_budget(messages, budget, min_recent)
            },
        )));

        // Inject a notice about excluded messages at the start
        // Note: The actual count will be determined at runtime after filtering
        // For now, we inject a generic notice
        let notice = Message::system(
            "[Context Notice: Earlier messages may not be shown. Use `query_history` tool to search history.]",
        );
        query.injections.push(InjectedMessage {
            position: InjectionPosition::Start,
            message: notice,
        });
    }

    fn priority(&self) -> i32 {
        // Run after other filters
        100
    }
}

// ============================================================================
// Query History Tool
// ============================================================================

/// Tool for querying conversation history
///
/// This tool is provided by the InfinityContextCapability to allow the LLM
/// to search and retrieve messages that were excluded from the current context.
pub struct QueryHistoryTool {
    /// Stored excluded messages for this session (populated at runtime)
    excluded_messages: Arc<std::sync::RwLock<Vec<Message>>>,
    config: Arc<std::sync::RwLock<ContextStrategyConfig>>,
}

impl QueryHistoryTool {
    pub fn new() -> Self {
        Self {
            excluded_messages: Arc::new(std::sync::RwLock::new(Vec::new())),
            config: Arc::new(std::sync::RwLock::new(ContextStrategyConfig::default())),
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

            // Calculate relevance score
            let mut score = 0.0;

            // Keyword match
            if content_lower.contains(&query_lower) {
                score += 1.0;

                // Exact word match bonus
                if content_lower.split_whitespace().any(|w| w == query_lower) {
                    score += 0.5;
                }
            }

            if score > 0.0 {
                // Recency boost (newer messages score higher)
                if config.boost_recency && !messages.is_empty() {
                    let recency_factor = (idx as f64 + 1.0) / messages.len() as f64;
                    score += recency_factor * 0.3;
                }

                // Message type boost
                if config.boost_conversation {
                    match msg.role {
                        MessageRole::User | MessageRole::Assistant => score += 0.2,
                        MessageRole::ToolResult => {} // No boost
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

        // Sort by score descending
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

/// Extract text content from a message
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

/// Query parameters for the query_history tool
#[derive(Debug, Deserialize)]
struct QueryHistoryParams {
    /// Search term to find relevant messages
    query: Option<String>,
    /// Range of messages to retrieve (0-indexed)
    message_range: Option<MessageRange>,
    /// Filter by message type
    message_types: Option<Vec<String>>,
    /// Maximum messages to return
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
        "Search or retrieve messages from earlier in this conversation that are not currently visible. Use this when you need to reference something discussed previously."
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
                        "from": {
                            "type": "integer",
                            "description": "Start index (0-based, oldest message is 0)"
                        },
                        "to": {
                            "type": "integer",
                            "description": "End index (exclusive)"
                        }
                    },
                    "description": "Retrieve messages by position range"
                },
                "message_types": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["user", "assistant", "tool_result"]
                    },
                    "description": "Filter by message type"
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

        let limit = params.limit.min(50); // Cap at 50
        let total = messages.len();

        // Handle range query
        if let Some(range) = params.message_range {
            drop(messages); // Release lock before calling get_range
            let range_messages = self.get_range(range.from, range.to, limit);
            return format_range_result(&range_messages, range.from, total);
        }

        // Handle search query
        if let Some(ref search_query) = params.query {
            drop(messages); // Release lock before calling search_messages
            let results = self.search_messages(search_query, limit);
            return format_search_result(&results, total);
        }

        // Default: return most recent excluded messages
        let recent: Vec<_> = messages.iter().rev().take(limit).cloned().collect();
        format_recent_result(&recent, total)
    }
}

fn format_range_result(
    messages: &[Message],
    start_idx: usize,
    total: usize,
) -> ToolExecutionResult {
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
                "role": msg.role.to_string(),
                "content": truncate_content(&extract_text_content(msg), 500),
                "timestamp": msg.created_at.to_rfc3339()
            })
        })
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "total_excluded": total,
        "count": formatted.len()
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
            json!({
                "index": r.index,
                "role": r.message.role.to_string(),
                "content": truncate_content(&extract_text_content(&r.message), 500),
                "relevance": format!("{:.2}", r.score),
                "timestamp": r.message.created_at.to_rfc3339()
            })
        })
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "total_excluded": total,
        "count": formatted.len()
    }))
}

fn format_recent_result(messages: &[Message], total: usize) -> ToolExecutionResult {
    let formatted: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(idx, msg)| {
            json!({
                "position": format!("{} most recent", idx + 1),
                "role": msg.role.to_string(),
                "content": truncate_content(&extract_text_content(msg), 500),
                "timestamp": msg.created_at.to_rfc3339()
            })
        })
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "total_excluded": total,
        "count": formatted.len()
    }))
}

fn truncate_content(content: &str, max_len: usize) -> String {
    if content.len() > max_len {
        format!("{}...", &content[..max_len])
    } else {
        content.to_string()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        let msg = Message::user("Hello, world!");
        let tokens = estimate_tokens(&msg);
        // "Hello, world!" is 13 chars, so ~3-4 tokens
        assert!(tokens >= 3 && tokens <= 5);
    }

    #[test]
    fn test_trim_messages_to_budget() {
        let messages: Vec<Message> = (0..100)
            .map(|i| Message::user(format!("Message number {}", i)))
            .collect();

        // Small budget should keep only recent messages
        let result = trim_messages_to_budget(messages, 100, 5);

        // Should have at least min_recent (5) messages
        assert!(result.kept.len() >= 5);
        // Should have kept the most recent
        let last_content = result.kept.last().unwrap().content[0].as_text().unwrap();
        assert!(last_content.contains("99"));
        // Should have excluded some messages
        assert!(!result.excluded.is_empty());
    }

    #[test]
    fn test_naive_trim_capability() {
        let cap = NaiveTrimCapability;
        assert_eq!(cap.id(), "naive_trim");
        assert!(cap.message_filter_provider().is_some());
        assert!(cap.tools().is_empty());
    }

    #[test]
    fn test_infinity_context_capability() {
        let cap = InfinityContextCapability;
        assert_eq!(cap.id(), "infinity_context");
        assert!(cap.message_filter_provider().is_some());
        assert_eq!(cap.tools().len(), 1);
        assert!(cap.system_prompt_addition().is_some());
    }

    #[test]
    fn test_query_history_tool_search() {
        let tool = QueryHistoryTool::new();

        // Set up some test messages
        let messages = vec![
            Message::user("Let's discuss the API design"),
            Message::assistant("Sure, what about authentication?"),
            Message::user("We should use JWT tokens for the API"),
        ];
        tool.set_excluded_messages(messages);

        // Search for "API"
        let results = tool.search_messages("API", 10);
        assert!(!results.is_empty());
        // Should find messages containing "API"
        assert!(
            results
                .iter()
                .any(|r| extract_text_content(&r.message).contains("API"))
        );
    }

    #[test]
    fn test_config_default() {
        let config = ContextStrategyConfig::default();
        assert_eq!(config.context_budget_tokens, 100_000);
        assert_eq!(config.min_recent_messages, 10);
        assert!(config.boost_recency);
        assert!(config.boost_conversation);
    }
}
