//! Local, SQLite-backed runtime backend stores for embedded Everruns hosts.
//!
//! This crate provides restart-survivable, file-backed implementations of the
//! runtime backend traits used by an in-process host:
//!
//! - [`LocalSessionTaskRegistry`] — a [`everruns_core::session_task::SessionTaskRegistry`]
//!   over SQLite, persisting tasks and their message channel.
//! - [`LocalScheduleStore`] — a [`everruns_core::session_services::SessionScheduleStore`]
//!   over SQLite, with an additive JSON metadata bag (see its module docs).
//! - [`LocalSessionStore`] — a durable session identity and metadata catalog;
//!   conversation history remains canonical-event-derived.
//! - [`LocalGitWorkspaceProvider`] — public isolated Git-worktree heads with
//!   explicit fork, reopen, archive, and destroy lifecycle. Drop never cleans
//!   up provider-owned worktrees or branches.
//! - [`LocalScheduleRunner`] — an explicitly managed in-process executor that
//!   claims due schedules and delivers them through [`LocalSessionRunner`].
//! - [`LocalPlatformStore`] — a [`everruns_platform::PlatformStore`]
//!   implementing the subagent-critical core honestly and returning explicit
//!   unsupported errors for platform-management-only operations.
//! - [`LocalProfile`] — named local env config (data dir, workspace, base URL,
//!   org/principal identity).
//! - [`LocalBackends`] — composable construction of `HostBackends` + the
//!   local stores, accepting a caller-provided event bus (via `HostBackends`)
//!   and file system factory (via the embedder's `HostComposition`).
//! - [`LocalRuntimeBuilder`] — optional sugar over `InProcessRuntimeBuilder`.
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and pairs with
//! [`everruns-host`](https://crates.io/crates/everruns-host), which owns
//! the optional host-backend slots these stores populate.
//!
//! Framework applications normally select local behavior through the
//! application-facing `everruns` crate. Advanced hosts use these focused types
//! directly. Both are part of the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_local::LocalProfile;
//!
//! let profile = LocalProfile::new("/var/lib/everruns")
//!     .with_workspace_root("/var/lib/everruns/workspace");
//! assert_eq!(profile.data_dir(), std::path::Path::new("/var/lib/everruns"));
//! ```

mod backends;
mod db;
mod error;
mod git_workspace;
mod platform_store;
mod profile;
mod runtime_builder;
mod schedule_runner;
mod schedule_store;
mod session_store;
mod task_registry;
mod wake_routing;

pub use backends::LocalBackends;
pub use db::SqliteDb;
pub use error::{LocalError, LocalResult};
pub use git_workspace::LocalGitWorkspaceProvider;
pub use platform_store::{LocalPlatformStore, LocalSessionRunner};
pub use profile::LocalProfile;
pub use runtime_builder::{LocalRuntimeBuilder, local_capability_registry};
pub use schedule_runner::{
    LocalScheduleRunner, LocalScheduleRunnerConfig, LocalScheduleRunnerHandle,
};
pub use schedule_store::LocalScheduleStore;
pub use session_store::LocalSessionStore;
pub use task_registry::LocalSessionTaskRegistry;
pub use wake_routing::{HostRoutedRunner, WakeRoutes};
