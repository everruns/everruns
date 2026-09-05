---
type: Specification
title: Native asynchronous tool calls
description: Opt-in streaming coordinator, pending-call persistence, and recovery boundaries for provider-native asynchronous tools.
tags:
  - execution
  - tools
  - durability
---

# Native asynchronous tool calls

Native async tools let response generation continue while the application executes
an accepted call. Results belong to the original provider call and continue the
latest response. They do not produce the handles or synthetic user messages used
by [background execution](background-execution.md).

The current implementation is an explicit **custom-host API**. It comprises the
[OpenAI request opt-in](../../crates/drivers/openai/src/async_tools.rs),
[portable coordinator](../../crates/engine/src/native_async.rs), and
[local durable journal and Act executor adapter](../../crates/host/src/native_async.rs).
The normal distributed worker's Reason/Act workflow does not install this
coordinator, and rejects unexpected native calls rather than dropping them.
No model defaults or existing background behavior change.

## Execution contract

- Only a complete streamed call may start execution. A completed sibling cannot
  make partially accumulated arguments executable. Async metadata and raw custom
  input survive separately from ordinary function-call snapshots.
- Explicitly select tool names. Start with slow read-only lookup functions such
  as `web_fetch`, with file-saving behavior excluded by the host's authorization
  policy. The initial request gate requires automatic, read-only tools without
  a conflict class. Read-only hints are not authorization; the runtime executor
  still runs the ordinary Act pipeline, including execution hooks and scoping.
- All accepted calls, including synchronous calls mixed into an async response,
  are journaled before execution. A shared concurrency cap covers all jobs.
  Conflict classes serialize their members; disabling parallel tool calls
  serializes the entire job set.
- Response consumption and tool execution progress together. Independent response
  continuations may occur while jobs remain pending. Completed results can arrive
  out of launch order. Synchronous results must be ready before continuation.
- A response is not conversation completion while accepted calls have undelivered
  results. The coordinator's full `run` method enforces this; hosts driving `pump`
  themselves must consult the checkpoint's completion gate.
- No wait tool is required: the runner delivers available outputs. A host adding a
  wait tool must make it synchronous and place newly available original-call
  outputs before that tool's status.
- Direct tools only: no programmatic tool calling, deferred tool-search path, or
  provider multi-agent mode. An earlier model keeps ordinary function definitions
  as the synchronous fallback. Pending native outputs cannot switch models or
  cross compaction boundaries.

## Persistence and recovery

The local journal is private host state, outside the session filesystem and tool
storage. Its OS file lock fences duplicate owners and releases on process death.
Checkpoints are atomically replaced and flushed before execution or HTTP result
delivery. Keep the journal directory for the entire conversation: delivered call
IDs remain tombstones, so a repeated call/result cannot execute or deliver twice.

On recovery after a completed response, safe running calls are reauthorized and
restarted. Calls without a trusted replay-safety grant become explicit interrupted
outputs. A policy change prevents automatic replay. Cancellation drops owned local
execution futures and preserves cancelled outputs; a late completion cannot
replace the first terminal result. Dropping a future cannot revoke an external
side effect already accepted by a tool service.

There is an unavoidable uncertainty window after HTTP submission but before a
completed response receipt is persisted. The journal retains the delivery intent
and refuses automatic replay in this case. An interrupted response stream likewise
requires reconciliation: the older response ID cannot be assumed current. A host
must recover a provider receipt or resolve the conversation explicitly before
resuming. This is an attention-required state, not successful completion and not
a claim of exactly-once provider delivery. Corrupt or unavailable journal storage
fails closed.

## Integration boundary and remaining work

The [HTTP integration test](../../crates/host/tests/native_async_http.rs) is an
executable custom-host composition using a file journal, OpenAI streaming driver,
and coordinator. It exercises both function and custom calls through subsequent
HTTP requests and checks the persisted completion state after reopening.

Distributed worker adoption still requires a database-backed fenced journal,
workflow scheduling/checkpoint integration, durable cancellation routing, and
session transcript/event integration for the coordinator's internal continuations.
Do not advertise a session capability until those paths are wired and tested.
The local journal is not a substitute for shared durable storage across workers.
Live GPT-6 Astra acceptance and recovery through provider response retrieval have
not been verified by the local fixture tests.

## Sources

Verified against official OpenAI documentation:
[GPT-6 Astra](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6-astra)
and [async tool calling](https://developers.openai.com/api/docs/guides/async-tool-calling).
