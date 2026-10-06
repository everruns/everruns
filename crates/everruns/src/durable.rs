//! Durable execution: run an [`Engine`](crate::Engine)'s turns as queued,
//! checkpointed steps instead of in the session's own task.
//!
//! **Experimental.** Behind the `durable` feature, outside the
//! [API stability](crate::stability) promises: the backend and the seam it
//! implements may change without a major version bump until the backend
//! conformance suite passes on every backend.
//!
//! By default an engine runs each turn in process, on the task that awaits
//! it. [`Backend::memory`](crate::durable::Backend::memory) or [`Backend::postgres`](crate::durable::Backend::postgres) selects the durable
//! backend from `everruns-durable-engine` instead: every turn step (input,
//! reason, act) is a task on an `everruns-durable` queue, its state
//! checkpointed after each step, and a pool of workers in this process runs
//! the steps on the session's own runtime. A turn then makes progress whether or not anyone
//! awaits it, and its steps run the same activities an in-process turn runs,
//! so answers and the session's event sequence are the same.
//!
//! Sessions behave as they do in process: [`Session::send`](crate::Session::send)
//! while a turn runs steers it at its next reason boundary, cancellation stops
//! the turn before its next step, and the engine's workers stop when the
//! engine and its sessions are dropped.
//!
//! Turns parked on client-side tool calls (AG-UI) and turns a process exit
//! interrupted in their tool calls
//! ([`Session::resume_interrupted_turn`](crate::Session::resume_interrupted_turn))
//! continue on this backend exactly as in process; the backend conformance
//! suite runs every scenario on every backend
//! and requires the same outcome.
//!
//! # Stores
//!
//! - [`Backend::memory`](crate::durable::Backend::memory) keeps the queue in memory, for as long as the
//!   engine lives: nothing beyond the session's own log survives the process.
//! - [`Backend::postgres`](crate::durable::Backend::postgres) keeps it in PostgreSQL, which several processes may
//!   share. A step needs its session's runtime, which lives only in the
//!   engine that opened the session, so each engine claims only its own
//!   sessions' steps. After a process exits, a session opened again (in a
//!   new process, or a new engine) first ends the turn the old engine left
//!   running in the database; a turn the exit cut off in its tool calls
//!   then continues from the session log with
//!   [`Session::resume_interrupted_turn`](crate::Session::resume_interrupted_turn),
//!   exactly as in process. For that the session's log must outlive the
//!   process too.
//!
//! # Example
//!
//! ```
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns::{Agent, Engine, Model, durable};
//!
//! let engine = Engine::builder()
//!     .backend(durable::Backend::memory().workers(4))
//!     .build();
//! let agent = Agent::builder()
//!     .instructions("You are concise.")
//!     .model(Model::simulated("4"))
//!     .build()?;
//!
//! let turn = engine.create(agent).send_and_wait("What is 2 + 2?").await?;
//! assert_eq!(turn.response, "4");
//! # Ok(())
//! # }
//! ```

use std::fmt;

use everruns_durable_engine::DurableBackend;

/// The PostgreSQL durable store [`Backend::postgres`](crate::durable::Backend::postgres) runs on; create it
/// with [`PostgresWorkflowEventStore::connect`].
pub use everruns_durable_engine::durable::PostgresWorkflowEventStore;

/// The default number of in-process workers a durable backend runs.
const DEFAULT_WORKERS: usize = 4;

/// A durable execution backend for an [`Engine`](crate::Engine), selected
/// with [`EngineBuilder::backend`](crate::EngineBuilder::backend).
///
/// **Experimental**; see the [module documentation](self).
#[derive(Clone)]
pub struct Backend {
    store: Store,
    workers: usize,
}

#[derive(Clone)]
enum Store {
    Memory,
    Postgres(PostgresWorkflowEventStore),
}

impl Backend {
    /// Queue turn steps on an in-memory durable store owned by the engine.
    ///
    /// Runs four workers unless
    /// [`workers`](Self::workers) says otherwise.
    pub fn memory() -> Self {
        Self {
            store: Store::Memory,
            workers: DEFAULT_WORKERS,
        }
    }

    /// Queue turn steps on a PostgreSQL durable store, which other engines
    /// and processes may share; see [Stores](self#stores).
    ///
    /// Runs four workers unless [`workers`](Self::workers) says otherwise.
    /// Each engine built from this backend claims only the steps of its own
    /// sessions.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// use everruns::durable::{Backend, PostgresWorkflowEventStore};
    /// use everruns::{Agent, Engine, Model};
    ///
    /// // Connects and applies the durable schema, which is idempotent.
    /// let store = PostgresWorkflowEventStore::connect("postgres://localhost/my_app").await?;
    /// let engine = Engine::builder()
    ///     .backend(Backend::postgres(store).workers(8))
    ///     .build();
    /// let agent = Agent::builder()
    ///     .instructions("You are concise.")
    ///     .model(Model::simulated("4"))
    ///     .build()?;
    ///
    /// let turn = engine.create(agent).send_and_wait("What is 2 + 2?").await?;
    /// assert_eq!(turn.response, "4");
    /// # Ok(())
    /// # }
    /// ```
    pub fn postgres(store: PostgresWorkflowEventStore) -> Self {
        Self {
            store: Store::Postgres(store),
            workers: DEFAULT_WORKERS,
        }
    }

    /// Run turn steps on `workers` in-process workers (at least one).
    ///
    /// Each worker runs one step at a time, and a session runs one step at a
    /// time, so this bounds how many sessions make progress at once.
    pub fn workers(mut self, workers: usize) -> Self {
        self.workers = workers.max(1);
        self
    }

    pub(crate) fn build(&self) -> DurableBackend {
        match &self.store {
            Store::Memory => DurableBackend::memory(self.workers),
            Store::Postgres(store) => DurableBackend::postgres(store.clone(), self.workers),
        }
    }
}

impl fmt::Debug for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backend")
            .field(
                "store",
                &match self.store {
                    Store::Memory => "memory",
                    Store::Postgres(_) => "postgres",
                },
            )
            .field("workers", &self.workers)
            .finish()
    }
}
