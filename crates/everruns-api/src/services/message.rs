// Message service for business logic
//
// Messages are stored as events in the events table. This service handles:
// - Creating user message events
// - Listing messages by querying message events
// - Workflow triggering for user messages

use crate::messages::{ContentPart, CreateMessageRequest, Message, MessageRole};
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_core::Controls;
use everruns_storage::{models::CreateEventRow, Database};
use everruns_worker::AgentRunner;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

pub struct MessageService {
    db: Arc<Database>,
    runner: Arc<dyn AgentRunner>,
}

impl MessageService {
    pub fn new(db: Arc<Database>, runner: Arc<dyn AgentRunner>) -> Self {
        Self { db, runner }
    }

    /// Create a user message from API request
    ///
    /// Only user messages can be created via the API. This method:
    /// - Creates a message event in the events table
    /// - Triggers workflow execution for the session
    pub async fn create(
        &self,
        agent_id: Uuid,
        session_id: Uuid,
        req: CreateMessageRequest,
    ) -> Result<Message> {
        // Convert InputContentPart array to ContentPart array
        let content: Vec<ContentPart> = req
            .message
            .content
            .into_iter()
            .map(ContentPart::from)
            .collect();

        // Generate a new message ID
        let message_id = Uuid::now_v7();
        let tags = req.tags.unwrap_or_default();

        // Create the event with data matching MessageUserData structure
        // This ensures consistency with events emitted by atoms
        let event = self
            .db
            .create_event(CreateEventRow {
                session_id,
                event_type: "message.user".to_string(),
                data: serde_json::json!({
                    "message": {
                        "id": message_id,
                        "role": "user",
                        "content": &content,
                        "controls": &req.controls,
                        "metadata": &req.metadata,
                        "created_at": chrono::Utc::now(),
                    },
                    "tags": &tags,
                }),
            })
            .await?;

        // Construct Message from the event
        let message = Message {
            id: message_id,
            session_id,
            sequence: event.sequence,
            role: MessageRole::User,
            content,
            controls: req.controls,
            metadata: req.metadata,
            created_at: event.created_at,
        };

        // Start workflow for user message (pass the message_id that triggered this turn)
        self.start_workflow(agent_id, session_id, message_id).await;

        Ok(message)
    }

    /// Start turn workflow for the session
    async fn start_workflow(&self, agent_id: Uuid, session_id: Uuid, input_message_id: Uuid) {
        if let Err(e) = self
            .runner
            .start_run(session_id, agent_id, input_message_id)
            .await
        {
            tracing::error!("Failed to start turn workflow: {}", e);
            // Don't fail the request, message is already persisted
        } else {
            tracing::info!(
                session_id = %session_id,
                input_message_id = %input_message_id,
                "Turn workflow started"
            );
        }
    }

    pub async fn list(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let events = self.db.list_message_events(session_id).await?;
        let mut messages = Vec::with_capacity(events.len());

        for event in events {
            match Self::event_to_message(session_id, &event.data, event.sequence, event.created_at)
            {
                Ok(message) => messages.push(message),
                Err(e) => {
                    tracing::warn!("Failed to parse message from event {}: {}", event.id, e);
                }
            }
        }

        Ok(messages)
    }

    /// Convert event data to API Message
    ///
    /// Handles both formats:
    /// - New format: { "message": { "id", "role", "content", ... } }
    /// - Legacy format: { "message_id", "role", "content", ... }
    fn event_to_message(
        session_id: Uuid,
        data: &serde_json::Value,
        sequence: i32,
        created_at: DateTime<Utc>,
    ) -> std::result::Result<Message, String> {
        // Try new format first (message wrapper)
        if let Some(message_obj) = data.get("message") {
            return Self::parse_message_object(session_id, message_obj, sequence, created_at);
        }

        // Fall back to legacy format (flat structure)
        Self::parse_legacy_format(session_id, data, sequence, created_at)
    }

    /// Parse message from new format with message wrapper
    fn parse_message_object(
        session_id: Uuid,
        message: &serde_json::Value,
        sequence: i32,
        created_at: DateTime<Utc>,
    ) -> std::result::Result<Message, String> {
        // Extract id (can be string or object with UUID)
        let id = message
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("missing message.id")?
            .parse::<Uuid>()
            .map_err(|e| format!("invalid message.id: {}", e))?;

        // Extract role
        let role_str = message
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or("missing message.role")?;
        let role = MessageRole::from(role_str);

        // Extract content
        let content: Vec<ContentPart> = message
            .get("content")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();

        // Extract optional controls
        let controls: Option<Controls> = message
            .get("controls")
            .filter(|v| !v.is_null())
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok());

        // Extract optional metadata
        let metadata: Option<HashMap<String, serde_json::Value>> = message
            .get("metadata")
            .filter(|v| !v.is_null())
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok());

        // Use created_at from message if present, otherwise from event
        let msg_created_at = message
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .unwrap_or(created_at);

        Ok(Message {
            id,
            session_id,
            sequence,
            role,
            content,
            controls,
            metadata,
            created_at: msg_created_at,
        })
    }

    /// Parse message from legacy format (flat structure)
    fn parse_legacy_format(
        session_id: Uuid,
        data: &serde_json::Value,
        sequence: i32,
        created_at: DateTime<Utc>,
    ) -> std::result::Result<Message, String> {
        // Extract message_id
        let id = data
            .get("message_id")
            .and_then(|v| v.as_str())
            .ok_or("missing message_id")?
            .parse::<Uuid>()
            .map_err(|e| format!("invalid message_id: {}", e))?;

        // Extract role
        let role_str = data
            .get("role")
            .and_then(|v| v.as_str())
            .ok_or("missing role")?;
        let role = MessageRole::from(role_str);

        // Extract content
        let content: Vec<ContentPart> = data
            .get("content")
            .cloned()
            .map(|v| serde_json::from_value(v).unwrap_or_default())
            .unwrap_or_default();

        // Extract optional controls
        let controls: Option<Controls> = data
            .get("controls")
            .filter(|v| !v.is_null())
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok());

        // Extract optional metadata
        let metadata: Option<HashMap<String, serde_json::Value>> = data
            .get("metadata")
            .filter(|v| !v.is_null())
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok());

        Ok(Message {
            id,
            session_id,
            sequence,
            role,
            content,
            controls,
            metadata,
            created_at,
        })
    }
}
