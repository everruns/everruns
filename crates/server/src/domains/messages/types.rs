// Messages domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{MessageId, SessionId, SessionParticipantId};
use everruns_contracts::{ExecutionPhase, PhaseSource};
use everruns_core::{ContentPart, Controls, InputContentPart};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

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

/// How a created user message reached the session's turn.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageDelivery {
    /// The message started a new turn.
    Started,
    /// A turn was already running; the message joins it at its next step
    /// (or starts a follow-up turn if that turn had already finished).
    Steered,
    /// The message resolved a turn parked on client tool results.
    Resumed,
    /// The same `client_message_id` was already stored; nothing new started.
    Duplicate,
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
    /// Array of content parts.
    ///
    /// Reasoning artifacts appear here as `reasoning` parts, in the order the
    /// provider emitted them, with opaque replay state (signatures, encrypted
    /// payloads) stripped.
    pub content: Vec<ContentPart>,
    /// Execution phase for agent messages: whether this is intermediate
    /// `commentary` or the turn's `final_answer`. Absent on user messages.
    ///
    /// Without this a client cannot tell an intermediate message from the
    /// answer, which is the single question most consumers of a session need
    /// answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<ExecutionPhase>,
    /// Whether `phase` was reported by the provider or inferred by the runtime
    /// from tool-call presence.
    ///
    /// `derived` is a weak signal: it means only "this message called tools",
    /// so a text-only preamble is reported as `final_answer`. Clients needing a
    /// dependable classification should treat `derived` accordingly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_source: Option<PhaseSource>,
    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    /// Message-level metadata (locale, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// External actor identity (for messages from external channels like Slack)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_actor: Option<everruns_core::ExternalActor>,
    /// How a message just created was delivered. Only set on the response to
    /// creating a message; absent when listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<MessageDelivery>,
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
    /// Optional active agent participant to address for this turn. When omitted,
    /// the session host remains the responder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(
        value_type = Option<String>,
        example = "part_01933b5a00007000800000000000001"
    )]
    pub addressed_participant_id: Option<SessionParticipantId>,
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
    /// Client-minted id for this send (a UUID). A repeat of the same id in the
    /// same session returns the stored message instead of starting another
    /// turn, so a client can retry a send whose response it never saw. The
    /// stored message echoes it as `everruns_client_message_id` metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "0199d0f2-6c1e-7a3b-9f00-1a2b3c4d5e6f")]
    pub client_message_id: Option<uuid::Uuid>,
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
            addressed_participant_id: None,
            controls: None,
            metadata: None,
            tags: None,
            external_actor: None,
            client_message_id: None,
        }
    }
}
