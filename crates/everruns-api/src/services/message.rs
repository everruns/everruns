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
    /// - If session is running, cancels the current workflow (interruption)
    /// - Creates a message event in the events table
    /// - Triggers workflow execution for the session
    pub async fn create(
        &self,
        agent_id: Uuid,
        session_id: Uuid,
        req: CreateMessageRequest,
    ) -> Result<Message> {
        // Check if session is currently running and cancel if needed
        if let Ok(Some(session)) = self.db.get_session(session_id).await {
            if session.status == "running" {
                tracing::info!(
                    session_id = %session_id,
                    "Session is running, cancelling before processing new message"
                );
                // Cancel the running workflow
                if let Err(e) = self.runner.cancel_run(session_id).await {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %e,
                        "Failed to cancel running workflow (continuing with message)"
                    );
                }
                // Create interruption event
                let interrupt_message_id = Uuid::now_v7();
                let interrupt_event = CreateEventRow {
                    session_id,
                    event_type: "session.interrupted".to_string(),
                    data: serde_json::json!({
                        "message_id": interrupt_message_id,
                        "role": "system",
                        "content": [{
                            "type": "text",
                            "text": "Execution interrupted by new message."
                        }]
                    }),
                };
                if let Err(e) = self.db.create_event(interrupt_event).await {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %e,
                        "Failed to create interruption event"
                    );
                }
            }
        }

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

        // Create the event
        let event = self
            .db
            .create_event(CreateEventRow {
                session_id,
                event_type: "message.user".to_string(),
                data: serde_json::json!({
                    "message_id": message_id,
                    "role": "user",
                    "content": &content,
                    "controls": &req.controls,
                    "metadata": &req.metadata,
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

        // Start workflow for user message
        self.start_workflow(agent_id, session_id).await;

        Ok(message)
    }

    /// Start workflow execution for the session
    async fn start_workflow(&self, agent_id: Uuid, session_id: Uuid) {
        if let Err(e) = self
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
    fn event_to_message(
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
