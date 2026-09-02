---
type: Specification
title: "Events"
description: "Event types, SSE streaming, contract and compatibility guarantees."
tags:
  - everruns
  - execution
---
# Events

## Purpose

Events are Everruns' durable session protocol. They are the source of truth for
conversation reconstruction, expose execution progress to clients, and feed
observability and reporting projections.

This document owns the event protocol's intent, compatibility rules, and
cross-event semantics. It deliberately does not repeat the event envelope,
payload structs, type registry, HTTP query fields, or SQL schema.

## Sources of truth

- [`crates/core/src/events.rs`](../../crates/core/src/events.rs) owns the serialized
  event envelope, event type constants, payload structs, the type-to-payload
  mapping, and the valid filter registry.
- [`docs/api/openapi.json`](../../docs/api/openapi.json) is the generated
  consumer-facing schema. It is the right source for SDK model generation and
  exact wire fields.
- [`crates/server/src/api/events.rs`](../../crates/server/src/api/events.rs) owns
  the JSON and SSE endpoints, query validation, filtering, pagination, and
  summary response.
- [`crates/server/src/services/event.rs`](../../crates/server/src/services/event.rs)
  owns emission and listener notification.
- [`crates/server/migrations/001_base_schema.sql`](../../crates/server/migrations/001_base_schema.sql)
  and later migrations own storage columns, constraints, indexes, sequence
  allocation, and immutability triggers.
- The tests in [`crates/core/src/events.rs`](../../crates/core/src/events.rs) cover
  serialization, round trips, forward compatibility, and the type mapping.
  [`crates/server/tests/workflow_test.rs`](../../crates/server/tests/workflow_test.rs)
  covers API filtering of unsupported events, while
  [`crates/server/tests/client_side_tools_test.rs`](../../crates/server/tests/client_side_tools_test.rs)
  covers the client-tool event contract.

Wire examples do not live here. Exact examples belong in the generated OpenAPI
document and contract tests, where a source change makes drift visible. Product
or SDK guides may show task-oriented excerpts generated from that contract, but
must not become a second schema registry.

## Public contract and compatibility

The event protocol is a public API contract.

The top-level envelope is stable. Its exact serialized fields and types are
defined by `Event` in `crates/core/src/events.rs`. Payload shape is selected by
the event's `type` through the single type-to-payload mapping in the same file.

The following changes are backward-compatible:

- adding an event type;
- adding an optional payload or context field;
- adding a string-enum value;
- relaxing a required field to optional.

The following require a major compatibility change:

- removing or renaming a supported event type;
- removing or renaming a field;
- changing a field's serialized type or meaning;
- making an optional field required.

Consumers must ignore unknown fields, tolerate new event types, and use
`sequence` rather than an identifier or timestamp for ordering within a
session. The server filters payloads it cannot decode into a supported typed
event before returning API or SSE results. Historical unsupported rows may
remain in storage for audit and aggregate queries.

Adding a constant is not enough to add an event. The payload type, mapping,
filter registry, OpenAPI export, emission path, and round-trip tests must stay
coherent. Tests in `crates/core/src/events.rs` are the executable contract for
that coherence.

## Embedding facade surfaces

`everruns::Session::events()` exposes a *reviewed* surface and a *canonical*
surface, and the split is deliberate. See
[`crates/everruns/src/events.rs`](../../crates/everruns/src/events.rs).

| Surface | Accessors | Contract |
|---|---|---|
| Reviewed | `SessionEventKind`, `SessionEvent::data()`, `as_json()` | Only fields someone promoted. Stable, safe to log or forward. |
| Canonical | `SessionEvent::canonical_json()` | The complete envelope, nothing withheld. Runtime-internal shape, may change in a patch release. |

Two rules govern it:

1. **Reviewed is opt-in, not opt-out.** A field reaches the reviewed surface
   only by being promoted onto a `SessionEventKind` variant. This is the
   embedding-facade half of the rule already stated under *Security and privacy*
   — unknown event data is not passed through as an untyped public payload.
   Passing internal payloads through by default makes every field added to the
   runtime a public API addition nobody reviewed, and ships it wherever the
   application forwards the envelopes.
2. **Canonical loses nothing.** Sanitizing in place would destroy data an
   embedder legitimately needs — auditing, replay, cost accounting — with no way
   to recover it, because the facade exposes no other event path. The default is
   narrow; the complete record stays one explicit call away.

The reviewed payload must be a **subset** of the canonical one: it may drop
fields, never invent them. A synthesised field is a value no consumer can
correlate with the durable record. `reviewed_data_never_exceeds_the_canonical_payload`
is the executable form of that invariant.

