// Message service for business logic
//
// Messages are stored as events in the events table. This service handles:
// - Creating user message events
// - Listing messages by querying message events
// - Workflow triggering for user messages

use crate::api::messages::{ContentPart, CreateMessageRequest, Message, MessageRole};
use crate::storage::{Database, EventService};
use anyhow::Result;
use chrono::Utc;
use everruns_core::events::{EventContext, EventRequest, MessageUserData};
use everruns_core::Event;
use everruns_worker::AgentRunner;
use std::sync::Arc;
use uuid::Uuid;

pub struct MessageService {
    db: Arc<Database>,
    event_service: EventService,
    runner: Arc<dyn AgentRunner>,
}

impl MessageService {
    pub fn new(db: Arc<Database>, runner: Arc<dyn AgentRunner>) -> Self {
        let event_service = EventService::new(db.clone());
        Self {
            db,
            event_service,
            runner,
        }
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
        let now = Utc::now();

        // Build the core message
        let core_message = everruns_core::Message {
            id: message_id,
            role: everruns_core::MessageRole::User,
            content: content.clone(),
            controls: req.controls.clone(),
            metadata: req.metadata.clone(),
            created_at: now,
        };

        // Emit as typed event using EventService
        let stored_event = self
            .event_service
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                MessageUserData::new(core_message),
            ))
            .await?;

        // Construct API Message
        let message = Message {
            id: message_id,
            session_id,
            sequence: stored_event.sequence.unwrap_or(0),
            role: MessageRole::User,
            content,
            controls: req.controls,
            metadata: req.metadata,
            created_at: now,
        };

        // Start workflow for user message in background (don't block the response)
        // The message is already persisted, so we can return immediately
        let runner = self.runner.clone();
        tokio::spawn(async move {
            if let Err(e) = runner.start_run(session_id, agent_id, message_id).await {
                tracing::error!(
                    session_id = %session_id,
                    input_message_id = %message_id,
                    error = %e,
                    "Failed to start turn workflow"
                );
            } else {
                tracing::info!(
                    session_id = %session_id,
                    input_message_id = %message_id,
                    "Turn workflow started"
                );
            }
        });

        Ok(message)
    }

    pub async fn list(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let events = self.db.list_message_events(session_id).await?;
        let mut messages = Vec::with_capacity(events.len());

        for event_row in events {
            match Self::event_to_message(session_id, &event_row.data, event_row.sequence) {
                Ok(message) => messages.push(message),
                Err(e) => {
                    tracing::warn!("Failed to parse message from event {}: {}", event_row.id, e);
                }
            }
        }

        Ok(messages)
    }

    /// Convert stored event data to API Message
    ///
    /// Events are stored as full Event structures with typed EventData.
    fn event_to_message(
        session_id: Uuid,
        data: &serde_json::Value,
        sequence: i32,
    ) -> std::result::Result<Message, String> {
        // Deserialize the full Event structure
        let event: Event =
            serde_json::from_value(data.clone()).map_err(|e| format!("invalid event: {}", e))?;

        // Extract message from typed EventData
        let core_message = match &event.data {
            everruns_core::EventData::MessageUser(data) => &data.message,
            everruns_core::EventData::MessageAgent(data) => &data.message,
            everruns_core::EventData::ToolCallCompleted(data) => {
                // Convert tool result to message
                let result: Option<serde_json::Value> = data.result.as_ref().map(|parts| {
                    if parts.len() == 1 {
                        if let everruns_core::ContentPart::Text(t) = &parts[0] {
                            return serde_json::Value::String(t.text.clone());
                        }
                    }
                    serde_json::to_value(parts).unwrap_or_default()
                });
                let msg = everruns_core::Message::tool_result(
                    &data.tool_call_id,
                    result,
                    data.error.clone(),
                );
                return Ok(Message {
                    id: msg.id,
                    session_id,
                    sequence,
                    role: MessageRole::from(msg.role.to_string().as_str()),
                    content: msg.content,
                    controls: None,
                    metadata: None,
                    created_at: msg.created_at,
                });
            }
            _ => return Err(format!("unexpected event type: {}", event.event_type)),
        };

        Ok(Message {
            id: core_message.id,
            session_id,
            sequence,
            role: MessageRole::from(core_message.role.to_string().as_str()),
            content: core_message.content.clone(),
            controls: core_message.controls.clone(),
            metadata: core_message.metadata.clone(),
            created_at: core_message.created_at,
        })
    }
}
