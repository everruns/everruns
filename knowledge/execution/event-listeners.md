---
type: Specification
title: Event listener delivery
description: How stored session events reach the server's listeners (usage, budgets, traces, notifications, titles and others), in order per session and off the worker's critical path.
tags:
  - execution
  - performance
  - events
---

# Event listener delivery

## Problem

About fifteen listeners turn stored events into more work: usage and budgets on
`llm.generation`, the observer matcher, approval audit and MCP events on
`tool.completed`, thread coordination, A2A push, notifications and titles on
`turn.*`, sandbox start and stop on `session.activated` / `session.idled`, and
tracing, OpenTelemetry and metrics on most events.

Until 2026-11, a worker's event append waited for all of them. After the
insert committed, the server published the event for live delivery and then ran
every matching listener one after another, and the gRPC call was answered only
then. Every blocking event of a turn (`turn.started`, `tool.completed`,
`reason.completed`, ...) paid for several listener database queries before the
worker could continue. Listeners also ran only in memory, so a server that
stopped between the commit and a listener lost that listener's work.

## Contract

1. **Order per session.** A listener sees one session's events in the order
   they were stored, whichever caller stored them.
2. **The session actor's holder never waits for listeners.** A fenced append
   (`EventService::emit_batch_fenced`, the worker's turn path) is answered once
   its events are stored and published for live delivery. Nothing the worker
   does next reads a listener's effect.
3. **Other callers still wait.** API and server paths that emit an event and
   then read what a listener wrote (usage, titles) keep that behavior.
4. **A listener failure is contained.** A panic in one listener does not stop
   the others or later events.
5. **Bounded memory.** A slow listener slows its callers down rather than
   queueing without limit.

## Design (as built)

`crates/server/src/services/event/listener_dispatch.rs`. Each `EventService`
owns a fixed set of queues; a session always maps to the same queue. A queue
handles one event at a time and runs its listeners one after another, each in
its own task. A caller either waits for its event's listeners (contract 3) or
only queues it (contract 2). Queues are bounded; a full queue makes the next
caller wait. `EventService::listeners_settled` waits for everything queued so
far, for tests. A stopping server waits up to 5 s for queued listener work
after its worker link stops, so a rolling deploy does not drop what fenced
appends queued.

Live delivery (NATS or in memory) is not a listener: it is still published
before the caller is answered, so SSE clients see events as soon as they
commit.

## Planned: tiers and a feed read from the database

Listeners still run in memory. The next step makes them restart safe without
making NATS a store of record (RDS remains the only record of a session):

- **Tiers.** Near (under a second, one event at a time): thread coordination,
  budgets, A2A push, notifications, MCP events, sandbox start and stop. Later
  (seconds, hundreds of events per transaction): usage, reporting, session
  trace, customer tracing, OpenTelemetry, titles, run summary, turn latency,
  observer matching, approval audit, metrics.
- **Commit-safe cursor.** Each event row records the id of the transaction
  that wrote it. A consumer reads events of its types between its cursor and
  the oldest transaction still running (`pg_snapshot_xmin`): everything below
  that point committed or rolled back, so a late commit is read on a later
  pass, never skipped. No outbox row per event and no relay. A long write
  transaction delays delivery but cannot lose it.
- **Cursors in the database,** advanced in the same transaction as a
  consumer's database effects (exactly once for those). Outside effects (A2A
  push, notifications, sandboxes) are at least once with an idempotency key
  derived from the event id.
- **Order across replicas.** Consumers are split into partitions by session
  hash, each held by one server replica at a time (advisory lock). A failing
  event blocks only its own session.
- NATS carries only live delivery and wake-ups, neither of which holds anything
  the database does not.

## Reporting

Reporting is its own projector with an outbox (`knowledge/evaluation/reporting.md`).
Per-call capability facts were the largest share of a turn's rows, so
`REPORTING_BACKGROUND_ENABLED=false` turns reporting off for a deployment,
including the outbox enqueue on event writes, until the projector is reworked
as a later-tier consumer that writes capability facts once per turn.

## Success bar

- Overhead per model call at 100 parallel agents on the stress bench does not
  regress, and improves when listeners are slow.
- The chaos harness stays green: live delivery of every stored event survives a
  dropped caller (`services/event/tests/cancelled_caller.rs`).
