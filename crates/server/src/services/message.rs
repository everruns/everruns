// Message service for business logic
//
// Messages are stored as events in the events table. This service handles:
// - Creating user message events
// - Listing messages by querying message events
// - Workflow triggering for user messages

use super::EventService;
use super::NotificationService;
use crate::api::messages::{ContentPart, CreateMessageRequest, Message, MessageRole};
use crate::execution_metadata;
use crate::storage::StorageBackend;
use anyhow::Result;
use chrono::Utc;
use everruns_core::Event;
use everruns_core::events::{
    EventContext, EventRequest, InputMessageData, OutputMessageCompletedData, ToolCompletedData,
};
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_worker::AgentRunner;
use std::sync::Arc;
use uuid::Uuid;

pub struct MessageService {
    db: Arc<StorageBackend>,
    event_service: EventService,
    notification_service: NotificationService,
    notifications_enabled: bool,
    runner: Arc<dyn AgentRunner>,
}

impl MessageService {
    pub fn new(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        notifications_enabled: bool,
    ) -> Self {
        let event_service = EventService::new(db.clone());
        let notification_service = NotificationService::new(db.clone());
        Self {
            db,
            event_service,
            notification_service,
            notifications_enabled,
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
        org_id: i64,
        user_id: Option<Uuid>,
        harness_id: Uuid,
        agent_id: Option<Uuid>,
        session_id: Uuid,
        req: CreateMessageRequest,
        event_metadata: Option<serde_json::Value>,
    ) -> Result<Message> {
        tracing::info!(
            session_id = %session_id,
            harness_id = %harness_id,
            agent_id = ?agent_id,
            "Creating user message"
        );

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
            id: message_id.into(),
            role: everruns_core::MessageRole::User,
            content: content.clone(),
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: req.controls.clone(),
            metadata: req.metadata.clone(),
            external_actor: req.external_actor.clone(),
            created_at: now,
        };

        // Convert to typed IDs for emit/runner
        let session_id_typed = SessionId::from_uuid(session_id);
        let harness_id_typed = HarnessId::from_uuid(harness_id);
        let agent_id_typed = agent_id.map(AgentId::from_uuid);
        let message_id_typed = MessageId::from_uuid(message_id);

        // Emit as typed event using EventService
        let event_metadata =
            event_metadata.or_else(|| execution_metadata::interactive_user_metadata(user_id));
        let mut event_request = EventRequest::new(
            session_id_typed,
            EventContext::empty(),
            InputMessageData::new(core_message),
        );
        if let Some(metadata) = event_metadata {
            event_request = event_request.with_metadata(metadata);
        }
        let stored_event = self.event_service.emit(event_request).await?;

        // Construct API Message
        let message = Message {
            id: message_id_typed,
            session_id: session_id_typed,
            sequence: stored_event.sequence.unwrap_or(0),
            role: MessageRole::User,
            content,
            controls: req.controls,
            metadata: req.metadata,
            external_actor: req.external_actor,
            created_at: now,
        };

        if self.notifications_enabled
            && let Some(user_id) = user_id
        {
            self.notification_service
                .create_turn_request(org_id, user_id, session_id_typed, message_id_typed)
                .await?;
        }

        // Start workflow for user message in background (don't block the response)
        // The message is already persisted, so we can return immediately
        let runner = self.runner.clone();
        tokio::spawn(async move {
            if let Err(e) = runner
                .start_run(
                    org_id,
                    session_id_typed,
                    harness_id_typed,
                    agent_id_typed,
                    message_id_typed,
                )
                .await
            {
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
        let events = self
            .db
            .list_message_events(SessionId::from_uuid(session_id))
            .await?;
        let mut messages = Vec::with_capacity(events.len());

        for event_row in events {
            match Self::event_to_message(
                session_id,
                &event_row.data,
                &event_row.event_type,
                event_row.sequence,
            ) {
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
    /// Handles two formats:
    /// - Legacy format: full Event struct with id, type, data, etc.
    /// - New format: EventData directly (InputMessageData, OutputMessageCompletedData, etc.)
    fn event_to_message(
        session_id: Uuid,
        data: &serde_json::Value,
        event_type: &str,
        sequence: i32,
    ) -> std::result::Result<Message, String> {
        // Helper to convert EventData to Message
        let convert =
            |event_data: everruns_core::EventData| -> std::result::Result<Message, String> {
                let core_message = match &event_data {
                    everruns_core::EventData::InputMessage(data) => &data.message,
                    everruns_core::EventData::OutputMessageCompleted(data) => &data.message,
                    everruns_core::EventData::ToolCompleted(data) => {
                        // Separate text and image parts from the result content
                        let mut images: Vec<everruns_core::tools::ToolResultImage> = Vec::new();
                        let result: Option<serde_json::Value> =
                            data.result
                                .as_ref()
                                .map(|parts: &Vec<everruns_core::ContentPart>| {
                                    for part in parts {
                                        if let everruns_core::ContentPart::Image(img) = part
                                            && let (Some(b64), Some(mt)) =
                                                (&img.base64, &img.media_type)
                                        {
                                            images.push(everruns_core::tools::ToolResultImage {
                                                base64: b64.clone(),
                                                media_type: mt.clone(),
                                            });
                                        }
                                    }
                                    let text_parts: Vec<&everruns_core::ContentPart> = parts
                                        .iter()
                                        .filter(|p| {
                                            matches!(p, everruns_core::ContentPart::Text(_))
                                        })
                                        .collect();
                                    if text_parts.len() == 1
                                        && let everruns_core::ContentPart::Text(t) = text_parts[0]
                                    {
                                        return serde_json::Value::String(t.text.clone());
                                    }
                                    serde_json::to_value(&text_parts).unwrap_or_default()
                                });
                        let msg = if images.is_empty() {
                            everruns_core::Message::tool_result(
                                &data.tool_call_id,
                                result,
                                data.error.clone(),
                            )
                        } else {
                            everruns_core::Message::tool_result_with_images(
                                &data.tool_call_id,
                                result,
                                images,
                            )
                        };
                        return Ok(Message {
                            id: msg.id,
                            session_id: SessionId::from_uuid(session_id),
                            sequence,
                            role: MessageRole::from(msg.role.to_string().as_str()),
                            content: msg.content,
                            controls: None,
                            metadata: None,
                            external_actor: None,
                            created_at: msg.created_at,
                        });
                    }
                    _ => return Err("unexpected event type".to_string()),
                };

                Ok(Message {
                    id: core_message.id,
                    session_id: SessionId::from_uuid(session_id),
                    sequence,
                    role: MessageRole::from(core_message.role.to_string().as_str()),
                    content: core_message.content.clone(),
                    controls: core_message.controls.clone(),
                    metadata: core_message.metadata.clone(),
                    external_actor: core_message.external_actor.clone(),
                    created_at: core_message.created_at,
                })
            };

        // First try to parse as full Event (legacy format)
        // This has required fields like id, type, session_id, data
        if let Ok(event) = serde_json::from_value::<Event>(data.clone()) {
            return convert(event.data);
        }

        // Fallback: try to parse as specific EventData type directly (new format)
        // We use the event_type hint since EventData's Raw variant catches everything
        match event_type {
            "input.message" => {
                let d: InputMessageData = serde_json::from_value(data.clone())
                    .map_err(|e| format!("invalid input.message data: {}", e))?;
                convert(everruns_core::EventData::InputMessage(d))
            }
            "output.message.completed" => {
                let d: OutputMessageCompletedData = serde_json::from_value(data.clone())
                    .map_err(|e| format!("invalid output.message.completed data: {}", e))?;
                convert(everruns_core::EventData::OutputMessageCompleted(d))
            }
            "tool.completed" => {
                let d: ToolCompletedData = serde_json::from_value(data.clone())
                    .map_err(|e| format!("invalid tool.completed data: {}", e))?;
                convert(everruns_core::EventData::ToolCompleted(d))
            }
            _ => Err(format!("unexpected event type for message: {}", event_type)),
        }
    }
}
