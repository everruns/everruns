// Event streaming HTTP routes (SSE)
// Events are notifications streamed to clients, NOT primary data storage
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

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
use everruns_core::typed_id::{EventId, SessionId};
use everruns_core::{Event, EventListener};
use serde::Deserialize;

use super::common::{ErrorResponse, ListResponse};
use super::sse::{DisconnectReason, SseConnectionTracker, SseStreamConfig};
use crate::event_notifications::EventNotificationBroadcaster;
use crate::services::EventService;
use futures::{
    StreamExt,
    stream::{self, Stream},
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast;
use tokio::time::Instant;
use uuid::Uuid;

use crate::services::SessionService;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for event listing
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct EventsQuery {
    /// Filter events with ID greater than this event ID (prefixed format: event_{32-hex})
    pub since_id: Option<EventId>,
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
    pub sse_tracker: Arc<SseConnectionTracker>,
    /// Push-based event notifications via pg_notify (None in DEV_MODE/in-memory)
    pub event_broadcaster: Option<Arc<EventNotificationBroadcaster>>,
    pub auth: AuthState,
}

impl AppState {
    /// Create app state with default event service (no listeners)
    #[allow(dead_code)]
    pub fn new(
        db: Arc<StorageBackend>,
        auth: AuthState,
        sse_tracker: Arc<SseConnectionTracker>,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: Arc::new(EventService::new(db)),
            sse_tracker,
            event_broadcaster: None,
            auth,
        }
    }

    /// Create app state with event listeners for observability
    pub fn with_listeners(
        db: Arc<StorageBackend>,
        listeners: Vec<Arc<dyn EventListener>>,
        auth: AuthState,
        sse_tracker: Arc<SseConnectionTracker>,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: Arc::new(EventService::with_listeners(db, listeners)),
            sse_tracker,
            event_broadcaster: None,
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
        .route("/v1/sessions/{session_id}/sse", get(stream_sse))
        .route("/v1/sessions/{session_id}/events", get(list_events))
        .with_state(state)
}

// ============================================
// HTTP Handlers
// ============================================

/// GET /v1/sessions/{session_id}/sse - Stream events (SSE notifications)
///
/// Establishes a Server-Sent Events (SSE) connection for real-time event streaming.
///
/// ## Connection Lifecycle Events
///
/// - **connected**: Sent immediately when the stream is established.
///   Data: `{"status":"connected"}`
///
/// - **disconnecting**: Sent before the server closes the connection for graceful cycling.
///   Data: `{"reason":"connection_cycle","retry_ms":100}`
///   Clients should reconnect immediately using the `since_id` of the last received event.
///
/// ## Connection Cycling
///
/// Connections are automatically cycled every 5 minutes to prevent stale connections
/// through proxies and load balancers. Before closing, the server sends a `disconnecting`
/// event so clients can reconnect seamlessly without missing events.
///
/// ## Retry Hints
///
/// Each SSE event includes a `retry:` field (in milliseconds) that hints how long
/// clients should wait before reconnecting if the connection is lost:
/// - During active streaming: 100ms (fast reconnect)
/// - During idle periods: increases with backoff up to 500ms
/// - After `disconnecting` event: 100ms (immediate reconnect)
///
/// ## Resuming Streams
///
/// Use the `since_id` query parameter to resume from a specific event. The server
/// will only send events with IDs greater than the specified value. Event IDs are
/// UUID v7 (monotonically increasing), ensuring reliable ordering.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/sse",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        EventsQuery
    ),
    responses(
        (status = 200, description = "Event stream. Includes 'connected' on open, domain events during streaming, and 'disconnecting' before graceful close.", content_type = "text/event-stream"),
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

    // Enforce SSE connection limits (EVE-8 / TM-DOS-003)
    let sse_guard = state
        .sse_tracker
        .try_acquire(org.org_id, session_id)
        .map_err(|rejection| {
            tracing::warn!(
                org_id = org.org_id,
                %session_id,
                reason = %rejection,
                "SSE connection rejected"
            );
            (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse {
                    error: rejection.to_string(),
                }),
            )
        })?;

    tracing::info!(session_id = %session_id, since_id = ?query.since_id, exclude = ?query.exclude, "Starting event stream");

    let event_service = state.event_service.clone();
    let initial_since_id = query.since_id.map(|id| id.uuid());
    let exclude_types = query.exclude;

    // Use realtime config for session events (fast updates for interactive UX)
    let config = SseStreamConfig::realtime();
    let connection_start = Instant::now();

    // Set up push notification channel: when pg_notify fires for this session,
    // the waker triggers immediate poll instead of waiting for backoff timeout.
    // Falls back to polling in DEV_MODE (no PostgreSQL).
    let event_waker = Arc::new(tokio::sync::Notify::new());
    if let Some(ref broadcaster) = state.event_broadcaster {
        let mut rx = broadcaster.subscribe();
        let waker = event_waker.clone();
        let target_session = session_id;
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(payload) if payload.session_id == target_session => {
                        waker.notify_one();
                    }
                    Ok(_) => {} // Different session, ignore
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(lagged = n, "Event notification receiver lagged, waking");
                        waker.notify_one();
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    // Stream state machine
    #[derive(Clone)]
    enum StreamPhase {
        /// Initial phase - send connected event
        SendConnected,
        /// Normal operation - poll for events
        Polling,
        /// Send disconnecting event then close
        SendDisconnecting,
        /// Stream has ended
        Closed,
    }

    #[derive(Clone)]
    struct StreamState {
        phase: StreamPhase,
        last_id: Option<Uuid>,
        backoff_ms: u64,
        config: SseStreamConfig,
        exclude_types: Vec<String>,
        connection_start: Instant,
        /// Waker triggered by pg_notify when events arrive for this session
        event_waker: Arc<tokio::sync::Notify>,
    }

    let initial_state = StreamState {
        phase: StreamPhase::SendConnected,
        last_id: initial_since_id,
        backoff_ms: config.min_backoff_ms,
        config,
        exclude_types,
        connection_start,
        event_waker,
    };

    // Create stream that replays events from database
    // Uses since_id (UUID v7) for tracking - monotonically increasing
    // SSE format: event: <type>, data: <full core::Event JSON>, id: <event UUID>
    // Features:
    // - Exponential backoff (100ms → 500ms) when no new events
    // - Connection cycling: graceful close after max_connection_duration with "disconnecting" event
    // - Retry hints: SSE `retry:` field hints reconnection timing based on backoff
    let stream = stream::unfold(initial_state, move |state| {
        let event_service = event_service.clone();
        async move {
            match state.phase {
                StreamPhase::Closed => {
                    // Stream has ended
                    None
                }

                StreamPhase::SendDisconnecting => {
                    // Send disconnecting event and close
                    tracing::info!(
                        session_id = %session_id,
                        duration_secs = state.connection_start.elapsed().as_secs(),
                        "SSE: connection cycling, sending disconnecting event"
                    );
                    let disconnect_data = format!(
                        r#"{{"reason":"{}","retry_ms":{}}}"#,
                        DisconnectReason::ConnectionCycle.as_str(),
                        state.config.disconnect_retry_ms
                    );
                    let disconnecting_event = Ok(SseEvent::default()
                        .event("disconnecting")
                        .data(disconnect_data)
                        .retry(state.config.disconnect_retry()));

                    let new_state = StreamState {
                        phase: StreamPhase::Closed,
                        ..state
                    };
                    Some((stream::iter(vec![disconnecting_event]), new_state))
                }

                StreamPhase::SendConnected => {
                    // Send initial "connected" event
                    tracing::debug!(session_id = %session_id, "SSE: sending connected event");
                    let connected_event = Ok(SseEvent::default()
                        .event("connected")
                        .data(r#"{"status":"connected"}"#)
                        .retry(state.config.retry_hint(state.backoff_ms)));

                    let new_state = StreamState {
                        phase: StreamPhase::Polling,
                        ..state
                    };
                    Some((stream::iter(vec![connected_event]), new_state))
                }

                StreamPhase::Polling => {
                    // Check for connection cycling - graceful close after max duration
                    if state.connection_start.elapsed() > state.config.max_connection_duration() {
                        let new_state = StreamState {
                            phase: StreamPhase::SendDisconnecting,
                            ..state
                        };
                        // Recurse immediately to send disconnecting event
                        return Some((stream::iter(vec![]), new_state));
                    }

                    // Fetch events since last ID
                    tracing::debug!(session_id = %session_id, last_id = ?state.last_id, "SSE: fetching events");
                    match event_service.list(session_id, None, state.last_id, &state.exclude_types).await {
                        Ok(events) if !events.is_empty() => {
                            // Get the last event ID for next iteration
                            let new_last_id = Some(events.last().unwrap().id.uuid());
                            let new_backoff = state.config.min_backoff_ms; // Reset backoff

                            tracing::debug!(
                                session_id = %session_id,
                                last_id = ?state.last_id,
                                new_last_id = ?new_last_id,
                                event_count = events.len(),
                                "SSE: fetched events"
                            );

                            // Convert events to SSE format with retry hint
                            let retry_duration = state.config.retry_hint(new_backoff);
                            let sse_events: Vec<Result<SseEvent, Infallible>> = events
                                .into_iter()
                                .map(|event| {
                                    let event_type = event.event_type.clone();
                                    let event_id = event.id.to_string();
                                    let json = serde_json::to_string(&event)
                                        .unwrap_or_else(|_| "{}".to_string());

                                    Ok(SseEvent::default()
                                        .event(&event_type)
                                        .data(json)
                                        .id(event_id)
                                        .retry(retry_duration))
                                })
                                .collect();

                            let new_state = StreamState {
                                phase: StreamPhase::Polling,
                                last_id: new_last_id,
                                backoff_ms: new_backoff,
                                config: state.config,
                                exclude_types: state.exclude_types,
                                connection_start: state.connection_start,
                                event_waker: state.event_waker,
                            };
                            Some((stream::iter(sse_events), new_state))
                        }
                        Ok(_) => {
                            // No new events — wait for push notification or fallback timeout.
                            // With pg_notify: waker fires in <5ms on event insert.
                            // Without pg_notify (DEV_MODE): falls back to polling at backoff.
                            let fallback = Duration::from_millis(state.backoff_ms);
                            tokio::select! {
                                _ = state.event_waker.notified() => {
                                    // Notification received — poll immediately with reset backoff
                                }
                                _ = tokio::time::sleep(fallback) => {
                                    // Fallback timeout — increase backoff
                                }
                            }

                            // Increase backoff for next iteration (reset on event found above)
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
            }
        }
    })
    .flatten();

    // Wrap stream to hold the SSE connection guard alive for the stream's lifetime.
    // When the client disconnects and the stream is dropped, the guard releases the slot.
    let guarded_stream = GuardedStream {
        inner: Box::pin(stream),
        _guard: sse_guard,
    };

    Ok(Sse::new(guarded_stream).keep_alive(KeepAlive::default()))
}

/// Stream wrapper that holds an SSE connection guard until the stream is dropped.
struct GuardedStream<S> {
    inner: std::pin::Pin<Box<S>>,
    _guard: super::sse::SseConnectionGuard,
}

impl<S: Stream> Stream for GuardedStream<S> {
    type Item = S::Item;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
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
        .list(
            session_id.uuid(),
            None,
            query.since_id.map(|id| id.uuid()),
            &query.exclude,
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Test that EventId correctly parses prefixed event IDs.
    /// This was a bug where since_id was typed as Uuid instead of EventId,
    /// causing "UUID parsing failed" errors when clients sent prefixed IDs
    /// like `event_019c263feac17809a9d442e25317890b`.
    #[test]
    fn test_event_id_parses_prefixed_format() {
        let event_id: EventId = "event_019c263feac17809a9d442e25317890b"
            .parse()
            .expect("Should parse prefixed event ID");

        assert_eq!(
            event_id.to_string(),
            "event_019c263feac17809a9d442e25317890b"
        );

        // Verify we can extract the UUID
        let uuid = event_id.uuid();
        assert_eq!(uuid.to_string(), "019c263f-eac1-7809-a9d4-42e25317890b");
    }

    /// Test that EventsQuery JSON deserialization works with prefixed event IDs.
    /// This simulates what axum Query extractor does when parsing query params.
    #[test]
    fn test_events_query_deserializes_prefixed_event_id() {
        // axum internally uses serde to deserialize query params
        // Test JSON deserialization to verify EventId's Deserialize impl works
        let json = r#"{"since_id": "event_019c263feac17809a9d442e25317890b", "exclude": []}"#;
        let query: EventsQuery = serde_json::from_str(json)
            .expect("Should deserialize EventsQuery with prefixed event ID");

        assert!(query.since_id.is_some());
        let event_id = query.since_id.unwrap();
        assert_eq!(
            event_id.to_string(),
            "event_019c263feac17809a9d442e25317890b"
        );
    }

    #[test]
    fn test_events_query_with_exclude() {
        let json = r#"{"since_id": "event_019c263feac17809a9d442e25317890b", "exclude": ["output.message.delta", "reason.thinking.delta"]}"#;
        let query: EventsQuery =
            serde_json::from_str(json).expect("Should deserialize EventsQuery");

        assert!(query.since_id.is_some());
        assert_eq!(query.exclude.len(), 2);
        assert!(query.exclude.contains(&"output.message.delta".to_string()));
        assert!(query.exclude.contains(&"reason.thinking.delta".to_string()));
    }

    #[test]
    fn test_events_query_empty_since_id() {
        let json = r#"{"exclude": []}"#;
        let query: EventsQuery =
            serde_json::from_str(json).expect("Should deserialize EventsQuery without since_id");

        assert!(query.since_id.is_none());
        assert!(query.exclude.is_empty());
    }
}
