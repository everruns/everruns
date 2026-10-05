//! Private entry point for running portable turns on durable workflows.
//!
//! Durable owns persistence primitives and database connections. This crate
//! owns the agent-turn conventions, checkpoint driver, and runner backends;
//! the worker composes process services through this boundary.
//!
//! Decision: this crate carries no transport. The worker's gRPC stores and
//! runner constructors live in `everruns-worker`, which implements this
//! crate's `TaskStore` and `DurableStoreBackend` for its own client type;
//! `scripts/lib/check-durable-isolation.sh` bans tonic and the internal
//! protocol here.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod durable_execution;
pub mod durable_runner;
pub mod durable_turn;
pub mod runner;
pub mod task_store;

pub use everruns_core as core;
pub use everruns_core::{engine, host};
pub use everruns_durable as durable;

pub use durable_execution::DurableExecution;
pub use durable_runner::{
    DirectDurableStore, DurableRunner, DurableStoreBackend, DurableTaskNotifier, DurableTurnInput,
    DurableTurnOutput, InMemoryDurableStore,
};
pub use runner::{AgentRunner, RunnerBackend, create_runner_with_backend};
