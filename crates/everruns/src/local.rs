//! Local Framework configuration.

use std::path::{Path, PathBuf};

/// Local application data and workspace locations.
///
/// Enabling this profile gives sessions a real-disk workspace and
/// SQLite-backed task/schedule state. Conversation persistence is intentionally
/// not configured here: canonical events are the durable record and history is
/// a rebuildable projection. Requires the `local` feature.
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

    pub(crate) fn profile(&self) -> everruns_local::LocalProfile {
        everruns_local::LocalProfile::new(self.data_dir.clone())
            .with_workspace_root(self.workspace_root.clone())
    }
}
