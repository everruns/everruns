---
type: Proposal
title: "Durable Execution Guarantees and Backend Qualification"
description: "Draft failure semantics, atomicity requirements, operation recovery rules and conformance gates for pluggable Everruns execution backends."
tags:
  - everruns
  - framework
  - durable
  - conformance
  - proposal
---

# Durable Execution Guarantees and Backend Qualification

The [Engine-owned sessions proposal](engine-durable-sessions.md) applies these
durability requirements to the public Framework API. Qualification through a
low-level authority client alone does not establish Engine/Session or serve
parity; those paths require the same failure tests through their ordinary APIs.

Implementation update: Engine-owned memory, native SQLite and Durable Object
profiles are implemented. See [Engine-Owned Durable Sessions](engine-durable-sessions.md)
and [Shared Execution with Durable Object Authority](durable-object-native.md)
for current scope and qualification. The broader design below remains a proposal.

**Status: draft proposal, 2026-10-16.** Companion to
[Portable Durable Agent Execution](durable-execution-portability.md), which owns
the source assessment, placement alternatives and rollout recommendation.
Requirements below describe the proposed contract, not guarantees every current
backend already satisfies. Proposed port names are conceptual, not implemented
Rust interfaces.

## Define what durability means

An acknowledged command survives the backend's declared failure domain, has one
stable identity, and remains either runnable, durably waiting, terminal, or
explicitly suspended with a diagnosable reason. Execution can resume without the
original caller or executor. Committed semantic results remain available after
restart. External side effects have the recovery semantics the tool declares;
no general exactly-once effect guarantee follows from actor ownership.

There are four independent properties:

| Property | Required evidence |
| --- | --- |
| Persistence | An acknowledged record survives the declared process/node/storage failure |
| Safety | Stale execution cannot append accepted state or claim a new unsafe dispatch |
| Liveness | Runnable work has a durable activation obligation and recovery mechanism |
| Effect handling | Stable operation identity, durable start/result boundaries and an explicit unknown-outcome policy |

A durable event log establishes only the first. A renewed lease alone does not
establish the other three. Local fsync can cover process crashes without covering
machine loss. Asynchronous bucket snapshots have a recovery window unless the
acknowledgement waits for durable publication. Backend metadata and documentation
must describe the actual acknowledgement boundary and failure scope.

## Session authority and the log

The authority owns the session's command deduplication, canonical event order,
input progress, execution epoch, waits and operation outcomes. PostgreSQL may
implement this over coordinated tables and DO over its private SQLite. Blob
storage and reporting projections can be outside this transaction domain.

The log is authoritative for conversation and turn lifecycle. Control metadata
is additional durable state: an accepted but unprocessed command, a wait deadline
or operation claim cannot always be reconstructed from a completed message.
Avoid claiming that conversation events alone provide every scheduling guarantee.
Control indexes must have consistency invariants and a repair/reconciliation path.

Preserve the current [EventLog](../../crates/core/src/host/events.rs) guarantees:
storage-owned IDs and sequences, read-after-write visibility, immutable canonical
events, bounded snapshot-aware reads, explicit durability class and real conditional
append. Keep ephemeral deltas out of canonical storage where the current protocol
does. An epoch check must be atomic with accepted remote writes; optimistic
sequence checks alone cannot revoke an old executor.

Use one authority for an active session. A PostgreSQL mirror of a DO log is an
asynchronous projection, not a coequal writer. Tenant/session routing and backend
binding must be durable and unambiguous. Multi-object operations use durable
messages/outboxes and deduplication; they do not inherit a distributed transaction
from DO storage.

## Commands, activation and ownership

### Admit once and retain the obligation to run

Before returning an accepted receipt, persist the command identity, relevant
input/control records and a discoverable obligation to activate the session.
Duplicate identity with equal content returns the same admission; different
content is a conflict. Client retry after a lost response must not create another
turn input. Session creation needs the same treatment, using a stable creation
identity and recoverable session ID rather than an unjournaled random allocation.

