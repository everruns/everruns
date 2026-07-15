//! Local, SQLite-backed runtime backend stores for embedded Everruns hosts.
//!
//! This crate provides restart-survivable, file-backed implementations of the
//! runtime backend traits used by an in-process host:
//!
//! - [`LocalSessionTaskRegistry`] — a [`everruns_core::session_task::SessionTaskRegistry`]
//!   over SQLite, persisting tasks and their message channel.
//! - [`LocalScheduleStore`] — a [`everruns_core::traits::SessionScheduleStore`]
//!   over SQLite, with an additive JSON metadata bag (see its module docs).
//! - [`LocalScheduleRunner`] — an explicitly managed in-process executor that
//!   claims due schedules and delivers them through [`LocalSessionRunner`].
//! - [`LocalPlatformStore`] — a [`everruns_core::platform_store::PlatformStore`]
//!   implementing the subagent-critical core honestly and returning explicit
//!   unsupported errors for platform-management-only operations.
//! - [`LocalProfile`] — named local env config (data dir, workspace, base URL,
//!   org/principal identity).
//! - [`LocalBackends`] — composable construction of `RuntimeBackends` + the
//!   local stores, accepting a caller-provided event bus (via `RuntimeBackends`)
//!   and file system factory (via the embedder's `PlatformDefinition`).
//! - [`LocalRuntimeBuilder`] — optional sugar over `InProcessRuntimeBuilder`.
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and pairs with
//! [`everruns-runtime`](https://crates.io/crates/everruns-runtime), which owns
//! the optional host-backend slots these stores populate.

mod backends;
mod db;
mod error;
mod platform_store;
mod profile;
mod runtime_builder;
mod schedule_runner;
mod schedule_store;
mod task_registry;

pub use backends::LocalBackends;
pub use db::SqliteDb;
pub use error::{LocalError, LocalResult};
pub use platform_store::{LocalPlatformStore, LocalSessionRunner};
pub use profile::LocalProfile;
pub use runtime_builder::LocalRuntimeBuilder;
pub use schedule_runner::{
    LocalScheduleRunner, LocalScheduleRunnerConfig, LocalScheduleRunnerHandle,
};
pub use schedule_store::LocalScheduleStore;
pub use task_registry::LocalSessionTaskRegistry;
