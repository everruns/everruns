//! Infinity Context Capability
//!
//! Keeps recent conversation turns in prompt context while exposing a
//! `query_history` tool for older messages that fell out of the active window.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::message::{ContentPart, Message, MessageRole};
use crate::message_filter::{
    ExcludedNoticeTransform, MessageFilterProvider, MessageQuery, anchored_window,
};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{ToolContext, ToolContextService};
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

/// Provider-bound Infinity Context view used when history retrieval is unsafe.
///
/// This preserves prompt-window filtering without advertising or exposing the
/// `query_history` tool.
pub struct InfinityContextFilterOnlyCapability;

impl Capability for InfinityContextFilterOnlyCapability {
    fn id(&self) -> &str {
        INFINITY_CONTEXT_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Infinity Context"
    }

    fn description(&self) -> &str {
        "Trims older conversation history out of the live provider prompt."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        Some(Arc::new(InfinityContextFilterProvider))
    }

    fn message_filter_config(&self, config: &Value, compaction_enabled: bool) -> Value {
        message_filter_config(config, compaction_enabled)
    }
}

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

    fn message_filter_config(&self, config: &Value, compaction_enabled: bool) -> Value {
        message_filter_config(config, compaction_enabled)
    }

    /// All three `InfinityContextConfig` fields are simple numeric knobs users
    /// tune, so all are exposed. Candidate-load sizing (overfetch factor,
    /// per-message token estimate, hard DB limit) is internal and derived from
    /// these values, so it has no schema surface.
    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "context_budget_tokens": {
                    "type": "integer",
                    "title": "Context budget (tokens)",
                    "description": "Maximum prompt budget reserved for message history.",
                    "minimum": 1,
                    "default": default_context_budget_tokens()
                },
                "min_recent_messages": {
                    "type": "integer",
                    "title": "Minimum recent messages",
                    "description": "Number of recent messages always kept, even when the token budget is tight.",
                    "minimum": 1,
                    "default": default_min_recent_messages()
                },
                "max_recent_messages": {
                    "type": "integer",
                    "title": "Maximum recent messages",
                    "description": "Optional hard cap on recent messages kept in the live prompt.",
                    "minimum": 1
                },
                "keep_first_messages": {
                    "type": "integer",
                    "title": "Anchored first messages",
                    "description": "Optional leading messages kept as an anchor (the original task), even under a tight budget. Additional to the maximum recent messages. The anchor is fetched as a bounded head+tail load (capped at 16), so it is guaranteed even for histories far longer than the candidate load window. Defaults to 0 so untrusted first messages cannot bypass the configured token budget or recent-message cap; raise it only for trusted sessions where anchoring leading context is worth the extra prompt cost.",
                    "minimum": 0,
                    "maximum": MAX_KEEP_FIRST_MESSAGES,
                    "default": default_keep_first_messages()
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let typed: InfinityContextConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid infinity_context config: {e}"))?;
        if typed.context_budget_tokens == 0 {
            return Err("context_budget_tokens must be >= 1".to_string());
        }
        if typed.min_recent_messages == 0 {
            return Err("min_recent_messages must be >= 1".to_string());
        }
        if typed.max_recent_messages == Some(0) {
            return Err("max_recent_messages must be >= 1".to_string());
        }
        if typed.keep_first_messages > MAX_KEEP_FIRST_MESSAGES {
            return Err(format!(
                "keep_first_messages must be <= {MAX_KEEP_FIRST_MESSAGES}"
            ));
        }
        Ok(())
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Controls the token budget for history and the minimum/maximum number \
                     of recent messages kept in the prompt.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Нескінченний контекст"),
                description: Some(
                    "Прибирає старішу історію розмови з активного запиту, зберігаючи її \
                     доступною через інструмент query_history.",
                ),
                config_description: Some(
                    "Визначає бюджет токенів для історії та мінімальну й максимальну \
                     кількість останніх повідомлень у запиті.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "context_budget_tokens": {
                            "title": "Бюджет контексту (токени)",
                            "description": "Максимальний бюджет запиту, зарезервований для історії повідомлень."
                        },
                        "min_recent_messages": {
                            "title": "Мінімум останніх повідомлень",
                            "description": "Кількість останніх повідомлень, які зберігаються завжди, навіть коли бюджет токенів обмежений."
                        },
                        "max_recent_messages": {
                            "title": "Максимум останніх повідомлень",
                            "description": "Необов'язкове жорстке обмеження кількості останніх повідомлень в активному запиті."
                        }
                    }
                })),
            },
        ]
    }
}

