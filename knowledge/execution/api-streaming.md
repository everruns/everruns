---
type: Specification
title: "Streaming APIs"
description: "SSE streaming conventions for API endpoints."
tags:
  - everruns
  - execution
---
# Streaming APIs

Cross-cutting conventions for Server-Sent Events (SSE) endpoints in
the Everruns HTTP API. Companion to [`apis.md`](apis.md) and
[`api-conventions.md`](api-conventions.md); the durable contract for
**how SSE responses are described in OpenAPI** so an LLM toolcaller
can subscribe and dispatch by event-type without parsing prose.

## SSE wire format

Every SSE endpoint serves `Content-Type: text/event-stream` and
follows the standard SSE framing
([RFC mdn](https://developer.mozilla.org/en-US/docs/Web/API/Server-sent_events/Using_server-sent_events)):

```
event: <type>
id: <event_id>
retry: <ms>
data: <json>

event: <type>
id: <event_id>
data: <json>

```

* `event:`, the **event type discriminator**. Maps directly to the
  IANA-style dot-notation type strings used throughout Everruns
  (`turn.started`, `tool.completed`, `reason.thinking.delta`, …).
  Lifecycle framing events (`connected`, `disconnecting`) use the
  same field but carry a minimal framing payload rather than an
  `Event`.
* `id:`, event id cursor (`event_{32-hex}` format). Reconnecting
  clients pass it back as the `since_id` query parameter to resume
  without missing events.
* `retry:`, server-suggested reconnect delay in milliseconds; see
  the per-endpoint description for the cycling policy.
* `data:`, the per-event JSON body. The shape depends on the
  `event:` value (see "Event-type catalog" below).

Streams are **automatically cycled** at a per-endpoint interval (5 min
for session events, 1 min for durable workflow snapshots) so proxies
and load balancers don't drop the connection silently. The server
emits a `disconnecting` event before closing; clients should
reconnect immediately using `since_id`.

## Event-type catalog

### `GET /v1/sessions/{session_id}/sse`

The body schema is the **`Event`** ToSchema (see `crates/core/src/events.rs`).
Each `data:` line is a serialized `Event`, whose `type` field matches
the SSE `event:` discriminator and whose `data` field carries the
event-type-specific payload defined by the `EventData` enum.

Closed `event:` vocabulary on this endpoint:

| `event:` value                  | `data:` shape                                                              |
| ------------------------------- | -------------------------------------------------------------------------- |
| `connected`                     | `{"status": "connected"}` (lifecycle; no `Event` wrapper)                  |
| `disconnecting`                 | `{"reason": "connection_cycle", "retry_ms": 100}` (lifecycle)              |
| `input.message`                 | `Event` (`data` = `InputMessageData`)                                      |
| `output.message.started`        | `Event` (`data` = `OutputMessageStartedData`)                              |
| `output.message.delta`          | `Event` (`data` = `OutputMessageDeltaData`)                                |
| `output.message.completed`      | `Event` (`data` = `OutputMessageCompletedData`)                            |
| `output.message.replaced`       | `Event` (`data` = `OutputMessageReplacedData`)                             |
| `turn.started`                  | `Event` (`data` = `TurnStartedData`)                                       |
| `turn.completed`                | `Event` (`data` = `TurnCompletedData`)                                     |
| `turn.failed`                   | `Event` (`data` = `TurnFailedData`)                                        |
| `turn.cancelled`                | `Event` (`data` = `TurnCancelledData`)                                     |
| `reason.started`                | `Event` (`data` = `ReasonStartedData`)                                     |
| `reason.completed`              | `Event` (`data` = `ReasonCompletedData`)                                   |
| `reason.thinking.started`       | `Event` (`data` = `ReasonThinkingStartedData`)                             |
| `reason.thinking.delta`         | `Event` (`data` = `ReasonThinkingDeltaData`)                               |
| `reason.thinking.completed`     | `Event` (`data` = `ReasonThinkingCompletedData`)                           |
| `reason.item`                   | `Event` (`data` = `ReasonItemData`)                                        |
| `act.started`                   | `Event` (`data` = `ActStartedData`)                                        |
| `act.completed`                 | `Event` (`data` = `ActCompletedData`)                                      |
| `tool.started`                  | `Event` (`data` = `ToolStartedData`)                                       |
| `tool.progress`                 | `Event` (`data` = `ToolProgressData`)                                      |
| `tool.completed`                | `Event` (`data` = `ToolCompletedData`)                                     |
| `tool.output.delta`             | `Event` (`data` = `ToolOutputDeltaData`)                                   |
| `tool.call_requested`           | `Event` (`data` = `ToolCallRequestedData`)                                 |
| `transcript.repaired`           | `Event` (`data` = `TranscriptRepairedData`)                                |
| `tool.call_repaired`            | `Event` (`data` = `ToolCallRepairedData`)                                  |
| `llm.generation`                | `Event` (`data` = `LlmGenerationData`)                                     |
| `capability.usage`              | `Event` (`data` = `CapabilityUsageData`)                                   |
| `session.started`               | `Event` (`data` = `SessionStartedData`)                                    |
| `session.activated`             | `Event` (`data` = `SessionActivatedData`)                                  |
| `session.idled`                 | `Event` (`data` = `SessionIdledData`)                                      |
| `context.compacting`            | `Event` (`data` = `ContextCompactingData`)                                 |
| `context.compacted`             | `Event` (`data` = `ContextCompactedData`)                                  |
| `file.written`                  | `Event` (`data` = `FileWrittenData`)                                       |
| `budget.warning`                | `Event` (`data` = `BudgetEventData`)                                       |
| `budget.paused`                 | `Event` (`data` = `BudgetEventData`)                                       |
| `budget.exhausted`              | `Event` (`data` = `BudgetEventData`)                                       |
| `budget.resumed`                | `Event` (`data` = `BudgetEventData`)                                       |
| `voice.session.started`         | `Event` (`data` = `VoiceSessionStartedData`)                               |
| `voice.input_transcript.delta`  | `Event` (`data` = `VoiceTranscriptData`)                                   |
| `voice.input_transcript.completed`  | `Event` (`data` = `VoiceTranscriptData`)                               |
| `voice.output_transcript.delta` | `Event` (`data` = `VoiceTranscriptData`)                                   |
| `voice.output_transcript.completed` | `Event` (`data` = `VoiceTranscriptData`)                               |
| `voice.session.ended`           | `Event` (`data` = `VoiceSessionEndedData`)                                 |
| `voice.session.failed`          | `Event` (`data` = `VoiceSessionFailedData`)                                |

The authoritative event-type → payload mapping is the `EventData`
enum in [`crates/core/src/events.rs`](../../crates/core/src/events.rs)
(`pub enum EventData` near line 2190). The generated OpenAPI spec
also surfaces this catalog per-event via the SSE schema components,
so LLM toolcallers can dispatch from machine-readable form rather
than this prose.

The set is exhaustive: events whose type is not in `EventData` are
filtered out before transmission. Clients must treat any unknown
`event:` value as informational only.

### `GET /v1/durable/sse` and `GET /v1/durable/workflows/{id}/sse`

These streams emit **snapshots**, not `Event`s. The closed `event:`
vocabulary is `{connected, snapshot, disconnecting}`:

| `event:` value  | `data:` shape                                                  |
| --------------- | -------------------------------------------------------------- |
| `connected`     | `{"status": "connected"}`                                      |
| `snapshot`      | Workflow/queue state snapshot, shape varies; treat as opaque  |
|                 | JSON until a typed snapshot schema lands.                      |
| `disconnecting` | `{"reason": "connection_cycle", "retry_ms": …}`                |

The snapshot endpoints poll the durable store at a fixed interval
and re-emit whenever the snapshot hash changes. They don't fan out
domain `Event`s, for the per-workflow domain event stream, use the
JSON polling endpoint `GET /v1/durable/workflows/{id}/events`.

## Adding new SSE endpoints

When wiring up a new SSE handler:

1. Decide whether the stream carries `Event`s (use the per-session
   pattern above) or a snapshot type (use the durable pattern).
2. Declare the response in `#[utoipa::path]` as
   `(status = 200, body = <Type>, content_type = "text/event-stream", …)`.
3. Document the closed `event:` vocabulary in this file under a new
   subsection.
4. The on-the-wire framing (`event:`/`id:`/`retry:`/`data:`) is
   the same for every endpoint and is documented once here, don't
   re-explain it per endpoint.
