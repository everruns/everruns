// Message HTTP routes and API contracts
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
//
// BREAKING CHANGE: Simplified message roles to just `user` and `agent`.
// - Tool results are conveyed via `tool.completed` events
// - System messages are internal and not exposed via API
//
// ContentPart and InputContentPart are defined in everruns-core.
// We re-export them here with ToSchema for OpenAPI documentation.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::messages::{CreateMessage, ExportSessionMessages, ListMessages};
use crate::middleware::RequestId;
use crate::storage::StorageBackend;
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use everruns_core::typed_id::{MessageId, SessionId};

use super::common::{ApiResult, ErrorResponse, ListResponse, impl_auth_state};
use everruns_worker::AgentRunner;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;

use everruns_core::Caller;

use crate::domains::messages::MessageService;
use crate::domains::sessions::SessionService;

// Re-export core types with ToSchema for OpenAPI
#[allow(unused_imports)]
pub use everruns_core::{
    ContentPart, ContentType, Controls, ImageContentPart, ImageFileContentPart, InputContentPart,
    ReasoningConfig, TextContentPart, ToolCallContentPart, ToolResultContentPart,
};

// ============================================
// Message API Contracts
// ============================================

/// Message role (API layer)
///
/// Simplified to only user and agent messages.
/// Tool results are conveyed via `tool.completed` events.
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
    /// Unique message ID (format: message_{32-hex})
    #[schema(value_type = String, example = "message_01933b5a00007000800000000000001")]
    pub id: MessageId,
    /// Session ID this message belongs to (format: session_{32-hex})
    #[schema(value_type = String, example = "session_01933b5a00007000800000000000001")]
    pub session_id: SessionId,
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
    /// External actor identity (for messages from external channels like Slack)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_actor: Option<everruns_core::ExternalActor>,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
}

/// Input message for creating a user message
///
/// Only user messages can be created via the API.
/// Agent messages are created internally by the workflow.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[schema(example = json!({"role": "user", "content": [{"type": "text", "text": "Why is the build failing on main?"}]}))]
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
    /// The message to create. Example shape is defined on `InputMessage`.
    pub message: InputMessage,
    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    /// Request-level metadata. Arbitrary key/value pairs persisted with the message
    /// for downstream filtering and analytics. Not interpreted by the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = json!({"source": "slack", "thread_ts": "1715000000.123456"}))]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Tags for the message. Free-form labels used for grouping and filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["bug-report", "from-slack"]))]
    pub tags: Option<Vec<String>>,
    /// External actor identity (for messages from external channels like Slack)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_actor: Option<everruns_core::ExternalActor>,
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
            external_actor: None,
        }
    }
}

// ============================================
// App State and Routes
// ============================================

/// App state for messages routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub message_service: Arc<MessageService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        auth: AuthState,
        notifications_enabled: bool,
        event_delivery: crate::event_delivery::EventDelivery,
    ) -> Self {
        Self {
            db: db.clone(),
            session_service: Arc::new(SessionService::new(db.clone())),
            message_service: Arc::new(MessageService::new(
                db,
                runner,
                notifications_enabled,
                event_delivery,
            )),
            auth,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_session_service(self.session_service.clone())
        .with_message_service(self.message_service.clone())
    }
}

impl_auth_state!(AppState);

/// Create message routes (nested under sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/messages",
            post(create_message).get(list_messages),
        )
        .route(
            "/v1/sessions/{session_id}/export",
            get(export_session_jsonl),
        )
        .with_state(state)
}

// ============================================
// HTTP Handlers
// ============================================

/// POST /v1/sessions/{session_id}/messages - Create message (user message triggers workflow)
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/messages",
    extensions(
        ("x-cost-tier" = json!("paid")),
    ),
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)")
    ),
    request_body = CreateMessageRequest,
    responses(
        (status = 201, description = "Message created successfully", body = Message),
        (status = 400, description = "Invalid ID format"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "messages"
)]
pub async fn create_message(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    req_id: Option<Extension<RequestId>>,
    Json(req): Json<CreateMessageRequest>,
) -> Result<(StatusCode, Json<Message>), (StatusCode, Json<ErrorResponse>)> {
    let request_id = req_id.map(|Extension(r)| r.0);
    let message = CreateMessage {
        session_id,
        message: req.message,
        controls: req.controls,
        metadata: req.metadata,
        tags: req.tags,
        external_actor: req.external_actor,
        request_id,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok((StatusCode::CREATED, Json(message)))
}

/// GET /v1/sessions/{session_id}/messages - List messages (PRIMARY data)
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/messages",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)")
    ),
    responses(
        (status = 200, description = "List of messages", body = ListResponse<Message>),
        (status = 400, description = "Invalid ID format"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "messages"
)]
pub async fn list_messages(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<ListResponse<Message>> {
    let messages = ListMessages {
        session_id,
        limit: None,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(ListResponse::new(messages)))
}

/// Export session messages as a JSONL file
///
/// Returns all materialized messages (user, agent) as newline-delimited JSON.
/// Delta events are excluded. Each line is a complete JSON object representing one message.
/// The response includes `Content-Disposition: attachment` for browser download.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/export",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "JSONL file with one message per line", content_type = "application/x-ndjson"),
        (status = 400, description = "Invalid ID format"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn export_session_jsonl(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let export = ExportSessionMessages {
        session_id: session_id.clone(),
    }
    .run(&state.ctx(&org))
    .await?;
    let filename = format!("{}.jsonl", session_id);
    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/x-ndjson".to_string(),
            ),
            (
                axum::http::header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            ),
        ],
        export.body,
    ))
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
