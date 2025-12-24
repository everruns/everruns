// Message service for business logic
// Messages are the PRIMARY conversation data store
//
// The service accepts API entities (CreateMessageRequest) and handles:
// - Conversion to database entities
// - Event emission for SSE notifications
// - Workflow triggering for user messages

use crate::messages::{
    ContentPart, CreateMessageRequest, ImageContentPart, InputContentPart, Message, MessageRole,
    TextContentPart,
};
use anyhow::Result;
use everruns_storage::{models::CreateEventRow, models::CreateMessageRow, Database};
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

    /// Create a message from API request
    /// Handles conversion, event emission, and workflow triggering
    pub async fn create(
        &self,
        agent_id: Uuid,
        session_id: Uuid,
        req: CreateMessageRequest,
    ) -> Result<Message> {
        // Convert InputContentPart array to ContentPart array for storage
        let content = input_content_parts_to_content_parts(&req.message.content);

        // Convert request metadata to JSON for storage
        let metadata = req.metadata.and_then(|m| serde_json::to_value(m).ok());

        // Get tags from request (empty if not provided)
        let tags = req.tags.unwrap_or_default();

        let input = CreateMessageRow {
            session_id,
            role: req.message.role.to_string(),
            content,
            metadata,
            tags,
            tool_call_id: None, // Tool call ID is derived from content for tool_result messages
        };

        let row = self.db.create_message(input).await?;
        let message = Self::row_to_message(row);

        // If this is a user message, emit event and start workflow
        if message.role == MessageRole::User {
            self.emit_user_message_event(session_id, &message).await;
            self.start_workflow(agent_id, session_id).await;
        }

        Ok(message)
    }

    /// Emit SSE event for user message
    async fn emit_user_message_event(&self, session_id: Uuid, message: &Message) {
        let event_input = CreateEventRow {
            session_id,
            event_type: "message.user".to_string(),
            data: serde_json::json!({
                "message_id": message.id,
                "content": message.content
            }),
        };
        if let Err(e) = self.db.create_event(event_input).await {
            tracing::warn!("Failed to emit user message event: {}", e);
        }
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

    #[allow(dead_code)] // Will be used by future endpoints
    pub async fn get(&self, id: Uuid) -> Result<Option<Message>> {
        let row = self.db.get_message(id).await?;
        Ok(row.map(Self::row_to_message))
    }

    pub async fn list(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let rows = self.db.list_messages(session_id).await?;
        Ok(rows.into_iter().map(Self::row_to_message).collect())
    }

    /// Convert database row to API Message
    fn row_to_message(row: everruns_storage::MessageRow) -> Message {
        let role = MessageRole::from(row.role.as_str());
        let metadata = row
            .metadata
            .and_then(|m| serde_json::from_value::<HashMap<String, serde_json::Value>>(m).ok());

        Message {
            id: row.id,
            session_id: row.session_id,
            sequence: row.sequence,
            role,
            content: row.content, // Already Vec<ContentPart> from database
            metadata,
            tool_call_id: row.tool_call_id,
            created_at: row.created_at,
        }
    }
}

/// Convert InputContentPart array to ContentPart array (for storage)
fn input_content_parts_to_content_parts(parts: &[InputContentPart]) -> Vec<ContentPart> {
    parts
        .iter()
        .map(|part| match part {
            InputContentPart::Text(t) => ContentPart::Text(TextContentPart {
                text: t.text.clone(),
            }),
            InputContentPart::Image(i) => ContentPart::Image(ImageContentPart {
                url: i.url.clone(),
                base64: i.base64.clone(),
                media_type: i.media_type.clone(),
            }),
        })
        .collect()
}
