//! Distributed execution for portable Everruns agents.
//!
//! [`ScaleEngine`] is the PostgreSQL-backed [`everruns::Engine`]
//! implementation. Unlike [`everruns::InMemoryEngine`], it never treats an
//! arbitrary facade [`everruns::Agent`] as serializable. Applications submit a
//! [`PortableAgent`] whose executable pieces are stable [`RegistrationPath`]
//! values. A process-local [`Registry`] resolves those paths only after the
//! portable value has crossed the persistence boundary.
//!
//! Product-neutral durable turn control lives here as [`DurableTurnDriver`].
//! Server DTOs and worker transports implement its narrow store/notifier
//! contracts outside this crate.
//!
//! Part of the [Everruns](https://everruns.com) ecosystem.
//!
//! ```rust
//! use everruns_scale::{PortableAgent, RegistrationPath};
//!
//! let agent = PortableAgent::builder(
//!     "Answer concisely.",
//!     "provider-model",
//!     RegistrationPath::new("providers/example/default")?,
//!     RegistrationPath::new("workspaces/example/default")?,
//! )
//! .build()?;
//! assert_eq!(agent.version, 1);
//! # Ok::<(), everruns_scale::PortableAgentError>(())
//! ```
//!
//! Process-local implementations are deliberately not accepted by the
//! portable builder:
//!
//! ```compile_fail
//! use everruns_scale::{PortableAgent, RegistrationPath};
//!
//! let local_tool = || "captured process state";
//! let definition = PortableAgent::builder(
//!     "Answer concisely.",
//!     "provider-model",
//!     RegistrationPath::new("providers/example/default")?,
//!     RegistrationPath::new("workspaces/example/default")?,
//! )
//! .tool(local_tool)
//! .build()?;
//! # Ok::<(), everruns_scale::PortableAgentError>(())
//! ```

#![deny(missing_docs)]

mod control;
mod engine;
mod migration;
mod persistence;
mod portable;

pub use control::{
    DurableStoreBackend, DurableTaskNotifier, DurableTurnDriver, DurableTurnInput,
    DurableTurnOutput, InMemoryDurableStore, PostgresDurableStore, RunController,
};
pub use engine::ScaleEngine;
pub use migration::{MIGRATION_TABLE, migrate};
pub use portable::{
    ComponentKind, PortableAgent, PortableAgentBuilder, PortableAgentError, RegistrationPath,
    Registry,
};