fn message_filter_config(base: &Value, compaction_enabled: bool) -> Value {
    if !compaction_enabled {
        return base.clone();
    }
    let mut config = base.clone();
    match config.as_object_mut() {
        Some(map) => {
            map.insert("compaction_active".to_string(), Value::Bool(true));
        }
        None => config = json!({ "compaction_active": true }),
    }
    config
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

    /// Optional leading messages kept as an anchor (the original task / goal),
    /// regardless of token budget. Defaults to 0 so untrusted first messages
    /// cannot bypass the configured token budget or recent-message cap. The
    /// anchor is additional to `max_recent_messages` when explicitly enabled.
    #[serde(default = "default_keep_first_messages")]
    keep_first_messages: usize,

    /// Derived (not user-facing): set by capability collection when the
    /// `compaction` capability is also enabled. When true, infinity context
    /// stops doing token-budget eviction and lets compaction own reduction, so
    /// compaction's summary — not a bare "hidden" notice — covers old turns.
    #[serde(default)]
    compaction_active: bool,
}

fn default_context_budget_tokens() -> usize {
    100_000
}

fn default_min_recent_messages() -> usize {
    10
}

fn default_keep_first_messages() -> usize {
    0
}

impl Default for InfinityContextConfig {
    fn default() -> Self {
        Self {
            context_budget_tokens: default_context_budget_tokens(),
            min_recent_messages: default_min_recent_messages(),
            max_recent_messages: None,
            keep_first_messages: default_keep_first_messages(),
            compaction_active: false,
        }
    }
}

const CANDIDATE_AVG_TOKENS_PER_MESSAGE: usize = 250;
const CANDIDATE_OVERFETCH_FACTOR: usize = 4;
const CANDIDATE_MAX_MESSAGES: usize = 2_000;
const MAX_KEEP_FIRST_MESSAGES: usize = 16;

struct InfinityContextFilterProvider;

impl MessageFilterProvider for InfinityContextFilterProvider {
    fn apply_filters(&self, query: &mut MessageQuery, config: &Value) {
        let config: InfinityContextConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();

        query.limit = Some(resolve_candidate_load_limit(&config) as i64);
        // When explicitly configured (`keep_first_messages > 0`), fetch the first
        // messages alongside the latest-N tail so the task/goal anchor survives
        // even when history is far longer than the candidate load window (the
        // tail-only LIMIT would otherwise never fetch the genuine first message).
        // The default is 0 so an untrusted first message cannot bypass the token
        // budget or recent-message cap, and the value is clamped to
        // `MAX_KEEP_FIRST_MESSAGES` to bound the extra head load.
        let keep_first_messages = resolve_keep_first_messages(&config);
        if keep_first_messages > 0 {
            query.keep_head = Some(keep_first_messages);
        }
        query.prepend_transform = Some(Arc::new(ExcludedNoticeTransform::infinity_context()));
    }

