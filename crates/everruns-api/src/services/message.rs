// Message service for business logic
// Messages are the PRIMARY conversation data store
//
// The service accepts API entities (CreateMessageRequest) and handles:
// - Conversion to database entities
// - Event emission for SSE notifications
// - Workflow triggering for user messages

use crate::messages::{ContentPart, CreateMessageRequest, Message, MessageRole};
use anyhow::Result;
use everruns_storage::{models::CreateEventRow, models::CreateMessageRow, Database};
use everruns_worker::AgentRunner;
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
    /// - Converts input content to storage format
    /// - Persists the message to the database
    /// - Emits an SSE event for the user message
    /// - Triggers workflow execution for the session
    pub async fn create(
        &self,
        agent_id: Uuid,
        session_id: Uuid,
        req: CreateMessageRequest,
    ) -> Result<Message> {
        // Convert InputContentPart array to ContentPart array for storage
        let content: Vec<ContentPart> = req
            .message
            .content
            .into_iter()
            .map(ContentPart::from)
            .collect();

        // Get tags from request (empty if not provided)
        let tags = req.tags.unwrap_or_default();

        // API only creates user messages
        let input = CreateMessageRow {
            session_id,
            role: MessageRole::User.to_string(),
            content,
            controls: req.controls,
            metadata: req.metadata,
            tags,
        };

        let row = self.db.create_message(input).await?;
        let message = Self::row_to_message(row);

        // Emit event and start workflow for user message
        self.emit_user_message_event(session_id, &message).await;
        self.start_workflow(agent_id, session_id).await;

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

    pub async fn list(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let rows = self.db.list_messages(session_id).await?;
        Ok(rows.into_iter().map(Self::row_to_message).collect())
    }

    /// Convert database row to API Message
    fn row_to_message(row: everruns_storage::MessageRow) -> Message {
        Message {
            id: row.id,
            session_id: row.session_id,
            sequence: row.sequence,
            role: MessageRole::from(row.role.as_str()),
            content: row.content,
            controls: row.controls,
            metadata: row.metadata,
            created_at: row.created_at,
        }
    }
}
