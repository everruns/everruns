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
                (idx + 1).min(messages.len())
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
    use crate::message::Message;
    use uuid::Uuid;

    #[test]
    fn test_message_query_builder() {
        let session_id: SessionId = Uuid::now_v7().into();
        let query = MessageQuery::new(session_id)
            .with_filter(MessageFilter::EventTypes(vec!["input.message".to_string()]))
            .with_limit(50)
            .with_offset(10);

        assert_eq!(query.session_id, session_id);
        assert_eq!(query.filters.len(), 1);
        assert_eq!(query.limit, Some(50));
        assert_eq!(query.offset, Some(10));
    }

    #[test]
    fn test_message_query_has_filters() {
        let session_id: SessionId = Uuid::now_v7().into();

        let query_with_db_filter =
            MessageQuery::new(session_id).with_filter(MessageFilter::Search("hello".to_string()));
        assert!(query_with_db_filter.has_db_filters());
        assert!(!query_with_db_filter.has_custom_filters());

        let query_with_custom =
            MessageQuery::new(session_id).with_filter(MessageFilter::Custom(Arc::new(|_| true)));
        assert!(!query_with_custom.has_db_filters());
        assert!(query_with_custom.has_custom_filters());
    }

    #[test]
    fn test_injection_at_start() {
        let session_id: SessionId = Uuid::now_v7().into();
        let injected = Message::system("Injected at start");

        let query = MessageQuery::new(session_id)
            .with_injection(InjectedMessage::at_start(injected.clone()));

        let mut messages = vec![Message::user("First"), Message::user("Second")];

        query.apply_injections(&mut messages);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].text(), Some("Injected at start"));
        assert_eq!(messages[1].text(), Some("First"));
    }

    #[test]
    fn test_injection_at_end() {
        let session_id: SessionId = Uuid::now_v7().into();
        let injected = Message::system("Injected at end");

        let query =
            MessageQuery::new(session_id).with_injection(InjectedMessage::at_end(injected.clone()));

        let mut messages = vec![Message::user("First"), Message::user("Second")];

        query.apply_injections(&mut messages);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].text(), Some("Injected at end"));
    }

    #[test]
    fn test_apply_windowing_keeps_latest_messages_chronological() {
        let session_id: SessionId = Uuid::now_v7().into();
        let query = MessageQuery::new(session_id).with_limit(2);
        let mut messages = vec![
            Message::user("oldest"),
            Message::user("middle"),
            Message::user("newest"),
        ];

        query.apply_windowing(&mut messages);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text(), Some("middle"));
        assert_eq!(messages[1].text(), Some("newest"));
    }

    #[test]
    fn test_apply_window_bounds_keep_head_anchors_first_messages() {
        let session_id: SessionId = Uuid::now_v7().into();
        let query = MessageQuery::new(session_id)
            .with_limit(2)
            .with_keep_head(1);
        let mut messages = vec![
            Message::user("goal"),
            Message::user("m1"),
            Message::user("m2"),
            Message::user("m3"),
            Message::user("m4"),
        ];

        query.apply_window_bounds(&mut messages);

        // First (anchor) + latest 2; the middle is dropped.
        let texts: Vec<_> = messages.iter().filter_map(|m| m.text()).collect();
        assert_eq!(texts, vec!["goal", "m3", "m4"]);
    }

    #[test]
    fn test_apply_window_bounds_keep_head_no_duplicate_on_overlap() {
        let session_id: SessionId = Uuid::now_v7().into();
        // keep_head + limit >= len -> nothing dropped, no duplication.
        let query = MessageQuery::new(session_id)
            .with_limit(3)
            .with_keep_head(2);
        let mut messages = vec![Message::user("a"), Message::user("b"), Message::user("c")];

        query.apply_window_bounds(&mut messages);

        let texts: Vec<_> = messages.iter().filter_map(|m| m.text()).collect();
        assert_eq!(texts, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_apply_window_bounds_keep_head_zero_matches_tail_only() {
        let session_id: SessionId = Uuid::now_v7().into();
        let query = MessageQuery::new(session_id)
            .with_limit(2)
            .with_keep_head(0);
        let mut messages = vec![Message::user("a"), Message::user("b"), Message::user("c")];

        query.apply_window_bounds(&mut messages);

        let texts: Vec<_> = messages.iter().filter_map(|m| m.text()).collect();
        assert_eq!(texts, vec!["b", "c"]);
    }

    #[test]
    fn test_apply_window_bounds_keep_head_exceeds_total() {
        let session_id: SessionId = Uuid::now_v7().into();
        // keep_head larger than the list -> keep everything, no panic/duplication.
        let query = MessageQuery::new(session_id)
            .with_limit(1)
            .with_keep_head(10);
        let mut messages = vec![Message::user("a"), Message::user("b")];

        query.apply_window_bounds(&mut messages);

        let texts: Vec<_> = messages.iter().filter_map(|m| m.text()).collect();
        assert_eq!(texts, vec!["a", "b"]);
    }

    #[test]
    fn test_apply_windowing_prepends_excluded_notice() {
        let session_id: SessionId = Uuid::now_v7().into();
        let query = MessageQuery::new(session_id)
            .with_limit(1)
            .with_prepend_transform(Arc::new(ExcludedNoticeTransform::new("{} hidden")));
        let mut messages = vec![Message::user("first"), Message::user("second")];

        query.apply_windowing(&mut messages);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text(), Some("1 hidden"));
        assert_eq!(messages[1].text(), Some("second"));
    }

    #[test]
    fn test_injection_before_index() {
        let session_id: SessionId = Uuid::now_v7().into();
        let injected = Message::system("Injected before index 1");

        let query = MessageQuery::new(session_id)
            .with_injection(InjectedMessage::before_index(1, injected.clone()));

        let mut messages = vec![
            Message::user("First"),
            Message::user("Second"),
            Message::user("Third"),
        ];

        query.apply_injections(&mut messages);

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].text(), Some("First"));
        assert_eq!(messages[1].text(), Some("Injected before index 1"));
        assert_eq!(messages[2].text(), Some("Second"));
    }

    #[test]
    fn test_multiple_injections() {
        let session_id: SessionId = Uuid::now_v7().into();

        let query = MessageQuery::new(session_id)
            .with_injection(InjectedMessage::at_start(Message::system("Start")))
            .with_injection(InjectedMessage::at_end(Message::system("End")));

        let mut messages = vec![Message::user("Middle")];

        query.apply_injections(&mut messages);

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].text(), Some("Start"));
        assert_eq!(messages[1].text(), Some("Middle"));
        assert_eq!(messages[2].text(), Some("End"));
    }

    #[test]
    fn test_filter_debug() {
        let filter = MessageFilter::TimeRange {
            from: None,
            to: None,
        };
        let debug_str = format!("{:?}", filter);
        assert!(debug_str.contains("TimeRange"));

        let custom = MessageFilter::Custom(Arc::new(|_| true));
        let debug_str = format!("{:?}", custom);
        assert!(debug_str.contains("Custom"));
        assert!(debug_str.contains("<fn>"));
    }

    // ========================================================================
    // Additional edge case tests
    // ========================================================================

    #[test]
    fn test_with_filters_multiple() {
        let session_id: SessionId = Uuid::now_v7().into();
        let query = MessageQuery::new(session_id).with_filters([
            MessageFilter::EventTypes(vec!["input.message".to_string()]),
            MessageFilter::Search("hello".to_string()),
        ]);

        assert_eq!(query.filters.len(), 2);
        assert!(query.has_db_filters());
        assert!(!query.has_custom_filters());
    }

    #[test]
    fn test_with_injections_multiple() {
        let session_id: SessionId = Uuid::now_v7().into();
        let query = MessageQuery::new(session_id).with_injections([
            InjectedMessage::at_start(Message::system("First")),
            InjectedMessage::at_end(Message::system("Last")),
        ]);

        assert_eq!(query.injections.len(), 2);
        assert!(query.has_injections());
    }

    #[test]
    fn test_injection_after_index() {
        let session_id: SessionId = Uuid::now_v7().into();
        let injected = Message::system("Injected after index 0");

        let query = MessageQuery::new(session_id)
            .with_injection(InjectedMessage::after_index(0, injected.clone()));

        let mut messages = vec![
            Message::user("First"),
            Message::user("Second"),
            Message::user("Third"),
        ];

        query.apply_injections(&mut messages);

        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0].text(), Some("First"));
        assert_eq!(messages[1].text(), Some("Injected after index 0"));
        assert_eq!(messages[2].text(), Some("Second"));
        assert_eq!(messages[3].text(), Some("Third"));
    }

    #[test]
    fn test_injection_into_empty_list() {
        let session_id: SessionId = Uuid::now_v7().into();

        let query = MessageQuery::new(session_id)
            .with_injection(InjectedMessage::at_start(Message::system("Start")))
            .with_injection(InjectedMessage::at_end(Message::system("End")));

        let mut messages: Vec<Message> = vec![];

        query.apply_injections(&mut messages);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text(), Some("Start"));
        assert_eq!(messages[1].text(), Some("End"));
    }

    #[test]
    fn test_injection_before_index_out_of_bounds() {
        let session_id: SessionId = Uuid::now_v7().into();
        let injected = Message::system("Injected before index 10");

        // Index 10 is out of bounds for a 2-element list
        let query = MessageQuery::new(session_id)
            .with_injection(InjectedMessage::before_index(10, injected.clone()));

        let mut messages = vec![Message::user("First"), Message::user("Second")];

        query.apply_injections(&mut messages);

        // Should insert at the end (min(10, 2) = 2)
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].text(), Some("Injected before index 10"));
    }

    #[test]
    fn test_injection_after_index_out_of_bounds() {
        let session_id: SessionId = Uuid::now_v7().into();
        let injected = Message::system("Injected after index 10");

        // Index 10 is out of bounds for a 2-element list
        let query = MessageQuery::new(session_id)
            .with_injection(InjectedMessage::after_index(10, injected.clone()));

        let mut messages = vec![Message::user("First"), Message::user("Second")];

        query.apply_injections(&mut messages);

        // Should insert at the end (min(11, 2) = 2)
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[2].text(), Some("Injected after index 10"));
    }

    #[test]
    fn test_multiple_start_injections_preserve_order() {
        let session_id: SessionId = Uuid::now_v7().into();

        // Multiple start injections should maintain their order
        let query = MessageQuery::new(session_id)
            .with_injection(InjectedMessage::at_start(Message::system("First injected")))
            .with_injection(InjectedMessage::at_start(Message::system(
                "Second injected",
            )));

        let mut messages = vec![Message::user("Original")];

        query.apply_injections(&mut messages);

        assert_eq!(messages.len(), 3);
        // Both should be at the start, first one should come first
        assert_eq!(messages[0].text(), Some("First injected"));
        assert_eq!(messages[1].text(), Some("Second injected"));
        assert_eq!(messages[2].text(), Some("Original"));
    }

    #[test]
    fn test_combined_db_and_custom_filters() {
        let session_id: SessionId = Uuid::now_v7().into();

        let query = MessageQuery::new(session_id)
            .with_filter(MessageFilter::Search("hello".to_string()))
            .with_filter(MessageFilter::Custom(Arc::new(|msg| {
                msg.role == crate::MessageRole::User
            })));

        assert!(query.has_db_filters());
        assert!(query.has_custom_filters());
        assert_eq!(query.filters.len(), 2);
    }

    #[test]
    fn test_query_default() {
        let query = MessageQuery::default();
        assert_eq!(query.session_id, Uuid::nil());
        assert!(query.filters.is_empty());
        assert!(query.injections.is_empty());
        assert_eq!(query.limit, None);
        assert_eq!(query.offset, None);
    }

    // ========================================================================
    // Anchored window tests
    // ========================================================================

    #[test]
    fn test_anchored_window_keeps_head_and_tail_drops_middle() {
        // 5 messages, anchor 1, min tail 2, tiny budget: only head + tail survive.
        let costs = [10, 10, 10, 10, 10];
        let win = anchored_window(&costs, 1, 2, None, 1);
        assert_eq!(win.head_len, 1);
        assert_eq!(win.recent_start, 3);
        assert_eq!(win.hidden(), 2); // messages 1 and 2 dropped
    }

    #[test]
    fn test_anchored_window_grows_recent_block_within_budget() {
        // Generous budget pulls older messages back into the contiguous tail.
        let costs = [10, 10, 10, 10, 10];
        let win = anchored_window(&costs, 1, 1, None, 1_000);
        assert_eq!(win.head_len, 1);
        assert_eq!(win.recent_start, 1); // everything kept
        assert_eq!(win.hidden(), 0);
    }

    #[test]
    fn test_anchored_window_respects_max_tail_cap() {
        // max_tail caps the recent block; the head anchor is additional to it.
        let costs = [10; 10];
        let win = anchored_window(&costs, 1, 5, Some(2), 1_000);
        assert_eq!(win.head_len, 1);
        assert_eq!(win.recent_start, 8); // keep msg 0 + msgs 8,9
        assert_eq!(win.hidden(), 7);
    }

    #[test]
    fn test_anchored_window_keeps_anchor_and_tail_even_when_over_budget() {
        let costs = [100, 100, 100, 100];
        // Budget 0 still preserves head (1) and min tail (1): never drops anchors.
        let win = anchored_window(&costs, 1, 1, None, 0);
        assert_eq!(win.head_len, 1);
        assert_eq!(win.recent_start, 3);
        assert_eq!(win.hidden(), 2);
    }

    #[test]
    fn test_anchored_window_no_middle_when_head_and_tail_overlap() {
        let costs = [10, 10, 10];
        let win = anchored_window(&costs, 1, 10, None, 1);
        assert_eq!(win.hidden(), 0);
    }

    #[test]
    fn test_anchored_window_empty() {
        let win = anchored_window(&[], 1, 2, None, 100);
        assert_eq!(win.head_len, 0);
        assert_eq!(win.recent_start, 0);
        assert_eq!(win.hidden(), 0);
    }
}
