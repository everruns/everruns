//! Observable and cancellable turns.
//!
//! This module promotes two capabilities onto the [`Session`](crate::Session)
//! surface without leaking host/runtime internals or core event/store types:
//!
//! - **Observation.** [`Session::events`](crate::Session::events) returns an
//!   [`EventStream`] — a live feed of [`SessionEvent`]s projected from the
//!   host's canonical event emitter. Events retain the canonical event
//!   protocol as JSON and also exposes a typed convenience projection. The
//!   stream is bounded, so a slow or dropped consumer can never stall the turn
//!   that produces the events; lag is reported explicitly to the consumer.
//! - **Cancellation.** [`Session::run_with`](crate::Session::run_with) accepts a
//!   [`RunOptions`] carrying an optional [`CancellationToken`]. Cancelling the
//!   token stops the in-flight turn by dropping its future — the same
//!   cooperative, drop-based cancellation the runtime already uses to tear down
//!   tool work — and the turn resolves to a stable [`Turn`](crate::Turn) with
//!   [`TurnStopReason::Cancelled`](crate::TurnStopReason::Cancelled).
//!
//! Only facade types appear on the public surface: no `EventBus`,
//! `EventEmitter`, `EventRequest`, or core event/store types.
//!
//! The live subscriber is not persistence. Everruns' host `EventLog` commits
//! durable canonical events; its `EventHistory` is a read-only projection built
//! by replay. The host `EventSink` and this facade's [`EventStream`] observe
//! post-commit durable events plus live-only ephemeral events. Sink absence,
//! lag, or failure cannot create, roll back, or replace conversation history.

use std::sync::Mutex;

use everruns_core::events::{
    self, Event, EventContext, EventData, EventRequest, InputMessageData,
    OutputMessageCompletedData, OutputMessageDeltaData, OutputMessageReplacedData,
    OutputMessageStartedData, ReasonCompletedData, ToolCompletedData, ToolOutputDeltaData,
    ToolProgressData, ToolStartedData, TurnCancelledData, TurnFailedData,
};
use everruns_host::{EventSink, EventSinkError};
use everruns_provider::typed_id::{SessionId, TurnId};
use serde_json::Value;
use tokio::sync::broadcast;

/// Capacity of the per-session broadcast channel that backs [`EventStream`].
///
/// A turn normally emits well under this many events. If a consumer falls
/// behind, the channel evicts its oldest unread events and reports the exact
/// gap through [`EventStreamError::Lagged`] rather than applying backpressure to
/// the turn — the runner must never block on an observer.
pub const EVENT_STREAM_CAPACITY: usize = 4096;

/// A single observed event from a running [`Session`](crate::Session).
///
/// This is a typed/raw bridge to Everruns' canonical event
/// protocol. [`kind`](SessionEvent::kind) is the ergonomic typed projection used
/// by renderers, while [`as_json`](SessionEvent::as_json) returns the canonical
/// event envelope with the live-delta exception described below. The JSON
/// form includes the event timestamp, optional persisted sequence, correlation
/// context, event-specific data, metadata, and tags. For
/// `output.message.delta` events, the redundant `data.accumulated` prefix is
/// omitted to keep buffered stream memory proportional to output size rather
/// than quadratic in it. Incremental text remains in
/// [`SessionEventKind::TextDelta`].
///
/// The correlation ids (`event_id`, `session_id`, and the optional `turn_id`)
/// are also promoted as strings for common logging and matching tasks.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SessionEvent {
    /// Opaque id uniquely identifying this event.
    pub event_id: String,
    /// Opaque id of the session that produced the event.
    pub session_id: String,
    /// Opaque id of the turn this event belongs to, when turn-scoped.
    pub turn_id: Option<String>,
    /// The event-specific projection.
    pub kind: SessionEventKind,
    event_type: String,
    timestamp: String,
    data: Value,
    raw: Value,
}

