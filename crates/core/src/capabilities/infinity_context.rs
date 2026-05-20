//! Infinity Context Capability
//!
//! Keeps recent conversation turns in prompt context while exposing a
//! `query_history` tool for older messages that fell out of the active window.

use super::{Capability, CapabilityStatus};
use crate::message::{ContentPart, Message, MessageRole};
use crate::message_filter::{ExcludedNoticeTransform, MessageFilterProvider, MessageQuery};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::io::{self, Write};
use std::sync::Arc;

/// Capability ID for infinity context.
pub const INFINITY_CONTEXT_CAPABILITY_ID: &str = "infinity_context";

/// Infinity context capability.
pub struct InfinityContextCapability;

impl Capability for InfinityContextCapability {
    fn id(&self) -> &str {
        INFINITY_CONTEXT_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Infinity Context"
    }

    fn description(&self) -> &str {
        r#"Trims older conversation history out of the live prompt while keeping it queryable with `query_history`.

> [!TIP]
> Use this for long-running sessions where earlier discussion still matters but should not consume prompt budget every turn."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("infinity")
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
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

const INFINITY_CONTEXT_SYSTEM_PROMPT: &str = r#"## Conversation history

Earlier messages may be trimmed from the live prompt. Use `query_history`
to retrieve them when needed. The window is trimmed automatically; do not
abandon tasks for token reasons — persist important state via file or
memory tools when available."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InfinityContextConfig {
    /// Maximum prompt budget reserved for message history.
    #[serde(default = "default_context_budget_tokens")]
    context_budget_tokens: usize,

    /// Minimum number of recent messages to keep even when the budget is tight.
    #[serde(default = "default_min_recent_messages")]
    min_recent_messages: usize,

    /// Optional hard cap on recent messages kept in the live prompt.
    ///
    /// Useful for public support chats where the prompt must stay small even
    /// when the token-budget estimate would allow more messages.
    #[serde(default)]
    max_recent_messages: Option<usize>,
}

fn default_context_budget_tokens() -> usize {
    100_000
}

fn default_min_recent_messages() -> usize {
    10
}

impl Default for InfinityContextConfig {
    fn default() -> Self {
        Self {
            context_budget_tokens: default_context_budget_tokens(),
            min_recent_messages: default_min_recent_messages(),
            max_recent_messages: None,
        }
    }
}

const CANDIDATE_AVG_TOKENS_PER_MESSAGE: usize = 250;
const CANDIDATE_OVERFETCH_FACTOR: usize = 4;
const CANDIDATE_MAX_MESSAGES: usize = 2_000;

struct InfinityContextFilterProvider;

impl MessageFilterProvider for InfinityContextFilterProvider {
    fn apply_filters(&self, query: &mut MessageQuery, config: &Value) {
        let config: InfinityContextConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();

        query.limit = Some(resolve_candidate_load_limit(&config) as i64);
        query.prepend_transform = Some(Arc::new(ExcludedNoticeTransform::infinity_context()));
    }

    fn post_load(&self, messages: &mut Vec<Message>, config: &Value) {
        let config: InfinityContextConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();
        let existing_notice_count = take_existing_excluded_notice(messages);
        let excluded_count = trim_messages_to_token_budget(messages, &config);
        let total_excluded_count = existing_notice_count.saturating_add(excluded_count);
        if total_excluded_count > 0 {
            messages.insert(
                0,
                Message::system(
                    ExcludedNoticeTransform::infinity_context()
                        .format
                        .replace("{}", &total_excluded_count.to_string()),
                ),
            );
        }
    }

