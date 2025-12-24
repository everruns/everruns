// Message HTTP routes
// Messages are the PRIMARY data store for agent conversations

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use everruns_contracts::{CreateMessageRequest, ListResponse, Message};
use everruns_storage::{models::CreateMessage, Database};
use everruns_worker::AgentRunner;
use std::sync::Arc;
use uuid::Uuid;

use crate::services::{EventService, MessageService, SessionService};

/// App state for messages routes
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub event_service: Arc<EventService>,
    pub runner: Arc<dyn AgentRunner>,
}

impl AppState {
    pub fn new(db: Arc<Database>, runner: Arc<dyn AgentRunner>) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(db.clone())),
            event_service: Arc::new(EventService::new(db)),
            runner,
        }
    }
}

/// Create message routes (nested under agents/sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/messages",
            post(create_message).get(list_messages),
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
    use everruns_storage::models::CreateEvent;

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
