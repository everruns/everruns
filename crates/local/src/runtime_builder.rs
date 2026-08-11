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
use everruns_host::{InProcessRuntime, InProcessRuntimeBuilder, RealDiskSessionFileSystemFactory};

use crate::backends::LocalBackends;
use crate::profile::LocalProfile;

/// Convenience wrapper around [`InProcessRuntimeBuilder`] that wires a local
/// profile + SQLite-backed task/schedule stores and a workspace file store.
pub struct LocalRuntimeBuilder {
    profile: LocalProfile,
    inner: InProcessRuntimeBuilder,
    file_system_factory: Option<Arc<dyn SessionFileSystemFactory>>,
    /// Caller-supplied platform definition override. When `Some`, `build()`
    /// installs it as-is instead of the default local one; the caller owns the
    /// session filesystem factory in that case.
    platform_definition: Option<PlatformDefinition>,
}

impl LocalRuntimeBuilder {
    /// Start from a profile. Defaults the platform definition to built-ins via
    /// `InProcessRuntimeBuilder::new()`.
    pub fn new(profile: LocalProfile) -> Self {
        Self {
            profile,
            inner: InProcessRuntimeBuilder::new(),
            file_system_factory: None,
            platform_definition: None,
        }
    }

    /// Replace the platform definition (capabilities, drivers, ...). When set,
    /// `build()` respects this definition instead of constructing the default
    /// local one, so capability/driver overrides take effect. The caller is
    /// then responsible for the session filesystem factory on their definition.
    pub fn platform_definition(mut self, platform_definition: PlatformDefinition) -> Self {
        self.platform_definition = Some(platform_definition);
        self
    }

    /// Register `provider` (replacing any same-name registration) and default
    /// the runtime model to `model_id` on it when nothing else set one.
    ///
    /// Deterministic local runs pass the `llmsim` provider built by
    /// `everruns_test_support::llm_sim_provider(...)` here; the simulator no
    /// longer ships with the production crates (EVE-875).
    pub fn provider_with_default_model(
        mut self,
        provider: everruns_core::Provider,
        model_id: impl Into<String>,
    ) -> Self {
        self.inner = self.inner.provider_with_default_model(provider, model_id);
        self
    }

    /// Set the runtime default model.
    pub fn default_model(mut self, model: ResolvedModel) -> Self {
        self.inner = self.inner.default_model(model);
        self
    }

    /// Seed a harness definition (under its embedder-chosen id).
    pub fn harness(mut self, harness: everruns_host::SeededHarness) -> Self {
        self.inner = self.inner.harness(harness);
        self
    }

    /// Seed an agent.
    pub fn agent(mut self, agent: everruns_core::AgentDefinition) -> Self {
        self.inner = self.inner.agent(agent);
        self
    }

    /// Seed a session.
    pub fn session(mut self, session: everruns_core::ExecutionSession) -> Self {
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
            everruns_host::HostBackends::in_memory(),
        )?;

        // Respect a caller-supplied platform definition; otherwise build the
        // default local one rooted at the profile workspace. Only install the
        // default when the caller has not overridden it, so capability/driver
        // overrides via `platform_definition(...)` are not silently discarded.
        let platform_definition = match self.platform_definition {
            Some(pd) => pd,
            None => {
                let factory = self.file_system_factory.unwrap_or_else(|| {
                    Arc::new(RealDiskSessionFileSystemFactory::new(
                        self.profile.workspace_root.clone(),
                    ))
                });
                PlatformDefinition::builder()
                    .capability_registry(everruns_core::CapabilityRegistry::with_builtins())
                    .driver_registry(everruns_core::DriverRegistry::new())
                    .session_file_system_factory(factory)
                    .build()
            }
        };

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
