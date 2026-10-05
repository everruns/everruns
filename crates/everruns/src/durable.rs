//! Durable execution: run an [`Engine`](crate::Engine)'s turns as queued,
//! checkpointed steps instead of in the session's own task.
//!
//! **Experimental.** Behind the `durable` feature, outside the
//! [API stability](crate::stability) promises: the backend and the seam it
//! implements may change without a major version bump until the backend
//! conformance suite passes on every backend.
//!
//! By default an engine runs each turn in process, on the task that awaits
//! it. [`Backend::memory`] selects the durable backend from
//! `everruns-durable-engine` instead: every turn step (input, reason, act) is
//! a task on an `everruns-durable` queue, its state checkpointed after each
//! step, and a pool of workers in this process runs the steps on the
//! session's own runtime. A turn then makes progress whether or not anyone
//! awaits it, and its steps run the same activities an in-process turn runs,
//! so answers and the session's event sequence are the same.
//!
//! Sessions behave as they do in process: [`Session::send`](crate::Session::send)
//! while a turn runs steers it at its next reason boundary, cancellation stops
//! the turn before its next step, and the engine's workers stop when the
//! engine and its sessions are dropped.
//!
//! Not yet served on this backend; each fails the turn with a configuration
//! error: resuming a turn a process exit interrupted
//! ([`Session::resume_interrupted_turn`](crate::Session::resume_interrupted_turn)) and
//! continuing a turn parked on client-side tool results (AG-UI). The memory
//! store lives as long as the engine, so nothing survives the process yet; a
//! PostgreSQL store is planned.
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

/// The default number of in-process workers a durable backend runs.
const DEFAULT_WORKERS: usize = 4;

/// A durable execution backend for an [`Engine`](crate::Engine), selected
/// with [`EngineBuilder::backend`](crate::EngineBuilder::backend).
///
/// **Experimental**; see the [module documentation](self).
#[derive(Clone)]
pub struct Backend {
    workers: usize,
}

impl Backend {
    /// Queue turn steps on an in-memory durable store owned by the engine.
    ///
    /// Runs four workers unless
    /// [`workers`](Self::workers) says otherwise.
    pub fn memory() -> Self {
        Self {
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
        DurableBackend::memory(self.workers)
    }
}

impl fmt::Debug for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Backend")
            .field("store", &"memory")
            .field("workers", &self.workers)
            .finish()
    }
}
