// Message service for business logic
// Messages are the PRIMARY conversation data store
//
// The service accepts API entities (CreateMessageRequest) and handles:
// - Conversion to database entities
// - Message status management (pending -> processed)
// - Workflow triggering for user messages
//
// Design: User messages are saved with "pending" status. If the workflow is
// already running, it will pick up the pending message. If not running,
// a new workflow is started. The workflow processes pending messages,
// emits SSE events, and marks them as processed.

use crate::messages::{ContentPart, CreateMessageRequest, Message, MessageRole};
use anyhow::Result;
use everruns_storage::{models::CreateMessageRow, Database};
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
    /// - Persists the message to the database with "pending" status
    /// - Starts workflow if not already running (workflow will emit SSE events)
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

        // API creates user messages with "pending" status
        // Workflow will process them and emit events
        let input = CreateMessageRow {
            session_id,
            role: MessageRole::User.to_string(),
            status: "pending".to_string(),
            content,
            controls: req.controls,
            metadata: req.metadata,
            tags,
        };

        let row = self.db.create_message(input).await?;
        let message = Self::row_to_message(row);

        // Check if workflow is already running
        let is_running = self.runner.is_running(session_id).await;

        if is_running {
            tracing::info!(
                session_id = %session_id,
                message_id = %message.id,
                "Workflow already running, message will be picked up by process-pending-messages"
            );
            // Workflow is running - it will pick up the pending message
            // No need to start a new workflow
        } else {
            // Workflow not running - start it
            self.start_workflow(agent_id, session_id).await;
        }

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
            controls: row.controls.map(|j| j.0),
            metadata: row.metadata.map(|j| j.0),
            created_at: row.created_at,
        }
    }
}
