---
type: Specification
title: "AG-UI Channel"
description: "AG-UI 1.0 channel: wire types, runtime-event projection, the consumer pipeline, and the 1.0 rules each side keeps."
tags:
  - everruns
  - integrations
---
# AG-UI Channel

## Status

Implemented on AG-UI 1.0 for the endpoint `POST /v1/e/{endpoint_id}/ag-ui` and
Public Chat (`POST /v1/e/{endpoint_id}/public-chat`), which reuses the same
stream, including interrupts and resume, frontend tools, token usage,
subagents, run metadata, and a capabilities declaration at
`GET /v1/e/{endpoint_id}/ag-ui/capabilities`. The framework serves the same
protocol from any session behind the facade's `ag-ui` feature (see
[Framework](#framework)), and `serve` mounts it at `POST /v1/e/{agent}/ag-ui`
behind its own `ag-ui` feature (see [serve](#serve)), with the runnable
`examples/serve/ag-ui` driven by `@ag-ui/client`. The crate also has the
consumer half (below), which outbound delegation builds on.

## Pieces

- **Wire types**: the published [`everruns-ag-ui`](../../crates/ag-ui) crate.
  The upstream 1.0 JSON Schema and fixture corpus are vendored under
  `crates/ag-ui/spec/1.0` and are the crate's test suite. Hand-written serde
  types beat generated ones because the schema leans on `allOf` plus
  `unevaluatedProperties`, which generators turn into unidiomatic Rust.
- **Projection**: `everruns_ag_ui::projection::Projector` (feature `core`)
  turns canonical runtime events into AG-UI events. It is transport-free so the
  server endpoint, the framework and `serve` share one projection.
- **Consumer**: `everruns_ag_ui::consumer` decodes a producer's events,
  enforces the 1.0 consumer rules and assembles a `RunResult`;
  `ResumeBuilder` answers interrupts. The HTTP/SSE `client` (feature `client`)
  feeds it. See [Consumer rules](#consumer-rules).
- **Server adapter**: [`crates/server/src/api/ag_ui.rs`](../../crates/server/src/api/ag_ui.rs)
  validates input, runs the turn and feeds the session's events to the
  projector.
- **Framework**: `Session::ag_ui` in
  [`crates/everruns/src/ag_ui.rs`](../../crates/everruns/src/ag_ui.rs), one run
  per request over a session, with interrupts read through `InterruptSource`.
- **serve channel**: [`crates/serve/src/ag_ui.rs`](../../crates/serve/src/ag_ui.rs)
  maps each thread to a session and implements `InterruptSource` over serve's
  pending approvals and questions.

## Framework

The `everruns` facade's opt-in `ag-ui` feature adds `Session::ag_ui` and
`ag_ui_with` ([`crates/everruns/src/ag_ui.rs`](../../crates/everruns/src/ag_ui.rs)):
one AG-UI run per request over the session's live event stream and the shared
`Projector`, with the trusted policy by default (reasoning, usage and runtime
errors visible), because the developer owns both ends.

- **Input.** The session owns the conversation, so a run sends only the last
  user message. Earlier messages, `state`, `context`, `forwardedProps` and
  frontend tools are not read; the host maps `threadId` to a session.
- **Interrupts without a durable park.** In-process `ask_user` and approvals
  block the turn on a responder instead of parking it, so the facade ships
  `InterruptGate`, a responder for both that parks each request in memory
  under (session, call id), the same shape as `serve`'s gate. A park ends the
  run with the interrupt outcome; the resuming run's entries complete the
  waiting responder and stream the rest of the same turn. The producer rules
  match the server's: validate every entry before applying any, re-interrupt
  on a missing entry or a new message, ignore unknown ids. Approval
  interrupts are always client-answerable here (`tool_approval`); there is no
  operator. A process exit cancels a parked turn.
- **No HTTP server dependency.** The facade adds only `everruns-ag-ui`; an
  axum handler is a few lines over the returned stream, kept as the
  `ag_ui_axum` example and the public `framework/ag-ui` page rather than a
  helper.

- **Host interrupt sources.** `InterruptSource` is the seam between a run
  and whatever parks requests: it lists a session's open interrupts, names
  sessions as requests park, and applies resume entries under the same
  producer rules. `InterruptGate` is the built-in source; a host that already
  parks requests for an API of its own implements the trait instead, so an
  AG-UI client and that API see the same request. A run exposes the message
  it sent (`AgUiStream::sent`), so a host that tracks turns (cancel, status)
  follows the turn the run started.

## serve

`serve`'s `ag-ui` feature serves `POST /v1/e/{agent}/ag-ui` for every
top-level agent, listed in the manifest's routes and the agent card's `ag_ui`
map. The path has the server's channel shape so a CopilotKit front end moves
between serve and Everruns by base URL and id alone; the id is the agent name
because every top-level agent is an endpoint and none is declared separately.

- **Built in, not a `Channel`.** That trait answers a webhook and delivers
  later; AG-UI streams in the response.
- **Threads.** One session per (agent, `threadId`), kept in the thread map
  channels use under channel key `ag-ui:{agent}`, so a thread survives a
  restart.
- **Interrupts.** serve's own parked approvals and questions, through
  `InterruptSource`: an interrupt can be answered by a `resume` entry, by
  `/question-answers` or by `/approvals/{tool_call_id}`, and the reverse.
  They live in memory, like every serve park.
- **Policy.** Trusted: reasoning, usage and runtime errors visible, because
  the developer owns both ends. A public deployment puts serve behind its own
  auth, as with every serve route.
- **Errors.** A body that is not a `RunAgentInput`, an empty `threadId`, or
  input the run cannot use is a `400` problem; an unknown agent or a subagent
  is `404`. SSE framing matches the server: unnamed `data:` events and a
  `keepalive` comment every 15 seconds.

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

## Consumer rules

When Everruns is the AG-UI client, the producer is untrusted, so the consumer
holds it to the rules rather than repairing its stream:

- **Processing model.** An unknown event type is dropped and an unknown union
  member in an optional or list slot (outcome, content part, snapshot message,
  JSON Patch op) is stripped, each with a warning; a known field with a
  malformed value fails the stream.
- **Sequencing.** The stream opens with `RUN_STARTED` (or `RUN_ERROR`); after a
  terminal event only a new run may start; continuations need an open opener;
  a run may not finish with a message, tool call, reasoning span, step or
  subagent open; a continuation's `subagentRunId` must agree with its opener.
  The `*_CHUNK` shorthand is expanded into triads first. A body that ends
  inside a run is an error.
- **Resume coverage.** A resume answers every open interrupt exactly once
  (resolved or cancelled) and nothing else; `ResumeBuilder` refuses anything
  less before it is sent.
- **Bounded input.** The SSE reader caps one event's size (4 MiB by default).

The rules are ported from the reference TypeScript client and tested against
upstream's client conformance corpus, vendored under
`crates/ag-ui/spec/1.0/conformance` (`tests/conformance.rs`): every stream the
corpus accepts is accepted and every one it rejects is rejected for the same
reason. Its warning, reducer and request assertions describe the TypeScript
client's own state handling and are not checked.

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

## Subagents

With `subagents_visible` (default off) a run projects its subagent tasks
(`task.*` of kind `subagent`, what `spawn_agent` creates) as `SUBAGENT_*`, with
`subagentRunId` set to the task id and `name` to its display name. The parent
stream sees what crosses the task boundary, not the child transcript (see
[session tasks](../runtime-resources/session-tasks.md)), so attribution
covers that: text the child posts to its parent and its final summary become
assistant text messages carrying `subagentRunId`, structured progress becomes
an `everruns.subagent` activity. Child tool calls stay in the child session.
A child that fails or is cancelled reports `SUBAGENT_ERROR`, its error passed
through the run's error policy, so public endpoints sanitize it. The task spec
(instructions, schemas) is never projected.

A run never finishes with an invocation open. A foreground child ends inside
its parent's turn; a background child that is still running when the run
ends gets an `everruns.subagent` activity saying so, then every open segment
closes with `SUBAGENT_FINISHED` and the `suspended` outcome, which 1.0 defines
as terminal for the stream and not for the subagent. Success or error would
claim an outcome nobody has seen. Its later completion belongs to whatever
turn wakes the parent, not to this run.

## Run metadata

`RUN_STARTED`, `RUN_FINISHED` and `RUN_ERROR` carry `metadata.everruns`
(the `ag-ui` key is the protocol's): `turnId` once the turn is known, `model`
(the last model called) only when `usage_visible` is set, since usage already
names it, and `sessionId` only for an identified caller on the AG-UI endpoint
itself, never an anonymous or Public Chat visitor.

## Capabilities

`GET /v1/e/{endpoint_id}/ag-ui/capabilities` returns a 1.0 `AgentCapabilities`
behind the same auth, gates and rate limit as a run
([`ag_ui_capabilities.rs`](../../crates/server/src/api/ag_ui_capabilities.rs)).
It is derived from the endpoint config only: identity (the endpoint name and
description, `type: everruns`), streaming, client-provided tools, interrupts,
and the opt-ins (reasoning, approvals, subagents, and usage under
`custom.everruns`). The agent's own tools are not listed, and `multiAgent` is
undeclared unless subagents are visible. The stream stays authoritative.

## Public endpoints

Anonymous endpoints use a public projection policy: errors go through
`PublicError` (see [public endpoints](../execution/public-endpoints.md)), tool
activity uses only the endpoint's configured text. The decided policy for the
1.0 additions: anonymous endpoints may receive `ask_user` interrupts, while
approval interrupts (`tool_approval_interrupts`), token usage
(`usage_visible`) and subagents (`subagents_visible`) stay off unless the
endpoint enables them. Public Chat keeps all three off. See TM-TENANT-016,
TM-TOOL-052 and TM-API-026.
