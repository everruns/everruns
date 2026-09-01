//! Observable and cancellable turns.
//!
//! This module promotes two capabilities onto the [`Session`](crate::Session)
//! surface without leaking host/runtime internals or core event/store types:
//!
//! - **Observation.** [`Session::events`](crate::Session::events) returns an
//!   [`EventStream`] — a live feed of [`SessionEvent`]s projected from the
//!   host's canonical event emitter. Each event carries a reviewed typed
//!   projection, and the full canonical envelope stays available through
//!   [`SessionEvent::canonical_json`]. The stream is bounded, so a slow or
//!   dropped consumer can never stall the turn that produces the events; lag is
//!   reported explicitly to the consumer.
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
/// Two surfaces, deliberately separated:
///
/// - **Reviewed** — [`kind`](SessionEvent::kind) and the matching
///   [`data`](SessionEvent::data)/[`as_json`](SessionEvent::as_json). Carries
///   only fields someone chose to promote, so a new internal field cannot
///   silently become this crate's public API or reach wherever an application
///   forwards these envelopes.
/// - **Canonical** — [`canonical_json`](SessionEvent::canonical_json). The
///   complete envelope, nothing withheld, for auditing and replay. It follows
///   the runtime's internal shape, may contain prompts and tool results, and is
///   not covered by this crate's stability guarantees.
///
/// Envelope fields — timestamp, optional persisted sequence, correlation
/// context, metadata, and tags — are complete on both. For
/// `output.message.delta` events, the redundant `data.accumulated` prefix is
/// omitted from both to keep buffered stream memory proportional to output size
/// rather than quadratic in it. Incremental text remains in
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
    canonical: Value,
}

