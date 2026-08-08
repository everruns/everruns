//! The turn engine: the sans-IO turn planner for the Everruns agentic runtime.
//!
//! `everruns-engine` owns the authoritative, deterministic turn-planning brain
//! for the Sans-IO Turn State epic
//! (`knowledge/foundations/sans-io-turn-state.md`). Given a serializable
//! [`TurnState`], a parsed [`ActivityOutcome`], and any host-resolved
//! [`HostFacts`], its pure functions return the next [`TurnPlan`] together with
//! the [`TurnLifecycleEffect`]s the host must perform.
//!
//! The engine performs no I/O: no stores, sockets, process execution, event
//! emission, or `Utc::now()`. Hosts resolve those facts, pass `now` in, call the
//! planner, and perform the returned effects. This makes the loop testable as
//! table tests over values, and lets an in-process host and a durable host share
//! one implementation of turn semantics.
//!
//! The dependency direction is strictly `engine -> core`; the engine never
//! depends on runtime/server/worker/platform/durable.

mod turn;

pub use turn::{
    ActOutcome, ActPlan, ActSchedulingFacts, ActivityOutcome, HostFacts, TurnLifecycleEffect,
    TurnPlan, TurnState, plan_after_act, plan_after_process_input, plan_after_reason,
    plan_next_turn, reason_schedules_act,
};
