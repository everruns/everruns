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
    /// Maps to: `WHERE data::text ILIKE '%' || $query || '%'`
    /// For production, consider using pg_trgm or tsvector for better performance.
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
/// use everruns_core::typed_id::SessionId;
/// use uuid::Uuid;
/// use chrono::Utc;
///
/// let session_id: SessionId = Uuid::now_v7().into();
/// let query = MessageQuery::new(session_id)
///     .with_filter(MessageFilter::TimeRange {
///         from: Some(Utc::now() - chrono::Duration::hours(24)),
///         to: None,
///     })
///     .with_limit(100);
/// ```
#[derive(Debug, Clone)]
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
}

impl Default for MessageQuery {
    fn default() -> Self {
        Self {
            session_id: SessionId::from_seed(0),
            filters: Vec::new(),
            injections: Vec::new(),
            limit: None,
            offset: None,
        }
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
        }
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

    /// Check if this query has any DB-mappable filters
    pub fn has_db_filters(&self) -> bool {
        self.filters
            .iter()
            .any(|f| !matches!(f, MessageFilter::Custom(_)))
    }

    /// Check if this query has any in-memory-only filters
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
        index_injections.sort_by(|a, b| b.0.cmp(&a.0));

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
    fn test_filter_exclude_ids() {
        let id1 = EventId::new();
        let id2 = EventId::new();

        let filter = MessageFilter::ExcludeIds(vec![id1, id2]);
        let debug_str = format!("{:?}", filter);
        assert!(debug_str.contains("ExcludeIds"));
    }

    #[test]
    fn test_filter_include_ids() {
        let id1 = EventId::new();
        let id2 = EventId::new();

        let filter = MessageFilter::IncludeIds(vec![id1, id2]);
        let debug_str = format!("{:?}", filter);
        assert!(debug_str.contains("IncludeIds"));
    }

    #[test]
    fn test_filter_tool_name() {
        let filter = MessageFilter::ToolName("get_weather".to_string());
        let debug_str = format!("{:?}", filter);
        assert!(debug_str.contains("ToolName"));
        assert!(debug_str.contains("get_weather"));
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
    // MessageFilterProvider tests
    // ========================================================================

    struct TestFilterProvider {
        priority: i32,
    }

    impl MessageFilterProvider for TestFilterProvider {
        fn apply_filters(&self, query: &mut MessageQuery, config: &serde_json::Value) {
            // Add a filter based on config
            if let Some(search) = config.get("search").and_then(|v| v.as_str()) {
                query
                    .filters
                    .push(MessageFilter::Search(search.to_string()));
            }
        }

        fn priority(&self) -> i32 {
            self.priority
        }
    }

    #[test]
    fn test_filter_provider_apply() {
        let provider = TestFilterProvider { priority: 0 };
        let session_id: SessionId = Uuid::now_v7().into();
        let mut query = MessageQuery::new(session_id);

        let config = serde_json::json!({ "search": "hello" });
        provider.apply_filters(&mut query, &config);

        assert_eq!(query.filters.len(), 1);
        assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "hello"));
    }

    #[test]
    fn test_filter_provider_priority() {
        let low_priority = TestFilterProvider { priority: -10 };
        let high_priority = TestFilterProvider { priority: 10 };
        let default_priority = TestFilterProvider { priority: 0 };

        assert_eq!(low_priority.priority(), -10);
        assert_eq!(high_priority.priority(), 10);
        assert_eq!(default_priority.priority(), 0);
    }

    #[test]
    fn test_injection_position_debug() {
        // Test debug representation of InjectionPosition variants
        let start = InjectionPosition::Start;
        let debug = format!("{:?}", start);
        assert!(debug.contains("Start"));

        let end = InjectionPosition::End;
        let debug = format!("{:?}", end);
        assert!(debug.contains("End"));

        let before = InjectionPosition::BeforeIndex(5);
        let debug = format!("{:?}", before);
        assert!(debug.contains("BeforeIndex"));
        assert!(debug.contains("5"));

        let after = InjectionPosition::AfterIndex(3);
        let debug = format!("{:?}", after);
        assert!(debug.contains("AfterIndex"));
        assert!(debug.contains("3"));
    }
}
