---
type: Specification
title: "Credit Shortfall"
description: "When a provider refuses a request for lack of credits, the engine retries once with fewer max output tokens when the balance covers a useful answer, and otherwise waits up to an hour on durable schedule checks that read the balance without spending tokens, resumes once, or stands down."
tags:
  - everruns
  - execution
  - providers
  - durability
---

# Credit Shortfall

## Problem

A GitHub-triggered review session on OpenRouter failed with
`provider_quota_exhausted`: the request asked for 65,536 max output tokens and
the key could afford 283. The turn simply failed, and nobody was watching to
send it again once the key was topped up. Two different situations hide in
that error: a balance that covers most of an answer, and one that covers none.

## Flow

The policy lives in
[`credit_shortfall.rs`](../../crates/core/src/engine/credit_shortfall.rs); the
reason step calls it from
[`provider_managed_compaction.rs`](../../crates/core/src/engine/execution/reason/provider_managed_compaction.rs)
and [`reason/credit.rs`](../../crates/core/src/engine/execution/reason/credit.rs),
so every host that runs the reason step (worker, in-process runtime) behaves
the same.

1. **Shrink.** A refusal that says what the balance covers
   (`affordable_tokens`, see [`provider_limit.rs`](../../crates/contracts/src/provider_limit.rs))
   of at least 4,096 tokens is retried once, the same request with max output
   tokens lowered to that figure minus a margin. A 402 comes before any
   output, so the retry repeats nothing.
2. **Wait.** Anything else fails the turn as before, with `credit_wait: true`
   on the error so the copy says the work continues by itself, and a one-shot
   session schedule fires a check a minute later. No worker is held.
3. **Check.** The check's turn reads the balance before any model call
   (`ChatDriver::credit_status`; OpenRouter `GET /auth/key`, no tokens).
   Still out: schedule the next check and end quietly, without a model call or
   a reply. Back: cancel queued checks and continue the work in that turn.
   A provider with no balance API is checked by the request itself, on the
   same timer.
4. **Stand down.** When the next check would land past the hour, the turn
   fails with the ordinary `provider_quota_exhausted` error and no new wait
   starts from it. A resumed request the provider still refuses goes back to
   waiting until the deadline, since a key limit's remainder can be above zero
   and still too small for the request.

Each decision records a `turn.credit_shortfall` event (`shrunk`,
`wait_started`, `still_waiting`, `resumed`, `gave_up`), so the trace shows the
whole wait.

## Durable timer

The wait reuses session schedules, the timer the rate-limit retry
(`rate_limit_auto_retry`) and `usage_limit_auto_continue` already use, rather
than a loop of its own. Schedules are durable rows the scheduler claims under
a lease, and they carry the same limits as any other schedule. A one-shot
schedule keeps no end time, so the wait's deadline travels in the check's own
prompt ("... Waiting until <RFC 3339>."), and every check of one wait shares
it. Each check is a short scheduled run in the session, visible in its
history; that is the cost of not holding a worker.

## Durability

- A check fires once: the scheduler claims a one-shot row under a lease.
- A check replayed after a worker restart finds the next check already
  queued and adds no second one.
- A resume cancels queued checks, so a later check cannot resume the same
  wait again.
- The shrunk retry is not recorded as state: a replayed reason asks with the
  original ceiling, is refused, and shrinks again.

A schedule whose prompt imitates a check can only make its turn read the
balance first; the marker grants nothing else.

## Tests

`crates/worker/src/actor_runtime/tests/credit_shortfall.rs` drives the session
actor with llmsim behind a refusing, balance-scripted model: smaller retry,
wait below 4,096 tokens, checks that resume once, no balance API, a wait that
runs out, and a worker restart during a check.
