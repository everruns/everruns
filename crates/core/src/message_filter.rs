//! Message Filter Abstraction
//!
//! This module provides a composable filter system for message retrieval.
//! Capabilities can contribute filters that modify how messages are loaded,
//! enabling features like:
//! - Time-based filtering (from/to timestamps)
//! - Event type filtering
//! - Tool name filtering (for tool results)
//! - Full-text search
//! - Ephemeral message injection
//!
//! Design decisions:
//! - Filters are stackable: capabilities apply filters in priority order
//! - DB-mapped where possible: most filters translate to SQL for efficiency
//! - In-memory fallback: custom filters use Rust predicates
//! - Injection support: ephemeral messages can be added without persistence

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;

use crate::message::Message;
use crate::typed_id::{EventId, SessionId};

// ============================================================================
// MessageFilter - Filter specifications for message queries
// ============================================================================

/// Filter specification for message queries.
///
/// Each variant can be mapped to either:
/// - SQL WHERE clause (for PostgreSQL)
/// - Rust predicate (for in-memory filtering)
///
/// Filters are applied in order and combined with AND semantics.
#[derive(Clone)]
pub enum MessageFilter {
    /// Filter by time range (inclusive bounds)
    ///
    /// Maps to: `WHERE created_at >= $from AND created_at <= $to`
    TimeRange {
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    },

    /// Filter by event types (whitelist)
    ///
    /// Maps to: `WHERE event_type = ANY($types)`
    /// Default types if not specified: input.message, output.message.completed, tool.completed
    EventTypes(Vec<String>),

    /// Filter tool results by tool name
    ///
    /// Maps to: `WHERE event_type = 'tool.completed' AND data->>'tool_name' = $name`
    ToolName(String),

    /// Full-text search in message content
    ///
    /// Maps to: `WHERE search_vector @@ plainto_tsquery('english', $query)`
    /// Uses a GIN-indexed tsvector generated column on data->>'content'.
    Search(String),

    /// Exclude specific event IDs
    ///
    /// Maps to: `WHERE id != ALL($ids)`
    ExcludeIds(Vec<EventId>),

    /// Include only specific event IDs
    ///
    /// Maps to: `WHERE id = ANY($ids)`
    IncludeIds(Vec<EventId>),

    /// Custom predicate (in-memory only)
    ///
    /// Use sparingly - this filter cannot be pushed to the database.
    /// For complex filtering that can't be expressed in SQL.
    Custom(Arc<dyn Fn(&Message) -> bool + Send + Sync>),
}

impl fmt::Debug for MessageFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeRange { from, to } => f
                .debug_struct("TimeRange")
                .field("from", from)
                .field("to", to)
                .finish(),
            Self::EventTypes(types) => f.debug_tuple("EventTypes").field(types).finish(),
            Self::ToolName(name) => f.debug_tuple("ToolName").field(name).finish(),
            Self::Search(query) => f.debug_tuple("Search").field(query).finish(),
            Self::ExcludeIds(ids) => f.debug_tuple("ExcludeIds").field(ids).finish(),
            Self::IncludeIds(ids) => f.debug_tuple("IncludeIds").field(ids).finish(),
            Self::Custom(_) => f.debug_tuple("Custom").field(&"<fn>").finish(),
        }
    }
}

// ============================================================================
// InjectedMessage - Ephemeral messages to add to results
// ============================================================================

/// Position for injecting ephemeral messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InjectionPosition {
    /// Insert at the beginning (before all messages)
    Start,
    /// Insert at the end (after all messages)
    End,
    /// Insert before the message at the given index
    BeforeIndex(usize),
    /// Insert after the message at the given index
    AfterIndex(usize),
}

/// An ephemeral message to inject into the result set.
///
/// Injected messages are not persisted - they're added during retrieval
/// for context augmentation purposes (e.g., summaries, system reminders).
#[derive(Debug, Clone)]
pub struct InjectedMessage {
    /// Where to insert this message
    pub position: InjectionPosition,
    /// The message to inject
    pub message: Message,
}

impl InjectedMessage {
    /// Create an injection at the start of the message list
    pub fn at_start(message: Message) -> Self {
        Self {
            position: InjectionPosition::Start,
            message,
        }
    }