    fn priority(&self) -> i32 {
        100
    }
}

fn resolve_candidate_load_limit(config: &InfinityContextConfig) -> usize {
    if let Some(max_recent_messages) = config.max_recent_messages {
        return max_recent_messages.max(1);
    }

    (config.context_budget_tokens / CANDIDATE_AVG_TOKENS_PER_MESSAGE)
        .saturating_mul(CANDIDATE_OVERFETCH_FACTOR)
        .min(CANDIDATE_MAX_MESSAGES)
        .max(config.min_recent_messages)
        .max(1)
}

fn estimate_message_tokens(message: &Message) -> usize {
    const TOKEN_CHARS: usize = 4;
    let role_overhead = message.role.to_string().len() + 8;
    let content_len: usize = message
        .content
        .iter()
        .map(|part| match part {
            ContentPart::Text(text) => text.text.len(),
            ContentPart::Image(image) => {
                image.url.as_ref().map_or(0, String::len)
                    + image.base64.as_ref().map_or(50, String::len)
                    + image.media_type.as_ref().map_or(0, String::len)
            }
            ContentPart::ImageFile(file) => {
                file.image_id.to_string().len() + file.filename.as_ref().map_or(0, String::len)
            }
            ContentPart::ToolCall(call) => {
                call.id.len() + call.name.len() + estimate_json_value_len(&call.arguments) + 20
            }
            ContentPart::ToolResult(result) => {
                result.tool_call_id.len()
                    + result.result.as_ref().map_or(0, estimate_json_value_len)
                    + result.error.as_ref().map_or(0, String::len)
                    + 20
            }
        })
        .sum();
    (role_overhead + content_len) / TOKEN_CHARS
}

struct CountingWriter {
    len: usize,
}

impl Write for CountingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.len = self.len.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn estimate_json_value_len(value: &Value) -> usize {
    let mut writer = CountingWriter { len: 0 };
    serde_json::to_writer(&mut writer, value)
        .map(|_| writer.len)
        .unwrap_or(0)
}

fn take_existing_excluded_notice(messages: &mut Vec<Message>) -> usize {
    let Some(first) = messages.first() else {
        return 0;
    };
    let Some(count) = parse_excluded_notice_count(first) else {
        return 0;
    };

    messages.remove(0);
    count
}

fn parse_excluded_notice_count(message: &Message) -> Option<usize> {
    let text = message.text()?;
    let rest = text.strip_prefix("[IMPORTANT: ")?;
    let (count, rest) = rest.split_once(' ')?;
    if !rest.starts_with("earlier messages are NOT visible in this context.") {
        return None;
    }
    count.parse().ok()
}

fn trim_messages_to_token_budget(
    messages: &mut Vec<Message>,
    config: &InfinityContextConfig,
) -> usize {
    if messages.is_empty() {
        return 0;
    }

    let original_count = messages.len();
    if let Some(max_recent_messages) = config.max_recent_messages {
        let max_recent_messages = max_recent_messages.max(1);
        if messages.len() > max_recent_messages {
            let drop_count = messages.len() - max_recent_messages;
            messages.drain(0..drop_count);
        }
    }

    let capped_count = messages.len();
    let min_recent_start = capped_count.saturating_sub(config.min_recent_messages);
    let mut selected = Vec::new();
    let mut selected_tokens = 0usize;

    for (idx, message) in messages.iter().enumerate().skip(min_recent_start) {
        selected.push((idx, message.clone()));
        selected_tokens = selected_tokens.saturating_add(estimate_message_tokens(message));
    }

    let mut remaining_budget = config.context_budget_tokens.saturating_sub(selected_tokens);
    for (idx, message) in messages[..min_recent_start].iter().enumerate().rev() {
        let tokens = estimate_message_tokens(message);
        if tokens <= remaining_budget {
            selected.push((idx, message.clone()));
            remaining_budget -= tokens;
        }
    }

    selected.sort_by_key(|(idx, _)| *idx);
    *messages = selected.into_iter().map(|(_, message)| message).collect();
    original_count.saturating_sub(messages.len())
}

/// Tool for querying earlier conversation history.
pub struct QueryHistoryTool;

#[derive(Debug, Deserialize)]
struct QueryHistoryParams {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    message_range: Option<MessageRange>,
    #[serde(default = "default_query_limit")]
    limit: usize,
}

#[derive(Debug, Deserialize)]
struct MessageRange {
    from: usize,
    to: usize,
}

fn default_query_limit() -> usize {
    20
}

#[async_trait]
impl Tool for QueryHistoryTool {
    fn name(&self) -> &str {
        "query_history"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Query History")
    }

