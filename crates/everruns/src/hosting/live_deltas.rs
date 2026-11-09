//! Live deltas: the token stream of a running turn, for hosts whose watchers
//! are not in this process (data plane v2 plan step 1, "Live streaming").
//!
//! Decision: deltas (`output.message.delta`, `reason.thinking.delta`,
//! `tool.output.delta`, voice transcript deltas) are never stored, so
//! Engine's listeners, which carry committed events only, never see them.
//! A host that streams to watchers elsewhere (a cell's stream node) takes
//! them from its epoch with [`SessionEpoch::live_deltas`](super::SessionEpoch::live_deltas).
//!
//! Every delta carries what a stream node needs to fence it:
//! - the epoch `fence` it was produced under: a node accepts deltas only
//!   from the session's current epoch;
//! - its `attempt`: the turn and the model call (or tool call) it streams,
//!   as `(turn, stream)`; the committed event that ends that stream
//!   (`output.message.completed`, `reason.completed`, `tool.completed`)
//!   closes the attempt, and later deltas for it are
//!   dropped, here and on the node;
//! - a `seq`, increasing within the epoch, so a node can drop duplicates
//!   and reordering.
//!
//! Deltas leave in emission order, after the committed events they follow
//! (`HostEventEmitter` holds a delta behind a pending commit), and
//! publishing never waits: a sink that can't keep up drops.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use everruns_contracts::runtime::events::{Event, EventData};
use everruns_contracts::typed_id::{SessionId, TurnId};
use everruns_core::host::events::{EventSink, EventSinkError};

/// One live delta, fenced to the epoch that produced it. Stability:
/// experimental.
#[derive(Debug, Clone)]
pub struct LiveDelta {
    /// The session it streams.
    pub session: SessionId,
    /// The authority fence of the epoch that produced it.
    pub fence: u64,
    /// The stream it belongs to; see [`LiveAttempt`].
    pub attempt: LiveAttempt,
    /// Increasing within the epoch, across attempts.
    pub seq: u64,
    /// The ephemeral event itself (`sequence` is `None`).
    pub event: Event,
}

/// The stream a delta belongs to: the turn, and the assistant message,
/// thinking block or tool call it streams. Stability: experimental.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LiveAttempt {
    /// The turn, when the delta is turn-scoped.
    pub turn: Option<TurnId>,
    /// The streamed item: an assistant message id, or a tool call id, or
    /// the atom execution for thinking and voice deltas.
    pub stream: String,
}

/// Receives a session's live deltas. Stability: experimental.
pub trait LiveDeltaSink: Send + Sync + 'static {
    /// Publish one delta without waiting; drop it when the sink is full.
    fn publish(&self, delta: LiveDelta);
}

/// The stream an event belongs to, for deltas and the events that end them.
fn stream_of(event: &Event) -> Option<String> {
    match &event.data {
        EventData::OutputMessageDelta(data) => Some(data.message_id.to_string()),
        EventData::OutputMessageCompleted(data) => Some(data.message.id.to_string()),
        EventData::ToolOutputDelta(data) => Some(data.tool_call_id.clone()),
        EventData::ToolCompleted(data) => Some(data.tool_call_id.clone()),
        _ => event.context.exec_id.map(|exec| exec.to_string()),
    }
}

/// An event that ends the stream it belongs to.
fn ends_stream(event: &Event) -> bool {
    matches!(
        event.data,
        EventData::OutputMessageCompleted(_)
            | EventData::ToolCompleted(_)
            | EventData::ReasonCompleted(_)
    )
}

/// A terminal turn event.
fn ends_turn(event: &Event) -> bool {
    matches!(
        event.data,
        EventData::TurnCompleted(_) | EventData::TurnFailed(_) | EventData::TurnCancelled(_)
    )
}

/// The epoch's event sink: forwards everything to the in-process sink and
/// tees deltas to the host's [`LiveDeltaSink`].
pub(crate) struct LiveDeltaTee {
    pub(crate) inner: Arc<dyn EventSink>,
    pub(crate) sink: Arc<dyn LiveDeltaSink>,
    pub(crate) session: SessionId,
    pub(crate) fence: u64,
    seq: AtomicU64,
    closed: std::sync::Mutex<HashSet<LiveAttempt>>,
}

impl LiveDeltaTee {
    pub(crate) fn new(
        inner: Arc<dyn EventSink>,
        sink: Arc<dyn LiveDeltaSink>,
        session: SessionId,
        fence: u64,
    ) -> Self {
        Self {
            inner,
            sink,
            session,
            fence,
            seq: AtomicU64::new(0),
            closed: std::sync::Mutex::default(),
        }
    }