    /// Create an injection at the end of the message list
    pub fn at_end(message: Message) -> Self {
        Self {
            position: InjectionPosition::End,
            message,
        }
    }

    /// Create an injection before a specific index
    pub fn before_index(index: usize, message: Message) -> Self {
        Self {
            position: InjectionPosition::BeforeIndex(index),
            message,
        }
    }

    /// Create an injection after a specific index
    pub fn after_index(index: usize, message: Message) -> Self {
        Self {
            position: InjectionPosition::AfterIndex(index),
            message,
        }
    }
}

// ============================================================================
// PrependTransform - Dynamic message prepending based on filter results
// ============================================================================

/// Context passed to prepend transform functions
#[derive(Debug, Clone)]
pub struct FilterContext {
    /// Total messages before any filtering
    pub total_count: usize,
    /// Messages remaining after filtering
    pub filtered_count: usize,
    /// Number of messages excluded by filters/limit
    pub excluded_count: usize,
}

/// A transform that can prepend a message based on filter results.
///
/// This enables dynamic message injection that depends on how many messages
/// were filtered out, useful for "N messages hidden" notices.
pub trait PrependTransform: Send + Sync {
    /// Optionally generate a message to prepend based on filter context.
    ///
    /// Return `Some(message)` to prepend, or `None` to skip.
    fn transform(&self, ctx: &FilterContext) -> Option<Message>;
}

/// Simple implementation that prepends a message when messages were excluded
pub struct ExcludedNoticeTransform {
    /// Format string with {} placeholder for excluded count
    pub format: String,
}

impl ExcludedNoticeTransform {
    /// Create a new excluded notice transform with the given format string.
    ///
    /// The format should contain `{}` which will be replaced with the excluded count.
    pub fn new(format: impl Into<String>) -> Self {
        Self {
            format: format.into(),
        }
    }

    /// Default format for infinity context
    pub fn infinity_context() -> Self {
        Self::new(
            "[IMPORTANT: {} earlier messages are NOT visible in this context. \
To answer questions about earlier parts of the conversation, \
you MUST call the `query_history` tool to search for the relevant information.]",
        )
    }
}

impl PrependTransform for ExcludedNoticeTransform {
    fn transform(&self, ctx: &FilterContext) -> Option<Message> {
        if ctx.excluded_count > 0 {
            let text = self.format.replace("{}", &ctx.excluded_count.to_string());
            Some(Message::system(&text))
        } else {
            None
        }
    }
}

// ============================================================================
// MessageQuery - Query specification for message retrieval
// ============================================================================

/// Query specification for message retrieval with filters and injections.
///
/// This is the main interface for filtered message retrieval. Capabilities
/// contribute to this query through `MessageFilterProvider`.
///
/// # Example
///
/// ```
/// use everruns_core::message_filter::{MessageQuery, MessageFilter};
/// use everruns_provider::typed_id::SessionId;
/// use chrono::Utc;
///
/// let query = MessageQuery::new(SessionId::new())
///     .with_filter(MessageFilter::TimeRange {
///         from: Some(Utc::now() - chrono::Duration::hours(24)),
///         to: None,
///     })
///     .with_limit(100);
/// ```
#[derive(Clone)]
pub struct MessageQuery {
    /// Session to load messages from
    pub session_id: SessionId,

    /// Filters to apply (combined with AND)
    pub filters: Vec<MessageFilter>,

    /// Ephemeral messages to inject after loading
    pub injections: Vec<InjectedMessage>,

    /// Maximum number of messages to return
    pub limit: Option<i64>,

    /// Number of messages to skip
    pub offset: Option<i64>,

    /// Number of leading (oldest) messages to always retain as a head anchor,
    /// in addition to the latest `limit` tail.
    ///
    /// `limit` alone fetches a tail-only window (latest N), so for histories
    /// longer than that window the genuine first message is never loaded and any
    /// head anchor silently degrades to the oldest *loaded* message. When
    /// `keep_head` is set, the head and tail are fetched together (de-duplicated
    /// on overlap), so the original task/goal survives no matter how long the
    /// history grows. Used by `infinity_context`'s `keep_first_messages`.
    pub keep_head: Option<usize>,

    /// Internal event cursor used to load the raw suffix after a checkpoint.
    pub after_sequence: Option<i64>,

