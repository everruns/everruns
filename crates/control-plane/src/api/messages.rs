// Message HTTP routes and API contracts
//
// BREAKING CHANGE: Simplified message roles to just `user` and `agent`.
// - Tool results are conveyed via `tool.call_completed` events
// - System messages are internal and not exposed via API
//
// ContentPart and InputContentPart are defined in everruns-core.
// We re-export them here with ToSchema for OpenAPI documentation.

use crate::storage::Database;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Utc};

use super::common::ListResponse;
use everruns_worker::AgentRunner;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::services::{MessageService, SessionService};

// Re-export core types with ToSchema for OpenAPI
#[allow(unused_imports)]
pub use everruns_core::{
    ContentPart, ContentType, Controls, ImageContentPart, InputContentPart, ReasoningConfig,
    TextContentPart, ToolCallContentPart, ToolResultContentPart,
};

// ============================================
// Message API Contracts
// ============================================

/// Message role (API layer)
///
/// Simplified to only user and agent messages.
/// Tool results are conveyed via `tool.call_completed` events.
/// System messages are internal and not exposed via API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// User message (input from the user)
    User,
    /// Agent message (response from the AI agent)
    Agent,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Agent => write!(f, "agent"),
        }
    }
}

impl From<&str> for MessageRole {
    fn from(s: &str) -> Self {
        match s {
            // Map both "agent" and legacy "assistant" to Agent role
            "agent" | "assistant" => MessageRole::Agent,
            _ => MessageRole::User,
        }
    }
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
    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    /// Message-level metadata (locale, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    pub created_at: DateTime<Utc>,
}

/// Input message for creating a user message
///
/// Only user messages can be created via the API.
/// Agent messages are created internally by the workflow.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InputMessage {
    /// Message role (always "user" for API-created messages)
    #[serde(default = "default_user_role")]
    pub role: MessageRole,
    /// Array of content parts (text and image only)
    pub content: Vec<InputContentPart>,
}

fn default_user_role() -> MessageRole {
    MessageRole::User
}

/// Request to create a message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateMessageRequest {
    /// The message to create
    pub message: InputMessage,
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

#[cfg(test)]
impl CreateMessageRequest {
    /// Create a user message with text (for tests)
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            message: InputMessage {
                role: MessageRole::User,
                content: vec![InputContentPart::text(text)],
            },
            controls: None,
            metadata: None,
            tags: None,
        }
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
}

impl AppState {
    pub fn new(db: Arc<Database>, runner: Arc<dyn AgentRunner>) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(db, runner)),
        }
    }
}

/// Create message routes (nested under sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/messages",
            post(create_message).get(list_messages),
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
    let message = state
        .message_service
        .create(agent_id, session_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create message: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

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
        assert_eq!(req.message.content.len(), 1);
        assert_eq!(
            req.message.content[0].as_text(),
            Some("Hello, how are you?")
        );
    }

    #[test]
    fn test_message_role_display() {
        assert_eq!(MessageRole::User.to_string(), "user");
        assert_eq!(MessageRole::Agent.to_string(), "agent");
    }

    #[test]
    fn test_message_role_from_str() {
        assert_eq!(MessageRole::from("user"), MessageRole::User);
        assert_eq!(MessageRole::from("agent"), MessageRole::Agent);
        // Legacy "assistant" maps to Agent
        assert_eq!(MessageRole::from("assistant"), MessageRole::Agent);
        // Unknown roles default to User
        assert_eq!(MessageRole::from("unknown"), MessageRole::User);
    }
}