/// The event-specific payload of a [`SessionEvent`].
///
/// Non-exhaustive on purpose: event kinds useful to an application renderer are
/// promoted here, while every other event is surfaced through
/// [`Other`](SessionEventKind::Other). The full event remains available through
/// [`SessionEvent::as_json`] for every variant, including newly added event
/// types that this version of the crate does not recognize.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SessionEventKind {
    /// A user input message was committed to the canonical event history.
    InputMessage {
        /// Stable id of the user message.
        message_id: String,
    },
    /// Assistant output began streaming.
    OutputStarted {
        /// Stable id shared by the output's deltas and completion.
        message_id: String,
    },
    /// A turn began executing.
    TurnStarted,
    /// A turn finished successfully.
    TurnCompleted,
    /// A turn ended with an unrecoverable failure.
    TurnFailed {
        /// Human-readable failure message.
        error: String,
    },
    /// A turn was cancelled before completing.
    TurnCancelled,
    /// An incremental chunk of assistant output text.
    TextDelta {
        /// The new text appended by this delta.
        delta: String,
    },
    /// Streaming output was replaced, for example by an output guardrail.
    OutputReplaced {
        /// Stable id of the assistant message being replaced.
        message_id: String,
        /// Replacement text that is safe to render.
        replacement: String,
    },
    /// An assistant output message completed.
    OutputCompleted {
        /// Stable id shared by the output's start and deltas.
        message_id: String,
    },
    /// A tool call started executing.
    ToolStarted {
        /// Opaque id of the tool call.
        tool_call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
    },
    /// A tool call finished executing.
    ToolCompleted {
        /// Opaque id of the tool call.
        tool_call_id: String,
        /// Name of the tool that was invoked.
        tool_name: String,
        /// Whether the tool call succeeded.
        success: bool,
    },
    /// A running tool reported human-readable progress.
    ToolProgress {
        /// Opaque id of the tool call.
        tool_call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
        /// Progress message suitable for a timeline or terminal.
        message: String,
    },
    /// A running tool produced an incremental output chunk.
    ToolOutputDelta {
        /// Opaque id of the tool call.
        tool_call_id: String,
        /// Name of the tool being invoked.
        tool_name: String,
        /// Output stream identifier, such as `"stdout"` or `"stderr"`.
        stream: String,
        /// New output appended to that stream.
        delta: String,
    },
    /// An LLM inference step began.
    ///
    /// This is the reason half of the reason/act loop, not model reasoning: it
    /// says an inference call started, nothing about whether the model reasoned.
    /// For that, match [`Self::ReasoningDelta`] and [`Self::ReasoningItem`].
    ReasonStarted,
    /// An LLM inference step completed. See [`Self::ReasonStarted`].
    ReasonCompleted {
        /// Whether the model call succeeded.
        success: bool,
        /// Provider/model failure detail, when the step failed.
        error: Option<String>,
    },
    /// Incremental model reasoning. Belongs to the reasoning channel and must
    /// never be rendered as assistant text.
    ReasoningDelta {
        /// New reasoning text since the last event.
        delta: String,
        /// Reasoning so far, for convenience.
        accumulated: String,
    },
    /// Model reasoning finished for this turn.
    ReasoningCompleted {
        /// The complete reasoning text.
        text: String,
    },
    /// One provider reasoning artifact completed.
    ///
    /// Carries identity and provider-curated summary text only; opaque replay
    /// state never reaches this surface.
    ReasoningItem {
        /// Provider that produced it (`anthropic`, `openai`, `google`).
        provider: String,
        /// Provider-assigned identifier, when one was issued.
        item_id: Option<String>,
        /// Provider-curated summary segments. Never raw chain-of-thought.
        summary: Vec<String>,
    },
    /// A complete model-generation record was emitted.
    ///
    /// The model input, output, tool requests, usage, and timing remain in the
    /// canonical payload returned by [`SessionEvent::data`].
    ModelGeneration,
    /// Any other canonical event, carried through unprojected.
    ///
    /// `event_type` is the stable dot-notation type string (e.g.
    /// `"act.started"`); `payload` is the raw event data as JSON. This is the
    /// forward-compatibility fallback — match it with a wildcard arm.
    Other {
        /// Stable dot-notation event type string.
        event_type: String,
        /// Raw event payload as JSON.
        payload: Value,
    },
}

impl SessionEventKind {
    /// Whether this event ends its associated turn.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::TurnCompleted | Self::TurnFailed { .. } | Self::TurnCancelled
        )
    }
}

impl SessionEvent {
    /// The stable dot-notation type string for this event (e.g.
    /// `"turn.started"`), matching the canonical event protocol.
    pub fn event_type(&self) -> &str {
        &self.event_type
    }

    /// Persisted replay sequence within this session.
    ///
    /// Durable events return `Some(sequence)` with a monotonically increasing
    /// per-session position. Ephemeral live-only events such as output deltas
    /// return `None`. [`EventStream`] preserves live channel arrival order; this
    /// canonical field must not be treated as a delivery counter.
    pub fn sequence(&self) -> Option<i32> {
        self.raw
            .get("sequence")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
    }

    /// RFC 3339 timestamp from the canonical event envelope.
    pub fn timestamp(&self) -> &str {
        &self.timestamp
    }