Promoting a field is the intended way to answer "the reviewed surface lacks
what I need". Judge it on whether it carries conversation content: identity,
outcome, token usage, timing, and operator-authored display text are promoted;
prompts, tool arguments, tool results, and unrecognized event payloads are not.
Model-generation accounting is promoted precisely because tracking spend must
not require reaching for the unstable surface.

## Correlation and tracing

Turn-scoped events carry correlation context. A turn is the trace root;
reasoning, acting, LLM, and tool activity form child spans. Started/completed
pairs for one operation share a span identity so tracing backends can merge
them.

Correlation identifiers use Everruns' public prefixed identifier format. The
exact optional context fields live on `EventContext`; emitters should populate
the strongest context they possess without fabricating missing ancestry.
Session-level events may have empty turn context.

## Lifecycle conventions

Long-running operations use paired lifecycle events where useful:

- `started` establishes identity and lets clients show provisional state;
- zero or more `delta` or `progress` events provide ephemeral updates;
- `completed`, `failed`, `cancelled`, or `sealed` establishes the durable
  outcome.

Not every operation needs a separate failure event. A completed payload may
carry a failure status when that is the established contract. The source
registry is authoritative for the current type set.

Delta and progress events are informational. Consumers accumulate them only for
live rendering; the durable completion event is authoritative. The
`is_ephemeral_event_type` function is the single source of truth for which
events may be omitted from durable storage.

### Assistant messages

One assistant message is identified by its message ID across started, delta,
replacement, and completed events. A turn may contain several assistant
messages, so consumers must never group streamed text by turn alone.

Completion is authoritative for message content and phase. A guardrail
replacement discards only the accumulated text for the affected message ID;
the subsequent completed event persists the replacement as the canonical
assistant message. Suppressed model text must not be persisted or replayed.

Streamed phase is a best-effort hint. It may refine once from unknown to
commentary or final answer, but must not flip between phases or regress to
unknown. Missing or unknown phase always falls back to ordinary assistant text,
never to reasoning.

Phase carries a source. Providers with native phase support report it; for every
other provider the runtime infers it from tool-call presence, where "commentary"
means nothing more than "this message called tools". Those are different claims
and consumers must be able to tell them apart, so the completed message publishes
`phase_source` alongside `phase`. A derived phase is a weak signal: a text-only
preamble with no tool calls is indistinguishable from a final answer, and a
consumer that needs certainty must treat `derived` accordingly rather than trust
the label.

### Turn terminal states

A completed turn succeeded. A failed turn ended because of an error. A
cancelled turn stopped at user request. A sealed turn was deliberately stopped
to prevent waste, such as repeated recovery without progress or exhausted work
budget.

Sealing is terminal and non-retryable, but it does not create a separate
session status. A user-visible assistant completion and an idle session
transition accompany the sealed event. See
[`durable-execution-engine.md`](../operations/durable-execution-engine.md) for forward-progress
and dead-letter behavior.

User-facing failure events carry stable error classification and interpolation
fields when available. Clients localize from those fields rather than matching
English fallback text. Disclosure policy is defined in
[`error-disclosure.md`](error-disclosure.md).

### Model changes

A session's answering model can change between turns, which silently changes
every answer that follows. `session.model.changed` records that switch as part
of the conversation. It is emitted where the runtime resolves a turn's model —
the host's reason path, at the turn's first iteration — so every host records
it: the durable worker behind the API and the in-process framework runtime
alike. An emitter bound to one API's message-create path would leave embedded
sessions unmarked.

Only an explicit override replacing a different explicit override is reported.
A turn without an override runs on an inherited default, and the history the
host sees is capability-filtered: treating a missing override as "the default"
would report a switch whenever an older message was filtered out. A requested
model that does not survive resolution is not reported either, because the turn
then runs on a fallback the marker would misname.

The payload carries the provider's own model identifiers, captured at emission
time, so a transcript stays readable after a model is renamed or removed.
Consumers that still hold the model may prefer its display name.

The marker is turn-scoped and best effort. A retried reason activity can emit
it twice for one turn; consumers that render it collapse a repeat of the same
switch rather than treating the second as new history.

`llm.generation` remains the per-call record of which model actually ran. It is
diagnostic, not conversational, and does not replace this marker.

### Session tasks

Session-task events carry full task snapshots for reconciliation. Message
events carry the task identifier and stored task message. The task contract
lives in [`session-tasks.md`](../runtime-resources/session-tasks.md).

Legacy `subagent.*` events are retired. They are no longer emitted or parsed as
typed events; historical rows follow the unsupported-event behavior. New
integrations consume `task.*`.

### Context compaction

