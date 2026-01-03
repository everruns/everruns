// Event service for business logic
//
// Events are SSE notifications following the standard event protocol.
// Events are stored in the events table and streamed to clients via SSE.
// This service is the central entry point for event ingestion from both
// HTTP API and gRPC service.

use crate::storage::{models::CreateEventRow, Database};
use anyhow::Result;
use everruns_core::{Event, EventContext, EventData};
use std::sync::Arc;
use uuid::Uuid;

pub struct EventService {
    db: Arc<Database>,
}

impl EventService {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Emit a typed event and store it in the database.
    /// Returns the stored event with its assigned sequence number.
    ///
    /// This is the primary method for event ingestion, used by both
    /// HTTP API and gRPC service.
    pub async fn emit(&self, event: Event) -> Result<Event> {
        // Serialize the full event to JSON for storage
        let data = serde_json::to_value(&event)?;

        let create_row = CreateEventRow {
            session_id: event.session_id,
            event_type: event.event_type.clone(),
            data,
        };

        let row = self.db.create_event(create_row).await?;

        // Return the event with the assigned sequence number
        Ok(Event {
            sequence: Some(row.sequence),
            ..event
        })
    }

    /// Emit a batch of typed events and store them in the database.
    /// Returns the count of successfully stored events.
    ///
    /// This method is optimized for bulk event ingestion from workers.
    pub async fn emit_batch(&self, events: Vec<Event>) -> Result<i32> {
        let mut count = 0i32;

        for event in events {
            // Serialize the full event to JSON for storage
            let data = serde_json::to_value(&event)?;

            let create_row = CreateEventRow {
                session_id: event.session_id,
                event_type: event.event_type.clone(),
                data,
            };

            self.db.create_event(create_row).await?;
            count += 1;
        }

        Ok(count)
    }

    /// Create an event from raw row data (legacy API support)
    #[allow(dead_code)]
    pub async fn create(&self, input: CreateEventRow) -> Result<Event> {
        let row = self.db.create_event(input).await?;
        Ok(Self::row_to_event(row))
    }

    pub async fn list(&self, session_id: Uuid, since_sequence: Option<i32>) -> Result<Vec<Event>> {
        let rows = self.db.list_events(session_id, since_sequence).await?;
        Ok(rows.into_iter().map(Self::row_to_event).collect())
    }

    fn row_to_event(row: crate::storage::EventRow) -> Event {
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