    fn post_load(&self, messages: &mut Vec<Message>, config: &Value) {
        let config: InfinityContextConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();
        let existing_notice_count = take_existing_excluded_notice(messages);

        // P2 composition: when compaction is the active reducer, defer
        // token-budget eviction to it. Only re-surface any candidate-window
        // overflow notice (messages the DB load could not fetch).
        if config.compaction_active {
            if existing_notice_count > 0 {
                insert_excluded_notice(
                    messages,
                    resolve_keep_first_messages(&config),
                    existing_notice_count,
                );
            }
            return;
        }

        let outcome = trim_messages_to_token_budget(messages, &config);
        let mut total_excluded_count = existing_notice_count.saturating_add(outcome.hidden_count);
        if total_excluded_count > 0 {
            // The reduction must not leave a visible call whose result was
            // evicted. Result-only deltas remain eligible here because a
            // stateful Responses request may reference their calls through
            // `previous_response_id`; ReasonAtom resolves that distinction at
            // the final runtime boundary.
            let head_ids: std::collections::HashSet<String> = messages
                .iter()
                .take(outcome.head_len)
                .map(|message| message.id.to_string())
                .collect();
            let count_before_integrity = messages.len();
            *messages = everruns_core::retain_complete_message_tool_exchanges(messages, true);
            total_excluded_count = total_excluded_count
                .saturating_add(count_before_integrity.saturating_sub(messages.len()));
            let retained_head_len = messages
                .iter()
                .take_while(|message| head_ids.contains(&message.id.to_string()))
                .count();
            // Place the notice right after the anchored head so the layout reads
            // [task anchor] -> [N hidden] -> [recent window].
            insert_excluded_notice(messages, retained_head_len, total_excluded_count);
        }
    }

    fn priority(&self) -> i32 {
        100
    }
}

fn resolve_keep_first_messages(config: &InfinityContextConfig) -> usize {
    // Bound the always-preserved head anchor independently of validation: filters
    // can run over configs loaded before the cap existed or provided by tests.
    config.keep_first_messages.min(MAX_KEEP_FIRST_MESSAGES)
}