    fn description(&self) -> &str {
        "Search or retrieve earlier messages from this conversation that may not be visible in the current prompt."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keyword search over earlier messages"
                },
                "message_range": {
                    "type": "object",
                    "properties": {
                        "from": { "type": "integer", "minimum": 0, "description": "Start index (0-based, inclusive)" },
                        "to": { "type": "integer", "minimum": 0, "description": "End index (0-based, exclusive)" }
                    },
                    "required": ["from", "to"],
                    "additionalProperties": false,
                    "description": "Retrieve messages by absolute position in the conversation"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "default": 20,
                    "description": "Maximum number of messages to return"
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "query_history requires session context. Execute it with ToolContext.",
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
            Ok(params) => params,
            Err(error) => {
                return ToolExecutionResult::tool_error(format!("Invalid parameters: {error}"));
            }
        };

        let Some(retriever) = &context.message_retriever else {
            return ToolExecutionResult::tool_error("No message retriever available");
        };

        let messages = match retriever.load(context.session_id).await {
            Ok(messages) => messages,
            Err(error) => {
                return ToolExecutionResult::internal_error(error);
            }
        };

        if messages.is_empty() {
            return ToolExecutionResult::success(json!({
                "count": 0,
                "message": "No history available."
            }));
        }

        let limit = params.limit.min(50);
        let total = messages.len();

        if let Some(range) = params.message_range {
            let from = range.from.min(total);
            let to = range.to.min(total).max(from);
            let range_messages: Vec<_> = messages[from..to].iter().take(limit).collect();
            return format_range_result(&range_messages, from, total);
        }

        if let Some(query) = params.query.as_deref() {
            let results = search_messages(&messages, query, limit);
            return format_search_result(&results, total);
        }

        let recent: Vec<_> = messages.iter().rev().take(limit).collect();
        format_recent_result(&recent, total)
    }
}

struct SearchResult<'a> {
    index: usize,
    message: &'a Message,
    score: f64,
}

fn search_messages<'a>(
    messages: &'a [Message],
    query: &str,
    limit: usize,
) -> Vec<SearchResult<'a>> {
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for (index, message) in messages.iter().enumerate() {
        let content = extract_text_content(message).to_lowercase();
        if !content.contains(&query_lower) {
            continue;
        }

        let mut score = 1.0;

        if content.split_whitespace().any(|word| word == query_lower) {
            score += 0.5;
        }

        if !messages.is_empty() {
            score += (index as f64 / messages.len() as f64) * 0.3;
        }

        match message.role {
            MessageRole::User | MessageRole::Agent => score += 0.2,
            MessageRole::System => score += 0.1,
            MessageRole::ToolResult => {}
        }

        results.push(SearchResult {
            index,
            message,
            score,
        });
    }

    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    results.truncate(limit);
    results
}

fn extract_text_content(message: &Message) -> String {
    message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.text.clone()),
            ContentPart::ToolResult(result) => result.result.as_ref().map(ToString::to_string),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_content(content: &str, max_len: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_len {
        return content.to_string();
    }

    format!("{}...", content.chars().take(max_len).collect::<String>())
}

fn format_message(message: &Message, index: usize, total: usize) -> Value {
    json!({
        "index": index,
        "position": format!("{}/{}", index + 1, total),
        "role": message.role.to_string(),
        "created_at": message.created_at.to_rfc3339(),
        "content": truncate_content(&extract_text_content(message), 500)
    })
}

fn format_range_result(
    messages: &[&Message],
    start_index: usize,
    total: usize,
) -> ToolExecutionResult {
    if messages.is_empty() {
        return ToolExecutionResult::success(json!({
            "count": 0,
            "message": "No messages in the requested range."
        }));
    }

    let formatted: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(offset, message)| format_message(message, start_index + offset, total))
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": messages.len(),
        "total_in_history": total,
        "range": format!("{}-{}", start_index + 1, start_index + messages.len())
    }))
}

