//! Local Framework configuration.

//! Local, SQLite-backed runtime support for embedded Everruns applications.
//!
//! Enable the `local` feature to get crash-durable canonical events, session
//! identity, task and schedule state, plus reopenable Git workspace heads.

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

use std::path::{Path, PathBuf};

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

/// Local application data and workspace locations.
///
/// Enabling this profile gives sessions a real-disk workspace, SQLite-backed
/// task/schedule state and session catalog, and a crash-durable canonical event
/// log. Conversation history remains a read-only event projection rather than
/// a second writable store. Requires the `local` feature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalConfig {
    pub(crate) data_dir: PathBuf,
    pub(crate) workspace_root: PathBuf,
}

impl LocalConfig {
    /// Store local state under `data_dir`; workspace files default to
    /// `data_dir/workspace`. Select this directory from trusted application
    /// configuration rather than model or request input.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        let workspace_root = data_dir.join("workspace");
        Self {
            data_dir,
            workspace_root,
        }
    }

    /// Override the real-disk workspace root.
    pub fn workspace(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = root.into();
        self
    }

    /// Directory containing local state.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Root exposed to the agent as `/workspace`.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub(crate) fn profile(&self) -> LocalProfile {
        LocalProfile::new(self.data_dir.clone()).with_workspace_root(self.workspace_root.clone())
    }
}
