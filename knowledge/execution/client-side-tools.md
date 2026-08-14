---
type: Specification
title: "Client-Side Tools"
description: "Client-side tools for API/SDK consumers."
tags:
  - everruns
  - execution
---
# Client-Side Tools

## Purpose

Client-side tools let an API or SDK consumer provide tool definitions whose
implementation runs outside Everruns. The server exposes the definition to the
model, pauses when the model calls it, emits a request event, and resumes only
after the client submits results or the wait is resolved another way.

This spec owns the pause/resume semantics and security invariants. It does not
copy tool JSON, event payloads, endpoint bodies, database columns, limits, or
status-code tables.

## Sources of truth

- [`crates/provider/src/tool_types.rs`](../../crates/provider/src/tool_types.rs)
  owns the tagged tool-definition model and exact client-side tool fields.
- [`crates/server/src/domains/agents/types.rs`](../../crates/server/src/domains/agents/types.rs)
  owns agent create/update request validation and deprecation behavior.
- [`crates/engine/src/execution/act_hooks.rs`](../../crates/engine/src/execution/act_hooks.rs)
  owns client-call detection, request-event emission, output limiting, and
  pause signaling.
- [`crates/core/src/events.rs`](../../crates/core/src/events.rs) owns the exact
  tool-request event payload.
- [`crates/server/src/api/tool_results.rs`](../../crates/server/src/api/tool_results.rs)
  owns result submission, response shape, status validation, event persistence,
  and workflow resume.
- [`crates/server/src/tool_result_timeout.rs`](../../crates/server/src/tool_result_timeout.rs)
  owns timeout recovery.
- [`docs/api/openapi.json`](../../docs/api/openapi.json) is the exact SDK/wire
  contract.
- [`crates/server/tests/client_side_tools_test.rs`](../../crates/server/tests/client_side_tools_test.rs)
  and strict-mode tests cover serialization and compatibility behavior.

Wire examples belong in OpenAPI and contract tests, not in this spec.

## Definition and ownership

Client-side definitions are stored with agent/session configuration and sent to
the model alongside executable server tools. Their tagged discriminator and
fields are defined by `ToolDefinition` and `ClientSideTool`; consumers must not
infer them from an old example.

The server has no implementation for a client-side tool. A matching model tool
call cannot fall through to a same-named server implementation. Reserved naming
rules prevent collisions with generated tool families.

Definitions may include display metadata, category, schema-defer policy, and
semantic hints as supported by the source type. Exact current fields and
validation limits are generated into OpenAPI.

## Lifecycle

The normal lifecycle is:

1. a client configures an agent or session with client-side definitions;
2. the model requests one or more of those tools;
3. Everruns emits the typed client-tool request event;
4. the session enters the waiting-for-tool-results state;
5. the client executes every requested call and submits correlated results;
6. Everruns records tool completion events, restores active execution, and
   resumes the durable turn;
7. the model receives the results and continues normally.

The request event is the authoritative list of pending calls. Clients correlate
by tool-call ID and should use optional server-authored display summaries rather
than inventing user-facing narration when present.

The exact event envelope and payload live in `crates/core/src/events.rs`; see
[`events.md`](events.md) for compatibility rules.

## Result submission

Result submission is accepted only when the caller can access the session and
the session is currently waiting for client results.

Each submitted item identifies the pending tool call and carries either a JSON
result or a safe client error. The API source and OpenAPI own exact optionality,
validation, response fields, and status codes.

Accepted results become ordinary tool-completion events before execution
resumes. This keeps replay and later model context consistent with server-side
tool execution.

Clients should submit the complete current batch and preserve every tool-call ID
exactly. The current endpoint accepts a non-empty list while the session is in
the waiting state and records the supplied IDs; it does not promise
set-equality validation against the last request event. If stronger correlation
validation is added, it must land in the endpoint and contract tests before
this spec claims it.

## Mixed batches

A model response may contain server-side and client-side calls together.
Server-side calls execute through the normal scheduler. Client-side calls are
collected into the request event, and the turn pauses after server work reaches
the established boundary.

On resume, the next model step sees both result families in their original
logical batch. Client submission does not re-execute completed server tools.

Ordering and concurrency details belong to the act atom and scheduler tests.

## Abandonment, cancellation, and timeout

A new user message while waiting abandons the pending client-tool request under
the session lifecycle contract. The runtime synthesizes interrupted results as
needed to keep the transcript valid, then starts the new turn.

If the client never responds, the timeout worker resolves the wait with safe
error results and resumes execution so the session does not remain stuck.
Exact timeout configuration and wording belong to
`tool_result_timeout.rs`.

Explicit session/turn cancellation follows the ordinary cancellation contract.
All recovery paths must leave a well-formed assistant-tool transcript; see the
repair rules in [`events.md`](events.md).

## Result content

Client results are untrusted tool output. The submission endpoint currently
relies on shared HTTP request limits and does not define a separate per-result
size contract. A dedicated limit must be implemented and covered by endpoint
tests before product or SDK documentation promises one.

The server must not execute, interpolate into a shell, or otherwise treat
client result JSON as trusted code. It is model-visible content plus durable
tool history.

## Compatibility window for legacy tool definitions

Agent and session `tools` fields are client-side-only public configuration.
Older clients may still send obsolete non-client variants.

During the compatibility window, request deserialization may drop unsupported
entries and emit a structured warning. An operator-controlled strict mode turns
that into request rejection. Update semantics distinguish an explicit empty
list from a list whose every entry was dropped, preventing an old client from
accidentally clearing valid tools.

Warnings include only safe discriminator/count information, never the request
payload. The exact environment switch, accepted values, cutoff policy, and
deserializer implementation live in the agent/session request source. Remove
the compatibility path when the release policy permits; do not extend it by
adding more copied examples here.

## Client requirements

Clients integrating this feature must:

- generate or consume definitions from the current OpenAPI model;
- subscribe to typed session events and recognize the waiting state;
- execute only calls present in the latest request event;
- preserve tool-call IDs exactly;
- submit JSON-safe results or concise errors;
- tolerate additive event fields;
- handle timeout, cancellation, reconnect, and duplicate UI delivery without
  executing a call twice.

SDK guides may show end-to-end examples, but those examples should be generated
or tested against OpenAPI.

## Security invariants

- Session authorization applies to event streaming and result submission.
- Client-side definitions never select server code.
- Clients preserve pending tool-call IDs; the server gates submission by
  session ownership and waiting state.
- Result content is treated as untrusted and remains subject to transport
  request limits.
- New user input, cancellation, and timeout cannot leave the session stuck.
- Warnings and errors do not log tool arguments, results, prompts, or secrets.
- Mixed batches do not permit client callers to overwrite server-tool results.