    /// Optional transform to prepend a message based on filter results.
    /// Applied after filtering, receives context about excluded messages.
    pub prepend_transform: Option<Arc<dyn PrependTransform>>,
}

impl Default for MessageQuery {
    fn default() -> Self {
        Self {
            session_id: SessionId::from_seed(0),
            filters: Vec::new(),
            injections: Vec::new(),
            limit: None,
            offset: None,
            keep_head: None,
            after_sequence: None,
            prepend_transform: None,
        }
    }
}

impl std::fmt::Debug for MessageQuery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageQuery")
            .field("session_id", &self.session_id)
            .field("filters", &self.filters)
            .field("injections", &self.injections)
            .field("limit", &self.limit)
            .field("offset", &self.offset)
            .field("keep_head", &self.keep_head)
            .field("after_sequence", &self.after_sequence)
            .field("prepend_transform", &self.prepend_transform.is_some())
            .finish()
    }
}

impl MessageQuery {
    /// Create a new query for a session
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            filters: Vec::new(),
            injections: Vec::new(),
            limit: None,
            offset: None,
            keep_head: None,
            after_sequence: None,
            prepend_transform: None,
        }
    }

    /// Set a prepend transform to dynamically add a message based on filter results.
    ///
    /// The transform receives filter context (total, filtered, excluded counts) and
    /// can optionally return a message to prepend.
    pub fn with_prepend_transform(mut self, transform: Arc<dyn PrependTransform>) -> Self {
        self.prepend_transform = Some(transform);
        self
    }

    /// Add a filter to the query
    pub fn with_filter(mut self, filter: MessageFilter) -> Self {
        self.filters.push(filter);
        self
    }

    /// Add multiple filters to the query
    pub fn with_filters(mut self, filters: impl IntoIterator<Item = MessageFilter>) -> Self {
        self.filters.extend(filters);
        self
    }

    /// Add an injection to the query
    pub fn with_injection(mut self, injection: InjectedMessage) -> Self {
        self.injections.push(injection);
        self
    }

    /// Add multiple injections to the query
    pub fn with_injections(
        mut self,
        injections: impl IntoIterator<Item = InjectedMessage>,
    ) -> Self {
        self.injections.extend(injections);
        self
    }

    /// Set the maximum number of messages to return
    pub fn with_limit(mut self, limit: i64) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set the number of messages to skip
    pub fn with_offset(mut self, offset: i64) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Set the head-anchor size: always retain the first `keep_head` messages in
    /// addition to the latest `limit` tail. See [`MessageQuery::keep_head`].
    pub fn with_keep_head(mut self, keep_head: usize) -> Self {
        self.keep_head = Some(keep_head);
        self
    }

    pub fn after_sequence(mut self, sequence: i64) -> Self {
        self.after_sequence = Some(sequence);
        self
    }

    /// Check if this query has any DB-mappable filters
    pub fn has_db_filters(&self) -> bool {
        self.filters
            .iter()
            .any(|f| !matches!(f, MessageFilter::Custom(_)))
    }

    /// Check if this query has any in-memory-only filters (Custom predicates)
    pub fn has_custom_filters(&self) -> bool {
        self.filters
            .iter()
            .any(|f| matches!(f, MessageFilter::Custom(_)))
    }

    /// Check if this query has any injections
    pub fn has_injections(&self) -> bool {
        !self.injections.is_empty()
    }

    /// Apply injections to a message list (in-place)
    ///
    /// Injections are applied in order. Note that indices may shift
    /// as messages are inserted.
    pub fn apply_injections(&self, messages: &mut Vec<Message>) {
        // Sort injections by position to handle index shifts correctly
        // Process End injections last, Start first, then indices in reverse order
        let mut start_injections = Vec::new();
        let mut end_injections = Vec::new();
        let mut index_injections: Vec<_> = Vec::new();

        for inj in &self.injections {
            match &inj.position {
                InjectionPosition::Start => start_injections.push(inj.message.clone()),
                InjectionPosition::End => end_injections.push(inj.message.clone()),
                InjectionPosition::BeforeIndex(idx) => {
                    index_injections.push((*idx, true, inj.message.clone()))
                }
                InjectionPosition::AfterIndex(idx) => {
                    index_injections.push((*idx, false, inj.message.clone()))
                }
            }
        }

        // Insert start injections (in reverse order to maintain original order)
        for msg in start_injections.into_iter().rev() {
            messages.insert(0, msg);
        }

        // Sort index injections by index (descending) to avoid index shifts affecting later insertions
        index_injections.sort_by_key(|entry| std::cmp::Reverse(entry.0));

        for (idx, is_before, msg) in index_injections {
            let insert_idx = if is_before {
                idx.min(messages.len())
            } else {
                idx.saturating_add(1).min(messages.len())
            };
            messages.insert(insert_idx, msg);
        }

        // Insert end injections
        messages.extend(end_injections);
    }

    /// Apply offset and latest-message limiting.
    ///
    /// `limit` means "keep the latest N messages" while preserving chronological
    /// order in the returned slice. This is the prompt-window behavior expected
    /// by long-context capabilities such as `infinity_context`.
    ///
    /// When `keep_head` is set, the first `keep_head` messages are additionally
    /// retained as an anchor: the kept set is `[first keep_head] + [latest limit]`
    /// with the middle dropped, de-duplicated when the two windows overlap. This
    /// mirrors the head+tail load performed by the storage backends so the
    /// original task/goal survives in histories longer than the tail window.
    pub fn apply_window_bounds(&self, messages: &mut Vec<Message>) {
        if let Some(offset) = self.offset {
            let offset = offset.max(0) as usize;
            if offset < messages.len() {
                messages.drain(0..offset);
            } else {
                messages.clear();
            }
        }

        if let Some(limit) = self.limit {
            let limit = limit.max(0) as usize;
            let keep_head = self.keep_head.unwrap_or(0).min(messages.len());
            // Drop the middle `[keep_head, len - limit)`, keeping the head anchor
            // and the latest `limit`. When the windows overlap (keep_head + limit
            // >= len) nothing is dropped and no message is duplicated.
            if messages.len() > keep_head + limit {
                let drain_end = messages.len() - limit;
                messages.drain(keep_head..drain_end);
            }
        }
    }

    /// Prepend the dynamic hidden-history notice, if configured.
    pub fn prepend_excluded_notice(&self, messages: &mut Vec<Message>, count_before_limit: usize) {
        if let Some(ref transform) = self.prepend_transform {
            let ctx = FilterContext {
                total_count: count_before_limit,
                filtered_count: messages.len(),
                excluded_count: count_before_limit.saturating_sub(messages.len()),
            };
            if let Some(prepend_msg) = transform.transform(&ctx) {
                messages.insert(0, prepend_msg);
            }
        }
    }

    /// Apply offset, latest-message limiting, and optional prepend notice.
    pub fn apply_windowing(&self, messages: &mut Vec<Message>) {
        let count_before_limit = messages.len();
        self.apply_window_bounds(messages);
        self.prepend_excluded_notice(messages, count_before_limit);
    }
}

