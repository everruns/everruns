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

The `native_async_tools` capability explicitly selects tools for supported
providers. The [normal host Reason activity](../../crates/host/src/native_async.rs)
uses the [streaming coordinator](../../crates/engine/src/native_async.rs) for
internal HTTP continuations. The distributed scheduler receives a final outcome
only after all accepted outputs have provider receipts, including aggregate
usage and internal call counts. No model defaults or existing background
behavior change. Unsupported providers retain ordinary synchronous execution.

Distributed workers use an authenticated
[worker RPC](../../crates/internal-protocol/proto/worker.proto) backed by an
[encrypted PostgreSQL journal](../../crates/server/src/storage/native_async_store.rs).
Custom hosts can install the same store contract or use the private local file
journal with the lower-level coordinator API.

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
  are journaled before execution. Synchronous calls wait for successful response
  completion; a rejected or truncated response cannot release them. A shared concurrency cap covers all jobs.
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
- Early dispatch cannot run under hooks that require finalized output or finalized
  tool calls. Those combinations fail configuration before any call executes.
- Native call metadata and raw custom input persist in the assistant transcript.
  Tool results may be recorded while the assistant still streams; model replay
  orders early results after their original calls without inventing failures.
- Direct tools only: no programmatic tool calling, deferred tool-search path, or
  provider multi-agent mode. An earlier model keeps ordinary function definitions
  as the synchronous fallback. Pending native outputs cannot switch models or
  cross compaction boundaries.

## Persistence and recovery

The shared journal is private runtime state, outside session tools and file
storage. Database-clock leases fence every checkpoint read, write, renewal, and
release by tenant, session, turn, and owner. Lease renewal and owned tool tasks
progress concurrently so a shared RPC transport cannot deadlock persistence.
Losing ownership stops local jobs. Checkpoints are encrypted and bounded below
the internal transport's message-size limit; session deletion owns retention.
Forks do not copy live provider calls or ownership.

The local journal uses an OS file lock for exclusive ownership and atomically
replaces and flushes checkpoints. Both journals retain delivered call IDs as
tombstones. Results and delivery intent persist before continuation requests.

On recovery after a completed response, safe running calls are reauthorized and
restarted. Calls without a trusted replay-safety grant become explicit interrupted
outputs. A policy change prevents automatic replay. Cancellation aborts and joins owned local
execution tasks and preserves cancelled outputs. Recovery/cancellation errors are
recorded as canonical tool results before delivery; a late completion cannot
replace the first terminal result. Dropping a future cannot revoke an external
side effect already accepted by a tool service.

There is an unavoidable uncertainty window after HTTP submission but before a
completed response receipt is persisted. The journal retains the delivery intent
and refuses automatic replay in this case. Native continuations also disable the
ordinary missing-output fallback to a repaired stateless transcript. An interrupted response stream likewise
requires reconciliation: the older response ID cannot be assumed current. A host
must recover a provider receipt or resolve the conversation explicitly before
resuming. This is an attention-required state, not successful completion and not
a claim of exactly-once provider delivery. Corrupt or unavailable journal storage
fails closed.

## Integration and validation

The [HTTP integration tests](../../crates/host/tests/native_async_http.rs) exercise
both custom-host composition and the normal Reason/Act runtime, including raw
custom input, original-call outputs, transcript metadata, and completion gating.
The [PostgreSQL conformance test](../../crates/server/tests/repository_conformance_test.rs)
checks tenant isolation, competing owners, expiry, stale writes, encryption, and
recovery. A regression test exercises journal writes and tool jobs sharing one
transport lock. An isolated PostgreSQL/gRPC/API/worker run also verified early
`web_fetch` execution before the assistant message completed, a single canonical
result, original-call continuation, and final turn counts.

The expected assistant message boundary is persisted while its transcript is
being committed. On worker recovery, a matching canonical message reconciles
that boundary only when its saved response summary also matches; a missing message
or summary fails closed. Usage, first-token latency, and response budgets survive
worker recovery. A final host outcome is retained
for activity replay, preventing another provider request after the turn's work
has already finished.

Live GPT-6 Astra acceptance passed on 2026-09-06 through the Rust coordinator:
two asynchronous function/custom calls, raw executor-input preservation, original-call
output delivery across three responses, and final answers using both results. The
ignored `native_async_astra_live_function_and_custom_calls` test in the HTTP suite
reproduces this with synthetic data, bounded response/token budgets, and funded
`OPENAI_API_KEY` credentials. The ordinary fixture run leaves this live test ignored.

Recovery through provider response retrieval remains unverified. Ambiguous receipt
windows still fail closed; the live acceptance run does not establish exactly-once
submission or automatic reconciliation.

## Sources

Verified against official OpenAI documentation:
[GPT-6 Astra](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6-astra)
and [async tool calling](https://developers.openai.com/api/docs/guides/async-tool-calling).
