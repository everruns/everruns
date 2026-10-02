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
stream, including interrupts and resume, frontend tools and token usage.
Subagents and the outbound client are planned follow-ups of the same upgrade.

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
  cancelled turn with `outcome: { type: "cancelled" }`, a turn parked on a
  question or an approval with the interrupt outcome, a turn parked only on
  frontend tool calls with success and `pendingToolCallIds`, a failed turn with
  `RUN_ERROR`. Nothing follows the terminal event.
- **Usage.** When the endpoint sets `usage_visible`, `RUN_FINISHED` and
  `RUN_ERROR` carry the run's token usage summed per provider and model from
  `llm.generation`, translated to AG-UI's accounting: `inputTokens` includes
  cache reads and writes, which Everruns counts apart.
- **Reasoning.** Model thinking streams as `REASONING_*` (the 1.0 replacement
  for `THINKING_*`). Tool activity, when an endpoint opts into
  `public_tool_activity_text`, is shown as fixed text on the reasoning channel,
  never as assistant text (see [events](../execution/events.md)).
- **Ids.** Client-supplied `threadId`, `runId` and message ids are 1 to 128
  characters of `[A-Za-z0-9-_.]` (TM-TENANT-009). Emitted assistant message ids
  are the runtime message UUID, so a client can correlate them with history.

## Interrupts and resume

A parked turn becomes interrupts, built in
[`ag_ui_interrupts.rs`](../../crates/server/src/api/ag_ui_interrupts.rs):

| Parked on | `reason` | Answer (`ResumeEntry.payload`) |
|---|---|---|
| `ask_user` | `everruns.ask_user` | the question-answers request body; `responseSchema` is the same schema A2A advertises |
| `ask_user` with a `secret` question | `everruns.secret_required` | none: only abandoning is accepted |
| tool approval, endpoint opted in | `tool_approval` | `{ "decision": "allow" \| "allow_always" \| "reject" \| "reject_always" }` |
| tool approval, default | `everruns.operator_approval` | none: an operator decides; abandoning rejects |

The interrupt id is the parked call's id. A resuming run sends no new user
message; its entries go through the same resolvers as the session API, and
the run streams the resumed turn. Producer rules from the 1.0 pattern: an
entry naming no open interrupt is ignored and logged, an interrupt with no
entry is never treated as abandoned (the run interrupts again and resolves
nothing), and an entry that cannot be applied is refused before the stream
opens. Abandoning a question declines it; abandoning an approval rejects the
call. Frontend tool calls do not interrupt; see below.

## Frontend tools

`RunAgentInput.tools` become the session's client-side tools, the same kind
the session API declares, so the turn parks on a call to one
([`ag_ui_frontend_tools.rs`](../../crates/server/src/api/ag_ui_frontend_tools.rs)).
The consumer sends its tools on every run and the session takes the latest
set. A parked call to one of the run's frontend tools streams as
`TOOL_CALL_START`/`ARGS`/`END` under the assistant message that made it, with
its real name and arguments whatever the endpoint's tool visibility, because
the consumer runs it. With no interrupt beside it the run finishes in success
with `pendingToolCallIds`. The next run's trailing `tool` messages are the
results: they land on the waiting-turn resolution the tool-results endpoint
uses, and the run streams the resumed turn. Results for calls that are not
parked are ignored and logged; while a parked frontend call has no result,
nothing is recorded and the run reports the calls again. No other `tool`
message is used or seeded into history (TM-LLM-020). Definitions are bounded
and the `mcp_` prefix is refused (TM-DOS-044, TM-CLIENT-004).

A batch that parks on both a frontend call and a question or approval ends in
the interrupt with the calls streamed beside it. A run carrying resume entries
resolves only those, and a frontend call left unanswered is closed as missing
when the turn resumes. Answering both in one run is a known gap.

## Public endpoints

Anonymous endpoints use a public projection policy: errors go through
`PublicError` (see [public endpoints](../execution/public-endpoints.md)), tool
activity uses only the endpoint's configured text. The decided policy for the
1.0 additions: anonymous endpoints may receive `ask_user` interrupts, while
approval interrupts (`tool_approval_interrupts`) and token usage
(`usage_visible`) stay off unless the endpoint enables them. Public Chat keeps
both off. See TM-TENANT-016 and TM-TOOL-052.
