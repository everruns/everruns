// Message HTTP routes
// Messages are PRIMARY data store, Events are SSE notifications

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use everruns_contracts::{CreateMessageRequest, ListResponse, Message};
use everruns_storage::{models::CreateEvent, Database};
use everruns_worker::AgentRunner;
use futures::{
    stream::{self, Stream},
    StreamExt,
};
use std::{convert::Infallible, sync::Arc, time::Duration};
use uuid::Uuid;

use crate::services::{EventService, MessageService, SessionService};

/// App state for messages routes
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

/// Create message routes (nested under sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Messages under session (PRIMARY data)
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/messages",
            axum::routing::post(create_message).get(list_messages),
        )
        // Events under session (SSE notifications)
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/events",
            get(stream_events),
        )
        .with_state(state)
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
    use everruns_storage::models::CreateMessage;

    // Convert ContentPart array to JSON for storage
    let content = content_parts_to_json(&req.message.role, &req.message.content);

    // Convert message metadata to JSON
    let metadata = req
        .message
        .metadata
        .and_then(|m| serde_json::to_value(m).ok());

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
