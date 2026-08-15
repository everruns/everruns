//! Host-applied effects produced while an engine phase is running.
//!
//! Unlike transition lifecycle effects, phase effects cannot be buffered until
//! Input/Reason/Act returns: output deltas, tool progress, and tool completion
//! are live protocol signals. This contract still keeps recording outside the
//! phase algorithm. The phase describes an effect and the injected host sink
//! applies it immediately, preserving streaming and durable event order.

use async_trait::async_trait;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{Event, EventRequest};
use everruns_provider::error::Result;
use std::sync::Arc;

/// One effect produced during Input/Reason/Act execution.
#[derive(Debug)]
pub enum PhaseEffect {
    /// Append and publish one canonical event.
    EmitEvent(EventRequest),
}

/// Host-applied sink for live phase effects.
///
/// Engine phase constructors adapt the host's existing [`EventEmitter`] to
/// this contract, so custom hosts retain the open event-storage boundary while
/// phase algorithms no longer invoke the persistence-shaped operation directly.
#[async_trait]
pub trait PhaseEffectSink: Send + Sync {
    /// Apply one phase effect and return the accepted canonical event.
    async fn apply_phase_effect(&self, effect: PhaseEffect) -> Result<Event>;

    /// Convenience for the common canonical-event effect.
    async fn emit_phase_event(&self, request: EventRequest) -> Result<Event> {
        self.apply_phase_effect(PhaseEffect::EmitEvent(request))
            .await
    }
}

/// Adapter installed by engine phases around a host event emitter.
pub(crate) struct PhaseEffectEmitter<E: ?Sized> {
    inner: Arc<E>,
}

impl<E: ?Sized> Clone for PhaseEffectEmitter<E> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<E: ?Sized> PhaseEffectEmitter<E> {
    pub(crate) fn new(inner: Arc<E>) -> Self {
        Self { inner }
    }

    pub(crate) async fn emit(&self, request: EventRequest) -> Result<Event>
    where
        E: PhaseEffectSink,
    {
        self.inner
            .apply_phase_effect(PhaseEffect::EmitEvent(request))
            .await
    }
}

impl AsRef<dyn EventEmitter> for PhaseEffectEmitter<dyn PhaseEffectSink> {
    fn as_ref(&self) -> &(dyn EventEmitter + 'static) {
        self
    }
}

#[async_trait]
impl<T: EventEmitter + ?Sized> PhaseEffectSink for T {
    async fn apply_phase_effect(&self, effect: PhaseEffect) -> Result<Event> {
        match effect {
            PhaseEffect::EmitEvent(request) => self.emit(request).await,
        }
    }
}

#[async_trait]
impl<T: PhaseEffectSink + ?Sized> EventEmitter for PhaseEffectEmitter<T> {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        PhaseEffectEmitter::emit(self, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::Message;
    use everruns_core::events::{EventContext, INPUT_MESSAGE, InputMessageData};
    use everruns_provider::typed_id::SessionId;

    #[tokio::test]
    async fn phase_effect_adapter_preserves_host_sequence_and_event_type() {
        let host = Arc::new(crate::test_fixtures::TestEventEmitter::new());
        let sink = PhaseEffectEmitter::new(host.clone());
        let session_id = SessionId::new();

        let first = sink
            .emit_phase_event(EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(Message::user("first")),
            ))
            .await
            .unwrap();
        let second = sink
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(Message::user("second")),
            ))
            .await
            .unwrap();

        assert_eq!((first.sequence, second.sequence), (Some(1), Some(2)));
        assert_eq!(
            (first.event_type.as_str(), second.event_type.as_str()),
            (INPUT_MESSAGE, INPUT_MESSAGE)
        );
        let recorded = host.events().await;
        assert_eq!(recorded.len(), 2);
        assert_eq!(recorded[0].id, first.id);
        assert_eq!(recorded[1].id, second.id);
    }
}
