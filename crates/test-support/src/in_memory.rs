//! Deterministic in-memory event fixture for isolated tests.
//!
//! The writable message store moved to `everruns_builtins::InMemoryMessageRetriever`
//! (a production API) and is re-exported from the crate root for existing users.
//! This module keeps the event-emitter fixture, which stays test-scoped.

use async_trait::async_trait;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{Event, EventRequest};
use everruns_provider::error::Result;
use everruns_provider::typed_id::EventId;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Event emitter fixture with deterministic, process-local sequencing.
#[derive(Debug, Default, Clone)]
pub struct InMemoryEventEmitter {
    events: Arc<RwLock<Vec<Event>>>,
    sequence: Arc<RwLock<i32>>,
}

impl InMemoryEventEmitter {
    /// Create an empty event fixture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return all emitted events in order.
    pub async fn events(&self) -> Vec<Event> {
        self.events.read().await.clone()
    }

    /// Return the number of emitted events.
    pub async fn event_count(&self) -> usize {
        self.events.read().await.len()
    }

    /// Remove all events and reset deterministic sequencing.
    pub async fn clear(&self) {
        self.events.write().await.clear();
        *self.sequence.write().await = 0;
    }

    /// Return events with the requested protocol type.
    pub async fn events_by_type(&self, event_type: &str) -> Vec<Event> {
        self.events
            .read()
            .await
            .iter()
            .filter(|event| event.event_type == event_type)
            .cloned()
            .collect()
    }

    /// Return events emitted for one session UUID.
    pub async fn events_for_session(&self, session_id: Uuid) -> Vec<Event> {
        self.events
            .read()
            .await
            .iter()
            .filter(|event| event.session_uuid() == session_id)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl EventEmitter for InMemoryEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let mut sequence = self.sequence.write().await;
        *sequence += 1;
        let event = request.into_event(EventId::new(), *sequence);
        self.events.write().await.push(event.clone());
        Ok(event)
    }
}
