//! Observable and cancellable turns (EVE-833).
//!
//! This module promotes two capabilities onto the [`Session`](crate::Session)
//! surface without leaking the runtime's event bus or the core event/store
//! types:
//!
//! - **Observation.** [`Session::events`](crate::Session::events) returns an
//!   [`EventStream`] — a live feed of [`SessionEvent`]s projected from the
//!   runtime's in-process event bus. The stream is backed by a broadcast
//!   channel, so a slow or dropped consumer can never stall the turn that
//!   produces the events; it only lags and skips.
//! - **Cancellation.** [`Session::run_with`](crate::Session::run_with) accepts a
//!   [`RunOptions`] carrying an optional [`CancellationToken`]. Cancelling the
//!   token stops the in-flight turn by dropping its future — the same
//!   cooperative, drop-based cancellation the runtime already uses to tear down
//!   tool work — and the turn resolves to a stable [`Turn`](crate::Turn) with
//!   [`TurnStopReason::Cancelled`](everruns_core::turn::TurnStopReason::Cancelled).
//!
//! Only facade types appear on the public surface: no `EventBus`,
//! `EventEmitter`, `EventRequest`, or core event/store types.

use std::sync::atomic::{AtomicI32, Ordering};

use async_trait::async_trait;
use everruns_core::error::Result as CoreResult;
use everruns_core::events::{
    self, Event, EventData, EventRequest, OutputMessageDeltaData, ToolCompletedData,
    ToolProgressData, ToolStartedData, TurnFailedData,
};
use everruns_core::traits::EventEmitter;
use everruns_core::typed_id::EventId;
use everruns_runtime::EventBus;
use serde_json::Value;
use tokio::sync::broadcast;

/// Capacity of the per-session broadcast channel that backs [`EventStream`].
///
/// A turn emits well under this many events, so a consumer that keeps up never
/// lags. A consumer that falls behind by more than this drops the oldest events
/// (reported as a skip) rather than applying back-pressure to the turn — the
/// runner must never block on an observer.
const EVENT_CHANNEL_CAPACITY: usize = 4096;

/// A single observed event from a running [`Session`](crate::Session).
///
/// A small, stable projection of one runtime event. The correlation ids
/// (`event_id`, `session_id`, and the optional `turn_id`) are preserved on every
/// event so a consumer can line events up with a [`Turn`](crate::Turn); the
/// [`kind`](SessionEvent::kind) carries the event-specific payload.
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
}

/// The event-specific payload of a [`SessionEvent`].
///
/// Non-exhaustive on purpose: new hosted event types are surfaced through the
/// [`Other`](SessionEventKind::Other) fallback, which carries the stable event
/// type string and the raw JSON payload, so promoting a new event to a typed
/// variant later never breaks a consumer that already matches with a wildcard.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SessionEventKind {
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
    /// A best-effort progress update emitted by an in-flight tool.
    ToolProgress {
        /// Opaque id of the tool call.
        tool_call_id: String,
        /// Name of the tool emitting progress.
        tool_name: String,
        /// Human-readable interim status.
        message: String,
    },
    /// Any other runtime event, carried through unprojected.
    ///
    /// `event_type` is the stable dot-notation type string (e.g.
    /// `"reason.started"`); `payload` is the raw event data as JSON. This is the
    /// forward-compatibility fallback — match it with a wildcard arm.
    Other {
        /// Stable dot-notation event type string.
        event_type: String,
        /// Raw event payload as JSON.
        payload: Value,
    },
}

impl SessionEvent {
    /// The stable dot-notation type string for this event (e.g.
    /// `"turn.started"`), matching the runtime event protocol.
    pub fn event_type(&self) -> &str {
        match &self.kind {
            SessionEventKind::TurnStarted => events::TURN_STARTED,
            SessionEventKind::TurnCompleted => events::TURN_COMPLETED,
            SessionEventKind::TurnFailed { .. } => events::TURN_FAILED,
            SessionEventKind::TurnCancelled => events::TURN_CANCELLED,
            SessionEventKind::TextDelta { .. } => events::OUTPUT_MESSAGE_DELTA,
            SessionEventKind::ToolStarted { .. } => events::TOOL_STARTED,
            SessionEventKind::ToolCompleted { .. } => events::TOOL_COMPLETED,
            SessionEventKind::ToolProgress { .. } => events::TOOL_PROGRESS,
            SessionEventKind::Other { event_type, .. } => event_type,
        }
    }

