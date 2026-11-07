---
type: Specification
title: Event log group commit
description: Fenced event appends from many workers share one PostgreSQL statement, commit and WAL flush, answered only after it commits.
tags:
  - execution
  - performance
  - events
---

# Event log group commit

## Problem

A worker stores a session's events with a fenced append
(`Database::create_events_fenced`, or `create_events_detached` for a detached
tool call). Each append was its own transaction: create or lock the session's
`event_sequences` row, check the lease fence on `session_actors` with
`FOR SHARE`, insert, commit. Every commit waits for a WAL flush. At 500
parallel agents in production the flushes were the limit (WALWrite waits,
2026-10-29), and the capacity target (8,000 agents) needs several thousand
appends per second.

## Contract

1. **Same durability.** A caller is answered only after the statement that
   holds its events commits.
2. **Same fence.** An append lands only while its holder holds the lease (or
   the detached call) on every session it writes to, checked under
   `FOR SHARE` in the statement that inserts it.
3. **Isolation.** An append whose lease is gone is refused alone; the others
   in its group still commit. Any other failure before the commit reaches only
   the append that caused it.
4. **Order.** A session's appends keep their arrival order and consecutive
   sequences.
5. **No added wait at low load.** Nothing waits for a batch to fill.

## Design

`crates/server/src/storage/repositories/events/group_commit.rs`.

- Each server process has a few lanes (`EVENT_GROUP_COMMIT_LANES`, default 4).
  A session always maps to the same lane. A lane takes the first queued
  append and everything else queued behind it (up to 64 appends and 1,000
  events), and stores them in one autocommitted statement: lock every
  session's `event_sequences` row (`FOR NO KEY UPDATE`, session id order),
  check every (append, session) pair's fence (`FOR SHARE`, joined to those
  locked rows), reserve sequences for the accepted appends, insert their
  events. Then it answers each caller.
- One statement, not a transaction. As a transaction an append cost six
  statements in the stress bench (BEGIN, a no-op upsert as the lock, the fence
  check, the insert, COMMIT, and a foreign-key check on `sessions` that
  PostgreSQL runs when a row version written by the same transaction is
  updated again). As one statement it costs one, and the sequence row is
  updated once (2026-11-07: 307 to 237 statements per turn at 100 agents).
- A session without an `event_sequences` row yet (no event so far) cannot be
  locked by the statement; its appends run alone after the group, in order,
  through the single append, which creates the row.
- The lane commits as soon as its previous commit returns. Alone, an append
  costs what it did plus an in-process hand-off. Under load an append waits
  for at most the commit in flight, and groups grow by themselves.
- If PostgreSQL refuses the statement (a payload it rejects, a deadlock),
  nothing of it landed, and the group runs again append by append, so the
  error lands on its cause. A connection lost mid-statement is not retried,
  because its outcome is unknown; each caller gets the error, as a lone
  append's caller did.
- Locks are taken as every other writer takes them: a session's sequence row
  before its actor row, sessions in id order, in every lane and replica, so
  lanes, replicas and message intake cannot deadlock on them. Server
  replicas need no coordination: each groups what it receives.
- An append inside a command transaction, or one larger than a group, runs
  alone on the caller's connection.
- `EVENT_GROUP_COMMIT=off` turns grouping off, for bench comparisons.

PostgreSQL's own `commit_delay` groups only WAL flushes across connections,
and adds a fixed wait under load. Grouping here also saves the statements,
round trips and connections of each append.

## Success bar

- At 500 agents on the stress bench, event commits per turn and WAL flush
  waits drop; overhead per model call at 100 agents does not regress.
- Repository tests: a group stores every live append and refuses a stale one;
  a failing append fails alone; detached appends are checked against their
  call; concurrent appends through the lanes land in order; a session's first
  event goes alone while the rest of its group lands; grouped appends, intake
  and lease renewals on the same sessions do not deadlock and leave gap-free
  sequences (`group_commit::tests`). The existing fenced-append tests
  (`session_actors/tests.rs`) run through the same path.
