// Session CRUD and Events HTTP routes (M2)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use everruns_contracts::{
    CreateEventRequest, CreateSessionRequest, Event, ListResponse, Session, UpdateSessionRequest,
};
use everruns_storage::{
    models::{CreateEvent, CreateSession, UpdateSession},
    Database,
};
use everruns_worker::AgentRunner;
use futures::{
    stream::{self, Stream},
    StreamExt,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use uuid::Uuid;

use crate::services::{EventService, SessionService};

/// App state for sessions routes
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
    pub event_service: Arc<EventService>,
    pub runner: Arc<dyn AgentRunner>,
    pub db: Arc<Database>,
}

impl AppState {
    pub fn new(db: Arc<Database>, runner: Arc<dyn AgentRunner>) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: Arc::new(EventService::new(db.clone())),
            runner,
            db,
        }
    }
}

/// Create session routes (nested under harnesses)
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Session CRUD under harness
        .route(
            "/v1/harnesses/{harness_id}/sessions",
            post(create_session).get(list_sessions),
        )
        .route(
            "/v1/harnesses/{harness_id}/sessions/{session_id}",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        // Events under session
        .route(
            "/v1/harnesses/{harness_id}/sessions/{session_id}/events",
            post(create_event).get(stream_events),
        )
        .route(
            "/v1/harnesses/{harness_id}/sessions/{session_id}/messages",
            get(list_messages),
        )
        .with_state(state)
}

/// POST /v1/harnesses/{harness_id}/sessions - Create a new session
#[utoipa::path(
    post,
    path = "/v1/harnesses/{harness_id}/sessions",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID")
    ),
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Session created successfully", body = Session),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn create_session(
    State(state): State<AppState>,
    Path(harness_id): Path<Uuid>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), StatusCode> {
    let input = CreateSession {
        harness_id,
        title: req.title,
        tags: req.tags,
        model_id: req.model_id,
    };

    let session = state.session_service.create(input).await.map_err(|e| {
        tracing::error!("Failed to create session: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((StatusCode::CREATED, Json(session)))
}

/// GET /v1/harnesses/{harness_id}/sessions - List sessions in harness
#[utoipa::path(
    get,
    path = "/v1/harnesses/{harness_id}/sessions",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID")
    ),
    responses(
        (status = 200, description = "List of sessions", body = ListResponse<Session>),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_sessions(
    State(state): State<AppState>,
    Path(harness_id): Path<Uuid>,
) -> Result<Json<ListResponse<Session>>, StatusCode> {
    let sessions = state.session_service.list(harness_id).await.map_err(|e| {
        tracing::error!("Failed to list sessions: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ListResponse::new(sessions)))
}

/// GET /v1/harnesses/{harness_id}/sessions/{session_id} - Get session
#[utoipa::path(
    get,
    path = "/v1/harnesses/{harness_id}/sessions/{session_id}",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session found", body = Session),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session(
    State(state): State<AppState>,
    Path((_harness_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Session>, StatusCode> {
    let session = state
        .session_service
        .get(session_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(session))
}

/// PATCH /v1/harnesses/{harness_id}/sessions/{session_id} - Update session
#[utoipa::path(
    patch,
    path = "/v1/harnesses/{harness_id}/sessions/{session_id}",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = UpdateSessionRequest,
    responses(
        (status = 200, description = "Session updated successfully", body = Session),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn update_session(
    State(state): State<AppState>,
    Path((_harness_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<Session>, StatusCode> {
    let input = UpdateSession {
        title: req.title,
        tags: req.tags,
        model_id: req.model_id,
        ..Default::default()
    };

    let session = state
        .session_service
        .update(session_id, input)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(session))
}

/// DELETE /v1/harnesses/{harness_id}/sessions/{session_id} - Delete session
#[utoipa::path(
    delete,
    path = "/v1/harnesses/{harness_id}/sessions/{session_id}",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    responses(
        (status = 204, description = "Session deleted successfully"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn delete_session(
    State(state): State<AppState>,
    Path((_harness_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .session_service
        .delete(session_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// POST /v1/harnesses/{harness_id}/sessions/{session_id}/events - Add event (user message)
#[utoipa::path(
    post,
    path = "/v1/harnesses/{harness_id}/sessions/{session_id}/events",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = CreateEventRequest,
    responses(
        (status = 201, description = "Event created successfully", body = Event),
        (status = 500, description = "Internal server error")
    ),
    tag = "events"
)]
pub async fn create_event(
    State(state): State<AppState>,
    Path((harness_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CreateEventRequest>,
) -> Result<(StatusCode, Json<Event>), StatusCode> {
    let input = CreateEvent {
        session_id,
        event_type: req.event_type.clone(),
        data: req.data,
    };

    let event = state.event_service.create(input).await.map_err(|e| {
        tracing::error!("Failed to create event: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // If this is a user message, start the session workflow
    if req.event_type == "message.user" {
        // Start the workflow execution
        if let Err(e) = state
            .runner
            .start_run(session_id, harness_id, session_id)
            .await
        {
            tracing::error!("Failed to start session workflow: {}", e);
            // Don't fail the request, event is already persisted
        } else {
            tracing::info!(session_id = %session_id, "Session workflow started");
        }
    }

    Ok((StatusCode::CREATED, Json(event)))
}

/// GET /v1/harnesses/{harness_id}/sessions/{session_id}/events - Stream events (SSE)
#[utoipa::path(
    get,
    path = "/v1/harnesses/{harness_id}/sessions/{session_id}/events",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID"),
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
    Path((_harness_id, session_id)): Path<(Uuid, Uuid)>,
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

/// GET /v1/harnesses/{harness_id}/sessions/{session_id}/messages - Get message events only
#[utoipa::path(
    get,
    path = "/v1/harnesses/{harness_id}/sessions/{session_id}/messages",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "List of message events", body = ListResponse<Event>),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "events"
)]
pub async fn list_messages(
    State(state): State<AppState>,
    Path((_harness_id, session_id)): Path<(Uuid, Uuid)>,
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

    let events = state
        .event_service
        .list_messages(session_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list message events: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(ListResponse::new(events)))
}
