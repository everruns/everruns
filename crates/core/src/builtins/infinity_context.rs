//! Infinity Context Capability
//!
//! Keeps recent conversation turns in prompt context while exposing a
//! `query_history` tool for older messages that fell out of the active window.

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::builtins::message::{ContentPart, RuntimeMessage, RuntimeMessageRole};
use crate::builtins::message_filter::{
    ExcludedNoticeTransform, MessageFilterProvider, MessageQuery,
};
use crate::builtins::tool_types::ToolHints;
use crate::builtins::tools::{Tool, ToolExecutionResult};
use crate::{tool_context::ToolContext, tool_context::ToolContextService};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::cmp::Ordering;
use std::io::{self, Write};
use std::sync::Arc;

#[path = "infinity_context/config.rs"]
mod config;
#[path = "infinity_context/window.rs"]
mod window;
use config::{
    CANDIDATE_AVG_TOKENS_PER_MESSAGE, CANDIDATE_MAX_MESSAGES, CANDIDATE_OVERFETCH_FACTOR,
    InfinityContextConfig, MAX_KEEP_FIRST_MESSAGES, default_context_budget_tokens,
    default_keep_first_messages, default_min_recent_messages,
};
use window::trim_messages_to_token_budget;

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
        CapabilityStatus::Deprecated
    }

    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        Some(Arc::new(InfinityContextFilterProvider))
    }

    fn message_filter_config(
        &self,
        config: &Value,
        compaction_enabled: bool,
        provider_managed_reduction: bool,
    ) -> Value {
        message_filter_config(config, compaction_enabled, provider_managed_reduction)
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
        r#"Deprecated compatibility capability. Use `compaction` for managed long-session context. Trims older conversation history out of the live prompt while keeping it queryable with `query_history`.

> [!TIP]
> Use this for long-running sessions where earlier discussion still matters but should not consume prompt budget every turn."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Deprecated
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

    fn message_filter_config(
        &self,
        config: &Value,
        compaction_enabled: bool,
        provider_managed_reduction: bool,
    ) -> Value {
        message_filter_config(config, compaction_enabled, provider_managed_reduction)
    }

    fn provider_managed_reduction_budget(&self, config: &Value) -> Option<usize> {
        Some(
            serde_json::from_value::<InfinityContextConfig>(config.clone())
                .unwrap_or_default()
                .context_budget_tokens,
        )
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

fn message_filter_config(
    base: &Value,
    compaction_enabled: bool,
    provider_managed_reduction: bool,
) -> Value {
    if !compaction_enabled && !provider_managed_reduction {
        return base.clone();
    }
    let mut config = base.clone();
    match config.as_object_mut() {
        Some(map) => {
            map.insert(
                "compaction_active".to_string(),
                Value::Bool(compaction_enabled),
            );
            map.insert(
                "provider_managed_reduction".to_string(),
                Value::Bool(provider_managed_reduction),
            );
        }
        None => {
            config = json!({
                "compaction_active": compaction_enabled,
                "provider_managed_reduction": provider_managed_reduction,
            })
        }
    }
    config
}

const INFINITY_CONTEXT_SYSTEM_PROMPT: &str = r#"## Conversation history

Earlier messages may be trimmed from the live prompt. Use `query_history`
to retrieve them when needed. The window is trimmed automatically; do not
abandon tasks for token reasons — persist important state via file or
memory tools when available."#;

struct InfinityContextFilterProvider;

impl MessageFilterProvider for InfinityContextFilterProvider {
    fn apply_filters(&self, query: &mut MessageQuery, config: &Value) {
        let config: InfinityContextConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();

        if config.provider_managed_reduction {
            return;
        }

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

    fn post_load(&self, messages: &mut Vec<RuntimeMessage>, config: &Value) {
        let config: InfinityContextConfig =
            serde_json::from_value(config.clone()).unwrap_or_default();
        if config.provider_managed_reduction {
            return;
        }
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

        let outcome = trim_messages_to_token_budget(messages, &config, existing_notice_count);
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
            *messages = crate::retain_complete_message_tool_exchanges(messages, true);
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

fn estimate_message_tokens(message: &RuntimeMessage) -> usize {
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
            // The model receives the image, not the reference: about 1,600 tokens.
            ContentPart::ImageFile(_) => 1_600 * TOKEN_CHARS,
            ContentPart::File(file) => {
                file.file_id.to_string().len() + file.filename.as_ref().map_or(0, String::len)
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
            // Reasoning replays as provider-native artifacts, so its opaque
            // payloads and text still occupy the request budget.
            ContentPart::Reasoning(reasoning) => {
                reasoning.provider.len()
                    + reasoning.item_id.as_ref().map_or(0, String::len)
                    + reasoning.signature.as_ref().map_or(0, String::len)
                    + reasoning.encrypted.as_ref().map_or(0, String::len)
                    + reasoning.display_text().map_or(0, |text| text.len())
            }
            // `ContentPart` is `#[non_exhaustive]`, so this arm is required
            // from outside `everruns-core`. Unreachable in-workspace: every
            // crate here compiles against the same core version.
            _ => 0,
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

fn take_existing_excluded_notice(messages: &mut Vec<RuntimeMessage>) -> usize {
    let Some(first) = messages.first() else {
        return 0;
    };
    let Some(count) = parse_excluded_notice_count(first) else {
        return 0;
    };

    messages.remove(0);
    count
}

fn parse_excluded_notice_count(message: &RuntimeMessage) -> Option<usize> {
    let text = message.text()?;
    let rest = text.strip_prefix('[')?;
    let (count, rest) = rest.split_once(' ')?;
    if !rest.starts_with("earlier messages are not in this context.") {
        return None;
    }
    count.parse().ok()
}

/// Insert the hidden-history notice at `position`, clamped to the message list.
fn insert_excluded_notice(messages: &mut Vec<RuntimeMessage>, position: usize, count: usize) {
    let text = ExcludedNoticeTransform::infinity_context()
        .format
        .replace("{}", &count.to_string());
    messages.insert(position.min(messages.len()), RuntimeMessage::system(text));
}

/// Tool for querying earlier conversation history.
pub struct QueryHistoryTool;

#[derive(Debug, Deserialize)]
struct QueryHistoryParams {
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    message_id: Option<crate::typed_id::MessageId>,
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    offset: usize,
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
        tool_call: &crate::builtins::tool_types::ToolCall,
        phase: crate::builtins::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::builtins::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::builtins::tool_narration::narrate_query_history(
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
                "message_id": {"type": "string", "description": "Retrieve a matching message by the ID returned by search"},
                "tool_call_id": {"type": "string", "description": "Retrieve the full result of an earlier tool call, such as one hidden to save context"},
                "tool_name": {"type": "string", "description": "With tool_call_id: the tool's name, which narrows the lookup"},
                "offset": {"type": "integer", "minimum": 0, "default": 0, "description": "With message_id or tool_call_id: character offset to continue a long result from"},
                "query": {
                    "type": "string",
                    "description": "Search earlier messages with a distinctive word or exact phrase. Results show matching excerpts; use message_id to retrieve the original message."
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

        let limit = params.limit.clamp(1, 50);
        if let Some(id) = params.message_id {
            return match retriever.get(context.session_id, id).await {
                Ok(Some(message)) => message_page(&message, params.offset),
                Ok(None) => ToolExecutionResult::tool_error("Message not found in this session"),
                Err(error) => ToolExecutionResult::internal_error(error),
            };
        }
        if let Some(tool_call_id) = params.tool_call_id.as_deref() {
            return match find_tool_result(
                retriever.as_ref(),
                context.session_id,
                tool_call_id,
                params.tool_name.as_deref(),
            )
            .await
            {
                Ok(Some(message)) => message_page(&message, params.offset),
                Ok(None) => ToolExecutionResult::tool_error(
                    "No result for that tool call among recent tool results; search with query instead",
                ),
                Err(error) => ToolExecutionResult::internal_error(error),
            };
        }
        let mut query = MessageQuery::new(context.session_id).with_limit(limit as i64);
        if let Some(range) = params.message_range.as_ref() {
            query.oldest_first = true;
            query.offset = Some(range.from.min(i64::MAX as usize) as i64);
            query.limit = Some(range.to.saturating_sub(range.from).min(limit) as i64);
        } else if let Some(search) = params.query.as_ref() {
            query
                .filters
                .push(crate::message_filter::MessageFilter::Search(search.clone()));
        }
        let messages = match retriever.load_filtered(query).await {
            Ok(messages) => messages,
            Err(error) => return ToolExecutionResult::internal_error(error),
        };
        if let Some(range) = params.message_range {
            return format_range_result(&messages.iter().collect::<Vec<_>>(), range.from, 0);
        }
        if let Some(query) = params.query.as_deref() {
            let results = search_messages(&messages, query, limit);
            return format_search_result(&results, 0);
        }
        if messages.is_empty() {
            return ToolExecutionResult::success(
                json!({"count": 0, "message": "No history available."}),
            );
        }
        let formatted: Vec<_> = messages
            .iter()
            .map(|message| {
                let mut value = format_message(message, 0, 0);
                if let Some(object) = value.as_object_mut() {
                    object.remove("index");
                }
                value
            })
            .collect();
        ToolExecutionResult::success(json!({"messages": formatted, "count": formatted.len()}))
    }
}

struct SearchResult<'a> {
    index: usize,
    message: &'a RuntimeMessage,
    score: f64,
    match_start_chars: usize,
}

fn search_messages<'a>(
    messages: &'a [RuntimeMessage],
    query: &str,
    limit: usize,
) -> Vec<SearchResult<'a>> {
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for (index, message) in messages.iter().enumerate() {
        let original = extract_text_content(message);
        let content = original.to_lowercase();
        let Some(match_byte) = content.find(&query_lower) else {
            continue;
        };
        // Lowercasing can expand a character (e.g. İ). Map the folded byte
        // position back to original characters before cutting a UTF-8 excerpt.
        let mut folded_bytes = 0;
        let match_start_chars = original
            .chars()
            .position(|c| {
                folded_bytes += c.to_lowercase().map(char::len_utf8).sum::<usize>();
                folded_bytes > match_byte
            })
            .unwrap_or(0);

        let mut score = 1.0;

        if content.split_whitespace().any(|word| word == query_lower) {
            score += 0.5;
        }

        if !messages.is_empty() {
            score += (index as f64 / messages.len() as f64) * 0.3;
        }

        match message.role {
            RuntimeMessageRole::User | RuntimeMessageRole::Agent => score += 0.2,
            RuntimeMessageRole::System => score += 0.1,
            RuntimeMessageRole::ToolResult => {}
        }

        results.push(SearchResult {
            index,
            message,
            score,
            match_start_chars,
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

fn extract_text_content(message: &RuntimeMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(text) => Some(text.text.clone()),
            ContentPart::ToolResult(result) => result.result.as_ref().map(ToString::to_string),
            ContentPart::ToolCall(call) => Some(format!(
                "Tool call: {} with arguments: {}",
                call.name, call.arguments
            )),
            other => crate::attachment_labels::runtime_attachment_label(other),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Characters of one message returned per `query_history` read.
const MESSAGE_PAGE_CHARS: usize = 16_000;

/// Tool results scanned for a `tool_call_id` lookup, newest first. Stores have
/// no tool-call-id index; a masked result is recent, so a bounded scan finds it.
const TOOL_RESULT_SCAN: i64 = 500;

/// One page of a message's text, with where the next page starts.
fn message_page(message: &RuntimeMessage, offset: usize) -> ToolExecutionResult {
    let text = extract_text_content(message);
    let total = text.chars().count();
    let content: String = text.chars().skip(offset).take(MESSAGE_PAGE_CHARS).collect();
    let end = offset.saturating_add(MESSAGE_PAGE_CHARS).min(total);
    let mut value = json!({
        "message_id": message.id,
        "role": message.role.to_string(),
        "content": content,
        "truncated": end < total,
        "total_chars": total,
    });
    if end < total {
        value["next_offset"] = json!(end);
    }
    ToolExecutionResult::success(value)
}

async fn find_tool_result(
    retriever: &dyn crate::message_retriever::MessageRetriever,
    session_id: crate::typed_id::SessionId,
    tool_call_id: &str,
    tool_name: Option<&str>,
) -> crate::error::Result<Option<RuntimeMessage>> {
    let mut query = MessageQuery::new(session_id).with_limit(TOOL_RESULT_SCAN);
    query
        .filters
        .push(crate::message_filter::MessageFilter::EventTypes(vec![
            "tool.completed".to_string(),
        ]));
    if let Some(name) = tool_name {
        query
            .filters
            .push(crate::message_filter::MessageFilter::ToolName(
                name.to_string(),
            ));
    }
    let messages = retriever.load_filtered(query).await?;
    Ok(messages.into_iter().find(|message| {
        message.content.iter().any(|part| {
            matches!(part, ContentPart::ToolResult(result) if result.tool_call_id == tool_call_id)
        })
    }))
}

fn truncate_content(content: &str, max_len: usize) -> String {
    let char_count = content.chars().count();
    if char_count <= max_len {
        return content.to_string();
    }

    format!("{}...", content.chars().take(max_len).collect::<String>())
}

fn format_message(message: &RuntimeMessage, index: usize, total: usize) -> Value {
    let mut value = json!({
        "message_id": message.id,
        "index": index,
        "position": format!("{}/{}", index + 1, total),
        "role": message.role.to_string(),
        "created_at": message.created_at.to_rfc3339(),
        "content": truncate_content(&extract_text_content(message), 500)
    });
    if total == 0
        && let Some(object) = value.as_object_mut()
    {
        object.remove("position");
    }
    value
}

fn format_range_result(
    messages: &[&RuntimeMessage],
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
            let content = extract_text_content(result.message);
            let characters = content.chars().count();
            let start = result
                .match_start_chars
                .saturating_sub(100)
                .min(characters.saturating_sub(500));
            let mut excerpt = content.chars().skip(start).take(500).collect::<String>();
            if start > 0 {
                excerpt.insert_str(0, "...");
            }
            if start.saturating_add(500) < characters {
                excerpt.push_str("...");
            }
            message["content"] = json!(excerpt);
            message["truncated"] = json!(characters > 500);
            message["content_offset_chars"] = json!(start);
            if total == 0
                && let Some(object) = message.as_object_mut()
            {
                object.remove("index");
            }
            message["relevance_score"] = json!(format!("{:.2}", result.score));
            message
        })
        .collect();

    ToolExecutionResult::success(json!({
        "messages": formatted,
        "count": results.len(),
    }))
}

#[cfg(test)]
#[path = "infinity_context/provider_managed_tests.rs"]
mod provider_managed_tests;

#[cfg(test)]
#[path = "infinity_context/tests.rs"]
mod tests;
