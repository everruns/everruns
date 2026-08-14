//! Local plugin loading through the Framework API.

use std::fmt;
use std::path::{Path, PathBuf};

use everruns_capability::plugin_capability_id;
use everruns_core::plugins::{PluginFileSet, compile_plugin};

/// Failure while reading or compiling a local plugin directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginError {
    path: PathBuf,
    message: String,
}

impl PluginError {
    /// Plugin directory that failed to load.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "plugin {}: {}", self.path.display(), self.message)
    }
}

impl std::error::Error for PluginError {}

pub(crate) struct LoadedPlugin {
    pub capability: crate::CapabilitySpec,
    pub warnings: Vec<String>,
}

pub(crate) fn load(path: &Path) -> Result<LoadedPlugin, PluginError> {
    // THREAT[TM-PLUGIN-004] / THREAT[TM-PLUGIN-005]: the shared loader rejects
    // traversal and symlinks, enforces file/byte limits, and compiles data-only
    // content.
    let files = PluginFileSet::from_dir(path).map_err(|message| PluginError {
        path: path.to_path_buf(),
        message,
    })?;
    let compiled = compile_plugin(&files).map_err(|message| PluginError {
        path: path.to_path_buf(),
        message,
    })?;
    let capability_id = plugin_capability_id(&compiled.definition.name);
    let config = serde_json::to_value(&compiled.definition).map_err(|error| PluginError {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    Ok(LoadedPlugin {
        capability: crate::CapabilityRef::new(capability_id)
            .config(config)
            .into(),
        warnings: compiled.warnings,
    })
}
