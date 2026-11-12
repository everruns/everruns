---
type: Specification
title: "Turn Completion Check"
description: "A finishing turn on an unattended session is reviewed on the session's own model, and an announced but unfinished one continues automatically within a bounded budget, durably across worker restarts."
tags:
  - everruns
  - execution
  - turns
  - durability
---

# Turn Completion Check

## Problem

A turn can end on an announcement ("Submitting the review now") without the
call that does it. In chat a person notices and says "go on". A session started
by a trigger or a schedule has nobody watching, so the work is silently lost.

## Setting

`completion_check` is an agent setting
([`completion_check.rs`](../../crates/contracts/src/runtime/completion_check.rs)):
`off`, `unattended` (default) and `always`. `unattended` checks sessions whose
source is a trigger or a schedule (`SessionSource::is_unattended`, carried to
the runtime as `ExecutionSession.unattended`); `always` checks chat too. The
default is on for unattended work because that is where the loss is invisible,
and off for chat because the check costs a model call and a person is there.

## Flow

[`turn_completion_check.rs`](../../crates/core/src/host/turn_completion_check.rs),
called from the shared turn loop
([`turn_loop.rs`](../../crates/core/src/host/turn_loop.rs)) when a reason step
would finish the turn:

1. The cheap gate (`turn_completion::gate_turn`, see
   [Tool Execution](tool-execution.md#turn-completion-gate)) decides the
   clear-cut cases. Only `Evaluate` (tool-using work with a candidate answer)
   goes further. Questions, length stops and live background work never
   continue. One exception: under `always`, a plain answer that ran no tools
   is reviewed too. A coding agent that says "Running the tests now" and stops
   has made the same unkept promise without a tool call (yolop's own check
   caught these before it moved to the engine). The reason step carries the
   setting (`ReasonResult::review_tool_free_answers`), so agents on `off` or
   `unattended` still finish a tool-free answer without loading the session.
2. The setting and the session origin decide whether the check applies.
3. A review call runs on the session's own model through the normal provider
   path (purpose `completion_review`, 45 s timeout). Its verdict is `achieved`,
   `in_progress` or `blocked`.
4. `in_progress` within budget appends a continuation input to the same turn,
   and the turn reasons again under the same id. Anything else finishes.

Every check records `turn.completion_checked` with the verdict, the action
(`finished`, `continued`, `budget_exhausted`, `review_failed`) and the budget
used. A failed review never blocks the turn: it finishes.

## Continuation input

The continuation is an `input.message` with origin `turn_continuation`. The
model sees it prefixed as an automatic continuation, not written by the person,
and the review's evidence excludes it from the person's requests, so the
platform's prompt never reads as user intent.

## Budget

`ContinuationBudget` bounds continuations per original input in turns, tokens
and wall-clock. It is rebuilt from the log (the continuation inputs of the turn
carry what the first one saw), so a resumed turn does not get a fresh budget.
The agent's iteration cap still applies on top. Review-call tokens are not
counted against it.

The time limit is a window, not a deadline: a continuation that ran tools
restarts it, while turns and tokens stay cumulative. A long task that keeps
doing real work is then bounded by turns and tokens, and only a stalled or
chatty loop runs out of time. Yolop learned this the hard way: a clock pinned to
the first continuation cut long coding tasks at ten minutes while they were
still passing checks. Each continuation records the window start and the turn's
tool count, so the window also survives a resume.

An agent may replace the default limits with `ContinuationLimits`
(`AgentBuilder::continuation_limits` in core and the facade). It is
embedder-only for now: platform agents keep the runtime defaults, since a
coding agent such as yolop is the host that needs different limits.

## Durability

The check event is written before the continuation input. On resume:

- A continuation that is already stored is steering: the turn reasons on it
  ([`turn_resume.rs`](../../crates/core/src/host/turn_resume.rs)). No second
  check of the earlier answer, no second continuation.
- A crash during the review leaves the answer stored with no continuation, and
  the turn completes without checking again. That check is lost rather than
  risking a double continuation.

## Tests

`crates/worker/src/actor_runtime/tests/completion_check.rs` drives the session
actor with llmsim: continuation then tool call, completed turn not continued,
setting and origin matrix, budget exhaustion, agent limits with the restarting
window, and both crash points. `crates/everruns/tests/facade/completion_check.rs`
covers the embedder setting.
