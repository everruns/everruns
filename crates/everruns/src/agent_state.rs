//! Per-agent runtime state shared across clones of an [`Agent`]: the workspace
//! backends it has bound, and the engine backends it owns when built with
//! [`AgentBuilder::backends`].

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use everruns_host::{HostBackends, WorkspaceBackend, WorkspaceBackendId};
use tokio::sync::OnceCell;

use crate::agent::{Agent, AgentBuilder};
use crate::engine::EngineBackends;

pub(crate) struct AgentState {
    workspace_backends: Mutex<HashMap<WorkspaceBackendId, Arc<dyn WorkspaceBackend>>>,
    /// Engine backends for an agent built with [`AgentBuilder::backends`]: owned by the agent
    /// (shared across its clones) instead of the engine-wide in-memory cell.
    pub(crate) custom_backends: Arc<OnceCell<Arc<EngineBackends>>>,
}

impl AgentState {
    pub(crate) fn new(
        workspace_backends: HashMap<WorkspaceBackendId, Arc<dyn WorkspaceBackend>>,
    ) -> Self {
        Self {
            workspace_backends: Mutex::new(workspace_backends),
            custom_backends: Arc::new(OnceCell::new()),
        }
    }

    pub(crate) fn remember_backend(&self, backend: Arc<dyn WorkspaceBackend>) -> bool {
        let mut backends = self
            .workspace_backends
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let id = backend.id();
        match backends.get(&id) {
            Some(existing) => Arc::ptr_eq(existing, &backend),
            None => {
                backends.insert(id, backend);
                true
            }
        }
    }

    pub(crate) fn backend(&self, id: &WorkspaceBackendId) -> Option<Arc<dyn WorkspaceBackend>> {
        self.workspace_backends
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(id)
            .cloned()
    }
}

impl fmt::Debug for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentState")
            .field(
                "workspace_backend_count",
                &self
                    .workspace_backends
                    .lock()
                    .map_or(0, |backends| backends.len()),
            )
            .finish()
    }
}

impl Agent {
    /// Host backends supplied through [`AgentBuilder::backends`], if any.
    pub(crate) fn custom_backends(&self) -> Option<&HostBackends> {
        self.backends.as_ref()
    }

    /// Per-agent cell for the engine backends derived from [`Self::custom_backends`].
    pub(crate) fn custom_backend_cell(&self) -> Arc<OnceCell<Arc<EngineBackends>>> {
        self.state.custom_backends.clone()
    }
}

impl AgentBuilder {
    /// Use caller-provided host backends (event log, session catalog, compaction
    /// checkpoints, …) instead of the in-memory defaults.
    ///
    /// Lets an embedding host persist canonical events in its own store (for
    /// example Postgres) while keeping the facade's loop and lifecycle hooks.
    /// The host log stays the sole write path and sequence owner.
    ///
    /// Mutually exclusive with [`local`](Self::local): [`build`](Self::build)
    /// returns [`BuildError::ConflictingBackends`](crate::BuildError::ConflictingBackends)
    /// when both are set.
    pub fn backends(mut self, backends: HostBackends) -> Self {
        self.backends = Some(backends);
        self
    }
}
