---
title: Events
description: "SSE streaming of session events: categories, structure, and the wire protocol."
appliesTo: [platform, cloud]
---

Every action during a session, user input, LLM responses, tool calls, lifecycle transitions, emits an **event**. The event log is the source of truth for session state; the SSE stream is a live tail of that log.

For *why* the platform is shaped this way, see [Events as the primary store](/explanation/events/). For the full catalog of event types and payloads, see the [Event Reference](/event-reference/).

## Event categories

| Category | Examples | Description |
|---|---|---|
| **Input** | `input.message` | User messages submitted to the session |
| **Output** | `output.message.started`, `output.message.delta`, `output.message.completed` | Agent response lifecycle |
| **Turn** | `turn.started`, `turn.completed`, `turn.failed`, `turn.cancelled` | Turn lifecycle |
| **Thinking** | `reason.thinking.*` | Extended thinking content (Claude, GPT-5.x, o-series) |
| **Atom** | `reason.*`, `act.*`, `tool.*` | Internal execution phases |
| **LLM** | `llm.generation` | Full LLM API call details |
| **Session** | `session.started`, `session.activated`, `session.idled`, `session.model.changed` | Session state changes |
| **Sandbox** | `sandbox.instance_lost`, `sandbox.recovered` | Managed compute replacement and durable-workspace recovery |
| **Subagent** | `subagent.*` | Subagent lifecycle |

## Event structure

```json
{
  "id": "event_01933b5a00007000800000000000001",
  "type": "turn.completed",
  "ts": "2024-01-15T10:30:00.000Z",
  "session_id": "session_01933b5a00007000800000000000002",
  "sequence": 42,
  "context": {
    "turn_id": "turn_...",
    "input_message_id": "message_...",
    "trace_id": "turn_...",
    "span_id": "abc123",
    "parent_span_id": "def456"
  },
  "data": { /* type-specific payload */ }
}
```

| Field | Type | Description |
|---|---|---|
| `id` | string | Unique event ID (UUIDv7) |
| `type` | string | Event type in dot notation |
| `ts` | string | ISO 8601 with millisecond precision |
| `session_id` | string | Session this event belongs to |
| `sequence` | integer | Monotonic per-session sequence (ordering source of truth) |
| `context` | object | Correlation IDs for tracing |
| `data` | object | Event-specific payload |

Ordering is by `sequence`, not `ts`. Two events with the same wall-clock time still have a strict order.

## Common patterns

### Started → completed → failed

Long-running operations follow a lifecycle:

```
turn.started → turn.completed
            ↘ turn.failed
            ↘ turn.cancelled
```

These boundaries are what your UI uses to manage state and surface errors.

### Issues versus errors

A failed tool call is not a failed turn. The model gets the failure back as a result and carries on, so `tool.completed` marks it `"severity": "issue"` and `turn.completed` counts it in `issue_count`. Only `turn.failed` (or a failed task) means the work stopped. Render issues as warnings and keep error styling for those. See [Issues and errors](/event-reference/#issues-and-errors).

### Delta streaming

Streaming content uses delta events with accumulated state:

```json
{
  "type": "output.message.delta",
  "data": {
    "turn_id": "turn_...",
    "delta": "Hello",
    "accumulated": "Hello"
  }
}
```

Deltas are batched at ~100ms to reduce volume. When a provider stream breaks after some text, the runtime retries it under the same `message_id`: it emits `output.message.started` for that message again, and `accumulated` starts over. Render `accumulated`, or clear the text you added up on a repeated `output.message.started`.

## Inspecting raw events in the UI

The session's **Events** tab shows bounded raw JSON previews. Long strings,
deeply nested values and large collections are shortened so screenshots and
large tool results do not force the browser to lay out entire payloads in each
table row. A notice identifies shortened previews. **Copy** still includes the
complete original event; stored events and API responses are unchanged.

## Forward-compatibility

Events follow a defined contract. Consumers must tolerate evolution:

| Change | Allowed |
|---|---|
| New event types | yes |
| New optional fields | yes |
| New enum values | yes |
| Removing or retyping fields | no |

Your deserializer should ignore unknown fields, ignore unknown event types, and treat optional fields as optional.

## Consume the stream

- [Stream events with the SDK](/how-to/stream-events/), the convenient path (Python, Rust, TypeScript).
- [Consume events via raw SSE](/how-to/stream-events/#with-raw-sse), protocol-level, when the SDK isn't available.

## Optional idle following in the React UI client

The UI client follows session events continuously by default. Keep this behavior
for schedules, channels, shared sessions, and applications that need immediate
updates from other clients.

An embedding that drives its own session can opt in to idle suspension:

```tsx
const { data: events, isSuspended, resume } = useEvents(sessionId, {
  followIdle: false,
});
```

The hook closes SSE only after the session is settled, all background tasks are
terminal, and its durable cursor matches the server tail. Active turns, parked
tools, pending approvals, sign-ins, and tasks awaiting input keep following.
An unavailable state check keeps the connection open.

The UI client's message, tool-result, question-answer, approval, and consent APIs
reopen the stream before their request and keep it live until the request ends.
Custom resume or write operations should use `withSessionActivity(sessionId,
async () => { /* perform the operation */ })` from
`everruns-ui/lib/session-activity`. The hook also exposes `resume()` to catch up
explicitly and catches up on browser focus or network recovery. Replay uses the
last durable cursor and deduplicates already displayed events.

While suspended, events from other clients wait until the next wake-up. Other
subscriptions, including the task list's independent stream, are unaffected.
This option makes no measured resource or cost savings claim.

## See also

- [Event Reference](/event-reference/), complete event type catalog with payloads.
- [Events as the primary store](/explanation/events/), design rationale.


## Public metadata and replay state

Message APIs, event lists and SSE, trace previews, and JSONL message exports return
reviewed public annotations rather than the runtime's metadata map. Client message
IDs, origin/source, locale, model and reasoning effort, provider trace identifiers,
user-facing error fields, initiator identity, and page context/actions remain
available. Page draft descriptors retain the browser binding and revision; they
do not expose arbitrary form snapshots. Unknown keys, script execution inputs,
and recorded tool-change replay state are withheld, including on historical
messages. External actor identity and usage fields retain their existing contracts.
Event-envelope metadata contains public initiator identity and participant/operation
identifiers, without execution authority or arbitrary source envelopes.

Applications that need arbitrary annotations returned with a message should put
them in `metadata.custom`, for example `{"custom":{"ticket":"SUP-123"}}`. This
namespace is caller-owned JSON: do not put secrets there. Existing unknown
metadata keys remain in storage but are no longer returned. Move those annotations
to `custom` on new writes; existing records are not automatically reclassified as
caller-owned because their provenance cannot be established safely.

Full generation diagnostics still intentionally expose the prompt, declared tools,
and provider request (with provider replay artifacts removed). Agent API visibility
settings continue to restrict which diagnostics an execution caller sees. A `full`
visibility setting does not restore hidden message metadata.

Filtering happens when reading. Stored events, internal worker traffic, and trusted
Framework `canonical_json()` / local journal backups retain the original replay
state. Treat those trusted records as sensitive; do not forward them as public API
responses. Deploy updated API/Serve/Workers binaries to cover historical and new
reads. Previously downloaded exports, client caches and external telemetry are not
rewritten by an upgrade.
