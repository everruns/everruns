// Message service for business logic (M2)
// Messages are the PRIMARY conversation data store

use anyhow::Result;
use everruns_contracts::{Message, MessageRole};
use everruns_storage::{models::CreateMessage, Database};
use std::sync::Arc;
use uuid::Uuid;

pub struct MessageService {
    db: Arc<Database>,
}

impl MessageService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateMessage) -> Result<Message> {
        let row = self.db.create_message(input).await?;
        Ok(Self::row_to_message(row))
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

    fn row_to_message(row: everruns_storage::MessageRow) -> Message {
        Message {
            id: row.id,
            session_id: row.session_id,
            sequence: row.sequence,
            role: MessageRole::from(row.role.as_str()),
            content: row.content,
            tool_call_id: row.tool_call_id,
            created_at: row.created_at,
        }
    }
}
