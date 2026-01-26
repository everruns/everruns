// Event streaming HTTP routes (SSE)
// Events are notifications streamed to clients, NOT primary data storage
// Routes use ResolvedOrg: org derived from auth context (API key or X-Org-Id header)

use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::get,
};
use everruns_core::typed_id::SessionId;
use everruns_core::{Event, EventListener};
use serde::Deserialize;

use super::common::{ErrorResponse, ListResponse};
use super::sse::SseStreamConfig;
use crate::services::EventService;
use futures::{
    StreamExt,
    stream::{self, Stream},
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
    /// Event types to exclude from the response (can be specified multiple times).
    /// Common delta events to exclude: output.message.delta, reason.thinking.delta
    #[serde(default)]
    #[param(style = Form, explode = true)]
    pub exclude: Vec<String>,
}

// ============================================
// App State and Routes
// ============================================

/// App state for events routes
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
    pub event_service: Arc<EventService>,
    pub auth: AuthState,
}

impl AppState {
    /// Create app state with default event service (no listeners)
    #[allow(dead_code)]
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: Arc::new(EventService::new(db)),
            auth,
        }
    }

    /// Create app state with event listeners for observability
    pub fn with_listeners(
        db: Arc<StorageBackend>,
        listeners: Vec<Arc<dyn EventListener>>,
        auth: AuthState,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: Arc::new(EventService::with_listeners(db, listeners)),
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create event routes (nested under sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/:session_id/sse", get(stream_sse))
        .route("/v1/sessions/:session_id/events", get(list_events))
        .with_state(state)
}

// ============================================
// HTTP Handlers
// ============================================

/// GET /v1/sessions/{session_id}/sse - Stream events (SSE notifications)
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/sse",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        EventsQuery
    ),
    responses(
        (status = 200, description = "Event stream", content_type = "text/event-stream"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "events"
)]
pub async fn stream_sse(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, (StatusCode, Json<ErrorResponse>)>
{
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    // Verify session exists
    let _session = state
        .session_service
        .get(org.org_id, &org.public_id, session_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!("Failed to get session: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Session not found".to_string(),
                }),
            )
        })?;

    let session_id = session_id.uuid();
    tracing::info!(session_id = %session_id, since_id = ?query.since_id, exclude = ?query.exclude, "Starting event stream");

    let event_service = state.event_service.clone();
    let initial_since_id = query.since_id;
    let exclude_types = query.exclude;

    // Use realtime config for session events (fast updates for interactive UX)
    let config = SseStreamConfig::realtime();

    // State for stream: (last_id, backoff_ms, sent_connected, exclude_types)
    #[derive(Clone)]
    struct StreamState {
        last_id: Option<Uuid>,
        backoff_ms: u64,
        sent_connected: bool,
        config: SseStreamConfig,
        exclude_types: Vec<String>,
    }

    let initial_state = StreamState {
        last_id: initial_since_id,
        backoff_ms: config.min_backoff_ms,
        sent_connected: false,
        config,
        exclude_types,
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
                tracing::debug!(session_id = %session_id, "SSE: sending connected event");
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
            tracing::debug!(session_id = %session_id, last_id = ?state.last_id, "SSE: fetching events");
            match event_service.list(session_id, None, state.last_id, &state.exclude_types).await {
                Ok(events) if !events.is_empty() => {
                    // Get the last event ID for next iteration (extract UUID for db query)
                    let new_last_id = Some(events.last().unwrap().id.uuid());

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
                        backoff_ms: state.config.min_backoff_ms,
                        sent_connected: true,
                        config: state.config,
                        exclude_types: state.exclude_types,
                    };
                    Some((stream::iter(sse_events), new_state))
                }
                Ok(_) => {
                    // No new events, wait with exponential backoff
                    tokio::time::sleep(Duration::from_millis(state.backoff_ms)).await;

                    // Increase backoff for next iteration (double, up to max)
                    let new_backoff = state.config.next_backoff(state.backoff_ms);
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

/// GET /v1/sessions/{session_id}/events - List events (JSON)
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/events",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        EventsQuery
    ),
    responses(
        (status = 200, description = "Events list", body = ListResponse<Event>),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "events"
)]
pub async fn list_events(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<ListResponse<Event>>, (StatusCode, Json<ErrorResponse>)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    // Verify session exists
    let _session = state
        .session_service
        .get(org.org_id, &org.public_id, session_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!("Failed to get session: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Session not found".to_string(),
                }),
            )
        })?;

    // Fetch events using EventService (converts rows to core::Event)
    // Optional since_id filter for incremental fetching
    // Optional exclude filter for filtering out event types (e.g., delta events)
    let events = state
        .event_service
        .list(session_id.uuid(), None, query.since_id, &query.exclude)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list events: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    Ok(Json(ListResponse { data: events }))
}