    /// The canonical event envelope as JSON.
    ///
    /// This is the same serialized representation used by Everruns' public
    /// event protocol. No event fields are omitted when [`kind`](Self::kind) is a compact
    /// renderer-oriented projection or [`SessionEventKind::Other`], except
    /// `data.accumulated` on `output.message.delta`. That redundant growing
    /// prefix is omitted so a slow subscriber cannot retain quadratic memory.
    pub fn as_json(&self) -> &Value {
        &self.raw
    }

    /// Consume the event and return its complete canonical JSON envelope.
    pub fn into_json(self) -> Value {
        self.raw
    }

    /// The canonical event-specific `data` payload.
    ///
    /// Use this when a renderer needs a field not promoted onto
    /// [`SessionEventKind`], such as tool narration, a completed message's
    /// structured content, model token usage, or failure classification.
    pub fn data(&self) -> &Value {
        &self.data
    }

    /// Human-readable tool narration, when the event carries it.
    pub fn narration(&self) -> Option<&str> {
        self.data().get("narration").and_then(Value::as_str)
    }

    /// Project a core [`Event`] into a facade [`SessionEvent`].
    ///
    /// Known event types map to typed [`SessionEventKind`] variants; every other
    /// type is carried through the [`Other`](SessionEventKind::Other) fallback,
    /// so no event is ever dropped.
    fn from_core_event(event: &Event) -> Self {
        let mut raw = serde_json::to_value(event).expect("canonical events are JSON serializable");
        let mut data = serde_json::to_value(&event.data)
            .expect("canonical event payloads are JSON serializable");
        if event.event_type == events::OUTPUT_MESSAGE_DELTA {
            // THREAT[TM-DOS-037]: accumulated repeats the entire output prefix
            // in every delta. Retaining those prefixes in the broadcast ring
            // would turn an n-byte streamed response into O(n^2) memory.
            data.as_object_mut()
                .and_then(|data| data.remove("accumulated"));
            raw.get_mut("data")
                .and_then(Value::as_object_mut)
                .and_then(|data| data.remove("accumulated"));
        }
        let turn_id = event.context.turn_id.map(|id| id.to_string());
        let kind = match event.event_type.as_str() {
            events::INPUT_MESSAGE => match &event.data {
                EventData::InputMessage(InputMessageData { message }) => {
                    SessionEventKind::InputMessage {
                        message_id: message.id.to_string(),
                    }
                }
                _ => Self::other_kind(event),
            },
            events::OUTPUT_MESSAGE_STARTED => match &event.data {
                EventData::OutputMessageStarted(OutputMessageStartedData {
                    message_id, ..
                }) => SessionEventKind::OutputStarted {
                    message_id: message_id.to_string(),
                },
                _ => Self::other_kind(event),
            },
            events::TURN_STARTED => SessionEventKind::TurnStarted,
            events::TURN_COMPLETED => SessionEventKind::TurnCompleted,
            events::TURN_CANCELLED => SessionEventKind::TurnCancelled,
            events::TURN_FAILED => match &event.data {
                EventData::TurnFailed(TurnFailedData { error, .. }) => {
                    SessionEventKind::TurnFailed {
                        error: error.clone(),
                    }
                }
                _ => Self::other_kind(event),
            },
            events::OUTPUT_MESSAGE_DELTA => match &event.data {
                EventData::OutputMessageDelta(OutputMessageDeltaData { delta, .. }) => {
                    SessionEventKind::TextDelta {
                        delta: delta.clone(),
                    }
                }
                _ => Self::other_kind(event),
            },
            events::OUTPUT_MESSAGE_REPLACED => match &event.data {
                EventData::OutputMessageReplaced(OutputMessageReplacedData {
                    message_id,
                    replacement,
                    ..
                }) => SessionEventKind::OutputReplaced {
                    message_id: message_id.to_string(),
                    replacement: replacement.clone(),
                },
                _ => Self::other_kind(event),
            },
            events::OUTPUT_MESSAGE_COMPLETED => match &event.data {
                EventData::OutputMessageCompleted(OutputMessageCompletedData {
                    message, ..
                }) => SessionEventKind::OutputCompleted {
                    message_id: message.id.to_string(),
                },
                _ => Self::other_kind(event),
            },
            events::TOOL_STARTED => match &event.data {
                EventData::ToolStarted(ToolStartedData { tool_call, .. }) => {
                    SessionEventKind::ToolStarted {
                        tool_call_id: tool_call.id.clone(),
                        tool_name: tool_call.name.clone(),
                    }
                }
                _ => Self::other_kind(event),
            },
            events::TOOL_COMPLETED => match &event.data {
                EventData::ToolCompleted(ToolCompletedData {
                    tool_call_id,
                    tool_name,
                    success,
                    ..
                }) => SessionEventKind::ToolCompleted {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    success: *success,
                },
                _ => Self::other_kind(event),
            },
            events::TOOL_PROGRESS => match &event.data {
                EventData::ToolProgress(ToolProgressData {
                    tool_call_id,
                    tool_name,
                    message,
                    ..
                }) => SessionEventKind::ToolProgress {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    message: message.clone(),
                },
                _ => Self::other_kind(event),
            },
            events::TOOL_OUTPUT_DELTA => match &event.data {
                EventData::ToolOutputDelta(ToolOutputDeltaData {
                    tool_call_id,
                    tool_name,
                    stream,
                    delta,
                }) => SessionEventKind::ToolOutputDelta {
                    tool_call_id: tool_call_id.clone(),
                    tool_name: tool_name.clone(),
                    stream: stream.clone(),
                    delta: delta.clone(),
                },
                _ => Self::other_kind(event),
            },
            events::REASON_STARTED => SessionEventKind::ReasonStarted,
            events::REASON_COMPLETED => match &event.data {
                EventData::ReasonCompleted(ReasonCompletedData { success, error, .. }) => {
                    SessionEventKind::ReasonCompleted {
                        success: *success,
                        error: error.clone(),
                    }
                }
                _ => Self::other_kind(event),
            },
            events::REASON_THINKING_DELTA => match &event.data {
                EventData::ReasonThinkingDelta(data) => SessionEventKind::ReasoningDelta {
                    delta: data.delta.clone(),
                    accumulated: data.accumulated.clone(),
                },
                _ => Self::other_kind(event),
            },
            events::REASON_THINKING_COMPLETED => match &event.data {
                EventData::ReasonThinkingCompleted(data) => SessionEventKind::ReasoningCompleted {
                    text: data.thinking.clone(),
                },
                _ => Self::other_kind(event),
            },
            events::REASON_ITEM => match &event.data {
                EventData::ReasonItem(data) => SessionEventKind::ReasoningItem {
                    provider: data.provider.clone(),
                    item_id: (!data.item_id.is_empty()).then(|| data.item_id.clone()),
                    summary: data.summary.clone(),
                },
                _ => Self::other_kind(event),
            },
            events::LLM_GENERATION => SessionEventKind::ModelGeneration,
            _ => Self::other_kind(event),
        };
        Self {
            event_id: event.id.to_string(),
            session_id: event.session_id.to_string(),
            turn_id,
            kind,
            event_type: event.event_type.clone(),
            timestamp: event.ts.to_rfc3339(),
            data,
            raw,
        }
    }