// ============================================================================
// AnchoredWindow - "protect head + tail, drop the middle" selection
// ============================================================================

/// Which messages survive anchored token-budget trimming.
///
/// Kept messages are `[0, head_len)` (the anchor: the system goal / original
/// task) followed by `[recent_start, len)` (recent state). Messages in
/// `[head_len, recent_start)` — the middle — are dropped. [`Self::hidden`]
/// reports how many.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchoredWindow {
    /// Number of leading messages kept as the anchor.
    pub head_len: usize,
    /// Index where the contiguous recent block begins.
    pub recent_start: usize,
}

impl AnchoredWindow {
    /// Count of messages dropped from the middle.
    pub fn hidden(&self) -> usize {
        self.recent_start.saturating_sub(self.head_len)
    }
}

/// Select an anchored window over `costs.len()` messages.
///
/// Always preserves the first `keep_head` messages (the conversation's goal) and
/// the last `min_tail` messages (recent state), even if their combined token
/// `cost` exceeds `budget`. The recent block then grows backward toward the head
/// while the running total stays within `budget` and the tail stays within
/// `max_tail` (when set).
///
/// This is the "protect head + tail, drop the middle" policy. It keeps a single
/// contiguous recent block so tool-call/result adjacency and conversational flow
/// are preserved, leaving at most one gap (between the anchor and the recent
/// block). Anchoring the head is the conversational analog of StreamingLLM's
/// attention sinks and the "lost in the middle" finding: the first message (the
/// task/goal) is the one eviction you never want.
///
/// `max_tail` is a hard cap on the recent block and bounds `min_tail`; the head
/// anchor is additional to it.
pub fn anchored_window(
    costs: &[usize],
    keep_head: usize,
    min_tail: usize,
    max_tail: Option<usize>,
    budget: usize,
) -> AnchoredWindow {
    let len = costs.len();
    let keep_head = keep_head.min(len);

    // A hard recent cap also bounds the guaranteed tail.
    let mut min_tail = min_tail.min(len);
    if let Some(max_tail) = max_tail {
        min_tail = min_tail.min(max_tail.max(1));
    }

    let mut recent_start = len - min_tail;
    if recent_start <= keep_head {
        // Head and tail meet or overlap: keep everything, no middle to drop.
        return AnchoredWindow {
            head_len: keep_head,
            recent_start: keep_head,
        };
    }

    let head_cost: usize = costs[..keep_head].iter().sum();
    let mut window_cost: usize = head_cost + costs[recent_start..].iter().sum::<usize>();
    let mut tail_count = min_tail;

    // Grow the recent block backward (newest-first) while it fits both the token
    // budget and the optional hard recent cap. Head + min_tail are guaranteed
    // even when already over budget.
    while recent_start > keep_head {
        if let Some(max_tail) = max_tail
            && tail_count >= max_tail
        {
            break;
        }
        let next = recent_start - 1;
        let next_cost = costs[next];
        if window_cost + next_cost > budget {
            break;
        }
        recent_start = next;
        window_cost += next_cost;
        tail_count += 1;
    }

    AnchoredWindow {
        head_len: keep_head,
        recent_start,
    }
}

