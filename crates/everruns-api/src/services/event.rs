// Event service for business logic (M2)

use anyhow::Result;
use everruns_contracts::Event;
use everruns_storage::{models::CreateEvent, Database};
use std::sync::Arc;
use uuid::Uuid;

pub struct EventService {
    db: Arc<Database>,
}

impl EventService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateEvent) -> Result<Event> {
        let row = self.db.create_event(input).await?;
        Ok(Self::row_to_event(row))
    }

    #[allow(dead_code)] // Used by SSE streaming in sessions.rs through db directly
    pub async fn list(&self, session_id: Uuid, since_sequence: Option<i32>) -> Result<Vec<Event>> {
        let rows = self.db.list_events(session_id, since_sequence).await?;
        Ok(rows.into_iter().map(Self::row_to_event).collect())
    }

    pub async fn list_messages(&self, session_id: Uuid) -> Result<Vec<Event>> {
        let rows = self.db.list_message_events(session_id).await?;
        Ok(rows.into_iter().map(Self::row_to_event).collect())
    }

    fn row_to_event(row: everruns_storage::EventRow) -> Event {
        Event {
            id: row.id,
            session_id: row.session_id,
            sequence: row.sequence,
            event_type: row.event_type,
            data: row.data.clone(),
            created_at: row.created_at,
        }
    }
}
