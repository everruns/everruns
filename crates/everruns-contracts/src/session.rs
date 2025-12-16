// Session DTOs for public API (M2)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Session represents an instance of agentic loop execution
/// Multiple sessions can exist concurrently for a single harness
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Session {
    pub id: Uuid,
    pub harness_id: Uuid,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub model_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Request to create a session
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub model_id: Option<Uuid>,
}

/// Request to update a session
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateSessionRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub model_id: Option<Uuid>,
}

/// Event in a session
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Event {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub event_type: String,
    pub data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Request to create an event (add a message to session)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateEventRequest {
    pub event_type: String,
    pub data: serde_json::Value,
}

/// Message content in an event
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageContent {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

/// Message structure in event data
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageData {
    pub role: String,
    pub content: Vec<MessageContent>,
}

/// Event data wrapper for message events
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageEventData {
    pub message: MessageData,
}

/// Session action
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionAction {
    pub id: Uuid,
    pub session_id: Uuid,
    pub kind: String,
    pub payload: serde_json::Value,
    pub by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}