    fn other_kind(event: &Event) -> SessionEventKind {
        SessionEventKind::Other {
            event_type: event.event_type.clone(),
            payload: serde_json::to_value(&event.data)
                .expect("canonical event payloads are JSON serializable"),
        }
    }
}

/// A live feed of [`SessionEvent`]s for one [`Session`](crate::Session).
///
/// Obtain one from [`Session::events`](crate::Session::events) before running a
/// turn. Consume it with [`recv`](Self::recv) in a loop. Backed by a bounded
/// broadcast channel: dropping the stream never affects a running turn, and a
/// consumer that falls behind receives [`EventStreamError::Lagged`] rather than
/// blocking the runner or silently losing events.
pub struct EventStream {
    rx: broadcast::Receiver<SessionEvent>,
}

/// Why an [`EventStream`] could not deliver the next event losslessly.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EventStreamError {
    /// The consumer fell behind the bounded stream buffer.
    ///
    /// The next call can continue from the oldest retained event, but the
    /// consumer must treat its live projection as incomplete. Durable terminal
    /// events remain authoritative, and services that require replay should use
    /// a durable event transport rather than this in-process live stream.
    Lagged {
        /// Number of events evicted before this consumer could read them.
        missed: u64,
    },
}

impl std::fmt::Display for EventStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lagged { missed } => write!(f, "event stream lagged by {missed} events"),
        }
    }
}

impl std::error::Error for EventStreamError {}

impl EventStream {
    fn new(rx: broadcast::Receiver<SessionEvent>) -> Self {
        Self { rx }
    }

