// Event rows. Messages are stored as events with type "message.*"; the events
// table is the sole source of truth for conversation data.

use crate::kernel_imports::contracts::typed_id::{EventId, SessionId};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct EventRow {
    pub id: EventId,
    pub session_id: SessionId,
    pub sequence: i32,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub context: serde_json::Value,
    pub data: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateEventRow {
    pub session_id: SessionId,
    pub event_type: String,
    pub ts: DateTime<Utc>,
    pub context: serde_json::Value,
    pub data: serde_json::Value,
    pub metadata: Option<serde_json::Value>,
    pub tags: Option<Vec<String>>,
}

/// Parameters for the advanced event listing path used by debugging tools.
///
/// Keep field set additive: new debug filters should be added here, not as
/// new positional arguments on the legacy `list_events` storage method.
#[derive(Debug, Clone, Default)]
pub struct ListEventsParams {
    pub session_id: SessionId,
    /// Forward cursor: events with `sequence > after_sequence`.
    pub after_sequence: Option<i32>,
    /// Forward cursor by id; resolves to its sequence at query time.
    pub since_id: Option<EventId>,
    /// Backward cursor: events with `sequence < before_sequence`.
    pub before_sequence: Option<i32>,
    /// Window anchor: returns up to `window` events on each side of this id.
    /// Mutually exclusive with `before_sequence`, `after_sequence`, and `since_id`
    /// (the command layer rejects combinations with a 400).
    pub around_id: Option<EventId>,
    /// Window size for `around_id` (rows on each side). Defaults to 50; capped at 500.
    pub window: Option<i32>,
    /// Positive event-type filter (empty = no filter).
    pub filter_types: Vec<String>,
    /// Negative event-type filter (applied after `filter_types`).
    pub exclude_types: Vec<String>,
    /// `created_at >= from_ts`.
    pub from_ts: Option<DateTime<Utc>>,
    /// `created_at <= to_ts`.
    pub to_ts: Option<DateTime<Utc>>,
    /// `context->>'turn_id' = turn_id`.
    pub turn_id: Option<String>,
    /// `context->>'exec_id' = exec_id`.
    pub exec_id: Option<String>,
    /// `context->>'trace_id' = trace_id`.
    pub trace_id: Option<String>,
    /// `events.tags && tags` (any-match).
    pub tags: Vec<String>,
    /// Match `data->>'tool_name' = tool_name`.
    pub tool_name: Option<String>,
    /// Full-text search via `search_vector @@ plainto_tsquery('english', q)`.
    /// In-memory mode falls back to a case-insensitive substring scan.
    pub q: Option<String>,
    /// When true, return newest first (sequence DESC). Ignored when
    /// `around_id` is set — the window is always returned in ASC order.
    pub order_desc: bool,
    /// Max rows to return.
    pub limit: Option<i32>,
}

/// Per-type event counts plus aggregate timestamps for a session.
#[derive(Debug, Clone)]
pub struct EventTypeCount {
    pub event_type: String,
    pub count: i64,
}

/// One-shot debug summary for a session: counts by type, first/last timestamps.
#[derive(Debug, Clone, Default)]
pub struct EventsSummary {
    pub total: i64,
    pub by_type: Vec<EventTypeCount>,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
}