The existing platform transaction already couples input with a runnable marker.
Use the same semantic boundary on DO. Prefer an atomic local transaction that
couples command state and its alarm when supported by the pinned SDK/runtime.
Cloudflare's synchronous SQL transaction callback cannot contain promises; do
not describe an async `setAlarm` in `transactionSync` as atomic. Verify the actual
SDK binding. If alarm programming is separate, establish a durable recovery path
for the gap: an already armed alarm, transactional outbox plus independent
dispatcher/reconciler, or equivalent persistent discovery. A stored flag with no
guaranteed future reader is insufficient.
[SQLite storage API](https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/).

Native activation uses the existing runnable index with polling as correctness
fallback. DO activation uses alarms. Dispatching to a native service is an
at-least-once message: acknowledge durable admission first, arm recovery before
dispatch, and use stable activation identities. Lost HTTP replies must trigger
reconciliation, not simultaneous independent owners.

### Protect execution state and side-effect dispatch

A native executor claims a monotonically changing execution epoch before running.
The authority checks the epoch on log/progress writes, operation starts and owner
renewal. No successful renew for the allowed interval stops execution; rejected
appends stop it too. Expiry uses authority time. Duplicate starts within one
process must still serialize even if they share the lease-store holder.

An in-DO runtime can use platform ownership for its local state; it does not need
to simulate PostgreSQL polling. Still serialize semantic drive operations and
reject results from superseded attempts. An old instance or an already issued
remote tool request can finish externally after ownership changes. Neither a DO
nor a database fence retracts a network effect already sent.

Steering and cancel should remain responsive while a model request waits. Accept
commands concurrently, but apply them at the shared runtime's defined boundaries.
Input taken at a reason boundary cannot be forgotten on a crash. Commit input
progress only when the log contains enough evidence to reconstruct its inclusion;
never consume fresh input before its turn-start evidence is recoverable.

At finish, take pending steering or close the inbox atomically. Later input belongs
to the next turn and keeps the session runnable. Cancel targets a recorded turn
or input generation, has one authoritative terminal-event writer and cannot leak
onto a follow-up turn. Cancellation ends future dispatch; it does not promise to
undo already executed tools. Terminal writes must be idempotent under retry.

## Operations: tools and model calls

### Tools

Reuse [SideEffectClass](../../crates/contracts/src/tool_types.rs),
[ActAtom](../../crates/core/src/engine/execution/act.rs), the existing
[claim/settle store](../../crates/core/src/durability.rs) and
[recovery policy](../../crates/core/src/host/turn_resume.rs). Make the required
persistence barriers enforceable by a backend, not a comment that each adapter
may interpret differently.

| Persisted state of a call | Recovery |
| --- | --- |
| No durable dispatch authorization | Dispatch normally after policy checks and durable start |
| Settled result | Reuse the result; repair any missing canonical completion without dispatch |
| Started, no result; pure | Retry under the same logical call identity |
| Started, no result; idempotent | Retry with the same downstream idempotency/domain key and equivalent effective arguments |
| Started, no result; at-most-once | Report interrupted/unknown; do not automatically dispatch again |
| Waiting for approval or client result | Restore the wait; process a deduplicated resolution |

Operation identity must include session, turn and tool-call identity; store and
compare the tool implementation/version and effective-argument fingerprint.
Do not put the fingerprint in a key in a way that silently turns changed arguments
into a new allowed operation. Retry after a policy/argument rewrite must either
match the recorded authorized operation or fail explicitly.

Persist dispatch authorization before running an unsafe call. Current `tool.started`
already provides this barrier on supported native hosts; a staged event that is
later discarded with an act batch does not. A separate durable operation record
may implement the barrier, but the shared recovery path must understand it.
Results should be recorded per tool, so one failed call does not require repeating
its completed siblings.