    /// Await the next event.
    ///
    /// Returns `Ok(None)` once the session is dropped and no further events can
    /// arrive. Returns [`EventStreamError::Lagged`] if the consumer fell behind
    /// and events were evicted from the bounded buffer; no loss is hidden.
    pub async fn recv(&mut self) -> Result<Option<SessionEvent>, EventStreamError> {
        match self.rx.recv().await {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                Err(EventStreamError::Lagged { missed })
            }
            Err(broadcast::error::RecvError::Closed) => Ok(None),
        }
    }

    /// Return the next already-available event without waiting.
    ///
    /// Returns `Ok(None)` when no event is currently buffered (or the session
    /// has ended). Returns [`EventStreamError::Lagged`] rather than hiding a
    /// gap.
    pub fn try_recv(&mut self) -> Result<Option<SessionEvent>, EventStreamError> {
        match self.rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::TryRecvError::Lagged(missed)) => {
                Err(EventStreamError::Lagged { missed })
            }
            Err(broadcast::error::TryRecvError::Empty)
            | Err(broadcast::error::TryRecvError::Closed) => Ok(None),
        }
    }
}

/// Options controlling a single [`Session::run_with`](crate::Session::run_with).
///
/// Cheap to construct and clone; extend it over time without breaking callers.
#[derive(Clone, Default)]
pub struct RunOptions {
    pub(crate) cancel: Option<CancellationToken>,
}

impl RunOptions {
    /// Default options: no cancellation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Attach a cancellation token. Cancelling it stops the turn in flight.
    pub fn cancel_token(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }
}

/// A handle for cancelling an in-flight turn.
///
/// Clone it to hold cancellation from another task; every clone shares one
/// signal. Pass it into a run via [`RunOptions::cancel_token`], then call
/// [`cancel`](Self::cancel) to stop the turn. Cancellation is cooperative and
/// drop-based: the turn's future is dropped at the next await point, which tears
/// down any in-flight tool work, and the run resolves to a cancelled
/// [`Turn`](crate::Turn).
#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: tokio_util::sync::CancellationToken,
}

impl CancellationToken {
    /// Create a fresh, un-cancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Idempotent; safe to call from any task.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    /// Resolve once cancellation has been requested. Facade-internal: the run
    /// loop selects on this against the turn future.
    pub(crate) async fn cancelled(&self) {
        self.inner.cancelled().await;
    }
}

/// Facade event bus: the post-commit event sink handed to the host.
///
/// The host assigns ids and persisted sequence numbers, commits durable events,
/// then calls this sink. It fans each finalized event out to every live
/// [`EventStream`] as a projected [`SessionEvent`]. Sending never blocks, so an
/// observer can never stall or fail a turn.
pub(crate) struct FacadeEventBus {
    sender: broadcast::Sender<SessionEvent>,
    active_turn: Mutex<Option<EventContext>>,
}

impl FacadeEventBus {
    pub(crate) fn new() -> Self {
        Self::with_capacity(EVENT_STREAM_CAPACITY)
    }

    fn with_capacity(capacity: usize) -> Self {
        let (sender, _rx) = broadcast::channel(capacity);
        Self {
            sender,
            active_turn: Mutex::new(None),
        }
    }

    /// Subscribe a new [`EventStream`] to this bus.
    pub(crate) fn subscribe(&self) -> EventStream {
        EventStream::new(self.sender.subscribe())
    }

    /// Build a correlated terminal request after the facade drops a cancelled
    /// turn future. The host emitter commits it before live delivery.
    #[cfg(test)]
    pub(crate) fn cancellation_request(&self, session_id: SessionId) -> (TurnId, EventRequest) {
        self.cancellation_request_for_turn(session_id, TurnId::new())
    }

    /// Build a cancellation request with a caller-assigned turn id when the
    /// runtime has not emitted `turn.started` yet.
    pub(crate) fn cancellation_request_for_turn(
        &self,
        session_id: SessionId,
        fallback_turn_id: TurnId,
    ) -> (TurnId, EventRequest) {
        let context = self
            .active_turn
            .lock()
            .expect("active-turn lock poisoned")
            .take()
            .unwrap_or_else(|| EventContext {
                turn_id: Some(fallback_turn_id),
                ..EventContext::default()
            });
        let turn_id = context.turn_id.unwrap_or(fallback_turn_id);
        let request = EventRequest::new(
            session_id,
            EventContext {
                turn_id: Some(turn_id),
                ..context
            },
            TurnCancelledData {
                turn_id,
                reason: Some("cancelled by application".to_string()),
                usage: None,
            },
        );
        (turn_id, request)
    }

