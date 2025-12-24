// Event streaming HTTP routes (SSE)
// Events are notifications streamed to clients, NOT primary data storage

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::get,
    Router,
};
use everruns_storage::Database;
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
            "/v1/agents/:agent_id/sessions/:session_id/events",
            get(stream_events),
        )
        .with_state(state)
}

// ============================================
// HTTP Handlers
// ============================================

/// GET /v1/agents/{agent_id}/sessions/{session_id}/events - Stream events (SSE notifications)
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/events",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Event stream", content_type = "text/event-stream"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "events"
)]
pub async fn stream_events(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
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

    tracing::info!(session_id = %session_id, "Starting event stream");

    let db = state.db.clone();

    // Create stream that replays all events from database
    let stream = stream::unfold(0i32, move |last_sequence| {
        let db = db.clone();
        async move {
            // Fetch events since last sequence
            match db.list_events(session_id, Some(last_sequence)).await {
                Ok(events) if !events.is_empty() => {
                    // Get the last sequence number for next iteration
                    let new_sequence = events.last().unwrap().sequence;

                    // Convert events to SSE format
                    let sse_events: Vec<Result<SseEvent, Infallible>> = events
                        .into_iter()
                        .map(|event_row| {
                            let json = serde_json::to_string(&event_row.data)
                                .unwrap_or_else(|_| "{}".to_string());

                            Ok(SseEvent::default()
                                .event(&event_row.event_type)
                                .data(json)
                                .id(event_row.sequence.to_string()))
                        })
                        .collect();

                    Some((stream::iter(sse_events), new_sequence))
                }
                Ok(_) => {
                    // No new events, wait a bit before polling again
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    Some((stream::iter(vec![]), last_sequence))
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
