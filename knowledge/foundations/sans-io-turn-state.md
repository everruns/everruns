---
type: Specification
title: "Sans-IO Turn State"
description: "Converging the two turn-loop implementations on one serializable state with pure transitions."
tags:
  - everruns
  - foundations
---
# Sans-IO Turn State

## Abstract

The turn loop is implemented twice. `TurnStateMachine`
(`crates/core/src/turn.rs`) is a mutable, in-memory machine driven by the
in-process runtime; `RuntimeTurnState` + `plan_next_host_turn`
(`crates/runtime/src/turn_strategy.rs`) is a serializable state plus a planner
driven by the durable worker. They encode the same phases and the same
transitions, in different shapes, and neither can be derived from the other.

This spec proposes converging them on one **sans-IO** representation: a
serializable `TurnState` whose transitions are pure functions, with all I/O —
loading messages, calling the model, running tools, emitting events, persisting
— performed by the host that drives it. The intended end state is that a durable
host is an in-process host that persists between steps, rather than a second
implementation of the same loop.

Status: **stage 1 landed** (the value type and its equivalence proof). Later
stages are proposals, not commitments.

## Motivation

**Two representations drift.** Every change to turn semantics has to be made in
both places, and nothing detects when it is made in only one. The bug fixed in
#2937 — a turn that died between the model call and the recording of its result
left state the host had to reconstruct by hand — is the shape of failure this
produces: the durable path had to re-derive knowledge that the in-process path
simply held in memory.

**Atoms own their I/O, so the machine is not the whole truth.** `ReasonAtom`
loads messages, calls the provider, emits events, and stores results;
`ActAtom` does the same around tools. What a turn "is" therefore lives partly in
the state machine, partly in whatever each atom did on the way through, and
partly in what the host persisted between steps. Reconstructing a turn means
knowing all three.

**The consequences compound at the edges.** Crash recovery, cancellation
mid-phase, sealing, and replay each need to answer "what had already happened?"
Today each answers it separately, which is why `RuntimeTurnState` accumulated
fields (`llm_call_count`, `time_to_first_token_ms`, `final_answer_preview`) that
exist to reconstruct what the in-process path never lost.

## What sans-IO means here

A transition is a pure function of the state and the result being reported:

```text
(TurnState, TransitionInput) -> (TurnState, Vec<TurnEffect>)
```

The state machine never loads, stores, or emits. It *returns* what should be
recorded, and the host performs it. Three properties follow, and they are the
point:

1. **A turn is exactly its events plus a serializable state.** Resuming is
   deserializing, not reconstructing.
2. **One implementation, two hosts.** An in-process host applies effects
   immediately; a durable host persists the state and schedules the next step.
   Neither owns semantics, so they cannot disagree about them.
3. **The loop is testable without a provider or a database.** Transition tests
   are table tests over values.

## Non-goals

- Changing turn semantics. Every stage is behavior-preserving; a stage that
  changes what a turn does is a different change.
- Removing atoms. Atoms stay as the units of work; what moves is who performs
  their side effects.
- Replacing the durable engine. Task scheduling, retries, and the DLQ are
  unaffected — the durable host keeps owning those.

## Staged migration

Each stage is independently shippable and independently revertible. A stage that
cannot be verified against current behavior does not ship.

### Stage 1 — the value (landed)

`crates/core/src/turn_state.rs`: a serializable `TurnState` with consuming
transitions and a pure `next_action`. It mirrors `TurnStateMachine`'s semantics
exactly, and a conformance test drives identical sequences through both and
asserts the same actions and outcomes.

Nothing is rewired. The deliverable is a representation both hosts *could*
share, plus the evidence that it behaves identically.

### Stage 2 — fold in the durable bookkeeping

Move `RuntimeTurnState`'s fields (iteration, call counts, cumulative usage,
`previous_response_id`, first-token timing, final-answer preview) onto
`TurnState`, so one value carries everything a resume needs. `plan_next_host_turn`
becomes a thin projection of `next_action` rather than a parallel planner.

This is where the two representations actually converge, and it is the stage
that pays for stage 1.

### Stage 3 — introduce effects

Add `TurnEffect` and have transitions return the events that must be recorded.
Both hosts apply them through one applier. Atoms still emit their own events at
this stage; the effect list is asserted against what they emit, which is how the
vocabulary is kept honest rather than invented.

### Stage 4 — move the recording out of the atoms

One effect at a time: an atom stops emitting, the transition returns the effect
instead, the applier performs it. Each move is a small PR with an unchanged
event stream as its success bar (event-sequence tests over a fixed scenario).

### Stage 5 — the durable host becomes a persisting in-process host

With no I/O left in the machine, the durable path reduces to: deserialize state,
apply one operation, serialize, schedule. The parallel planner is deleted.

## Success bars

- **Behavior-preserving.** For a fixed scenario, the emitted event sequence
  before and after each stage is identical. This is the primary bar; a stage
  that cannot demonstrate it does not ship.
- **No new representation.** The stage that adds a field to `TurnState` removes
  it from `RuntimeTurnState` in the same change. Two copies of a field is the
  failure mode this whole effort exists to remove.
- **Durable equivalence.** A test that discards `TurnState` at every step and
  rebuilds it from the serialized value must produce the same outcome as one
  that keeps it in memory (agentyk's
  `durable_host_replays_state_between_every_engine_step` is the model).
- **Revertibility.** Until stage 5, `TurnStateMachine` stays; any stage can be
  reverted without a migration.

## Alternatives considered

**Leave it.** The cost is ongoing: every turn-semantics change is made twice,
and the failures show up in the durable path under crash and cancellation —
the hardest place to notice them and the most expensive place to debug.

**Rewrite the durable path onto the in-memory machine.** Cheaper, and wrong:
the mutable machine cannot be persisted mid-turn, which is the durable host's
whole requirement.

**Adopt agentyk's engine wholesale.** Agentyk built this shape from scratch
(`knowledge/foundations/plan.md`, Phase 2) and has the design worked out, but
adopting it means adopting its `Agent`/`Session` model along with everything
core layers on top — identity, catalog, persistence, tenancy. The staged
migration takes the idea without the rebuild, and leaves the door open to
converge later.

## References

- `crates/core/src/turn.rs` — `TurnStateMachine`, `TurnPhase`, `TurnAction`, `TurnOutcome`
- `crates/core/src/turn_state.rs` — the stage-1 value
- `crates/runtime/src/turn_strategy.rs` — `RuntimeTurnState`, `plan_next_host_turn`
- `knowledge/operations/durable-execution-engine.md` — the durable host this converges with