// ============================================================================
// MessageFilterProvider - Trait for capabilities to contribute filters
// ============================================================================

/// Trait for capabilities to contribute message filters.
///
/// Implement this trait to modify how messages are retrieved for sessions
/// using your capability. Filters are applied in capability priority order.
///
/// # Example
///
/// ```ignore
/// use everruns_core::message_filter::{MessageFilterProvider, MessageQuery, MessageFilter};
///
/// struct RecentMessagesProvider;
///
/// impl MessageFilterProvider for RecentMessagesProvider {
///     fn apply_filters(&self, query: &mut MessageQuery, config: &serde_json::Value) {
///         // Only load messages from the last 24 hours
///         let cutoff = chrono::Utc::now() - chrono::Duration::hours(24);
///         query.filters.push(MessageFilter::TimeRange {
///             from: Some(cutoff),
///             to: None,
///         });
///     }
/// }
/// ```
pub trait MessageFilterProvider: Send + Sync {
    /// Modify the message query by adding filters and/or injections.
    ///
    /// # Arguments
    ///
    /// * `query` - The query to modify (add filters, injections, etc.)
    /// * `config` - Per-agent capability configuration from the database
    fn apply_filters(&self, query: &mut MessageQuery, config: &serde_json::Value);

    /// Post-load transform: inspect and optionally modify loaded messages.
    /// Called after messages are loaded and query filters/injections applied.
    /// Default is no-op.
    fn post_load(&self, messages: &mut Vec<Message>, config: &serde_json::Value) {
        let _ = (messages, config);
    }

