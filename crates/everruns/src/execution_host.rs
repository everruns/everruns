//! Experimental composition for hosts that own durable session activation.

use crate::{Agent, SessionId};
use everruns_core::host::{HostBackends, InProcessRuntime};

impl Agent {
    /// Assemble this agent's execution host for an external session driver.
    ///
    /// Experimental host SPI. A host supplies coherent session stores and
    /// drives the returned runtime through core's session actor driver. Agent
    /// tools, provider configuration and tool policies use the normal Framework
    /// composition. The host owns activation and outer session lifecycle hooks;
    /// use Engine sessions for the application-owned lifecycle.
    pub async fn execution_host(
        &self,
        session_id: SessionId,
        backends: HostBackends,
    ) -> everruns_contracts::error::Result<InProcessRuntime> {
        let hooks = crate::hooks::HookRunState::new(self.lifecycle_hooks());
        self.build_runtime_with_backends(backends, session_id, None, None, None, Some(hooks))
            .await
    }
}