    /// Project a core [`Event`] into a facade [`SessionEvent`].
    ///
    /// Known event types map to typed [`SessionEventKind`] variants; every other
    /// type is carried through the [`Other`](SessionEventKind::Other) fallback,
    /// so no event is ever dropped.
    fn from_core_event(event: &Event) -> Self {
        let turn_id = event.context.turn_id.map(|id| id.to_string());
        let kind = match event.event_type.as_str() {
            events::TURN_STARTED => SessionEventKind::TurnStarted,
            events::TURN_COMPLETED => SessionEventKind::TurnCompleted,
            events::TURN_CANCELLED => SessionEventKind::TurnCancelled,
            events::TURN_FAILED => {
                let error = match &event.data {
                    EventData::TurnFailed(TurnFailedData { error, .. }) => error.clone(),
                    _ => String::new(),
                };
                SessionEventKind::TurnFailed { error }
            }
            events::OUTPUT_MESSAGE_DELTA => {
                let delta = match &event.data {
                    EventData::OutputMessageDelta(OutputMessageDeltaData { delta, .. }) => {
                        delta.clone()
                    }
                    _ => String::new(),
                };
                SessionEventKind::TextDelta { delta }
            }
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
            _ => Self::other_kind(event),
        };
        Self {
            event_id: event.id.to_string(),
            session_id: event.session_id.to_string(),
            turn_id,
            kind,
        }
    }

    fn other_kind(event: &Event) -> SessionEventKind {
        SessionEventKind::Other {
            event_type: event.event_type.clone(),
            payload: serde_json::to_value(&event.data).unwrap_or(Value::Null),
        }
    }
}

/// A live feed of [`SessionEvent`]s for one [`Session`](crate::Session).
///
/// Obtain one from [`Session::events`](crate::Session::events) before running a
/// turn. Consume it with [`recv`](Self::recv) in a loop. Backed by a broadcast
/// channel: dropping the stream never affects a running turn, and a consumer
/// that falls behind skips old events rather than blocking the runner.
pub struct EventStream {
    rx: broadcast::Receiver<SessionEvent>,
}

impl EventStream {
    fn new(rx: broadcast::Receiver<SessionEvent>) -> Self {
        Self { rx }
    }

    /// Await the next event.
    ///
    /// Returns `None` once the session is dropped and no further events can
    /// arrive. If the consumer fell behind and the channel dropped events, those
    /// events are skipped silently and the next available event is returned.
    pub async fn recv(&mut self) -> Option<SessionEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) => return Some(event),
                // The consumer lagged; skip the dropped events and keep reading.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// Return the next already-available event without waiting.
    ///
    /// Returns `None` when no event is currently buffered (or the session has
    /// ended). Skips past a lagged gap the same way [`recv`](Self::recv) does.
    pub fn try_recv(&mut self) -> Option<SessionEvent> {
        loop {
            match self.rx.try_recv() {
                Ok(event) => return Some(event),
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Empty)
                | Err(broadcast::error::TryRecvError::Closed) => return None,
            }
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

/// Facade event bus: the raw in-process event sink handed to the runtime.
///
/// Implements the runtime's [`EventBus`] contract (and thus core's
/// [`EventEmitter`]). It assigns each event an id and sequence exactly as the
/// runtime's built-in in-memory emitter does — so downstream message
/// persistence still works — then fans the event out to every live
/// [`EventStream`] as a projected [`SessionEvent`]. Sending never blocks and
/// never errors on a full or subscriber-less channel, so an observer can never
/// stall or fail a turn.
pub(crate) struct FacadeEventBus {
    sender: broadcast::Sender<SessionEvent>,
    sequence: AtomicI32,
}

impl FacadeEventBus {
    pub(crate) fn new() -> Self {
        let (sender, _rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            sender,
            sequence: AtomicI32::new(0),
        }
    }

    /// Subscribe a new [`EventStream`] to this bus.
    pub(crate) fn subscribe(&self) -> EventStream {
        EventStream::new(self.sender.subscribe())
    }
}

#[async_trait]
impl EventEmitter for FacadeEventBus {
    async fn emit(&self, request: EventRequest) -> CoreResult<Event> {
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let event = request.into_event(EventId::new(), seq);
        // Ignore the send result: an error means there are simply no live
        // subscribers, which must not affect the turn.
        let _ = self.sender.send(SessionEvent::from_core_event(&event));
        Ok(event)
    }
}

// The default `collected_events` (empty) is correct: this bus streams rather
// than retaining. Observers read the live `EventStream`, not a post-hoc buffer.
impl EventBus for FacadeEventBus {}