    fn observe(&self, event: &Event) -> Result<(), EventSinkError> {
        match event.event_type.as_str() {
            events::TURN_STARTED => {
                *self.active_turn.lock().expect("active-turn lock poisoned") =
                    Some(event.context.clone());
            }
            events::TURN_COMPLETED
            | events::TURN_FAILED
            | events::TURN_CANCELLED
            | events::TURN_SEALED => {
                self.active_turn
                    .lock()
                    .expect("active-turn lock poisoned")
                    .take();
            }
            _ => {}
        }
        // A broadcast sender with no current receiver is still open: callers
        // can subscribe before the next turn. Observation is best-effort and
        // absence is equivalent to the host's no-op sink, not a delivery
        // failure worth counting on every canonical append.
        let _ = self.sender.send(SessionEvent::from_core_event(event));
        Ok(())
    }
}

impl EventSink for FacadeEventBus {
    fn try_send(&self, event: Event) -> Result<(), EventSinkError> {
        self.observe(&event)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use everruns_core::event_emitter::EventEmitter;
    use everruns_core::events::{
        ActStartedData, OutputMessageDeltaData, OutputMessageReplacedData, ToolStartedData,
        TurnCancelledData, TurnStartedData,
    };
    use everruns_host::{HostEventEmitter, InMemoryEventLog};
    use everruns_provider::tool_types::ToolCall;
    use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
    use serde_json::json;

    use super::{EventStreamError, FacadeEventBus, SessionEventKind};
    use crate::{Agent, InMemoryEngine, Model};

    fn host(bus: Arc<FacadeEventBus>) -> HostEventEmitter {
        HostEventEmitter::new(Arc::new(InMemoryEventLog::new()), bus)
    }

    fn turn_started(session_id: SessionId, turn_id: TurnId) -> everruns_core::EventRequest {
        let input_message_id = MessageId::new();
        everruns_core::EventRequest::new(
            session_id,
            everruns_core::EventContext::turn(turn_id, input_message_id),
            TurnStartedData {
                turn_id,
                input_message_id,
                input_content: Some("hello".to_string()),
            },
        )
    }

    #[tokio::test]
    async fn public_event_json_is_the_exact_canonical_envelope() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let bus = Arc::new(FacadeEventBus::new());
        let emitter = host(bus.clone());
        let mut stream = bus.subscribe();

        let request = turn_started(session_id, turn_id)
            .with_metadata(json!({"provider": "simulated"}))
            .with_tags(vec!["terminal".to_string()]);
        let canonical = emitter.emit(request).await.expect("event emits");
        let observed = stream
            .recv()
            .await
            .expect("stream remains lossless")
            .expect("event is delivered");

        assert!(matches!(observed.kind, SessionEventKind::TurnStarted));
        assert_eq!(
            observed.as_json(),
            &serde_json::to_value(canonical).expect("canonical event serializes")
        );
        assert_eq!(observed.event_type(), "turn.started");
        assert_eq!(
            observed.turn_id.as_deref(),
            Some(turn_id.to_string().as_str())
        );
        assert_eq!(observed.as_json()["sequence"], 1);
        assert_eq!(observed.sequence(), Some(1));
        assert!(!observed.timestamp().is_empty());
        assert_eq!(observed.data()["input_content"], "hello");
        assert_eq!(observed.as_json()["metadata"]["provider"], "simulated");
        assert_eq!(observed.as_json()["tags"], json!(["terminal"]));
    }

    #[tokio::test]
    async fn bounded_stream_reports_lag_instead_of_hiding_loss() {
        let session_id = SessionId::new();
        let bus = Arc::new(FacadeEventBus::with_capacity(2));
        let emitter = host(bus.clone());
        let mut stream = bus.subscribe();

        for _ in 0..3 {
            emitter
                .emit(turn_started(session_id, TurnId::new()))
                .await
                .expect("event emits without observer backpressure");
        }

        assert!(matches!(
            stream.recv().await,
            Err(EventStreamError::Lagged { missed: 1 })
        ));
        assert!(stream.recv().await.expect("gap reported").is_some());
    }

    #[tokio::test]
    async fn no_subscriber_is_a_noop_not_a_closed_sink_failure() {
        let bus = Arc::new(FacadeEventBus::new());
        let emitter = host(bus);

        emitter
            .emit(turn_started(SessionId::new(), TurnId::new()))
            .await
            .expect("observation absence cannot reverse the append");

        assert_eq!(emitter.delivery_stats().closed, 0);
    }

    #[tokio::test]
    async fn live_arrival_interleaves_durable_and_sequence_less_ephemeral_events() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let message_id = MessageId::new();
        let bus = Arc::new(FacadeEventBus::new());
        let emitter = host(bus.clone());
        let mut stream = bus.subscribe();