Prefer settling the receipt and canonical result in one authority transaction.
Where compatibility requires separate writes, recovery must inspect both and
repair deterministically before dispatch. Current act code emits completion and
settles its optional result store sequentially; its existence alone does not
establish cross-port atomicity. The operation ledger and log must never authorize
conflicting reruns.

For at-most-once, a crash after recording start but before sending the request can
produce zero external executions and an unknown result. That is an intentional
safety tradeoff. For idempotent tools, the declaration must be true of the actual
implementation and downstream service; a framework-generated key only helps if
the receiver honors it. A durable outbox provides reliable delivery, and a
deduplicating receiver can make that delivery produce one logical effect. An
outbox alone does not provide exactly-once effects.

These guarantees apply per logical operation. A model may issue the same business
action later under a new tool-call ID. Payments, sending messages and other
business mutations may require a domain key, reconciliation or policy beyond
the runtime's call-level deduplication.

An approval wait is not an unsafe tool dispatch. After approval, the actual tool
still needs its start/result safety protocol. Recovering a missing result must
not assume that any approval-gated tool has never executed. Persist the decision
and authorized operation identity, recheck mutable security policy on resume, and
record any refusal instead of silently changing effective arguments.

### Models and streaming

Keep completed canonical assistant output and its tool-call IDs. Once a model
result is committed, recovery must not ask the provider for that same decision
again. An interrupted request with no committed result can be repeated, can incur
another charge and can produce different output. Provider idempotency or resumable
response support is an optional optimization with explicit semantics.

The existing partial-stream recovery contract may finalize usable partial text;
preserve it where configured and qualified. Do not promise that arbitrary provider
streams can resume from a token offset. Live deltas can disappear on restart;
canonical completion/replacement and cursor-based replay repair the UI. Attempt
identities prevent stale streams from overwriting a newer attempt. No side effect
should dispatch from an incomplete, uncommitted streamed tool call.

The current celld example's step-atomic staging can be retained for a reason's
result, but it must not erase safety evidence for external tools. Do not impose
an extra full turn-state checkpoint after every phase on native workers; reuse
the log fold and existing event commits. Versioned checkpoints are optional
accelerators, and require a known log position.

## Durable waits, resources and versions

Park approvals, questions, client tools, timers and external-job waits as durable
records with stable identity, turn context and optional deadline. Release active
compute. A resolution records its identity and result with reactivation; duplicate
answers cannot resume twice. Late answers must be checked against the active wait
generation. Booting a process should reconstruct waits, not depend on a lost
oneshot sender. A pending approval must not be mistaken for an in-flight effect.

External tools and subagents use durable request/reply identities. Persist the
parent-child relation before launching a child; deduplicate launch and completion.
Every declared durable background capability must survive through its own state
and activation contract. Session durability cannot make an arbitrary
`tokio::spawn` task durable.

Session state and workspace durability are distinct. A local file mutation or
sandbox job is not transactional with DO SQL. Use a durable workspace backend,
snapshots with a stated recovery point, or reconstructible files as appropriate.
Store large artifacts immutably before referencing them in accepted canonical
events; failed publication can leave collectible orphan blobs, not broken
acknowledged references. Do not store credentials in exportable runtime snapshots;
resolve scoped secrets at execution time.

Pin reconstructible agent/build identity, relevant tool policies and effective
turn configuration. Persisted prompt/configuration references must remain
resolvable; a build hash without retained executable behavior cannot resume a
closure. Bound external authorization to the actual session tenant and operation.
Preserve provider replay/compaction artifacts needed by conversation reconstruction.

