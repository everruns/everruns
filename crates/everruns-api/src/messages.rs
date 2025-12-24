// Message HTTP routes and API contracts
// Messages are PRIMARY data store, Events are SSE notifications

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::sse::{Event as SseEvent, KeepAlive, Sse},
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use everruns_contracts::ListResponse;
use everruns_storage::{models::CreateEvent, Database};
use everruns_worker::AgentRunner;
use futures::{
    stream::{self, Stream},
    StreamExt,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, convert::Infallible, sync::Arc, time::Duration};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::services::{EventService, MessageService, SessionService};

// ============================================
// Message API Contracts
// ============================================

/// Message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    ToolCall,
    ToolResult,
    System,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::ToolCall => write!(f, "tool_call"),
            MessageRole::ToolResult => write!(f, "tool_result"),
            MessageRole::System => write!(f, "system"),
        }
    }
}

impl From<&str> for MessageRole {
    fn from(s: &str) -> Self {
        match s {
            "assistant" => MessageRole::Assistant,
            "tool_call" => MessageRole::ToolCall,
            "tool_result" => MessageRole::ToolResult,
            "system" => MessageRole::System,
            _ => MessageRole::User,
        }
    }
}

/// A part of message content - can be text, image, tool_call, or tool_result
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Text content
    Text { text: String },
    /// Image content (base64 or URL)
    Image {
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base64: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
    /// Tool call content (assistant requesting tool execution)
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// Tool result content (result of tool execution)
    ToolResult {
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

impl ContentPart {
    /// Create a text content part
    #[allow(dead_code)]
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text { text: text.into() }
    }

    /// Get text if this is a text part
    #[allow(dead_code)]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentPart::Text { text } => Some(text),
            _ => None,
        }
    }
}

/// Reasoning configuration for the model
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReasoningConfig {
    /// Effort level for reasoning (low, medium, high)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// Runtime controls for message processing
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct Controls {
    /// Model ID to use for this message (format: "provider/model-name")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,

    /// Reasoning configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,

    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,

    /// Temperature for generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

/// Message - primary conversation data (API response)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Message {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub role: MessageRole,
    /// Array of content parts
    pub content: Vec<ContentPart>,
    /// Message-level metadata (locale, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Message input for creating a message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageInput {
    /// Message role
    pub role: MessageRole,
    /// Array of content parts
    pub content: Vec<ContentPart>,
    /// Message-level metadata (locale, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Request to create a message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateMessageRequest {
    /// The message to create
    pub message: MessageInput,
    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    /// Request-level metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Tags for the message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl CreateMessageRequest {
    /// Create a simple text message request
    #[allow(dead_code)]
    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            message: MessageInput {
                role,
                content: vec![ContentPart::text(text)],
                metadata: None,
            },
            controls: None,
            metadata: None,
            tags: None,
        }
    }

    /// Create a user message with text
    #[allow(dead_code)]
    pub fn user(text: impl Into<String>) -> Self {
        Self::text(MessageRole::User, text)
    }
}

// ============================================
// App State and Routes
// ============================================

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

// ============================================
// HTTP Handlers
// ============================================

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
        tool_call_id: None, // Tool call ID is derived from content for tool_result messages
    };

    let message = state.message_service.create(input).await.map_err(|e| {
        tracing::error!("Failed to create message: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // If this is a user message, start the session workflow
    if message.role == MessageRole::User {
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

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_part_text_serialization() {
        let part = ContentPart::text("Hello, world!");
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains(r#""text":"Hello, world!""#));
    }

    #[test]
    fn test_content_part_deserialization() {
        let json = r#"{"type":"text","text":"Hello!"}"#;
        let part: ContentPart = serde_json::from_str(json).unwrap();
        assert_eq!(part.as_text(), Some("Hello!"));
    }

    #[test]
    fn test_create_message_request_user() {
        let req = CreateMessageRequest::user("Hello, how are you?");
        assert_eq!(req.message.role, MessageRole::User);
        assert_eq!(req.message.content.len(), 1);
        assert_eq!(
            req.message.content[0].as_text(),
            Some("Hello, how are you?")
        );
    }

    #[test]
    fn test_message_role_display() {
        assert_eq!(MessageRole::User.to_string(), "user");
        assert_eq!(MessageRole::Assistant.to_string(), "assistant");
        assert_eq!(MessageRole::ToolCall.to_string(), "tool_call");
        assert_eq!(MessageRole::ToolResult.to_string(), "tool_result");
        assert_eq!(MessageRole::System.to_string(), "system");
    }

    #[test]
    fn test_message_role_from_str() {
        assert_eq!(MessageRole::from("user"), MessageRole::User);
        assert_eq!(MessageRole::from("assistant"), MessageRole::Assistant);
        assert_eq!(MessageRole::from("tool_call"), MessageRole::ToolCall);
        assert_eq!(MessageRole::from("unknown"), MessageRole::User);
    }
}
