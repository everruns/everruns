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

The turn loop was historically implemented twice: a mutable in-memory machine
and a serializable planner driven by the durable worker. They encoded the same
phases and transitions in different shapes, so semantics could drift.

Both real hosts now advance the engine-owned `TurnExecution`: the durable path
through `DurableExecution`, and the in-process runtime through
`InProcessExecution`. The former core/test-support state machines and the
stateless host compatibility planner have been removed.

This spec converges them on one **sans-IO** representation: a
serializable `TurnState` whose transitions are pure functions, with all I/O,
loading messages, calling the model, running tools, emitting events, persisting
, performed by portable engine phases through contracts injected by the host.
The intended end state is that a durable host is an in-process host that
persists between steps, rather than a second implementation of the same loop.

Status: the value, planner, stateful execution contract, immediate and durable
drivers, and shared Input/Reason/Act kernel have landed. Planning remains pure;
phase effects cross neutral `everruns-core`/`everruns-provider` contracts.

## Motivation

**Two representations drift.** Every change to turn semantics has to be made in
both places, and nothing detects when it is made in only one. The bug fixed in
#2937, a turn that died between the model call and the recording of its result
left state the host had to reconstruct by hand, is the shape of failure this
produces: the durable path had to re-derive knowledge that the in-process path
simply held in memory.

**Historically, atoms owned their I/O, so the machine was not the whole truth.** `ReasonAtom`
loads messages, calls the provider, emits events, and stores results;
`ActAtom` does the same around tools. What a turn "is" therefore lives partly in
the state machine, partly in whatever each atom did on the way through, and
partly in what the host persisted between steps. Reconstructing a turn means
knowing all three.

**The consequences compound at the edges.** Crash recovery, cancellation
mid-phase, sealing, and replay each need to answer "what had already happened?"
Historically each answered it separately, which is why the host turn state accumulated
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
- Renaming the durable activity protocol. Input, Reason, and Act remain the
  serialized units of work, but they are concrete engine phases rather than a
  generic public `Atom` polymorphism.
- Replacing the durable engine. Task scheduling, retries, and the DLQ are
  unaffected, the durable host keeps owning those.

## Staged migration

Each stage is independently shippable and independently revertible. A stage that
cannot be verified against current behavior does not ship.

### Stage 1, the value (landed)

The migration began with a temporary serializable value in core and a
conformance test against the former mutable machine. That scaffolding was
removed once `everruns-engine::TurnExecution` became the only representation.

Nothing is rewired. The deliverable is a representation both hosts *could*
share, plus the evidence that it behaves identically.

### Stage 1b, one planner, two hosts (landed)

`crates/engine`: the planner moved out of the runtime into `everruns-engine`
(EVE-840), then `InProcessRuntime::run_turn` was rewired onto it (EVE-842). The
in-process loop no longer decides reason-vs-act-vs-complete; it executes the host
operation each `TurnPlan` names and performs the returned `TurnLifecycleEffect`s.
`crates/host/tests/engine_planned_turn_test.rs` carries the behavior-preserving
evidence and the restart-between-steps property.

### Stage 1c, one execution kernel, two hosts (landed)

The concrete Input, Reason, and Act algorithms, phase I/O values, post-act
helpers, scheduler, and infrastructure hooks live beside the planner in
`everruns-engine`. `everruns-core` retains the neutral `ExecutionContext`,
per-tool hook contracts, and injected service traits. Both in-process and
durable paths call the same host composition over these engine executors; core
has no atom implementation or compatibility module.

### Stage 1d, one execution machine, two drivers (landed)

`everruns-engine::TurnExecution` owns state advancement as well as planning.
`everruns-host::InProcessExecution` retains it for the turn lifetime;
`everruns-durable::DurableExecution` checkpoints the same state between
activities. A cross-driver conformance test feeds identical outcomes into both
implementations and compares the resulting engine state.

