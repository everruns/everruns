// LocalRuntimeBuilder — optional convenience sugar over InProcessRuntimeBuilder.
//
// This wires a LocalProfile + LocalBackends (task registry + schedule store)
// and a real-disk workspace file store into an `InProcessRuntimeBuilder`. It is
// strictly optional: everything it does can be done by composing the pieces in
// `LocalBackends` directly. The platform store is intentionally NOT wired here
// because the local platform store needs a `LocalSessionRunner` that usually
// wraps the built runtime (a chicken/egg). Attach it after build via
// `LocalBackends::with_platform_runner` and rebuild, or use the standalone
// `LocalPlatformStore`.

use std::sync::Arc;

use everruns_core::error::Result;
use everruns_core::traits::{SessionFileSystemFactory, SessionFileSystemFactoryContext};
use everruns_core::{PlatformDefinition, ResolvedModel};
use everruns_runtime::{
    InProcessRuntime, InProcessRuntimeBuilder, RealDiskSessionFileSystemFactory,
};

use crate::backends::LocalBackends;
use crate::profile::LocalProfile;

/// Convenience wrapper around [`InProcessRuntimeBuilder`] that wires a local
/// profile + SQLite-backed task/schedule stores and a workspace file store.
pub struct LocalRuntimeBuilder {
    profile: LocalProfile,
    inner: InProcessRuntimeBuilder,
    file_system_factory: Option<Arc<dyn SessionFileSystemFactory>>,
}

impl LocalRuntimeBuilder {
    /// Start from a profile. Defaults the platform definition to built-ins via
    /// `InProcessRuntimeBuilder::new()`.
    pub fn new(profile: LocalProfile) -> Self {
        Self {
            profile,
            inner: InProcessRuntimeBuilder::new(),
            file_system_factory: None,
        }
    }

    /// Replace the platform definition (capabilities, drivers, ...).
    pub fn platform_definition(mut self, platform_definition: PlatformDefinition) -> Self {
        self.inner = self.inner.platform_definition(platform_definition);
        self
    }

    /// Register the built-in `llmsim` driver for deterministic local execution.
    pub fn llm_sim(mut self, config: everruns_core::llmsim_driver::LlmSimConfig) -> Self {
        self.inner = self.inner.llm_sim(config);
        self
    }

    /// Set the runtime default model.
    pub fn default_model(mut self, model: ResolvedModel) -> Self {
        self.inner = self.inner.default_model(model);
        self
    }

    /// Seed a harness.
    pub fn harness(mut self, harness: everruns_core::Harness) -> Self {
        self.inner = self.inner.harness(harness);
        self
    }

    /// Seed an agent.
    pub fn agent(mut self, agent: everruns_core::Agent) -> Self {
        self.inner = self.inner.agent(agent);
        self
    }

    /// Seed a session.
    pub fn session(mut self, session: everruns_core::Session) -> Self {
        self.inner = self.inner.session(session);
        self
    }

    /// Override the session filesystem factory. By default a real-disk factory
    /// rooted at `profile.workspace_root` is used.
    pub fn session_file_system_factory(
        mut self,
        factory: Arc<dyn SessionFileSystemFactory>,
    ) -> Self {
        self.file_system_factory = Some(factory);
        self
    }

    /// Access the underlying `InProcessRuntimeBuilder` for advanced wiring.
    pub fn inner_mut(&mut self) -> &mut InProcessRuntimeBuilder {
        &mut self.inner
    }

    /// Build the runtime and return it along with the constructed
    /// [`LocalBackends`] so the embedder can attach a platform runner and reuse
    /// the local stores. The task registry and schedule store factory are
    /// installed on the runtime; the platform store is left to the embedder.
    pub async fn build(self) -> Result<(InProcessRuntime, LocalBackends)> {
        self.profile
            .ensure_dirs()
            .map_err(|e| everruns_core::AgentLoopError::config(e.to_string()))?;

        let local = LocalBackends::new(
            self.profile.clone(),
            everruns_runtime::RuntimeBackends::in_memory(),
        )?;

        // Default to a real-disk workspace rooted at the profile workspace.
        let factory = self.file_system_factory.unwrap_or_else(|| {
            Arc::new(RealDiskSessionFileSystemFactory::new(
                self.profile.workspace_root.clone(),
            ))
        });
        let platform_definition = PlatformDefinition::builder()
            .capability_registry(everruns_core::CapabilityRegistry::with_builtins())
            .driver_registry(everruns_core::DriverRegistry::new())
            .session_file_system_factory(factory)
            .build();

        let runtime = self
            .inner
            .platform_definition(platform_definition)
            .session_file_system_factory_context(SessionFileSystemFactoryContext::new())
            .backends(local.runtime_backends.clone())
            .build()
            .await?;

        Ok((runtime, local))
    }
}