fn resolve_candidate_load_limit(config: &InfinityContextConfig) -> usize {
    // Cap the candidate window unconditionally at `CANDIDATE_MAX_MESSAGES` so a
    // large `min_recent_messages` or `max_recent_messages` cannot turn into an
    // unbounded DB `LIMIT`.
    let budget_derived_limit = (config.context_budget_tokens / CANDIDATE_AVG_TOKENS_PER_MESSAGE)
        .saturating_mul(CANDIDATE_OVERFETCH_FACTOR)
        .max(config.min_recent_messages)
        .clamp(1, CANDIDATE_MAX_MESSAGES);

    if let Some(max_recent_messages) = config.max_recent_messages {
        return budget_derived_limit.min(max_recent_messages.max(1));
    }

    budget_derived_limit
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

/// Outcome of token-budget trimming.
#[derive(Default)]
struct TrimOutcome {
    /// Messages dropped from the middle.
    hidden_count: usize,
    /// Length of the preserved leading anchor (where the notice is inserted).
    head_len: usize,
}

/// Insert the hidden-history notice at `position`, clamped to the message list.
fn insert_excluded_notice(messages: &mut Vec<Message>, position: usize, count: usize) {
    let text = ExcludedNoticeTransform::infinity_context()
        .format
        .replace("{}", &count.to_string());
    messages.insert(position.min(messages.len()), Message::system(text));
}

/// Trim the live window to the token budget while always keeping the first
/// `keep_first_messages` (the original task/goal) and the recent tail. Drops a
/// single contiguous block from the middle and reports how many were hidden.
fn trim_messages_to_token_budget(
    messages: &mut Vec<Message>,
    config: &InfinityContextConfig,
) -> TrimOutcome {
    if messages.is_empty() {
        return TrimOutcome::default();
    }

    let costs: Vec<usize> = messages.iter().map(estimate_message_tokens).collect();
    let window = anchored_window(
        &costs,
        resolve_keep_first_messages(config),
        config.min_recent_messages,
        config.max_recent_messages,
        config.context_budget_tokens,
    );

    let hidden_count = window.hidden();
    if hidden_count > 0 {
        // Rebuild as [0, head_len) ++ [recent_start, len) without cloning.
        let tail = messages.split_off(window.recent_start);
        messages.truncate(window.head_len);
        messages.extend(tail);
    }

    TrimOutcome {
        hidden_count,
        head_len: window.head_len,
    }
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
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_query_history(
            &tool_call.arguments,
            phase,
            locale,
        ))
    }

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

    fn required_context_services(&self) -> &'static [ToolContextService] {
        &[ToolContextService::MessageRetriever]
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
    use crate::test_fixtures::TestMessageRetriever;
    use crate::typed_id::SessionId;

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_provides_message_filter() {
        let capability = InfinityContextCapability;
        assert!(capability.message_filter_provider().is_some());
    }

    #[test]
    fn filter_only_view_preserves_filter_without_model_contributions() {
        let capability = InfinityContextFilterOnlyCapability;

        assert!(capability.message_filter_provider().is_some());
        assert!(capability.tools().is_empty());
        assert!(capability.system_prompt_addition().is_none());
    }

    #[test]
    fn test_config_schema_and_validate_config() {
        let capability = InfinityContextCapability;

        let schema = capability.config_schema().expect("config schema");
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["context_budget_tokens"].is_object());
        assert!(schema["properties"]["min_recent_messages"].is_object());
        assert!(schema["properties"]["max_recent_messages"].is_object());
        assert_eq!(
            schema["properties"]["keep_first_messages"]["maximum"],
            MAX_KEEP_FIRST_MESSAGES
        );

        // Null, empty, and valid configs are accepted.
        assert!(capability.validate_config(&Value::Null).is_ok());
        assert!(capability.validate_config(&json!({})).is_ok());
        assert!(
            capability
                .validate_config(&json!({
                    "context_budget_tokens": 50_000,
                    "min_recent_messages": 5,
                    "max_recent_messages": 100
                }))
                .is_ok()
        );

        // Wrong types and out-of-range values are rejected.
        assert!(
            capability
                .validate_config(&json!({"context_budget_tokens": "lots"}))
                .is_err()
        );
        assert!(
            capability
                .validate_config(&json!({"context_budget_tokens": 0}))
                .is_err()
        );
        assert!(
            capability
                .validate_config(&json!({"max_recent_messages": 0}))
                .is_err()
        );
        assert!(
            capability
                .validate_config(&json!({
                    "keep_first_messages": MAX_KEEP_FIRST_MESSAGES + 1
                }))
                .is_err()
        );
    }

    #[test]
    fn test_localizations_resolve_uk() {
        let capability = InfinityContextCapability;
        assert_eq!(
            capability.localized_name(Some("uk-UA")),
            "Нескінченний контекст"
        );
        assert!(capability.describe_schema(None).is_some());
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
        // The safe default omits the head anchor so a large untrusted first
        // message cannot bypass the configured budget or recent-message cap.
        assert_eq!(query.keep_head, None);
    }

    #[test]
    fn test_filter_provider_sets_keep_head_from_keep_first_messages() {
        let mut query = MessageQuery::new(SessionId::new());
        let provider = InfinityContextFilterProvider;
        provider.apply_filters(
            &mut query,
            &json!({"context_budget_tokens": 1_000, "keep_first_messages": 3}),
        );
        assert_eq!(query.keep_head, Some(3));
    }

    #[test]
    fn test_filter_provider_omits_keep_head_when_zero() {
        let mut query = MessageQuery::new(SessionId::new());
        let provider = InfinityContextFilterProvider;
        provider.apply_filters(
            &mut query,
            &json!({"context_budget_tokens": 1_000, "keep_first_messages": 0}),
        );
        assert_eq!(query.keep_head, None);
    }

    #[test]
    fn test_filter_provider_caps_keep_head_for_unvalidated_config() {
        let mut query = MessageQuery::new(SessionId::new());
        let provider = InfinityContextFilterProvider;
        provider.apply_filters(
            &mut query,
            &json!({
                "context_budget_tokens": 1_000,
                "keep_first_messages": usize::MAX
            }),
        );

        assert_eq!(query.keep_head, Some(MAX_KEEP_FIRST_MESSAGES));
    }

    #[test]
    fn test_filter_provider_caps_explicit_max_to_bounded_candidate_window() {
        let mut query = MessageQuery::new(SessionId::new());
        let provider = InfinityContextFilterProvider;
        provider.apply_filters(
            &mut query,
            &json!({
                "context_budget_tokens": 500_000,
                "min_recent_messages": 10,
                "max_recent_messages": 1_000_000
            }),
        );

        assert_eq!(query.limit, Some(CANDIDATE_MAX_MESSAGES as i64));
        assert!(query.prepend_transform.is_some());
    }

    #[test]
    fn test_filter_provider_caps_large_min_recent_messages() {
        let mut query = MessageQuery::new(SessionId::new());
        let provider = InfinityContextFilterProvider;
        provider.apply_filters(
            &mut query,
            &json!({
                "context_budget_tokens": 1_000,
                "min_recent_messages": 1_000_000,
            }),
        );

        assert_eq!(query.limit, Some(CANDIDATE_MAX_MESSAGES as i64));
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
            Message::user("the original task"),
            Message::assistant("old ".repeat(400)),
            Message::user("recent one"),
            Message::assistant("recent two"),
        ];

        provider.post_load(
            &mut messages,
            &json!({"context_budget_tokens": 1, "min_recent_messages": 2}),
        );

        // Default configuration has no head anchor: a first user message must not
        // bypass the configured token budget or recent-message cap.
        assert_eq!(messages.len(), 3);
        assert!(
            extract_text_content(&messages[0])
                .contains("2 earlier messages are NOT visible in this context")
        );
        assert_eq!(extract_text_content(&messages[1]), "recent one");
        assert_eq!(extract_text_content(&messages[2]), "recent two");
        assert!(
            !messages
                .iter()
                .any(|m| extract_text_content(m) == "the original task")
        );
    }

    #[test]
    fn test_filter_provider_applies_hard_cap_after_loading() {
        let provider = InfinityContextFilterProvider;
        let mut messages = vec![
            Message::user("one"),
            Message::assistant("two"),
            Message::user("three"),
            Message::assistant("four"),
            Message::user("five"),
        ];

        provider.post_load(
            &mut messages,
            &json!({
                "context_budget_tokens": 10_000,
                "min_recent_messages": 10,
                "max_recent_messages": 2
            }),
        );

        // Hard cap keeps only the 2 recent messages by default.
        assert_eq!(messages.len(), 3);
        assert!(
            extract_text_content(&messages[0])
                .contains("3 earlier messages are NOT visible in this context")
        );
        assert_eq!(extract_text_content(&messages[1]), "four");
        assert_eq!(extract_text_content(&messages[2]), "five");
    }

    #[test]
    fn test_filter_provider_anchors_task_through_full_flow() {
        let provider = InfinityContextFilterProvider;
        // Budget large enough that the candidate load fetches every message, but
        // small enough that the big middle turns are dropped by token budget.
        let config = json!({
            "context_budget_tokens": 600,
            "min_recent_messages": 2,
            "keep_first_messages": 1
        });
        let mut query = MessageQuery::new(SessionId::new());
        provider.apply_filters(&mut query, &config);
        let mut messages = vec![
            Message::user("TASK: build the widget"),
            Message::assistant("X".repeat(2000)),
            Message::assistant("Y".repeat(2000)),
            Message::user("recent a"),
            Message::assistant("recent b"),
        ];

        query.apply_windowing(&mut messages);
        provider.post_load(&mut messages, &config);

        // The original task is anchored at the front, the notice follows it, and
        // the recent tail is intact — the model still knows what it is doing.
        assert_eq!(extract_text_content(&messages[0]), "TASK: build the widget");
        assert!(
            extract_text_content(&messages[1])
                .contains("earlier messages are NOT visible in this context")
        );
        assert_eq!(extract_text_content(messages.last().unwrap()), "recent b");
        // The huge first assistant turn was dropped from the middle.
        assert!(
            !messages
                .iter()
                .any(|m| extract_text_content(m).starts_with("XXX"))
        );
    }

    #[test]
    fn message_filter_hook_coordinates_with_compaction_without_changing_user_config() {
        let base = json!({ "context_budget_tokens": 1000 });

        let coordinated = message_filter_config(&base, true);
        assert_eq!(coordinated["compaction_active"], json!(true));
        assert_eq!(coordinated["context_budget_tokens"], json!(1000));
        assert!(base.get("compaction_active").is_none());

        assert_eq!(message_filter_config(&base, false), base);
        assert_eq!(
            message_filter_config(&Value::Null, true),
            json!({ "compaction_active": true })
        );
    }

    #[test]
    fn test_filter_provider_defers_eviction_to_compaction() {
        let provider = InfinityContextFilterProvider;
        let mut messages = vec![
            Message::user("task"),
            Message::assistant("old ".repeat(400)),
            Message::user("recent one"),
            Message::assistant("recent two"),
        ];

        provider.post_load(
            &mut messages,
            &json!({
                "context_budget_tokens": 1,
                "min_recent_messages": 2,
                "compaction_active": true
            }),
        );

        // Compaction owns reduction: infinity context neither trims nor injects a
        // notice when compaction is active.
        assert_eq!(messages.len(), 4);
        assert!(
            messages
                .iter()
                .all(|m| !extract_text_content(m).contains("NOT visible"))
        );
    }

    #[test]
    fn test_filter_provider_caps_keep_first_messages_during_post_load() {
        let provider = InfinityContextFilterProvider;
        let mut messages: Vec<Message> = (0..20)
            .map(|idx| Message::user(format!("message {idx}")))
            .collect();

        provider.post_load(
            &mut messages,
            &json!({
                "context_budget_tokens": 1,
                "min_recent_messages": 1,
                "keep_first_messages": usize::MAX
            }),
        );

        assert_eq!(extract_text_content(&messages[0]), "message 0");
        assert_eq!(
            extract_text_content(&messages[MAX_KEEP_FIRST_MESSAGES - 1]),
            format!("message {}", MAX_KEEP_FIRST_MESSAGES - 1)
        );
        assert!(extract_text_content(&messages[MAX_KEEP_FIRST_MESSAGES]).contains("NOT visible"));
        assert_eq!(extract_text_content(messages.last().unwrap()), "message 19");
    }

    #[test]
    fn test_filter_provider_default_drops_oversized_first_message() {
        let provider = InfinityContextFilterProvider;
        let mut messages = vec![
            Message::user("attacker ".repeat(20_000)),
            Message::assistant("middle"),
            Message::user("recent one"),
            Message::assistant("recent two"),
        ];

        provider.post_load(
            &mut messages,
            &json!({
                "context_budget_tokens": 10,
                "min_recent_messages": 2,
                "max_recent_messages": 2
            }),
        );

        assert_eq!(messages.len(), 3);
        assert!(
            extract_text_content(&messages[0])
                .contains("2 earlier messages are NOT visible in this context")
        );
        assert_eq!(extract_text_content(&messages[1]), "recent one");
        assert_eq!(extract_text_content(&messages[2]), "recent two");
        assert!(
            !messages
                .iter()
                .any(|m| extract_text_content(m).starts_with("attacker"))
        );
    }

    #[test]
    fn test_filter_provider_keep_first_messages_anchors_multiple() {
        let provider = InfinityContextFilterProvider;
        let mut messages = vec![
            Message::user("anchor one"),
            Message::user("anchor two"),
            Message::assistant("mid ".repeat(400)),
            Message::user("recent"),
        ];

        provider.post_load(
            &mut messages,
            &json!({
                "context_budget_tokens": 1,
                "min_recent_messages": 1,
                "keep_first_messages": 2
            }),
        );

        assert_eq!(extract_text_content(&messages[0]), "anchor one");
        assert_eq!(extract_text_content(&messages[1]), "anchor two");
        assert!(extract_text_content(&messages[2]).contains("NOT visible"));
        assert_eq!(extract_text_content(messages.last().unwrap()), "recent");
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
        let retriever = TestMessageRetriever::new();
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
        let retriever = TestMessageRetriever::new();

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
        let retriever = TestMessageRetriever::new();
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
        let retriever = TestMessageRetriever::new();
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
        let retriever = TestMessageRetriever::new();
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
        let retriever = TestMessageRetriever::new();
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

    #[test]
    fn trim_preserves_locally_unmatched_tool_result_for_stateful_responses() {
        use crate::tool_types::ToolCall;

        let provider = InfinityContextFilterProvider;
        // min_recent_messages=3 keeps the last 3 messages. With a 1-token budget the
        // two older messages are dropped, including the assistant tool call. The
        // OpenAI Responses path may still have that call in previous_response_id
        // state, so InfinityContext must not drop the tool output before provider
        // serialization decides whether stateful continuation is active.
        let mut messages = vec![
            Message::user("old question"),
            Message::assistant_with_tools(
                "calling tool",
                vec![ToolCall {
                    id: "call_old".to_string(),
                    name: "edit_file".to_string(),
                    arguments: serde_json::json!({}),
                }],
            ),
            // This tool result is in the min-recent window but its call is trimmed away.
            Message::tool_result("call_old", Some(serde_json::json!("done")), None),
            Message::user("new question"),
            Message::assistant("answer"),
        ];

        provider.post_load(
            &mut messages,
            &serde_json::json!({"context_budget_tokens": 1, "min_recent_messages": 3}),
        );

        assert!(
            messages.iter().any(|m| m.role == MessageRole::ToolResult),
            "locally unmatched tool result must be preserved until provider serialization"
        );

        let llm_messages = messages
            .iter()
            .map(crate::llm_conversions::llm_message_from_message)
            .collect();
        let stateless_view =
            everruns_core::retain_complete_llm_tool_exchanges_for_request(llm_messages, false);
        assert!(
            stateless_view
                .iter()
                .all(|message| message.tool_call_id.is_none()),
            "a stateless runtime view must drop a result whose call was trimmed"
        );
    }

    #[test]
    fn trim_removes_a_visible_call_when_its_result_is_evicted() {
        use crate::tool_types::ToolCall;

        let provider = InfinityContextFilterProvider;
        let mut messages = vec![
            Message::user("original task"),
            Message::assistant_with_tools(
                "calling tool",
                vec![ToolCall {
                    id: "call_old".to_string(),
                    name: "bash".to_string(),
                    arguments: serde_json::json!({}),
                }],
            ),
            Message::tool_result(
                "call_old",
                Some(serde_json::json!("large result ".repeat(500))),
                None,
            ),
            Message::user("recent question"),
            Message::assistant("recent answer"),
        ];

        provider.post_load(
            &mut messages,
            &serde_json::json!({
                "context_budget_tokens": 1,
                "min_recent_messages": 2,
                "keep_first_messages": 2
            }),
        );

        assert!(
            messages
                .iter()
                .flat_map(Message::tool_calls)
                .next()
                .is_none(),
            "the anchored assistant message must not retain a call after its result is hidden"
        );
    }

    #[test]
    fn trim_keeps_tool_result_when_tool_call_is_visible() {
        use crate::tool_types::ToolCall;

        let provider = InfinityContextFilterProvider;
        // All 3 messages fit in the window: no orphan expected.
        let mut messages = vec![
            Message::assistant_with_tools(
                "calling tool",
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "read_file".to_string(),
                    arguments: serde_json::json!({}),
                }],
            ),
            Message::tool_result("call_1", Some(serde_json::json!("content")), None),
            Message::user("thanks"),
        ];

        provider.post_load(
            &mut messages,
            &serde_json::json!({"context_budget_tokens": 100_000, "min_recent_messages": 10}),
        );

        assert!(
            messages.iter().any(|m| m.role == MessageRole::ToolResult),
            "tool result must be kept when its tool call is visible"
        );
    }
}
