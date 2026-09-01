---
title: Canonical Framework events
description: Observe a complete agent turn through a bounded typed/raw bridge while keeping durability, live delivery, and derived history distinct.
sidebar:
  order: 2
---

`Session::events()` installs an in-process subscriber without exposing runtime
event buses or core event types. Subscribe before `Session::run()` so the stream
sees the turn from its first event.

```rust
use everruns::prelude::*;

let agent = Agent::builder()
    .instructions("Answer concisely.")
    .model(Model::simulated("Hello!"))
    .build()?;
let engine = Engine::new();
let mut session = engine.create(agent);
let mut events = session.events();

let observer = tokio::spawn(async move {
    let mut canonical_events = Vec::new();
    while let Some(event) = events.recv().await? {
        // Recording for replay, so take the canonical envelope: it withholds
        // nothing. `as_json()` carries the reviewed projection instead.
        canonical_events.push(event.canonical_json().clone());

        // Typed rendering for common terminal/service UI concerns.
        match event.kind {
            SessionEventKind::TextDelta { delta } => print!("{delta}"),
            SessionEventKind::ToolStarted { tool_name, .. } => {
                eprintln!("starting {tool_name}");
            }
            SessionEventKind::ToolCompleted {
                tool_name,
                success,
                ..
            } => eprintln!("{tool_name}: {success}"),
            SessionEventKind::TurnFailed { error } => eprintln!("failed: {error}"),
            SessionEventKind::TurnCancelled => eprintln!("cancelled"),
            _ => {}
        }
    }
    Ok::<_, EventStreamError>(canonical_events)
});

let turn = session.run("hello").await?;
drop(session); // closes the subscriber once buffered events are drained
let recorded = observer.await??;
assert!(!recorded.is_empty());
```

## One protocol, two views

`SessionEventKind` is a convenience projection for application renderers. It
promotes assistant output lifecycle and deltas, model reasoning/generation,
tool lifecycle/progress/output, and turn terminal states. It is non-exhaustive,
so match it with a fallback arm.

`SessionEvent` exposes two surfaces, and which one you want depends on whether
you are rendering or recording.

`SessionEvent::as_json()` and `SessionEvent::data()` return the **reviewed**
form: the event envelope — id, type, timestamp, optional persisted sequence,
correlation context, metadata, tags — with a `data` payload holding only the
fields promoted onto `SessionEventKind`. Nothing else reaches it, so a field
added inside the runtime cannot become part of the Framework's public surface,
or travel to wherever your application forwards these envelopes, without being
promoted first. This is the form to log, forward, or expose to clients.

`SessionEvent::canonical_json()` returns the **canonical** envelope with the
complete payload: prompts, tool arguments, tool results, structured assistant
messages, and the payloads of event types this version does not recognize.
Nothing observable is lost — this is the form for recording, auditing, and
replay. It follows the runtime's internal shape rather than the Framework's
reviewed surface, so treat what you read from it as unstable, and do not forward
it anywhere the conversation itself should not go.

Live `output.message.delta` envelopes omit the redundant `data.accumulated`
prefix on both surfaces; retaining every growing prefix in a slow subscriber's
buffer would use quadratic memory. Concatenate the typed `TextDelta::delta`
values to reconstruct streamed text, or use the subsequent
`output.message.completed` event for the complete message.

Model-generation accounting — model, provider, token counts, cost, and duration
— is promoted onto `SessionEventKind::ModelGeneration`, so tracking spend never
requires the unstable surface.

This bridge does not define a second wire schema. The canonical event contract,
compatibility rules, and lifecycle semantics remain documented in
[Events](/explanation/events/).

## Durability, observation, and derived history

These roles are deliberately separate:

- `EventLog` is the host's sole durable conversation write authority. It stores
  complete canonical event envelopes and provides bounded cursor replay.
- `EventSink` is the host's post-commit, nonblocking live-delivery boundary.
  `Session::events()` exposes that observation path as an ergonomic
  `EventStream` subscriber. Neither sink nor subscriber is durable or
  authoritative.
- `EventHistory` is one read-only message projection rebuilt from `EventLog`
  replay. It is an index/view, never a second writable message store.

Framework applications read that bounded projection through
[`Session::history()`](/framework/session-history/). It pages messages from a
stable event-log snapshot; it does not maintain or write an independent
transcript.

Rebuild a transcript in persisted sequence order from `input.message`,
`output.message.completed`, and relevant `tool.completed` events. An
`output.message.replaced` event alone creates no history message; the subsequent
completed message contains the safe replacement. If a crash leaves a
replacement without completion, replay correctly omits that incomplete output.
The Framework stream exposes canonical payloads, subject to the live-delta
exception above, and introduces no independent writable message history.

Canonical recordings can contain user messages, agent instructions, model
inputs, tool arguments, and tool results. Treat them as application data with
the same access controls and retention policy as the session itself; do not log
them indiscriminately. Model and tool text is untrusted: terminal renderers
should strip or escape control sequences, and web renderers should escape it as
content rather than interpreting it as markup or commands. Provider credentials
are not part of the event protocol.

## Implementing a custom event log

An advanced host can store canonical events itself. `everruns-host` exposes
`EventReader` and `EventLog` as a public SPI: an external crate implements both
against its own storage and supplies the result to composition through
`HostBackends::with_event_log`. No in-crate access is required, cursors and
pages are built with `EventCursor::continuation`, `EventCursor::after`, and
`EventPage::new`, which validate the shared invariants.