    fn observe(&self, event: &Event) {
        if event.sequence.is_some() && ends_turn(event) {
            // Nothing streams for a finished turn; keep the set small.
            self.closed()
                .retain(|done| done.turn != event.context.turn_id);
            return;
        }
        let Some(stream) = stream_of(event) else {
            return;
        };
        let attempt = LiveAttempt {
            turn: event.context.turn_id,
            stream,
        };
        let mut closed = self.closed();
        if event.sequence.is_some() {
            if ends_stream(event) {
                closed.insert(attempt);
            }
            return;
        }
        if !event.is_ephemeral() || closed.contains(&attempt) {
            return;
        }
        drop(closed);
        self.sink.publish(LiveDelta {
            session: self.session,
            fence: self.fence,
            attempt,
            seq: self.seq.fetch_add(1, Ordering::Relaxed),
            event: event.clone(),
        });
    }
}

impl LiveDeltaTee {
    fn closed(&self) -> std::sync::MutexGuard<'_, HashSet<LiveAttempt>> {
        self.closed.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl EventSink for LiveDeltaTee {
    fn prepare_causal_context(
        &self,
        request: &everruns_contracts::runtime::events::EventRequest,
    ) -> Option<String> {
        self.inner.prepare_causal_context(request)
    }
    fn turn_causal_context(
        &self,
        context: &everruns_contracts::runtime::events::EventContext,
    ) -> everruns_core::causal::CausalContext {
        self.inner.turn_causal_context(context)
    }
    fn restore_turn_context(
        &self,
        context: &everruns_contracts::runtime::events::EventContext,
        metadata: Option<&serde_json::Value>,
    ) {
        self.inner.restore_turn_context(context, metadata)
    }
    fn try_send(&self, event: Event) -> Result<(), EventSinkError> {
        self.observe(&event);
        self.inner.try_send(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::runtime::events::{
        EventBuilder, OutputMessageCompletedData, OutputMessageDeltaData, TurnCompletedData,
    };
    use everruns_contracts::runtime::message::RuntimeMessage;
    use everruns_contracts::typed_id::{MessageId, TurnId};

    #[derive(Default)]
    struct Collect(std::sync::Mutex<Vec<LiveDelta>>);
    impl LiveDeltaSink for Arc<Collect> {
        fn publish(&self, delta: LiveDelta) {
            self.0.lock().unwrap().push(delta);
        }
    }
    struct Nowhere;
    impl EventSink for Nowhere {
        fn try_send(&self, _event: Event) -> Result<(), EventSinkError> {
            Ok(())
        }
    }

    fn delta(session: SessionId, turn: TurnId, message: MessageId, text: &str) -> Event {
        EventBuilder::new(session)
            .with_turn(turn, MessageId::new())
            .build(OutputMessageDeltaData {
                turn_id: turn,
                message_id: message,
                delta: text.into(),
                accumulated: text.into(),
                phase: None,
            })
    }

    #[test]
    fn a_committed_completion_closes_its_stream_and_a_finished_turn_forgets_it() {
        let session = SessionId::new();
        let collect = Arc::new(Collect::default());
        let tee = LiveDeltaTee::new(Arc::new(Nowhere), Arc::new(collect.clone()), session, 7);
        let (turn, message) = (TurnId::new(), MessageId::new());
        tee.try_send(delta(session, turn, message, "a")).unwrap();
        let mut answer = RuntimeMessage::assistant("a");
        answer.id = message;
        let mut completed = EventBuilder::new(session)
            .with_turn(turn, MessageId::new())
            .build(OutputMessageCompletedData {
                message: answer,
                metadata: None,
                usage: None,
                error_code: None,
                error_fields: None,
                error_disclosure: Default::default(),
            });
        completed.sequence = Some(4);
        tee.try_send(completed).unwrap();
        // A late delta of the closed stream (a retried call's leftovers).
        tee.try_send(delta(session, turn, message, "late")).unwrap();
        // Another stream of the same turn still flows.
        tee.try_send(delta(session, turn, MessageId::new(), "b"))
            .unwrap();
        let published = collect.0.lock().unwrap().clone();
        assert_eq!(published.len(), 2);
        assert_eq!(published.iter().map(|d| d.seq).collect::<Vec<_>>(), [0, 1]);
        assert!(
            published
                .iter()
                .all(|d| d.fence == 7 && d.session == session)
        );
        assert_eq!(published[0].attempt.stream, message.to_string());

        let mut finished = EventBuilder::new(session)
            .with_turn(turn, MessageId::new())
            .build(TurnCompletedData {
                turn_id: turn,
                ..Default::default()
            });
        finished.sequence = Some(5);
        tee.try_send(finished).unwrap();
        assert!(tee.closed().is_empty(), "nothing kept for a finished turn");
    }
}
