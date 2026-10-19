//! Neutral event emission contract.

use crate::runtime::error::Result;
use crate::runtime::events::{Event, EventRequest};
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
#[async_trait]
pub trait EventEmitter: Send + Sync {
    /// Emit an event request
    ///
    /// Takes an EventRequest (without id/sequence) and returns the stored Event
    /// with id and sequence assigned by the storage layer.
    ///
    /// A host may store some lifecycle events in the background and return
    /// at once, but never the `tool.started` of an `AtMostOnce` call: that
    /// one is stored before `emit` returns, because resume reads a log without
    /// it as proof the call never ran.
    async fn emit(&self, request: EventRequest) -> Result<Event>;
}

/// Blanket impl: `Arc<E>` delegates to the inner emitter.
#[async_trait]
impl<E: EventEmitter + ?Sized> EventEmitter for Arc<E> {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        (**self).emit(request).await
    }
}

// Note: EventListener trait has been moved to event_listeners.rs module.
// Use `everruns_contracts::runtime::EventListener` or `everruns_contracts::runtime::event_listeners::EventListener`.