Version log/control/operation encodings and validate them before resuming. An
incompatible deployment must route to the retained build, perform an explicit
migration or suspend with a diagnosable reason. Cloudflare deployments can expose
mixed Worker/DO code versions; persistent state does not imply retention of old
agent code. Keep migration short and restart-safe, with explicit retained-build
or compatible-reader strategy.
[DO lifecycle and code updates](https://developers.cloudflare.com/durable-objects/concepts/durable-object-lifecycle/).

## Alarm and failure handling

Maintain the next required wake as the earliest of runnable continuation, operation
reconciliation, wait deadline and retry. One DO has one alarm, so represent multiple
deadlines durably and select the minimum. An alarm is a wakeup, not the execution
state itself. Process a bounded amount of work and re-arm before yielding.

Cloudflare alarms run at least once, automatically retry failures only up to six
times, and allow only one alarm handler per instance at a time. Persist attempt
state and explicitly schedule further retries or suspend/seal with a reason;
unbounded reliability cannot depend on the built-in retry budget. New commands
must not postpone an earlier required wake. Test cancellation, idle cleanup and
alarm deletion against commands arriving during external I/O.
[Alarms API](https://developers.cloudflare.com/durable-objects/api/alarms/).

Differentiate provider/tool failures, retryable authority/transport failures,
lease loss, invalid state and deterministic runtime bugs. Use bounded retry with
backoff and jitter, and preserve the platform's poison/no-progress guard. A
downstream outage should not be reported as a model failure. A sealed/suspended
turn remains inspectable; recovery or a later message must not be blocked forever
by a poison activation.

## Qualification matrix

Extend the existing [backend conformance tests](../../crates/everruns/tests/backend_conformance/main.rs)
and [worker actor tests](../../crates/worker/src/actor_runtime/tests/mod.rs) into
a reusable suite that third-party adapters can run. The existing suite describes
the contract but is currently repository test code, not a published adapter kit.
Scripted models are appropriate for fault tests; runnable examples use real models.

| Crash/race boundary | Required assertion |
| --- | --- |
| Admission committed, acknowledgement lost | Retry returns the same command/session; one logical input |
| Command stored before activation programming | Independent recovery runs it, or admission never succeeded |
| Native dispatch sent, reply lost | One valid epoch; duplicate activation cannot start a second owner |
| Input selected before turn-start commit | Restart neither loses input nor starts it twice |
| Completed model result before act | Same committed tool-call identities; no repeated model decision |
| Unsafe tool start stored before network send | Recovery reports unknown, without another automatic dispatch |
| Effect succeeded before result persisted | Idempotent receiver deduplicates; at-most-once stays unknown |
| One tool settled in a parallel batch | Completed sibling reused; only permitted unfinished calls retried |
| Result receipt and event interrupted between writes | Canonical repair, with no conflicting result or new dispatch |
| Owner paused past expiry, then resumed | Stale writes and new dispatch claims rejected |
| Input arrives as reason would finish | Steering or next turn exactly as shared boundary specifies; no loss |
| Cancel recorded, owner dies before release | One terminal event; follow-up message is not cancelled |
| Approval/question/client result across restart | Same wait identity; repeated resolution resumes once |
| Alarm repeated, exhausted or replaced | No duplicate committed result; recoverable obligation or explicit suspension |
| SSE disconnected during turn | Accepted work progresses; reconnect recovers canonical output by cursor |
| Build/schema/tool policy changes | Compatible replay or explicit rejection; no unsafe implicit migration |
| Parent/child creation or reply interrupted | One logical child and one applied completion |
| Session/workspace shared by concurrent requests | Correct authority and separately enforced resource ownership |
| Long history or large artifact | Bounded reads and memory; canonical references remain resolvable |

Run deterministic semantic tests against memory, SQLite/native, PostgreSQL,
celld and Cloudflare adapters where supported. Compare meaningful canonical event
ordering and results, normalizing nondeterministic IDs/timestamps and allowing
documented transport/progress differences. A profile cannot claim a capability
whose scenarios it omits.

In addition, run real process kills, node failover where available, authority
network partitions, stale executor reconnection, deployment replacement and
client disconnects. Local emulation proves API behavior, not provider replication
or production lifecycle guarantees. Verify the pinned celld release and actual
Cloudflare deployment separately before declaring a supported backend.