fn format_search_result(results: &[SearchResult<'_>], total: usize) -> ToolExecutionResult {
    if results.is_empty() {
        return ToolExecutionResult::success(json!({
            "count": 0,
            "message": "No matching messages found."
        }));
    }

    let formatted: Vec<Value> = results
        .iter()
        .map(|result| {
            let mut message = format_message(result.message, result.index, total);
            message["relevance_score"] = json!(format!("{:.2}", result.score));
            message
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
        .map(|(offset, message)| format_message(message, total - messages.len() + offset, total))
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": messages.len(),
        "total_in_history": total,
        "note": "Showing most recent history. Use `query` to search or `message_range` to fetch older messages."
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::InMemoryMessageRetriever;
    use crate::typed_id::SessionId;

    #[test]
    fn test_capability_metadata() {
        let capability = InfinityContextCapability;

        assert_eq!(capability.id(), INFINITY_CONTEXT_CAPABILITY_ID);
        assert_eq!(capability.name(), "Infinity Context");
        assert_eq!(capability.status(), CapabilityStatus::Available);
        assert_eq!(capability.category(), Some("Optimization"));
        assert_eq!(capability.tools().len(), 1);
        assert!(capability.message_filter_provider().is_some());
    }

    #[test]
    fn test_filter_provider_sets_bounded_candidate_load_limit_without_hard_cap() {
        let mut query = MessageQuery::new(SessionId::new());
        let provider = InfinityContextFilterProvider;
        provider.apply_filters(
            &mut query,
            &json!({"context_budget_tokens": 1_000, "min_recent_messages": 3}),
        );

        assert_eq!(query.limit, Some(16));
        assert!(query.prepend_transform.is_some());
    }

    #[test]
    fn test_filter_provider_allows_small_public_chat_window() {
        let mut query = MessageQuery::new(SessionId::new());
        let provider = InfinityContextFilterProvider;
        provider.apply_filters(
            &mut query,
            &json!({
                "context_budget_tokens": 10_000,
                "min_recent_messages": 10,
                "max_recent_messages": 30
            }),
        );

        assert_eq!(query.limit, Some(30));
        assert!(query.prepend_transform.is_some());
    }

    #[test]
    fn test_filter_provider_falls_back_to_defaults_for_invalid_config() {
        let mut query = MessageQuery::new(SessionId::new());
        let provider = InfinityContextFilterProvider;
        provider.apply_filters(
            &mut query,
            &json!({"context_budget_tokens": "not-a-number"}),
        );

        assert_eq!(query.limit, Some(1_600));
        assert!(query.prepend_transform.is_some());
    }

    #[test]
    fn test_filter_provider_trims_loaded_messages_by_token_budget() {
        let provider = InfinityContextFilterProvider;
        let mut messages = vec![
            Message::user("old tiny"),
            Message::assistant("old ".repeat(400)),
            Message::user("recent one"),
            Message::assistant("recent two"),
        ];

        provider.post_load(
            &mut messages,
            &json!({"context_budget_tokens": 1, "min_recent_messages": 2}),
        );

        assert_eq!(messages.len(), 3);
        assert!(
            extract_text_content(&messages[0])
                .contains("earlier messages are NOT visible in this context")
        );
        assert_eq!(extract_text_content(&messages[1]), "recent one");
        assert_eq!(extract_text_content(&messages[2]), "recent two");
    }

    #[test]
    fn test_filter_provider_applies_hard_cap_after_loading() {
        let provider = InfinityContextFilterProvider;
        let mut messages = vec![
            Message::user("one"),
            Message::assistant("two"),
            Message::user("three"),
        ];

        provider.post_load(
            &mut messages,
            &json!({
                "context_budget_tokens": 10_000,
                "min_recent_messages": 10,
                "max_recent_messages": 2
            }),
        );

        assert_eq!(messages.len(), 3);
        assert!(
            extract_text_content(&messages[0])
                .contains("earlier messages are NOT visible in this context")
        );
        assert_eq!(extract_text_content(&messages[1]), "two");
        assert_eq!(extract_text_content(&messages[2]), "three");
    }

    #[test]
    fn test_filter_provider_preserves_hard_cap_notice_through_full_flow() {
        let provider = InfinityContextFilterProvider;
        let config = json!({
            "context_budget_tokens": 10_000,
            "min_recent_messages": 10,
            "max_recent_messages": 2
        });
        let mut query = MessageQuery::new(SessionId::new());
        provider.apply_filters(&mut query, &config);
        let mut messages = vec![
            Message::user("one"),
            Message::assistant("two"),
            Message::user("three"),
        ];

        query.apply_windowing(&mut messages);
        provider.post_load(&mut messages, &config);

        assert_eq!(messages.len(), 3);
        assert!(
            extract_text_content(&messages[0])
                .contains("1 earlier messages are NOT visible in this context")
        );
        assert_eq!(extract_text_content(&messages[1]), "two");
        assert_eq!(extract_text_content(&messages[2]), "three");
    }

    #[test]
    fn test_estimate_json_value_len_matches_serialized_length() {
        let value = json!({
            "stdout": ["alpha", "beta"],
            "ok": true,
            "count": 2
        });

        assert_eq!(
            estimate_json_value_len(&value),
            serde_json::to_string(&value).unwrap().len()
        );
    }

    #[test]
    fn test_query_history_requires_context() {
        let tool = QueryHistoryTool;
        assert!(tool.requires_context());
    }

    #[tokio::test]
    async fn test_query_history_tool_errors_without_retriever() {
        let tool = QueryHistoryTool;
        let result = tool
            .execute_with_context(json!({"query": "api"}), &ToolContext::new(SessionId::new()))
            .await;

        match result {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("No message retriever available"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_query_history_tool_rejects_invalid_params() {
        let result = QueryHistoryTool.execute(json!({"limit": "oops"})).await;

        match result {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("requires session context"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }

        let session_id = SessionId::new();
        let retriever = InMemoryMessageRetriever::new();
        let result = QueryHistoryTool
            .execute_with_context(
                json!({"message_range": {"from": "bad", "to": 1}}),
                &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
            )
            .await;

        match result {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains("Invalid parameters"));
            }
            other => panic!("expected tool error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_query_history_tool_empty_history() {
        let session_id = SessionId::new();
        let retriever = InMemoryMessageRetriever::new();

        let result = QueryHistoryTool
            .execute_with_context(
                json!({}),
                &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
            )
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["count"], 0);
                assert_eq!(value["message"], "No history available.");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_query_history_tool_searches_history() {
        let session_id = SessionId::new();
        let retriever = InMemoryMessageRetriever::new();
        retriever
            .seed(
                session_id,
                vec![
                    Message::user("First topic"),
                    Message::assistant("The API key is abc123"),
                    Message::user("We should keep discussing logging"),
                ],
            )
            .await;

        let result = QueryHistoryTool
            .execute_with_context(
                json!({"query": "api key"}),
                &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
            )
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["count"], 1);
                assert_eq!(value["messages"][0]["content"], "The API key is abc123");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_query_history_tool_search_no_match() {
        let session_id = SessionId::new();
        let retriever = InMemoryMessageRetriever::new();
        retriever
            .seed(
                session_id,
                vec![Message::user("one"), Message::assistant("two")],
            )
            .await;

        let result = QueryHistoryTool
            .execute_with_context(
                json!({"query": "missing"}),
                &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
            )
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["count"], 0);
                assert_eq!(value["message"], "No matching messages found.");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_query_history_tool_reads_range() {
        let session_id = SessionId::new();
        let retriever = InMemoryMessageRetriever::new();
        retriever
            .seed(
                session_id,
                vec![
                    Message::user("one"),
                    Message::assistant("two"),
                    Message::user("three"),
                ],
            )
            .await;

        let result = QueryHistoryTool
            .execute_with_context(
                json!({"message_range": {"from": 1, "to": 3}, "limit": 10}),
                &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
            )
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["count"], 2);
                assert_eq!(value["messages"][0]["content"], "two");
                assert_eq!(value["messages"][1]["content"], "three");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_query_history_tool_clamps_out_of_bounds_range() {
        let session_id = SessionId::new();
        let retriever = InMemoryMessageRetriever::new();
        retriever
            .seed(
                session_id,
                vec![
                    Message::user("one"),
                    Message::assistant("two"),
                    Message::user("three"),
                ],
            )
            .await;

        let result = QueryHistoryTool
            .execute_with_context(
                json!({"message_range": {"from": 99, "to": 100}}),
                &ToolContext::new(session_id).with_message_retriever(Arc::new(retriever)),
            )
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["count"], 0);
                assert_eq!(value["message"], "No messages in the requested range.");
            }
            other => panic!("expected success, got {other:?}"),
        }
    }

    #[test]
    fn test_truncate_content_is_utf8_safe() {
        let truncated = truncate_content("hello🙂world", 6);
        assert_eq!(truncated, "hello🙂...");
    }
}
