// Event streaming HTTP routes (SSE)
// Events are notifications streamed to clients, NOT primary data storage
//
// Durable streams design:
// - Offset-based resumption using UUID7: Clients can resume from any event ID
// - next_offset in response: UUID7 of last event for continuation
// - Cache-Control for historical reads: Immutable past events are cacheable
//
// Why UUID7 for offsets:
// - UUID7 is time-ordered (first 48 bits are Unix timestamp in ms)
// - Already stored as event ID, no separate sequence needed
// - Globally unique across sessions
// - Comparison works correctly: WHERE id > $uuid

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse,
    },
    routing::get,
    Json, Router,
};
use everruns_storage::Database;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use futures::{
    stream::{self, Stream},
    StreamExt,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use uuid::Uuid;

use crate::services::SessionService;

// ============================================
// App State and Routes
// ============================================

/// App state for events routes
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
    pub db: Arc<Database>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            db,
        }
    }
}

/// Create event routes (nested under sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/sse",
            get(stream_sse),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/events",
            get(list_events),
        )
        .with_state(state)
}

// ============================================
// Query Parameters
// ============================================

/// Query parameters for SSE streaming
#[derive(Debug, Deserialize, IntoParams)]
pub struct SseQuery {
    /// Resume from this offset (event UUID7). Events with id > offset are returned.
    /// Omit to start from the beginning.
    #[param(example = "01945c8a-0000-7000-8000-000000000000")]
    pub offset: Option<Uuid>,
}

/// Query parameters for events list
#[derive(Debug, Deserialize, IntoParams)]
pub struct EventsQuery {
    /// Resume from this offset (event UUID7). Events with id > offset are returned.
    /// Omit to start from the beginning.
    #[param(example = "01945c8a-0000-7000-8000-000000000000")]
    pub offset: Option<Uuid>,
    /// Maximum number of events to return. Defaults to 100 if not specified.
    #[param(example = 100)]
    pub limit: Option<i32>,
}

// ============================================
// HTTP Handlers
// ============================================

/// GET /v1/agents/{agent_id}/sessions/{session_id}/sse - Stream events (SSE notifications)
///
/// Supports offset-based resumption: provide `?offset=UUID` to resume from that event ID.
/// The `id` field in each SSE event contains the event UUID7 for client-side tracking.
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/sse",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        SseQuery
    ),
    responses(
        (status = 200, description = "Event stream", content_type = "text/event-stream"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "events"
)]
pub async fn stream_sse(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<SseQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, StatusCode> {
    // Verify session exists
    let _session = state
        .session_service
        .get(session_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let initial_offset = query.offset;
    tracing::info!(session_id = %session_id, offset = ?initial_offset, "Starting event stream");

    let db = state.db.clone();

    // Create stream that replays events from the specified offset (UUID7)
    let stream = stream::unfold(initial_offset, move |last_id| {
        let db = db.clone();
        async move {
            // Fetch events since last UUID
            match db.list_events_after_id(session_id, last_id).await {
                Ok(events) if !events.is_empty() => {
                    // Get the last event ID for next iteration
                    let new_id = events.last().unwrap().id;

                    // Convert events to SSE format
                    let sse_events: Vec<Result<SseEvent, Infallible>> = events
                        .into_iter()
                        .map(|event_row| {
                            let json = serde_json::to_string(&event_row.data)
                                .unwrap_or_else(|_| "{}".to_string());

                            Ok(SseEvent::default()
                                .event(&event_row.event_type)
                                .data(json)
                                .id(event_row.id.to_string()))
                        })
                        .collect();

                    Some((stream::iter(sse_events), Some(new_id)))
                }
                Ok(_) => {
                    // No new events, wait a bit before polling again
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Some((stream::iter(vec![]), last_id))
                }
                Err(e) => {
                    tracing::error!("Failed to fetch events: {}", e);
                    None
                }
            }
        }
    })
    .flatten();

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

// ============================================
// List Events (JSON response for polling)
// ============================================

/// Event response type for SSE/polling
#[derive(Debug, Serialize, ToSchema)]
pub struct Event {
    /// Unique event ID.
    pub id: Uuid,
    /// Session this event belongs to.
    pub session_id: Uuid,
    /// Sequence number within the session (for offset-based resumption).
    pub sequence: i32,
    /// Event type (e.g., "message.user", "message.assistant", "checkpoint").
    pub event_type: String,
    /// Event payload as JSON. Structure depends on event_type.
    pub data: serde_json::Value,
    /// When the event was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Paginated response for events list with offset-based resumption.
#[derive(Debug, Serialize, ToSchema)]
pub struct EventsResponse {
    /// Array of events.
    pub data: Vec<Event>,
    /// Next offset (event UUID7) to use for pagination. Pass this as `?offset=` to get the next page.
    /// If null, there are no more events (you've caught up).
    pub next_offset: Option<Uuid>,
    /// Whether more events may be available beyond this page.
    pub has_more: bool,
}

const DEFAULT_LIMIT: i32 = 100;
const MAX_LIMIT: i32 = 1000;

/// GET /v1/agents/{agent_id}/sessions/{session_id}/events - List events (JSON)
///
/// Supports offset-based pagination for durable stream semantics:
/// - Use `?offset=UUID` to get events with id > UUID (UUID7 is time-ordered)
/// - Use `?limit=M` to limit the number of events returned
/// - Response includes `next_offset` (UUID7) for the next page
///
/// Cache-Control is set for historical reads when the session is not running.
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/events",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        EventsQuery
    ),
    responses(
        (status = 200, description = "Events list with pagination info", body = EventsResponse),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "events"
)]
pub async fn list_events(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse, StatusCode> {
    // Verify session exists and get its status for cache decisions
    let session = state
        .session_service
        .get(session_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let offset = query.offset;
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);

    // Fetch events with offset (UUID7) and limit+1 to detect has_more
    let event_rows = state
        .db
        .list_events_after_id_paginated(session_id, offset, limit + 1)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list events: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Check if there are more events beyond the limit
    let has_more = event_rows.len() > limit as usize;
    let event_rows: Vec<_> = event_rows.into_iter().take(limit as usize).collect();

    // Calculate next_offset from the last event's UUID7
    let next_offset = event_rows.last().map(|e| e.id);

    // Convert to Event response type
    let events: Vec<Event> = event_rows
        .into_iter()
        .map(|row| Event {
            id: row.id,
            session_id: row.session_id,
            sequence: row.sequence,
            event_type: row.event_type,
            data: row.data,
            created_at: row.created_at,
        })
        .collect();

    let response = EventsResponse {
        data: events,
        next_offset,
        has_more,
    };

    // Add Cache-Control header for historical reads
    // Events are immutable, so past pages can be cached indefinitely
    // Only cache when:
    // 1. Session is not running (no new events expected soon)
    // 2. There are more events (this is a historical page, not the tail)
    let cache_control = if session.status != everruns_core::SessionStatus::Running && has_more {
        // Historical page from a non-running session - cache for 1 year
        "public, max-age=31536000, immutable"
    } else {
        // Live tail or running session - don't cache
        "no-cache"
    };

    Ok(([(header::CACHE_CONTROL, cache_control)], Json(response)))
}
