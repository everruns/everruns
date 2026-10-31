---
type: Specification
title: Detached tool calls
description: Long server-side tool calls leave their turn, so the turn parks and frees its worker slot, lease and memory while the tool works.
tags:
  - execution
  - tools
  - durability
---

# Detached tool calls

## Problem

A worker leases a session and runs its whole turn. Before this change only client tools,
approvals and `ask_user` parked a turn. Every server-side call (a sandbox build, a long MCP call,
`web_fetch` on a slow site) kept, for its whole duration:

- one of the worker's execution slots (`MAX_CONCURRENT_TASKS`, default 50),
- the session lease, renewed every few seconds,
- the turn in memory: agent config, tool surface, history, write-behind buffers.

Model streaming already gave its slot back ([`model_wait`](../../crates/core/src/engine/execution/model_wait.rs)).
Tool waits did not, so 50 sessions waiting on a 20-minute build filled a worker. AWS Infinity
suspends at every tool call, so a waiting agent costs it nothing; this closes most of that gap.

## Decision

A server-side call still running after `WORKER_TOOL_DETACH_AFTER_SECS` (default 30, `0` turns
detaching off) is **detached**:

1. The call keeps running on the same worker, outside the slot budget. Only the tool's own
   future stays, with the rest of the call (output limits, distillation, the call's
   `tool.completed`).
2. The act parks the turn (`durable_retry::parked`). Its log already holds `tool.started`, so the
   actor releases the lease and the slot and drops the turn's memory. No `act.completed` is
   written for that act; the call's `tool.completed` settles it.
3. When the call finishes, its `tool.completed` is appended under the call's record, and
   completing the last record of the session makes its actor runnable.
4. Any worker leases the actor. The fold sees every call settled and goes on to the next reason,
   the path a turn parked on client tools already takes. There is no new resume code.

Calls that finish within the threshold run as before, with no extra writes. The threshold is
high because each detach costs a re-lease and a turn setup when the turn resumes.

### What can detach

See `may_detach` in [`tool_detach.rs`](../../crates/core/src/engine/execution/tool_detach.rs).
A call must run on the server, must not be `cpu_bound` (an in-process interpreter really uses
the worker's CPU, which the slot cap must keep counting), and must not shape its turn
(`stays_direct`, native-only tools such as `spawn_agent` and `ask_user`). An act that also has
client-side calls never detaches. There is no per-tool opt-in.

Hosts opt in through task-locals. The worker provides a detacher around each leased turn, and
only the act of that turn's own loop sees it; tools the act runs inline do not, so a child turn
(a subagent) never detaches into its parent's park. The in-process runtime (serve, embedded
`Engine`) has no slot budget and provides none, so its calls keep running inline.

`MAX_DETACHED_TOOL_CALLS` (default 1000) bounds the calls one worker holds. Past it a call stays
with its turn.

## Records and crash safety

Table `detached_tool_calls`, migration
[`20261026232314_detached_tool_calls.sql`](../../crates/server/migrations/20261026232314_detached_tool_calls.sql);
storage in
[`session_actors/detached/`](../../crates/server/src/storage/repositories/session_actors/detached/mod.rs),
worker side in
[`actor_runtime/detached_calls.rs`](../../crates/worker/src/actor_runtime/detached_calls.rs).

- **Gate.** While any row of a session exists, its actor is not leased. Steering input sent
  meanwhile waits for the call, as it waits for any act.
- **Detach.** Only the actor's live lease holder can insert a row (fenced on the lease).
- **Renew.** One batched call per worker every third of the TTL renews all of its rows. A row
  missing from the reply (cancelled, expired) stops its call.
- **Events.** A detached call's events (progress, `tool.completed`) wait until its turn parked
  with its own events stored, so `tool.completed` never lands before `tool.started`. They are
  appended under the row's fence: an append after the row is gone is refused.
- **Turn did not park.** A turn that ends any other way (cancel, lost lease, failed flush) drops
  its detached calls and expires their rows; the next holder runs the turn from the log.
- **Worker dies.** Renewal stops, the row expires, and the next lease call deletes it and makes
  the actor runnable. The fold applies the crash-mid-tool rule by the call's
  [`SideEffectClass`](../../crates/contracts/src/tool_types.rs): rerun a `Pure` or
  `Idempotent` call, settle any other as `interrupted`. Same outcome as a crash mid-tool, one TTL
  later.
- **Shutdown.** After the running turns' grace the worker stops its detached calls and expires
  their rows at once, so another worker resumes those turns without waiting out a TTL.
- **Cancel.** The cancel deletes the session's rows; the cancel notification pokes the worker to
  renew at once, which stops the call.

Lock order is the actor row first, then its detached rows, in completion, the expiry sweep and
cancel alike, so concurrent completions agree on which one was last.

## Effect

| | before | detached |
|---|---|---|
| slot per waiting call | 1 of 50 | 0 |
| lease renewals while waiting | every few s per session | one batch per worker |
| turn state in memory | full | the tool's future only |
| crash mid-wait | rerun or `interrupted` | same, one TTL later |

This is still not Infinity's zero: the tool's future lives on one worker, and a lost worker loses
the call. True zero needs tools that hand their work to an outside system and are polled or
called back, for example a sandbox command started in the background with a journal timer
polling its exit. That would be opt-in per tool.

## Tests

- Core act hand-off: [`tool_detach_tests.rs`](../../crates/core/src/engine/execution/tool_detach_tests.rs).
- Storage, gate, sweep and fencing: [`detached/tests.rs`](../../crates/server/src/storage/repositories/session_actors/detached/tests.rs).
- Worker end to end, including worker loss, `AtMostOnce` calls, cancel, a turn that does not
  park and the cap: [`actor_runtime/tests/detached.rs`](../../crates/worker/src/actor_runtime/tests/detached.rs).