### Stage 2, fold in the durable bookkeeping (landed)

`TurnState` owns iteration, call counts, cumulative usage,
`previous_response_id`, first-token timing, and the final-answer preview in
one value that carries everything a resume needs. Hosts use the engine type
directly; no prefixed alias remains.

This is where the two representations actually converge, and it is the stage
that pays for stage 1.

### Stage 3, introduce effects (landed)

`TurnLifecycleEffect` carries transition-owned lifecycle recording. Both hosts
apply it through one host applier. Engine phases still emit phase-local events
through the injected `EventEmitter`.

### Stage 4, host-applied phase recording (landed)

Phase-local canonical events are expressed through `PhaseEffectSink` and
applied by the injected host emitter. They are applied immediately because
streaming deltas and tool progress are observable while a phase is still
running; buffering them in the post-phase transition would break streaming.
Post-phase lifecycle changes remain ordered `TurnLifecycleEffect` values on the
transition. Cross-driver tests compare plans, checkpoints, and lifecycle-event
order while restoring the durable driver after every phase.

### Stage 5, the durable host becomes a persisting in-process host (landed)

The durable path restores `DurableExecution`, applies one engine transition,
checkpoints its `TurnState`, and schedules the returned plan. The immediate
path retains `InProcessExecution` and applies the same transition directly.

## Success bars

- **Behavior-preserving.** For a fixed scenario, the emitted event sequence
  before and after each stage is identical. This is the primary bar; a stage
  that cannot demonstrate it does not ship.
- **No new representation.** `everruns-engine::TurnState` is the only turn
  state. Two copies of a field is the failure mode this work prevents.
- **Durable equivalence.** A test that discards `TurnState` at every step and
  rebuilds it from the serialized value must produce the same outcome as one
  that keeps it in memory (agentyk's
  `durable_host_replays_state_between_every_engine_step` is the model).
- **Revertibility.** Execution checkpoints remain the existing serialized
  `TurnState`; no database migration is required by the driver split.

## Alternatives considered

**Leave it.** The cost is ongoing: every turn-semantics change is made twice,
and the failures show up in the durable path under crash and cancellation,
the hardest place to notice them and the most expensive place to debug.

**Rewrite the durable path onto the in-memory machine.** Cheaper, and wrong:
the mutable machine cannot be persisted mid-turn, which is the durable host's
whole requirement.

**Adopt agentyk's engine wholesale.** Agentyk built this shape from scratch
(`knowledge/foundations/plan.md`, Phase 2) and has the design worked out, but
adopting it means adopting its `Agent`/`Session` model along with everything
core layers on top, identity, catalog, persistence, tenancy. The staged
migration takes the idea without the rebuild, and leaves the door open to
converge later.

## References

- `crates/core/src/turn.rs`, the shared provider-neutral stop reason only
- `crates/engine/src/machine.rs`, the shared `Execution` contract and serializable
  `TurnExecution` state machine
- `crates/engine/src/turn.rs`, the pure, sans-IO turn planner (`TurnState`, `TurnPlan`,
  `plan_next_turn`, `TurnLifecycleEffect`), extracted from the runtime in EVE-840
- `crates/engine/src/execution/`, the shared Input/Reason/Act algorithms and
  engine-owned phase I/O values
- `crates/engine/src/phase_effects.rs`, live host-applied phase effects
- `crates/core/src/execution_context.rs` and `crates/core/src/tool_hooks.rs`,
  neutral contracts used by the engine and capability authors
- `crates/host/src/turn_strategy.rs`, `advance_host_execution`, the runtime
  host's thin I/O wrapper over an explicit engine driver, plus host-fact
  resolvers and the lifecycle-effect applier both drivers share
- `crates/host/src/runtime.rs`, `InProcessRuntime::run_turn`, the engine-planned
  in-process loop (EVE-842)
- `knowledge/operations/durable-execution-engine.md`, the durable host this converges with
