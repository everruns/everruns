---
type: Specification
title: Sticky sessions
description: A session's next turn goes back to the worker that ran its last one, whose history cache is warm, unless that worker cannot take it.
tags:
  - execution
  - performance
  - worker
---

# Sticky sessions

## Problem

Any worker could lease any runnable session, and every new message woke every
worker, which then raced to lease it. A turn usually landed on a worker that
did not run the session's previous turn, so the [worker history
cache](worker-history-cache.md) missed and the turn's first read loaded the
session's whole history. Every wake-up also cost one lease statement per
worker, of which all but one found nothing. Both grow with the fleet: the
capacity target is 8,000 parallel agents.

## Contract

1. **Same lease rules.** Who holds a lease, how it is renewed, released, handed
   off or taken over after it expires, and the fence on every write are
   unchanged. The preference only orders candidates and briefly holds them
   back from other workers.
2. **Never wait on a worker that cannot run the turn.** A preferred worker that
   is gone, silent, draining or full holds nothing back.
3. **Bounded wait otherwise.** Other workers may take a preferred session once
   it has been runnable for the grace (300 ms by default,
   `SESSION_ACTOR_STICKY_GRACE_MS`; 0 turns stickiness off).
4. **Any number of server replicas.** The preference lives in the database, not
   in a server's memory.

## Design

`crates/server/src/storage/repositories/session_actors/mod.rs`, migration
`20261103151954_session_actor_preferred_worker.sql`.

- Releasing a lease records the releasing worker on the actor row
  (`preferred_worker`). A hand-off (a draining worker giving a turn back
  mid-turn) clears it.
- `session_actor_worker_ready(worker)` decides whether the preferred worker can
  take a turn now, from the worker registry: registered, `active`, accepting
  tasks, a heartbeat in the last 30 s (three heartbeats) and below its slot
  count. Lease and wake-up use the same function, so they agree.
- A lease call scans its own preferred sessions first, then everyone else's it
  may take now: no preferred worker, past the grace, or a preferred worker
  that is not ready. The per-org cap still orders the batch first; own
  sessions go next.
- Marking a session runnable returns its preferred worker when it is ready.
  The wake-up then names that worker (`session_actor@<worker>` on the
  `task_available` channel, from the turn backend under NATS and from the
  `session_actors` trigger under PostgreSQL LISTEN), and the server's
  notification stream passes it only to that worker's subscriptions. Any
  other wake-up still goes to every worker. No worker change was needed.
- A lost wake-up costs what it did before: the lease loop polls at least
  every second (`ACTOR_POLL_BACKOFF_MAX`).

The registry's load is the worker's last heartbeat (every 10 s), so a worker
that filled up since then can still be preferred for one grace. Its own
lease call takes only what it has room for; the rest go to others once the
grace ends.

Kept for later, if the bench shows they still matter: keeping a turn's
agent, harness, model and provider reads across turns on the preferred
worker (today they live for one turn), cleared by the config change
notifications.

## Measured

Stress bench, 100 sessions on two workers (2026-11-03): with the [history
cache](worker-history-cache.md) keeping windows, cold window reads fell from
every model call (875 a run) to 142, of which 100 are first turns; 79% of
follow-up turns started on a warm worker, and message rows read fell 77%.
Overhead per model call is unchanged between grace 0 (stickiness off), 100 and
300 ms, within run-to-run noise.

## Success bar

- On the stress bench (two workers), history reads at the start of a turn
  fall to near zero and overhead per model call at 100 agents does not
  regress.
- Repository tests (`session_actors/sticky_tests.rs`): release records the
  worker and the next wake-up names it; the preferred worker gets the session
  back while another waits the grace; past the grace anyone takes it; a full,
  draining, silent or unregistered preferred worker holds nothing back and
  its wake-up goes to everyone; a takeover fences the old holder's renew and
  release out; a hand-off clears the preference; a worker's own sessions go
  before older ones.
- Notification tests: a targeted wake-up reaches only its worker's
  subscriptions, and a plain one reaches all.