    /// Priority for filter application (lower = earlier).
    ///
    /// Filters from lower-priority providers are applied first.
    /// Default is 0.
    fn priority(&self) -> i32 {
        0
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::MessageRole;

    fn messages() -> Vec<Message> {
        ["goal", "first", "second", "third", "latest"]
            .into_iter()
            .map(Message::user)
            .collect()
    }

    fn assert_messages(actual: &[Message], expected: &[Message]) {
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    #[test]
    fn query_builder_preserves_filter_and_window_settings() {
        let session = SessionId::new();
        let query = MessageQuery::new(session)
            .with_filter(MessageFilter::Search("needle".into()))
            .with_filters([MessageFilter::EventTypes(vec!["input.message".into()])])
            .with_limit(50)
            .with_offset(10)
            .with_keep_head(2)
            .after_sequence(7);
        assert_eq!(query.session_id, session);
        assert!(
            matches!(&query.filters[..], [MessageFilter::Search(s), MessageFilter::EventTypes(k)]
            if s == "needle" && k == &["input.message"])
        );
        assert_eq!(
            (
                query.limit,
                query.offset,
                query.keep_head,
                query.after_sequence
            ),
            (Some(50), Some(10), Some(2), Some(7))
        );
    }

    #[test]
    fn filter_classification_handles_empty_db_custom_and_mixed_queries() {
        let session = SessionId::new();
        let empty = MessageQuery::new(session);
        assert!(!empty.has_db_filters());
        assert!(!empty.has_custom_filters());
        let custom = MessageFilter::Custom(Arc::new(|m| m.role == MessageRole::User));
        let query = empty.clone().with_filter(custom.clone());
        assert!(!query.has_db_filters());
        assert!(query.has_custom_filters());
        for filter in [
            MessageFilter::Search("hello".into()),
            MessageFilter::EventTypes(vec!["input.message".into()]),
            MessageFilter::ToolName("lookup".into()),
            MessageFilter::ExcludeIds(vec![EventId::new()]),
            MessageFilter::IncludeIds(vec![EventId::new()]),
            MessageFilter::TimeRange {
                from: Some(Utc::now()),
                to: None,
            },
        ] {
            let db = empty.clone().with_filter(filter);
            assert!(db.has_db_filters());
            assert!(!db.has_custom_filters());
            let mixed = db.with_filter(custom.clone());
            assert!(mixed.has_db_filters());
            assert!(mixed.has_custom_filters());
        }
    }

    #[test]
    fn injections_preserve_messages_at_each_position_and_clamp_large_indices() {
        for original in [vec![], messages()] {
            let len = original.len();
            for (position, index) in [
                (InjectionPosition::Start, 0),
                (InjectionPosition::End, len),
                (InjectionPosition::BeforeIndex(1), 1.min(len)),
                (InjectionPosition::AfterIndex(1), 2.min(len)),
                (InjectionPosition::BeforeIndex(10), len),
                (InjectionPosition::AfterIndex(10), len),
                (InjectionPosition::BeforeIndex(usize::MAX), len),
                (InjectionPosition::AfterIndex(usize::MAX), len),
            ] {
                let inserted = Message::system("injected");
                let query = MessageQuery::new(SessionId::new()).with_injection(InjectedMessage {
                    position,
                    message: inserted.clone(),
                });
                assert!(query.has_injections());
                let mut actual = original.clone();
                query.apply_injections(&mut actual);
                let mut expected = original.clone();
                expected.insert(index, inserted);
                assert_messages(&actual, &expected);
            }
        }
    }

    #[test]
    fn start_and_end_injections_preserve_declared_order() {
        let injected: Vec<_> = ["start one", "start two", "end one", "end two"]
            .into_iter()
            .map(Message::system)
            .collect();
        let query = MessageQuery::new(SessionId::new())
            .with_injection(InjectedMessage::at_start(injected[0].clone()))
            .with_injections([
                InjectedMessage::at_start(injected[1].clone()),
                InjectedMessage::at_end(injected[2].clone()),
                InjectedMessage::at_end(injected[3].clone()),
            ]);
        for original in [vec![], messages()] {
            let expected = [
                injected[..2].to_vec(),
                original.clone(),
                injected[2..].to_vec(),
            ]
            .concat();
            let mut actual = original;
            query.apply_injections(&mut actual);
            assert_messages(&actual, &expected);
        }
    }

    #[test]
    fn indexed_injections_do_not_shift_other_original_targets() {
        let original = messages();
        let before = Message::system("before first");
        let after = Message::system("after third");
        for injections in [
            vec![
                InjectedMessage::before_index(1, before.clone()),
                InjectedMessage::after_index(3, after.clone()),
            ],
            vec![
                InjectedMessage::after_index(3, after.clone()),
                InjectedMessage::before_index(1, before.clone()),
            ],
        ] {
            let query = MessageQuery::new(SessionId::new()).with_injections(injections);
            let mut actual = original.clone();
            query.apply_injections(&mut actual);
            assert_messages(
                &actual,
                &[
                    original[0].clone(),
                    before.clone(),
                    original[1].clone(),
                    original[2].clone(),
                    original[3].clone(),
                    after.clone(),
                    original[4].clone(),
                ],
            );
        }
    }

    #[test]
    fn window_bounds_preserve_exact_head_and_tail_after_offset() {
        let original = messages();
        type WindowCase = (Option<i64>, Option<i64>, Option<usize>, &'static [usize]);
        let cases: &[WindowCase] = &[
            (None, None, None, &[0, 1, 2, 3, 4]),
            (None, Some(2), None, &[3, 4]),
            (None, Some(2), Some(0), &[3, 4]),
            (None, Some(2), Some(1), &[0, 3, 4]),
            (None, Some(3), Some(3), &[0, 1, 2, 3, 4]),
            (None, Some(1), Some(10), &[0, 1, 2, 3, 4]),
            (Some(1), Some(1), Some(1), &[1, 4]),
            (Some(-1), Some(2), None, &[3, 4]),
            (Some(5), Some(2), Some(1), &[]),
            (Some(9), None, None, &[]),
            (None, Some(0), None, &[]),
            (None, Some(-1), None, &[]),
            (None, Some(0), Some(1), &[0]),
            (None, None, Some(1), &[0, 1, 2, 3, 4]),
        ];
        for &(offset, limit, keep_head, indices) in cases {
            let query = MessageQuery {
                offset,
                limit,
                keep_head,
                ..MessageQuery::new(SessionId::new())
            };
            let mut actual = original.clone();
            query.apply_window_bounds(&mut actual);
            let expected: Vec<_> = indices.iter().map(|&i| original[i].clone()).collect();
            assert_messages(&actual, &expected);
        }
        let mut unchanged = original.clone();
        let default = MessageQuery::default();
        assert!(!default.has_injections());
        default.apply_windowing(&mut unchanged);
        default.apply_injections(&mut unchanged);
        assert_messages(&unchanged, &original);
    }

    #[test]
    fn windowing_notice_counts_offset_and_limit_without_losing_retained_messages() {
        let original = messages();
        let query = MessageQuery::new(SessionId::new())
            .with_offset(1)
            .with_limit(2)
            .with_prepend_transform(Arc::new(ExcludedNoticeTransform::new("{} hidden")));
        let mut actual = original.clone();
        query.apply_windowing(&mut actual);
        assert_eq!(actual[0].role, MessageRole::System);
        assert_eq!(actual[0].text(), Some("3 hidden"));
        assert_messages(&actual[1..], &original[3..]);
        let no_exclusion = MessageQuery::new(SessionId::new())
            .with_prepend_transform(Arc::new(ExcludedNoticeTransform::new("{} hidden")));
        let mut unchanged = original.clone();
        no_exclusion.apply_windowing(&mut unchanged);
        assert_messages(&unchanged, &original);
    }

    #[test]
    fn anchored_window_respects_exact_budget_and_contiguous_recent_block() {
        let costs = [5, 11, 7, 3, 2];
        for (budget, recent_start) in [(0, 4), (9, 4), (10, 3), (16, 3), (17, 2), (28, 1), (100, 1)]
        {
            let window = anchored_window(&costs, 1, 1, None, budget);
            assert_eq!(
                window,
                AnchoredWindow {
                    head_len: 1,
                    recent_start
                }
            );
            assert_eq!(window.hidden(), recent_start - 1);
        }
    }

    #[test]
    fn anchored_window_caps_recent_tail_and_preserves_over_budget_anchors() {
        let costs = [100, 11, 7, 3, 100];
        for (minimum, maximum, budget, recent_start) in [
            (1, None, 0, 4),
            (2, None, 1, 3),
            (5, Some(2), 1000, 3),
            (1, Some(2), 1000, 3),
            (0, Some(0), 0, 5),
        ] {
            assert_eq!(
                anchored_window(&costs, 1, minimum, maximum, budget),
                AnchoredWindow {
                    head_len: 1,
                    recent_start
                }
            );
        }
    }

    #[test]
    fn anchored_window_handles_empty_and_overlapping_anchors() {
        for (costs, head, tail, expected_head) in [
            (&[][..], 1, 2, 0),
            (&[10, 10, 10][..], 1, 10, 1),
            (&[10, 10, 10][..], 9, 1, 3),
            (&[10, 10, 10][..], 1, 2, 1),
        ] {
            let window = anchored_window(costs, head, tail, None, 1);
            assert_eq!(
                window,
                AnchoredWindow {
                    head_len: expected_head,
                    recent_start: expected_head
                }
            );
            assert_eq!(window.hidden(), 0);
        }
    }
}
