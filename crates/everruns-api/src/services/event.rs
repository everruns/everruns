// Event service for business logic (M2)
// Events are SSE notifications, NOT primary data storage
//
// Note: Currently unused as event emission is handled directly in MessageService.
// Kept for future use when event emission becomes more complex.

#![allow(dead_code)]

use anyhow::Result;
use everruns_core::Event;
use everruns_storage::{models::CreateEventRow, Database};
use std::sync::Arc;
use uuid::Uuid;

pub struct EventService {
    db: Arc<Database>,
}

impl EventService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    pub async fn create(&self, input: CreateEventRow) -> Result<Event> {
        let row = self.db.create_event(input).await?;
        Ok(Self::row_to_event(row))
    }

    pub async fn list(&self, session_id: Uuid, since_sequence: Option<i32>) -> Result<Vec<Event>> {
        let rows = self.db.list_events(session_id, since_sequence).await?;
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