        emitter
            .emit(turn_started(session_id, turn_id))
            .await
            .unwrap();
        emitter
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::EventContext::turn(turn_id, message_id),
                OutputMessageDeltaData {
                    turn_id,
                    message_id,
                    delta: "hi".to_string(),
                    accumulated: "hi".to_string(),
                    phase: None,
                },
            ))
            .await
            .unwrap();
        emitter
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::EventContext::turn(turn_id, message_id),
                TurnCancelledData {
                    turn_id,
                    reason: Some("test".to_string()),
                    usage: None,
                },
            ))
            .await
            .unwrap();

        let started = stream.recv().await.unwrap().unwrap();
        let delta = stream.recv().await.unwrap().unwrap();
        let cancelled = stream.recv().await.unwrap().unwrap();

        assert_eq!(started.event_type(), "turn.started");
        assert_eq!(delta.event_type(), "output.message.delta");
        assert_eq!(delta.data()["delta"], "hi");
        assert!(delta.data().get("accumulated").is_none());
        assert!(delta.as_json()["data"].get("accumulated").is_none());
        assert_eq!(cancelled.event_type(), "turn.cancelled");
        assert_eq!(started.sequence(), Some(1));
        assert_eq!(delta.sequence(), None);
        assert!(delta.as_json().get("sequence").is_none());
        assert_eq!(cancelled.sequence(), Some(2));
    }

    #[tokio::test]
    async fn cancellation_uses_the_active_turn_and_canonical_sequence() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let bus = Arc::new(FacadeEventBus::new());
        let emitter = host(bus.clone());
        let mut stream = bus.subscribe();
        emitter
            .emit(turn_started(session_id, turn_id))
            .await
            .expect("turn starts");

        let (cancelled_turn_id, request) = bus.cancellation_request(session_id);
        emitter.emit(request).await.expect("cancellation commits");
        assert_eq!(cancelled_turn_id, turn_id);

        let started = stream.recv().await.expect("no lag").expect("start event");
        let cancelled = stream.recv().await.expect("no lag").expect("cancel event");
        assert!(matches!(started.kind, SessionEventKind::TurnStarted));
        assert!(matches!(cancelled.kind, SessionEventKind::TurnCancelled));
        assert_eq!(
            cancelled.turn_id.as_deref(),
            Some(turn_id.to_string().as_str())
        );
        assert_eq!(cancelled.as_json()["sequence"], 2);
        assert_eq!(
            cancelled.data(),
            &serde_json::to_value(TurnCancelledData {
                turn_id,
                reason: Some("cancelled by application".to_string()),
                usage: None,
            })
            .expect("cancel data serializes")
        );
    }

    #[tokio::test]
    async fn output_replacement_retains_rebuildable_message_identity_and_text() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let message_id = MessageId::new();
        let bus = Arc::new(FacadeEventBus::new());
        let emitter = host(bus.clone());
        let mut stream = bus.subscribe();
        emitter
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::EventContext {
                    turn_id: Some(turn_id),
                    ..everruns_core::EventContext::default()
                },
                OutputMessageReplacedData {
                    turn_id,
                    message_id,
                    guardrail_capability_id: "guardrails".to_string(),
                    guardrail_id: "output-policy".to_string(),
                    reason_code: "blocked".to_string(),
                    replacement: "Response withheld.".to_string(),
                },
            ))
            .await
            .expect("replacement emits");

        let replacement = stream
            .recv()
            .await
            .expect("no lag")
            .expect("replacement delivered");
        assert!(matches!(
            &replacement.kind,
            SessionEventKind::OutputReplaced {
                message_id: observed_id,
                replacement,
            } if observed_id == &message_id.to_string() && replacement == "Response withheld."
        ));
        assert_eq!(replacement.data()["message_id"], message_id.to_string());
        assert_eq!(replacement.data()["replacement"], "Response withheld.");
    }

    #[tokio::test]
    async fn tool_narration_is_preserved_for_renderers() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let bus = Arc::new(FacadeEventBus::new());
        let emitter = host(bus.clone());
        let mut stream = bus.subscribe();

        emitter
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::EventContext {
                    turn_id: Some(turn_id),
                    ..everruns_core::EventContext::default()
                },
                ToolStartedData {
                    tool_call: ToolCall {
                        id: "call_1".to_string(),
                        name: "lookup".to_string(),
                        arguments: json!({"key": "answer"}),
                    },
                    tool_call_fingerprint: None,
                    display_name: Some("Knowledge lookup".to_string()),
                    narration: Some("Looking up the answer".to_string()),
                },
            ))
            .await
            .expect("tool start emits");

        let observed = stream.recv().await.unwrap().unwrap();
        assert_eq!(observed.narration(), Some("Looking up the answer"));
        assert_eq!(observed.data()["display_name"], "Knowledge lookup");
    }

    #[tokio::test]
    async fn unpromoted_event_kind_retains_its_complete_canonical_payload() {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let bus = Arc::new(FacadeEventBus::new());
        let emitter = host(bus.clone());
        let mut stream = bus.subscribe();
        let canonical = emitter
            .emit(everruns_core::EventRequest::new(
                session_id,
                everruns_core::EventContext {
                    turn_id: Some(turn_id),
                    ..everruns_core::EventContext::default()
                },
                ActStartedData {
                    tool_calls: Vec::new(),
                    headline: Some("running tools".to_string()),
                },
            ))
            .await
            .expect("event emits");

        let observed = stream.recv().await.unwrap().unwrap();
        assert!(matches!(
            &observed.kind,
            SessionEventKind::Other { event_type, payload }
                if event_type == "act.started" && payload["headline"] == "running tools"
        ));
        assert_eq!(
            observed.as_json(),
            &serde_json::to_value(canonical).expect("canonical event serializes")
        );
    }

    #[tokio::test]
    async fn provider_failure_retains_reason_and_turn_terminal_payloads() {
        let agent = Agent::builder()
            .instructions("Answer concisely.")
            .model(Model::simulated_error("provider unavailable"))
            .build()
            .expect("valid agent");
        let session = InMemoryEngine::new().create(agent.clone());
        let mut stream = session.events();
        let result = session
            .run("hello")
            .await
            .expect("provider failure resolves to a failed turn");
        assert!(!result.success);
        drop(session);

        let mut observed = Vec::new();
        while let Some(event) = stream.recv().await.expect("failure stream does not lag") {
            observed.push(event);
        }

        let reason_failure = observed
            .iter()
            .find(|event| {
                matches!(
                    event.kind,
                    SessionEventKind::ReasonCompleted { success: false, .. }
                )
            })
            .expect("reason.completed preserves the provider failure");
        assert!(
            reason_failure.data()["error"]
                .as_str()
                .is_some_and(|error| error.contains("provider unavailable"))
        );

        let turn_failure = observed
            .iter()
            .find(|event| matches!(event.kind, SessionEventKind::TurnFailed { .. }))
            .expect("turn.failed is the terminal event");
        assert_eq!(turn_failure.event_type(), "turn.failed");
        assert_eq!(
            turn_failure.turn_id.as_deref(),
            Some(result.turn_id.as_str())
        );
        assert!(turn_failure.data()["error"].as_str().is_some());
    }

    #[tokio::test]
    async fn tool_lifecycle_retains_arguments_result_and_order() {
        let tool = crate::FunctionTool::new(
            "lookup",
            "Look up a value.",
            json!({
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"]
            }),
            |arguments: serde_json::Value| async move {
                Ok::<_, String>(json!({ "value": arguments["key"] }))
            },
        );
        let agent = Agent::builder()
            .instructions("Use the lookup tool.")
            .model(Model::simulated_scripted(
                "done",
                vec![
                    vec![ToolCall {
                        id: "call_lookup_1".to_string(),
                        name: "lookup".to_string(),
                        arguments: json!({ "key": "answer" }),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent");
        let session = InMemoryEngine::new().create(agent.clone());
        let mut stream = session.events();
        let result = session.run("look it up").await.expect("tool turn runs");
        assert!(result.success);
        drop(session);

        let mut observed = Vec::new();
        while let Some(event) = stream.recv().await.expect("tool stream does not lag") {
            observed.push(event);
        }
        let started = observed
            .iter()
            .find(|event| matches!(event.kind, SessionEventKind::ToolStarted { .. }))
            .expect("tool.started");
        let completed = observed
            .iter()
            .find(|event| matches!(event.kind, SessionEventKind::ToolCompleted { .. }))
            .expect("tool.completed");

        assert!(
            started.sequence().expect("tool start is durable")
                < completed.sequence().expect("tool completion is durable")
        );
        assert_eq!(started.data()["tool_call"]["id"], "call_lookup_1");
        assert_eq!(started.data()["tool_call"]["arguments"]["key"], "answer");
        assert_eq!(completed.data()["tool_call_id"], "call_lookup_1");
        assert_eq!(completed.data()["status"], "success");
        assert!(completed.data()["result"].is_array());
    }
}
