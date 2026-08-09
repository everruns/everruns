//! Deterministic, sans-I/O turn planning for Everruns execution hosts.
//!
//! Given a serializable [`TurnState`], parsed [`ActivityOutcome`], and
//! host-resolved [`HostFacts`], its pure functions return the next [`TurnPlan`]
//! and [`TurnLifecycleEffect`]s. Framework applications use `everruns`; this
//! focused crate lets runtime, worker, durable, and custom hosts in the
//! [Everruns](https://everruns.com) ecosystem share one turn model.
//!
//! The engine performs no I/O: no stores, sockets, process execution, event
//! emission, or `Utc::now()`. Hosts resolve those facts, pass `now` in, call the
//! planner, and perform the returned effects. This makes the loop testable as
//! table tests over values, and lets an in-process host and a durable host share
//! one implementation of turn semantics.
//!
//! # Example
//!
//! ```
//! use everruns_engine::{TurnPlan, TurnState};
//!
//! fn accepts_plan(_state: &TurnState, _plan: &TurnPlan) {}
//! # let _ = accepts_plan;
//! ```

mod turn;

pub use turn::{
    ActOutcome, ActPlan, ActSchedulingFacts, ActivityOutcome, HostFacts, TurnLifecycleEffect,
    TurnPlan, TurnState, plan_after_act, plan_after_process_input, plan_after_reason,
    plan_next_turn, reason_schedules_act,
};
