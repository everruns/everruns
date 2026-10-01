---
type: Specification
title: "AG-UI Channel"
description: "AG-UI 1.0 inbound channel: wire types, runtime-event projection, and the 1.0 rules the stream keeps."
tags:
  - everruns
  - integrations
---
# AG-UI Channel

## Status

Implemented on AG-UI 1.0 for the endpoint `POST /v1/e/{endpoint_id}/ag-ui` and
Public Chat (`POST /v1/e/{endpoint_id}/public-chat`), which reuses the same
stream. Interrupts and resume, frontend tools, token usage, subagents and the
outbound client are planned follow-ups of the same upgrade.

## Pieces

- **Wire types**: the published [`everruns-ag-ui`](../../crates/ag-ui) crate.
  The upstream 1.0 JSON Schema and fixture corpus are vendored under
  `crates/ag-ui/spec/1.0` and are the crate's test suite. Hand-written serde
  types beat generated ones because the schema leans on `allOf` plus
  `unevaluatedProperties`, which generators turn into unidiomatic Rust.
- **Projection**: `everruns_ag_ui::projection::Projector` (feature `core`)
  turns canonical runtime events into AG-UI events. It is transport-free so the
  server endpoint, the framework and `serve` share one projection.
- **Server adapter**: [`crates/server/src/api/ag_ui.rs`](../../crates/server/src/api/ag_ui.rs)
  validates input, runs the turn and feeds the session's events to the
  projector.

## Rules the stream keeps

- **Absent means absent.** Nothing emitted carries `null`; input tolerates
  historical `null` and unknown fields, as 1.0 consumers must.
- **Version handshake.** `RUN_STARTED` carries `protocolVersion: "1.0"` only
  when the request declared one, so pre-1.0 clients see the stream they did.
- **Closed before terminal.** Every open text message, reasoning message and
  reasoning span is closed before `RUN_FINISHED` or `RUN_ERROR`, on every path
  including cancellation and failure. 1.0 fails a run that leaves one open.
- **Outcomes.** A completed turn finishes with no outcome (success), a
  cancelled turn with `outcome: { type: "cancelled" }`, a failed turn with
  `RUN_ERROR`. Nothing follows the terminal event.
- **Reasoning.** Model thinking streams as `REASONING_*` (the 1.0 replacement
  for `THINKING_*`). Tool activity, when an endpoint opts into
  `public_tool_activity_text`, is shown as fixed text on the reasoning channel,
  never as assistant text (see [events](../execution/events.md)).
- **Ids.** Client-supplied `threadId`, `runId` and message ids are 1 to 128
  characters of `[A-Za-z0-9-_.]` (TM-TENANT-009). Emitted assistant message ids
  are the runtime message UUID, so a client can correlate them with history.

## Public endpoints

Anonymous endpoints use a public projection policy: errors go through
`PublicError` (see [public endpoints](../execution/public-endpoints.md)), tool
activity uses only the endpoint's configured text. The decided policy for the
1.0 additions: anonymous endpoints may receive `ask_user` interrupts, while
approval interrupts and token usage stay off unless the endpoint enables them.
