// Event service for business logic
//
// Events are SSE notifications following the standard event protocol.
// Events are stored in the events table and streamed to clients via SSE.

#![allow(dead_code)]

use anyhow::Result;
use everruns_core::{Event, EventContext, EventData};
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
        // Try to deserialize the full event from the data column
        // (new format stores the complete event JSON)
        if let Ok(mut event) = serde_json::from_value::<Event>(row.data.clone()) {
            // Ensure sequence is set from the database
            event.sequence = Some(row.sequence);
            return event;
        }

        // Fallback for old format or direct data storage:
        // Reconstruct event from row fields using raw data
        Event {
            id: row.id,
            event_type: row.event_type,
            ts: row.created_at,
            session_id: row.session_id,
            context: EventContext::empty(),
            data: EventData::raw(row.data),
            metadata: None,
            tags: None,
            sequence: Some(row.sequence),
        }
    }
}
