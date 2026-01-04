// Event streaming HTTP routes (SSE)
// Events are notifications streamed to clients, NOT primary data storage

use crate::storage::Database;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use everruns_core::Event;
use serde::Deserialize;

use super::common::ListResponse;
use crate::services::EventService;
use futures::{
    stream::{self, Stream},
    StreamExt,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use uuid::Uuid;

use crate::services::SessionService;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for event listing
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct EventsQuery {
    /// Filter events with ID greater than this UUID v7 (monotonically increasing)
    pub since_id: Option<Uuid>,
}

// ============================================
// App State and Routes
// ============================================

/// App state for events routes
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
    pub event_service: Arc<EventService>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: Arc::new(EventService::new(db)),
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
// HTTP Handlers
// ============================================

/// GET /v1/agents/{agent_id}/sessions/{session_id}/sse - Stream events (SSE notifications)
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/sse",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        EventsQuery
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
    Query(query): Query<EventsQuery>,
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

    tracing::info!(session_id = %session_id, since_id = ?query.since_id, "Starting event stream");

    let event_service = state.event_service.clone();
    let initial_since_id = query.since_id;

    // Backoff configuration
    const MIN_BACKOFF_MS: u64 = 100;
    const MAX_BACKOFF_MS: u64 = 10_000;

    // State for stream: (last_id, backoff_ms, sent_connected)
    #[derive(Clone)]
    struct StreamState {
        last_id: Option<Uuid>,
        backoff_ms: u64,
        sent_connected: bool,
    }

    let initial_state = StreamState {
        last_id: initial_since_id,
        backoff_ms: MIN_BACKOFF_MS,
        sent_connected: false,
    };

    // Create stream that replays events from database
    // Uses since_id (UUID v7) for tracking - monotonically increasing
    // SSE format: event: <type>, data: <full core::Event JSON>, id: <event UUID>
    // Includes exponential backoff (100ms → 10s) when no new events
    let stream = stream::unfold(initial_state, move |state| {
        let event_service = event_service.clone();
        async move {
            // Send initial "connected" event on first iteration
            if !state.sent_connected {
                let connected_event = Ok(SseEvent::default()
                    .event("connected")
                    .data(r#"{"status":"connected"}"#));
                let new_state = StreamState {
                    sent_connected: true,
                    ..state
                };
                return Some((stream::iter(vec![connected_event]), new_state));
            }

            // Fetch events since last ID
            match event_service.list(session_id, None, state.last_id).await {
                Ok(events) if !events.is_empty() => {
                    // Get the last event ID for next iteration
                    let new_last_id = Some(events.last().unwrap().id);

                    tracing::debug!(
                        session_id = %session_id,
                        last_id = ?state.last_id,
                        new_last_id = ?new_last_id,
                        event_count = events.len(),
                        "SSE: fetched events"
                    );

                    // Convert events to SSE format with full Event as data
                    let sse_events: Vec<Result<SseEvent, Infallible>> = events
                        .into_iter()
                        .map(|event| {
                            let event_type = event.event_type.clone();
                            let event_id = event.id.to_string();
                            let json =
                                serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());

                            Ok(SseEvent::default()
                                .event(&event_type)
                                .data(json)
                                .id(event_id))
                        })
                        .collect();

                    // Reset backoff on new events
                    let new_state = StreamState {
                        last_id: new_last_id,
                        backoff_ms: MIN_BACKOFF_MS,
                        sent_connected: true,
                    };
                    Some((stream::iter(sse_events), new_state))
                }
                Ok(_) => {
                    // No new events, wait with exponential backoff
                    tokio::time::sleep(Duration::from_millis(state.backoff_ms)).await;

                    // Increase backoff for next iteration (double, up to max)
                    let new_backoff = (state.backoff_ms * 2).min(MAX_BACKOFF_MS);
                    let new_state = StreamState {
                        backoff_ms: new_backoff,
                        ..state
                    };
                    Some((stream::iter(vec![]), new_state))
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

/// GET /v1/agents/{agent_id}/sessions/{session_id}/events - List events (JSON)
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/events",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        EventsQuery
    ),
    responses(
        (status = 200, description = "Events list", body = ListResponse<Event>),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "events"
)]
pub async fn list_events(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<ListResponse<Event>>, StatusCode> {
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

    // Fetch events using EventService (converts rows to core::Event)
    // Optional since_id filter for incremental fetching
    let events = state
        .event_service
        .list(session_id, None, query.since_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list events: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(ListResponse { data: events }))
}
