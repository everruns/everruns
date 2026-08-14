//! Neutral event emission contract.

use crate::error::Result;
use crate::events::{Event, EventRequest};
use async_trait::async_trait;
use std::sync::Arc;

/// Trait for emitting events following the standard event protocol
///
/// Implementations can:
/// - Store events in a database
/// - Keep events in memory for testing
/// - Stream events via SSE/WebSocket
/// - Log events for debugging
///
/// Events follow a consistent schema: id, type, ts, context, data.
/// See knowledge/execution/events.md for the full event protocol specification.
#[async_trait]
pub trait EventEmitter: Send + Sync {
    /// Emit an event request
    ///
    /// Takes an EventRequest (without id/sequence) and returns the stored Event
    /// with id and sequence assigned by the storage layer.
    async fn emit(&self, request: EventRequest) -> Result<Event>;
}

/// Blanket impl: `Arc<E>` delegates to the inner emitter.
#[async_trait]
impl<E: EventEmitter + ?Sized> EventEmitter for Arc<E> {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        (**self).emit(request).await
    }
}

/// Core-local event emitter test double.
#[derive(Debug, Clone, Default)]
#[cfg(test)]
pub(crate) struct NoopEventEmitter;

#[cfg(test)]
#[async_trait]
impl EventEmitter for NoopEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        // Return a dummy event with sequence 0
        Ok(request.into_event(crate::typed_id::EventId::new(), 0))
    }
}

// Note: EventListener trait has been moved to event_listeners.rs module.
// Use `everruns_core::EventListener` or `everruns_core::event_listeners::EventListener`.