Three request shapes are distinguished by `EventReadRequest::cursor()`:

- no cursor is an initial read that captures the session's current
  high-watermark and reports it as `EventPage::snapshot_high_watermark()`;
- a cursor whose `snapshot_high_watermark()` is `Some` is a continuation pinned
  to that snapshot, so appends committed later stay invisible and paging neither
  skips nor duplicates;
- a cursor whose `snapshot_high_watermark()` is `None`, built by
  `EventCursor::after`, is a poll that captures a fresh snapshot and therefore
  does observe those later appends.

```rust
use async_trait::async_trait;
use everruns_core::events::{Event, EventRequest};
use everruns_provider::typed_id::EventId;
use everruns_host::{
    EventCursor, EventDurability, EventLog, EventLogError, EventPage, EventReadRequest,
    EventReader,
};

#[async_trait]
impl EventReader for MyEventLog {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        let session_id = request.session_id();
        let current_high = self.high_watermark(session_id);
        let (after, snapshot) = match request.cursor() {
            None => (0, current_high),
            Some(cursor) => {
                if cursor.session_id() != session_id {
                    return Err(EventLogError::CrossSessionCursor {
                        detail: "cursor belongs to another session".into(),
                    });
                }
                match cursor.snapshot_high_watermark() {
                    Some(snapshot) if snapshot > current_high => {
                        return Err(EventLogError::ExpiredCursor {
                            detail: "cursor snapshot is not available".into(),
                        });
                    }
                    // Pinned continuation, then the polling form.
                    Some(snapshot) => (cursor.after_sequence(), snapshot),
                    None => (cursor.after_sequence(), current_high),
                }
            }
        };

        let limit = request.limit().get();
        let mut events = self.events_in(session_id, after, snapshot, limit + 1);
        let has_more = events.len() > limit;
        if has_more {
            events.pop();
        }
        let next_cursor = has_more
            .then(|| {
                let last = events.last().and_then(|event: &Event| event.sequence).unwrap_or(after);
                EventCursor::continuation(session_id, last, snapshot)
            })
            .transpose()?;
        EventPage::new(events, next_cursor, snapshot)
    }
}

#[async_trait]
impl EventLog for MyEventLog {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        if request.is_ephemeral() {
            return Err(EventLogError::InvalidAppend {
                detail: "ephemeral events are sink-only".into(),
            });
        }
        // The log owns identity: assign the event id and the next per-session
        // sequence, persist, and return the finalized canonical envelope.
        let sequence = self.next_sequence(request.session_id);
        let event = request.into_event(EventId::new(), sequence);
        self.persist(&event)?;
        Ok(event)
    }

    fn durability(&self) -> EventDurability {
        EventDurability::CrashDurable
    }
}
```

The contract an implementation must uphold:

- an accepted append owns id and sequence assignment and returns the finalized
  canonical `Event`, visible to the next read of that session;
- durable sequences are unique and strictly increasing per session, and need not
  be contiguous, gaps are expected when a reader projects an append-only
  physical log into a filtered logical event sequence;
- a continuation stays pinned to the first page's high-watermark and cannot
  observe concurrent appends; a poll cursor can;
- cursor/session mismatches and inconsistent positions return the typed
  `EventLogError` variants above rather than panicking;
- the log is append-only. There is no truncate, rewind, or mutation contract,
  and `EventHistory` remains a read-only projection rather than a second
  writable message store.

`tests/fixtures/external-consumer/event-log` in the repository is a complete
out-of-workspace implementation exercised by repository CI.

## Ordering and bounded delivery

A subscriber receives events in channel arrival order and each session has its
own stream. The canonical `sequence` field is a replay position, not a live
delivery counter: durable events carry `Some(sequence)` and live-only ephemeral
events such as streaming deltas carry no sequence. Persisted sequences increase
monotonically per session and may have gaps. Ephemeral events do not consume
replay positions.

The live stream has a bounded buffer and never applies backpressure to the agent
turn. A dropped or slow subscriber cannot stall model or tool execution. If a
subscriber falls behind, `recv()` and `try_recv()` return
`EventStreamError::Lagged { missed }`; loss is never hidden. The next receive can
continue from the oldest retained event, but the renderer must treat its live
projection as incomplete.

Streaming deltas are provisional and sink-only. Completed assistant/tool events
are authoritative, and an output-replacement event means accumulated text for
that message must be discarded. Durable events reach the live sink only after
their log append commits; ephemeral events go directly to the sink and never
enter history. After live lag, a Framework application can rebuild its
persisted transcript with bounded
[`Session::history()` pages](/framework/session-history/). That projection
excludes ephemeral deltas by design. Applications that need raw durable
envelopes rather than derived messages can provide and read an `EventLog`
through the advanced `everruns-host` SPI; neither recovery path relies on the
in-process subscriber.

## Cancellation and failure

Pass a `CancellationToken` through `RunOptions` to stop a turn. Cancellation
produces both a `Turn` with `TurnStopReason::Cancelled` and a correlated
`turn.cancelled` event carrying the same `turn_id`.

Runtime failures remain available through `Session::run()`'s outcome/error
semantics and the event stream. Subscribe before running and continue draining
the stream after the run resolves to retain the terminal failure event and its
full structured payload.

The complete runnable example is
[`canonical_events.rs`](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/canonical_events.rs):

```bash
cargo run -p everruns --example canonical_events
```