Compaction emits a start event and emits a completion event only after a
material semantic reduction. Provider-opaque content, encrypted continuation
state, and checkpoint ciphertext must never appear in compaction event data.
Observation-only masking is not semantic compaction and does not emit a
completion event. See [`compaction.md`](../runtime-resources/compaction.md).

### Voice

Voice transcript events provide provisional and observational streaming around
the canonical message protocol. Final user speech also becomes an input
message; the durable agent turn's completed assistant message remains
canonical. Provider secrets, raw session descriptions, and unsanitized provider
payloads must not enter failure events. See [`voice.md`](../operations/voice.md).

## Projection channels

Downstream transports project the runtime stream onto three non-overlapping
channels:

| Runtime concept | Projection |
|---|---|
| Commentary and final answers | Assistant-text channel |
| Provider-exposed thinking and safe reasoning summaries | Reasoning channel |
| Tool narration, progress, and output | Tool channel |

Commentary is deliberate user-visible assistant text produced before or between
tool calls. A final answer is turn-ending assistant text. Thinking is only
reasoning explicitly exposed by the provider. A reasoning summary is a safe
provider-curated artifact; opaque or encrypted reasoning is never surfaced.

These three words are not interchangeable, and the `reason.` event prefix spans
two unrelated meanings that must not be conflated: `reason.started` and
`reason.completed` mark an LLM inference step in the reason/act loop, while
`reason.thinking.*` and `reason.item` carry model reasoning. An inference step
occurring says nothing about whether the model reasoned.

A reasoning summary is reasoning, not commentary. Routing one onto the
assistant-text channel does not merely mislabel it: the text is then persisted
as the model's answer and replayed to the provider as the model's own prior
output.

Opaque replay state is stored but never published. A reasoning artifact's
provider `signature` and `encrypted` payload must survive in the event log,
because replaying a turn rebuilds the message from it — and must be stripped
from every read. Both the message read path and the events read path (list and
SSE) apply that projection, via `Message::into_public` and
`EventData::into_public` respectively; publishing on one and not the other is
the bug EVE-933 records. A read boundary that carries a `Message` and skips the
projection is a leak.

A projection must not move content across channels to imitate a phase. In
particular, absent phase never makes assistant text into thinking, and tool
activity never becomes assistant text. AG-UI projection is implemented in
[`crates/server/src/api/ag_ui.rs`](../../crates/server/src/api/ag_ui.rs).

## Storage guarantees

Persisted events are append-only and immutable. Each session has an atomically
allocated, monotonically increasing sequence. Concurrent writers must not
derive a sequence with `MAX(sequence) + 1`.

The declared event type must agree with the typed payload before persistence.
Raw historical data may use the explicit unsupported path, but ordinary
emitters must not bypass type consistency.

Messages are reconstructed from canonical input, assistant completion, and
tool completion events. Streaming deltas are not required for replay. Tool
calls remain part of assistant content; tool results come from tool completion
events.

Reporting projections are asynchronous and backend-neutral. They must not
change the semantic event record. See [`reporting.md`](../evaluation/reporting.md).

## Streaming contract

The SSE event name matches the payload's event type. A connection begins with a
connection lifecycle signal and may end with a graceful cycling signal so the
client can reconnect without consuming its retry budget.

Heartbeat comments keep idle connections observable but are not events and do
not affect schema or replay. Connection cycling is jittered to avoid synchronized
reconnect storms. Exact intervals, retry hints, endpoint parameters, and
environment controls are implementation configuration owned by the streaming
handler and exported API documentation.

SDKs and clients must:

1. retain the last accepted event identity;
2. reconnect from that identity after a disconnect or stale read;
3. preserve event order by session sequence;
4. ignore SSE comments and unknown JSON fields;
5. treat duplicate delivery after reconnect as harmless.

Positive event filters narrow the stream; exclusion filters remove from the
narrowed set. Unknown filter types and unbounded filter arrays are rejected.
Exact query names and limits belong to `crates/server/src/api/events.rs` and the
OpenAPI export.

## Listeners

Listeners observe persisted events without becoming part of business logic.
They run in registration order, and one listener's failure or panic must not
prevent persistence or later listeners from running. Heavy listener work should
move off the emission path while preserving any ordering it requires.

Built-in and future listeners may project traces, metrics, analytics, or audit
records. Their output is derivative; it cannot mutate the source event.

## Security and privacy

- Event payloads are an external disclosure boundary. Do not include secrets,
  credentials, raw provider authentication material, hidden chain of thought,
  or unsanitized internal errors.
- Session ownership is checked before listing, streaming, filtering, or
  summarizing events.
- Unknown event data is retained only where storage/audit needs it and is not
  passed through as an untyped public payload.
- Full-text and debug filters preserve the same authorization and supported-type
  checks as ordinary event listing.
