//! Durable turn backend for the [Everruns](https://everruns.com) agent
//! framework: agent turns as queued, checkpointed steps on
//! [`everruns-durable`](everruns_durable), behind core's
//! [`TurnBackend`](everruns_core::host::TurnBackend) turn entry point.
//!
//! **Status: experimental.** `TurnBackend` is outside the Everruns API
//! stability promises until the backend conformance suite passes on every
//! backend; expect breaking changes between releases.
//!
//! Each turn step (`process_input`, `reason`, `act`) is a task on the durable
//! queue, and the turn's state is checkpointed after every step, so a turn a
//! crashed process held is reclaimed and continues from its last step.
//!
//! - [`DurableRunner`] implements [`TurnBackend`](everruns_core::host::TurnBackend):
//!   it starts, cancels and observes a session's turns. Starting a turn creates
//!   or claims the session's workflow and enqueues its first step before it
//!   returns; the ticket resolves when the workflow ends. It holds no session
//!   runtime, so it starts turns only from input the caller stored
//!   (`TurnInput::StoredMessage`, `TurnInput::RecordedToolResults`); the
//!   platform server persists every input and then calls it.
//! - [`DurableBackend`] runs a framework application's turns: in-process
//!   workers drive each step on the session's own `InProcessRuntime`, over an
//!   in-memory store ([`DurableBackend::memory`]) or a PostgreSQL one
//!   ([`DurableBackend::postgres`]) that several processes may share, each
//!   claiming only its own sessions' steps. [`DurableBackend::attach`] gives a
//!   session its [`TurnBackend`](everruns_core::host::TurnBackend). The
//!   `everruns` facade selects it with its `durable` feature.
//! - [`TurnTaskDriver`] runs one claimed turn task against any
//!   [`TurnStore`]; a [`TurnTaskHost`] supplies the runtime host each step
//!   runs on.
//! - [`TurnStore`] is the one store contract the runner and the driver share.
//!   Any `everruns-durable` `WorkflowEventStore` is a `TurnStore`: the
//!   in-memory store serves tests and development, and the PostgreSQL store
//!   runs on `everruns-durable`'s own schema.
//!
//! The crate carries no transport. The platform worker's gRPC
//! stores and runner constructors live in `everruns-worker`, which implements
//! [`TurnStore`] for its own
//! client type and builds its runner with [`DurableRunner::from_store`].
//! Another process boundary plugs in the same way. The platform server calls
//! [`DurableRunner`]'s `TurnBackend` directly; it has no other turn entry
//! point.
//!
//! # Example
//!
//! A turn started from a message the host already stored, then cancelled,
//! on the in-memory store. With nothing driving the queue the turn waits at
//! its first step, so cancelling it ends the ticket with `Cancelled`.
//!
//! ```
//! use everruns_contracts::error::AgentLoopError;
//! use everruns_contracts::typed_id::{HarnessId, MessageId, SessionId, TurnId};
//! use everruns_durable_engine::DurableRunner;
//! use everruns_durable_engine::host::{TurnBackend, TurnInput, TurnRequest, TurnScope};
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> everruns_contracts::error::Result<()> {
//! let runner = DurableRunner::new_in_memory();
//! let session_id = SessionId::new();
//!
//! let ticket = runner
//!     .start_turn(
//!         TurnRequest::new(
//!             session_id,
//!             TurnId::new(),
//!             TurnInput::StoredMessage {
//!                 message_id: MessageId::new(),
//!             },
//!         )
//!         // The runner cannot look the session up, so the request names it.
//!         .with_scope(TurnScope::new(1, HarnessId::new(), None)),
//!     )
//!     .await?;
//! assert!(runner.is_running(session_id).await);
//!
//! assert!(runner.cancel(session_id).await?);
//! assert!(matches!(ticket.await, Err(AgentLoopError::Cancelled)));
//! # Ok(())
//! # }
//! ```
//!
//! On PostgreSQL, apply `everruns-durable`'s schema and build the runner over
//! the pool. Turns advance while a worker loop claims their tasks and hands
//! each to a [`TurnTaskDriver`].
//!
//! ```no_run
//! use everruns_durable_engine::DurableRunner;
//! use everruns_durable_engine::durable::{PostgresPool, PostgresWorkflowEventStore};
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = PostgresPool::connect("postgres://localhost/my_app").await?;
//! PostgresWorkflowEventStore::migrate(&pool).await?;
//! let runner = DurableRunner::new_with_pool(pool);
//! # let _ = runner;
//! # Ok(())
//! # }
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

// Decision: no transport here. Worker gRPC stores live in `everruns-worker`;
// `scripts/lib/check-durable-isolation.sh` bans tonic and the internal
// protocol in this crate's normal and build edges.

// The README's examples compile and run as doctests without rendering the
// README twice in the API docs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

mod backend_store;
pub mod durable_backend;
#[cfg(test)]
mod durable_backend_postgres_tests;
#[cfg(test)]
mod durable_backend_tests;
pub mod durable_runner;
pub mod durable_turn;
pub mod task_error;
pub mod task_heartbeat;
#[cfg(test)]
mod task_heartbeat_tests;
pub mod turn_backend;
#[cfg(test)]
mod turn_backend_tests;
pub mod turn_driver;
#[cfg(test)]
mod turn_driver_tests;
mod turn_start;
pub mod turn_store;

pub use everruns_core as core;
pub use everruns_core::{engine, host};
pub use everruns_durable as durable;

pub use durable_backend::{DurableBackend, DurableSessionBackend};
pub use durable_runner::{DurableRunner, DurableTaskNotifier, DurableTurnInput, DurableTurnOutput};
pub use turn_driver::{TurnTaskDriver, TurnTaskHost};
pub use turn_store::{TurnStore, WorkflowEndSignal, WorkflowSnapshot};
