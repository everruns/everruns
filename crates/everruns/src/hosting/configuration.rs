//! Authority-owned mutable client configuration, separate from retained code.
#[cfg(not(target_arch = "wasm32"))]
use super::*;
#[cfg(not(target_arch = "wasm32"))]
use everruns_contracts::error::AgentLoopError;

/// Independent native workspace directories, reopened by stable session identity.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct DirectoryEnvironmentFactory {
    root: std::path::PathBuf,
}
#[cfg(not(target_arch = "wasm32"))]
impl DirectoryEnvironmentFactory {
    /// The host must persist this root separately from its session authority.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
}
#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl EnvironmentFactory for DirectoryEnvironmentFactory {
    async fn open(&self, session: SessionId) -> Result<crate::Environment> {
        crate::default_workspace::DefaultWorkspace::directory(self.root.join(session.to_string()))
            .environment(session)
            .await
            .map_err(|error| AgentLoopError::config(error.to_string()))
    }
}
