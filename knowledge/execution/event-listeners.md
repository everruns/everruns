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
stopped between the commit and a listener lost that listener's work. The first
step (everruns#5186) answered the worker before its listeners ran; the feed
below makes them restart safe.

## Contract

1. **Every committed event, at least once.** A listener fed from the database
   sees every committed event of its types, across server restarts, deploys
   and NATS outages. An event that never commits is never delivered.
2. **Order per session.** A listener sees one session's events in sequence
   order, whichever caller or transaction stored them.
3. **Nobody waits.** No caller (worker or API) waits for the listeners of its
   events. Code that needs a listener's effect waits for it explicitly
   (`EventService::listeners_settled`, tests only today).
4. **Retries do not duplicate effects that matter.** A listener's database
   effects are keyed on the event, or apply state rather than deltas (below).
5. **A listener failure is contained.** A panic or a hang (30 s) in one
   listener does not stop the others or later events.

## Design (as built)

`crates/server/src/services/event/feed.rs` (runner) and
`crates/server/src/storage/repositories/events/feed.rs` (queries).

- **Commit-safe cursor.** Each event row records the id of the transaction
  that wrote it (`events.xact_id`, default `pg_current_xact_id()`; rows from
  before the feed are NULL and never read). A pass reads the events between
  its cursor and the oldest transaction still running (`pg_snapshot_xmin`), in
  one statement. Everything below that point committed or rolled back, so a
  late commit is read on a later pass. No outbox row per event, no relay. A
  long write transaction anywhere in the database delays delivery, never
  loses it.
- **Order inside a session.** A transaction that took its id earlier can
  commit a session's later event (it waited for the session's sequence lock).
  The feed keeps each consumer's last delivered sequence per session
  (`event_feed_sessions`), delivers a session's committed lower-sequence events
  that sit above the window along with the window, and skips them when the
  read point reaches them.
- **Partitions.** Each consumer is split into 8 partitions by session
  (`idx_events_feed` holds the partition expression). A replica runs a
  partition while it holds the partition's advisory lock on one connection it
  keeps for that; the others try every 5 s, so a stopped replica's partitions
  move within seconds. Passes borrow pool connections per statement and hold
  no transaction while listeners run.
- **Cursors in the database** (`event_feed_cursors`), advanced in the same
  transaction that records the delivered sequences, after the listeners ran,
  and only from the value the pass started at. A replica that stops mid-pass
  hands the pass over again.
- **Tiers.** Near (polled every 250 ms, woken at once by this replica's own
  writes): thread coordination, A2A push, notifications, MCP events,
  sandbox lifecycle, and listeners an embedder adds. Later (polled every 2 s,
  pages of 1,000): usage, budgets, customer tracing, turn latency, titles, run
  summaries, observer matching, approval audit, reporting facts. Each tier
  has its own cursors, so a slow exporter never delays a coordination
  wake-up. Usage and budgets
  share the later tier because both read `llm.generation`, the largest event:
  reading and decoding it in one tier, not two, is a measurable share of the
  server's CPU per turn.
- **In-process.** OpenTelemetry ends a turn's anchor span, started when the
  turn's first event is prepared on the emitting replica, when it sees that
  event, so it stays on the emitting replica with Braintrust and Prometheus
  (`listener_dispatch`, queued per session as before). Ephemeral events never
  reach the database and also go there.

Effects under replay:

| Listener | Why a second delivery is harmless |
|---|---|
| Usage | `llm_generations.event_id` is unique; totals follow the insert |
| Budgets | Journal unique on the source event |
| Notifications | Dedupe key per turn |
| Observer matching | Score rows unique per observer, scorer and turn |
| Approval audit | Audit row id is the event id |
| Reporting facts | Fact rows unique on a source key from the event (capability snapshots: the turn) |
| MCP events | Carry the event id for receivers to dedupe |
| A2A push, coordination, sandbox | Apply the current state, not a delta |
| Customer tracing, turn latency, titles, run summary | May repeat a span, a log line or a check after a crash |

## Cost

Per stored event: one more index entry (`idx_events_feed`). Per pass with
work: one window read, one delivered-sequence read, one read for out-of-order
events, one commit. A woken partition waits 50 ms first, so one turn's burst of
events shares a pass. Polling is one probe statement per consumer for every
partition the replica holds, not one per partition, and only this replica's
writes of a type a near listener wants wake a partition at once. An idle
partition moves its cursor every 5 s.

The first cut polled each of the 8 partitions of both tiers on its own and
woke the near tier on every write. At 100 agents on the stress bench that added
a quarter more commits per turn, 40% more database load and a third more
pickup latency, with overhead per model call unchanged within noise.

## Reporting

Reporting facts are a later-tier listener (`ReportingProjection`); event
writes no longer enqueue reporting outbox rows. Capability snapshot facts are
keyed per turn, so a turn's later model calls write none
(`knowledge/evaluation/reporting.md`).

## Success bar

- Overhead per model call at 100 parallel agents on the stress bench does not
  regress, and improves when listeners are slow.
- Repository tests (`events::feed::tests`): a late commit is read on a later
  pass, never skipped; a session stays in order when an earlier transaction
  commits later; an unrecorded pass is handed over again; a moved cursor
  refuses the older pass; one lock connection holds a partition; a probe names
  only partitions with something to deliver.
- Service tests (`services/event/tests/fed_listeners.rs`): a fed listener sees
  every committed event in session order without the caller waiting.
- The chaos harness stays green: live delivery of every stored event survives a
  dropped caller (`services/event/tests/cancelled_caller.rs`).