/// The event-specific payload of a [`SessionEvent`].
///
/// Non-exhaustive on purpose: event kinds useful to an application renderer are
/// promoted here, while every other event is identified through
/// [`Other`](SessionEventKind::Other). No event is ever dropped — the complete
/// payload of every variant, including event types this version of the crate
/// does not recognize, stays available through
/// [`SessionEvent::canonical_json`].
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
    /// The accounting fields are promoted here because tracking spend and
    /// latency is a first-class reason to observe a session, and none of them
    /// carry conversation content. The generation's messages, tool requests,
    /// and output text stay off the reviewed surface; reach them with
    /// [`SessionEvent::canonical_json`] when you actually need them.
    ModelGeneration {
        /// Model identifier used for the generation.
        model: String,
        /// Provider that served the generation, when recorded.
        provider: Option<String>,
        /// Prompt tokens billed for this generation.
        input_tokens: Option<u32>,
        /// Completion tokens billed for this generation.
        output_tokens: Option<u32>,
        /// Best-effort USD cost: the provider's actual cost when it reported
        /// one, otherwise the price-table estimate.
        cost_usd: Option<f64>,
        /// Wall-clock duration of the generation.
        duration_ms: Option<u64>,
        /// Whether the generation completed successfully.
        success: bool,
    },
    /// Any other canonical event, identified but not projected.
    ///
    /// `event_type` is the stable dot-notation type string (e.g.
    /// `"act.started"`). This is the forward-compatibility fallback — match it
    /// with a wildcard arm. The event's payload is deliberately absent from the
    /// reviewed surface, because a type this version does not recognize has by
    /// definition not been reviewed for what it exposes; read it from
    /// [`SessionEvent::canonical_json`] if you need it.
    Other {
        /// Stable dot-notation event type string.
        event_type: String,
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

    /// The event envelope as JSON, carrying the reviewed `data` projection.
    ///
    /// Envelope fields — timestamp, sequence, correlation context, metadata,
    /// tags — are complete. `data` holds the same reviewed projection as
    /// [`data`](Self::data) rather than the raw internal payload, so a new
    /// internal field cannot become part of this crate's public surface, or
    /// reach whatever an application forwards these envelopes to, without
    /// someone choosing to promote it.
    ///
    /// Use [`canonical_json`](Self::canonical_json) for the untouched payload.
    pub fn as_json(&self) -> &Value {
        &self.raw
    }

    /// Consume the event and return its envelope with the reviewed `data`
    /// projection. See [`as_json`](Self::as_json).
    pub fn into_json(self) -> Value {
        self.raw
    }

    /// The complete canonical event envelope, including the raw internal
    /// payload.
    ///
    /// Nothing is withheld, so nothing observable is lost: this is the escape
    /// hatch for auditing, recording, and replay. `data.accumulated` on
    /// `output.message.delta` is the one exception — that redundant growing
    /// prefix is dropped at ingest so a slow subscriber cannot retain quadratic
    /// memory, and the incremental text is in
    /// [`SessionEventKind::TextDelta`].
    ///
    /// The payload here follows the runtime's internal shape rather than this
    /// crate's reviewed surface. It can contain prompts, tool arguments, and
    /// tool results, and it can gain or change fields in a patch release. Treat
    /// what you read from it as unstable, and do not forward it anywhere the
    /// conversation itself should not go.
    pub fn canonical_json(&self) -> &Value {
        &self.canonical
    }

    /// The reviewed event-specific `data` payload.
    ///
    /// Contains the fields promoted onto [`SessionEventKind`] for this event and
    /// nothing else, so it is safe to log or forward. A renderer needing a field
    /// that is not promoted should ask for it to be promoted; a consumer that
    /// genuinely needs the internal payload should call
    /// [`canonical_json`](Self::canonical_json) and accept its instability.
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
        // Kept whole before `raw` is narrowed to the reviewed projection, so
        // `canonical_json` can still hand back everything the runtime emitted.
        let canonical = raw.clone();
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
            events::LLM_GENERATION => match &event.data {
                EventData::LlmGeneration(data) => {
                    let usage = data.metadata.usage.as_ref();
                    SessionEventKind::ModelGeneration {
                        model: data.metadata.model.clone(),
                        provider: data.metadata.provider.clone(),
                        input_tokens: usage.map(|usage| usage.input_tokens),
                        output_tokens: usage.map(|usage| usage.output_tokens),
                        cost_usd: usage.and_then(|usage| usage.effective_cost_usd()),
                        duration_ms: data.metadata.duration_ms,
                        success: data.metadata.success,
                    }
                }
                _ => Self::other_kind(event),
            },
            _ => Self::other_kind(event),
        };
        // Narrow `data` to what `kind` promotes. Everything the runtime emitted
        // stays reachable through `canonical`; this is what makes an unreviewed
        // internal field a deliberate promotion rather than an accident.
        let data = Self::reviewed_data(&kind, &data);
        raw["data"] = data.clone();
        Self {
            event_id: event.id.to_string(),
            session_id: event.session_id.to_string(),
            turn_id,
            kind,
            event_type: event.event_type.clone(),
            timestamp: event.ts.to_rfc3339(),
            data,
            raw,
            canonical,
        }
    }

    /// Build the reviewed `data` payload for a projected event.
    ///
    /// Mirrors [`SessionEventKind`] field for field. Adding a variant field
    /// means adding it here; an event whose kind promotes nothing gets an empty
    /// object rather than its internal payload.
    fn reviewed_data(kind: &SessionEventKind, data: &Value) -> Value {
        // Operator-authored display text, reviewed as safe wherever it appears:
        // a human wrote it for a timeline, so it carries no model or tool
        // content. `narration()` reads the first of these back.
        const DISPLAY_FIELDS: [&str; 2] = ["narration", "display_name"];
        let mut reviewed = match kind {
            SessionEventKind::InputMessage { message_id }
            | SessionEventKind::OutputStarted { message_id } => {
                serde_json::json!({ "message_id": message_id })
            }
            SessionEventKind::OutputCompleted { message_id } => {
                serde_json::json!({ "message_id": message_id })
            }
            SessionEventKind::OutputReplaced {
                message_id,
                replacement,
            } => serde_json::json!({ "message_id": message_id, "replacement": replacement }),
            SessionEventKind::TurnFailed { error } => serde_json::json!({ "error": error }),
            SessionEventKind::TextDelta { delta } => serde_json::json!({ "delta": delta }),
            SessionEventKind::ToolStarted {
                tool_call_id,
                tool_name,
            } => serde_json::json!({ "tool_call_id": tool_call_id, "tool_name": tool_name }),
            SessionEventKind::ToolCompleted {
                tool_call_id,
                tool_name,
                success,
            } => serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "success": success,
            }),
            SessionEventKind::ToolProgress {
                tool_call_id,
                tool_name,
                message,
            } => serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "message": message,
            }),
            SessionEventKind::ToolOutputDelta {
                tool_call_id,
                tool_name,
                stream,
                delta,
            } => serde_json::json!({
                "tool_call_id": tool_call_id,
                "tool_name": tool_name,
                "stream": stream,
                "delta": delta,
            }),
            SessionEventKind::ReasoningDelta { delta, accumulated } => {
                serde_json::json!({ "delta": delta, "accumulated": accumulated })
            }
            SessionEventKind::ReasonCompleted { success, error } => {
                serde_json::json!({ "success": success, "error": error })
            }
            SessionEventKind::ReasoningCompleted { text } => serde_json::json!({ "text": text }),
            SessionEventKind::ReasoningItem {
                provider,
                item_id,
                summary,
            } => serde_json::json!({
                "provider": provider,
                "item_id": item_id,
                "summary": summary,
            }),
            SessionEventKind::ModelGeneration {
                model,
                provider,
                input_tokens,
                output_tokens,
                cost_usd,
                duration_ms,
                success,
            } => serde_json::json!({
                "model": model,
                "provider": provider,
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cost_usd": cost_usd,
                "duration_ms": duration_ms,
                "success": success,
            }),
            // Reviewed safe in full: turn id, cancellation reason, and usage
            // totals. The payload carries no conversation content.
            SessionEventKind::TurnCancelled => data.clone(),
            SessionEventKind::Other { .. }
            | SessionEventKind::TurnStarted
            | SessionEventKind::TurnCompleted
            | SessionEventKind::ReasonStarted => serde_json::json!({}),
        };
        if let Some(object) = reviewed.as_object_mut() {
            for field in DISPLAY_FIELDS {
                if let Some(value) = data.get(field) {
                    object.insert(field.to_string(), value.clone());
                }
            }
        }
        reviewed
    }

    fn other_kind(event: &Event) -> SessionEventKind {
        SessionEventKind::Other {
            event_type: event.event_type.clone(),
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

    use super::{EventStreamError, FacadeEventBus, SessionEvent, SessionEventKind};
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

    #[test]
    fn reviewed_data_never_exceeds_the_canonical_payload() {
        // The reviewed projection must be a *subset* of what the runtime
        // emitted: it may drop fields, never invent them. A synthesised field
        // would be a value no consumer could correlate with the canonical
        // record, and would quietly become API nobody reviewed.
        let canonical = json!({
            "tool_call_id": "call_1",
            "tool_name": "lookup",
            "success": true,
            "status": "success",
            "result": ["secret"],
            "narration": "Looking it up",
            "display_name": "Knowledge lookup",
        });
        let kind = SessionEventKind::ToolCompleted {
            tool_call_id: "call_1".to_string(),
            tool_name: "lookup".to_string(),
            success: true,
        };

        let reviewed = SessionEvent::reviewed_data(&kind, &canonical);
        let reviewed = reviewed.as_object().expect("reviewed data is an object");

        for (key, value) in reviewed {
            assert_eq!(
                Some(value),
                canonical.get(key),
                "reviewed key {key} is absent from or differs in the canonical payload"
            );
        }
        // Promoted identity and outcome survive; the tool's result does not.
        assert_eq!(reviewed["tool_call_id"], "call_1");
        assert_eq!(reviewed["narration"], "Looking it up");
        assert!(!reviewed.contains_key("result"));
    }

    #[tokio::test]
    async fn envelope_is_complete_while_data_stays_reviewed() {
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

        // `turn.started` carries the user's prompt in `data.input_content`.
        // `TurnStarted` promotes nothing, so the reviewed surface omits it —
        // this is the leak the split exists to close, since an application
        // forwarding these envelopes would otherwise ship the prompt with them.
        assert_eq!(observed.data(), &json!({}));
        assert!(!observed.as_json().to_string().contains("hello"));

        // Nothing is lost: the canonical envelope is byte-for-byte the event
        // the runtime emitted, prompt included.
        assert_eq!(
            observed.canonical_json(),
            &serde_json::to_value(canonical).expect("canonical event serializes")
        );
        assert_eq!(observed.canonical_json()["data"]["input_content"], "hello");

        // Envelope fields stay complete on the reviewed form; only `data` differs.
        assert_eq!(observed.event_type(), "turn.started");
        assert_eq!(
            observed.turn_id.as_deref(),
            Some(turn_id.to_string().as_str())
        );
        assert_eq!(observed.as_json()["sequence"], 1);
        assert_eq!(observed.sequence(), Some(1));
        assert!(!observed.timestamp().is_empty());
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
    async fn unpromoted_event_kind_is_identified_but_not_projected() {
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

        // Identified, so a renderer can still route it.
        assert!(matches!(
            &observed.kind,
            SessionEventKind::Other { event_type } if event_type == "act.started"
        ));

        // Not projected: an event type this version does not recognize has by
        // definition not been reviewed, so its payload stays off the reviewed
        // surface instead of becoming public API by accident.
        assert_eq!(observed.data(), &json!({}));
        assert!(!observed.as_json().to_string().contains("running tools"));

        // But nothing is lost — the complete envelope is one explicit call away.
        assert_eq!(
            observed.canonical_json(),
            &serde_json::to_value(canonical).expect("canonical event serializes")
        );
        assert_eq!(
            observed.canonical_json()["data"]["headline"],
            "running tools"
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
    async fn tool_lifecycle_keeps_order_and_reaches_arguments_canonically() {
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
        // Identity and outcome are promoted, so a timeline renders from the
        // reviewed surface alone.
        assert_eq!(started.data()["tool_call_id"], "call_lookup_1");
        assert_eq!(completed.data()["tool_call_id"], "call_lookup_1");
        assert_eq!(completed.data()["success"], true);

        // Arguments and results are the payload most worth not leaking by
        // default — a tool call can carry credentials in, and file contents out.
        assert!(started.data()["tool_call"].is_null());
        assert!(completed.data()["result"].is_null());

        // Still reachable for an auditor or recorder that asks explicitly.
        assert_eq!(
            started.canonical_json()["data"]["tool_call"]["arguments"]["key"],
            "answer"
        );
        assert_eq!(completed.canonical_json()["data"]["status"], "success");
        assert!(completed.canonical_json()["data"]["result"].is_array());
    }
}
