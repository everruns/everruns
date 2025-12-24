// Session CRUD, Messages, and Events HTTP routes (M2)
// Messages are PRIMARY data store, Events are SSE notifications

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use everruns_contracts::{
    CreateMessageRequest, CreateSessionRequest, ListResponse, Message, Session,
    UpdateSessionRequest,
};
use everruns_storage::{
    models::{CreateEvent, CreateMessage, CreateSession, UpdateSession},
    Database,
};
use everruns_worker::AgentRunner;
use futures::{
    stream::{self, Stream},
    StreamExt,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use uuid::Uuid;

use crate::services::{EventService, MessageService, SessionService};

/// App state for sessions routes
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub runner: Arc<dyn AgentRunner>,
    pub db: Arc<Database>,
}

impl AppState {
    pub fn new(db: Arc<Database>, runner: Arc<dyn AgentRunner>) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(db.clone())),
            event_service: Arc::new(EventService::new(db.clone())),
            runner,
            db,
        }
    }
}

/// Create session routes (nested under agents)
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Session CRUD under agent
        .route(
            "/v1/agents/:agent_id/sessions",
            post(create_session).get(list_sessions),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        // Messages under session (PRIMARY data)
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/messages",
            post(create_message).get(list_messages),
        )
        // Events under session (SSE notifications)
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/events",
            get(stream_events),
        )
        .with_state(state)
}

/// POST /v1/agents/{agent_id}/sessions - Create a new session
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
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
    Path(agent_id): Path<Uuid>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), StatusCode> {
    let input = CreateSession {
        agent_id,
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

/// GET /v1/agents/{agent_id}/sessions - List sessions in agent
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "List of sessions", body = ListResponse<Session>),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_sessions(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<ListResponse<Session>>, StatusCode> {
    let sessions = state.session_service.list(agent_id).await.map_err(|e| {
        tracing::error!("Failed to list sessions: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ListResponse::new(sessions)))
}

/// GET /v1/agents/{agent_id}/sessions/{session_id} - Get session
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
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
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
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

/// PATCH /v1/agents/{agent_id}/sessions/{session_id} - Update session
#[utoipa::path(
    patch,
    path = "/v1/agents/{agent_id}/sessions/{session_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
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
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<Session>, StatusCode> {
    let input = UpdateSession {
        title: req.title,
        tags: req.tags,
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

/// DELETE /v1/agents/{agent_id}/sessions/{session_id} - Delete session
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/sessions/{session_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
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
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
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

/// POST /v1/agents/{agent_id}/sessions/{session_id}/messages - Create message (user message triggers workflow)
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/messages",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = CreateMessageRequest,
    responses(
        (status = 201, description = "Message created successfully", body = Message),
        (status = 500, description = "Internal server error")
    ),
    tag = "messages"
)]
pub async fn create_message(
    State(state): State<AppState>,
    Path((agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CreateMessageRequest>,
) -> Result<(StatusCode, Json<Message>), StatusCode> {
    use crate::services::message::content_parts_to_json;

    // Convert ContentPart array to JSON for storage
    let content = content_parts_to_json(&req.message.role, &req.message.content);

    // Build metadata: merge message metadata with controls
    // Controls are stored as __controls key so they can be read by the workflow
    let mut metadata_map = serde_json::Map::new();

    // Add message metadata if present
    if let Some(msg_metadata) = req.message.metadata {
        for (k, v) in msg_metadata {
            metadata_map.insert(k, v);
        }
    }

    // Add controls if present (stored under __controls key)
    if let Some(controls) = &req.controls {
        if let Ok(controls_value) = serde_json::to_value(controls) {
            metadata_map.insert("__controls".to_string(), controls_value);
        }
    }

    // Convert to Option<Value> - only Some if we have data
    let metadata = if metadata_map.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(metadata_map))
    };

    // Get tags from request (empty if not provided)
    let tags = req.tags.unwrap_or_default();

    let input = CreateMessage {
        session_id,
        role: req.message.role.to_string(),
        content,
        metadata,
        tags,
        tool_call_id: req.message.tool_call_id,
    };

    let message = state.message_service.create(input).await.map_err(|e| {
        tracing::error!("Failed to create message: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // If this is a user message, start the session workflow
    if message.role == everruns_contracts::MessageRole::User {
        // Emit a user message event for SSE
        let event_input = CreateEvent {
            session_id,
            event_type: "message.user".to_string(),
            data: serde_json::json!({
                "message_id": message.id,
                "content": message.content
            }),
        };
        if let Err(e) = state.event_service.create(event_input).await {
            tracing::warn!("Failed to emit user message event: {}", e);
        }

        // Start the workflow execution
        if let Err(e) = state
            .runner
            .start_run(session_id, agent_id, session_id)
            .await
        {
            tracing::error!("Failed to start session workflow: {}", e);
            // Don't fail the request, message is already persisted
        } else {
            tracing::info!(session_id = %session_id, "Session workflow started");
        }
    }

    Ok((StatusCode::CREATED, Json(message)))
}

/// GET /v1/agents/{agent_id}/sessions/{session_id}/messages - List messages (PRIMARY data)
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/messages",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "List of messages", body = ListResponse<Message>),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "messages"
)]
pub async fn list_messages(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<ListResponse<Message>>, StatusCode> {
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

    let messages = state.message_service.list(session_id).await.map_err(|e| {
        tracing::error!("Failed to list messages: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ListResponse::new(messages)))
}

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
