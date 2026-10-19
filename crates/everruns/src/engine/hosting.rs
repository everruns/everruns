//! Engine-owned authority and retained behavior reconstruction.
use super::*;
use crate::hosting::{
    AgentBuild, SESSION_RUNTIME_VERSION, SessionBinding, SessionCompatibility, SessionHosting,
};
use everruns_contracts::error::{AgentLoopError, Result};

#[derive(Clone)]
pub(super) struct Registration {
    pub agent: Agent,
    pub harness: Option<Harness>,
    pub environment: Option<(String, Arc<dyn crate::hosting::EnvironmentFactory>)>,
}

impl EngineBuilder {
    /// Select the coherent session authority, admission and activation profile.
    pub fn hosting(mut self, profile: impl SessionHosting) -> Self {
        self.volatile_default = false;
        self.hosting = Some(Arc::new(profile));
        self
    }

    /// Retain executable behavior under an immutable, secret-free build key.
    /// Registration is configuration: changing behavior requires a new key.
    pub fn register(mut self, build: AgentBuild, agent: Agent) -> Self {
        // THREAT[TM-DURABLE-012]: retained identities name trusted host code;
        // an existing key cannot be rebound to a different executable Agent.
        assert!(
            !self.registry.contains_key(&build),
            "duplicate agent build registration"
        );
        self.registry.insert(
            build,
            Registration {
                agent,
                harness: None,
                environment: None,
            },
        );
        self
    }

    /// Register behavior together with its immutable Harness definition.
    pub fn register_with_harness(
        mut self,
        build: AgentBuild,
        agent: Agent,
        harness: Harness,
    ) -> Self {
        assert!(
            !self.registry.contains_key(&build),
            "duplicate agent build registration"
        );
        self.registry.insert(
            build,
            Registration {
                agent,
                harness: Some(harness),
                environment: None,
            },
        );
        self
    }
}

impl Engine {
    /// Engine-held live controls of a hosted session, synchronized with the
    /// authority's retained model and effort selections (EVE-1297).
    pub(crate) async fn hosted_live_controls(
        &self,
        session: SessionId,
    ) -> Result<Arc<crate::session::LiveControls>> {
        let snapshot = self.profile()?.query_snapshot(session).await?;
        let live = self.live_controls(session);
        live.sync_authority(&snapshot.configuration);
        Ok(live)
    }

    pub(crate) async fn query_runtime_hosted(
        &self,
        session: SessionId,
    ) -> Result<crate::session::HostedQuery> {
        let snapshot = self.profile()?.query_snapshot(session).await?;
        let live = self.live_controls(session);
        live.sync_authority(&snapshot.configuration);
        self.cache_hosted(session, snapshot.binding.clone())?;
        let registration = self.registration(&snapshot.binding.behavior)?;
        let environment = if snapshot.binding.environment.is_some() {
            self.hosted_environment(session).await?
        } else {
            registration
                .agent
                .default_session_environment(session)
                .await
                .map_err(|error| AgentLoopError::config(error.to_string()))?
        };
        let mut backends = self.profile()?.reader(session);
        backends.event_log = Arc::new(crate::hosting::query::InputFilterLog::new(
            backends.event_log,
            snapshot.rejected_inputs.into_iter().collect(),
        ));
        backends.event_log = Arc::new(crate::hosting::query::SnapshotLog::new(
            backends.event_log,
            session,
            snapshot.high_watermark,
        ));
        let metadata = HostBackends::in_memory().session_store;
        backends.session_store = metadata.clone();
        let runtime = registration
            .agent
            .build_context_runtime(
                backends,
                session,
                environment,
                registration.harness.as_ref(),
                Some(live.as_ref()),
            )
            .await?;
        for capability in self.registered_capabilities() {
            if let Err(error) = crate::session::register_engine_capability(&runtime, &capability) {
                tracing::warn!(%error, "Engine capability not registered on query runtime");
            }
        }
        if let Some(mut existing) = snapshot.execution {
            let seeded = metadata
                .get_session(session)
                .await?
                .ok_or_else(|| AgentLoopError::store("query metadata was not initialized"))?;
            // These are query-local association ids for the pinned definitions;
            // mutable settings remain from the captured authority record.
            existing.harness_id = seeded.harness_id;
            existing.agent_id = seeded.agent_id;
            metadata.add_session(existing).await?;
        }
        let mut record = metadata
            .get_session(session)
            .await?
            .ok_or_else(|| AgentLoopError::store("query metadata missing"))?;
        snapshot.configuration.apply(&mut record);
        // The session layer is the overlay only: Agent capabilities resolve
        // from the agent layer, and a copy here would look removable.
        let agent_ids = registration.agent.capability_ids();
        record
            .capabilities
            .retain(|capability| !agent_ids.contains(capability.capability_id()));
        record.mcp_servers = {
            let mut servers = record.mcp_servers;
            live.apply_mcp(&mut servers);
            servers
        };
        metadata.add_session(record).await?;
        Ok(crate::session::HostedQuery {
            runtime,
            store: metadata,
            agent: registration.agent,
        })
    }

    pub(crate) async fn inspect_hosted(&self, session: SessionId) -> Result<crate::SessionContext> {
        let query = self.query_runtime_hosted(session).await?;
        let commands = query.runtime.list_commands(session).await?;
        Ok(crate::SessionContext::from_runtime(
            query.runtime.load_context(session).await?,
            query.agent.plugin_warnings(),
            commands,
        ))
    }

    pub(crate) async fn interrupted_hosted(
        &self,
        session: SessionId,
    ) -> Result<
        Option<(
            crate::InterruptedTurn,
            everruns_contracts::typed_id::MessageId,
        )>,
    > {
        let status = self.session_status(session).await?;
        if status.executing || !status.waiting.is_empty() || !status.runnable {
            return Ok(None);
        }
        let runtime = self.query_runtime_hosted(session).await?.runtime;
        Ok(runtime.interrupted_tool_calls(session).await?.map(|turn| {
            (
                crate::InterruptedTurn {
                    turn_id: turn.turn_id.to_string(),
                    tool_calls: turn.tool_calls,
                    not_rerun: turn.not_rerun,
                },
                turn.input_message_id,
            )
        }))
    }

    /// Whether creation and admission use a coherent session authority.
    pub fn has_session_hosting(&self) -> bool {
        self.inner.hosting.is_some()
    }

    /// Add an immutable retained build at host startup. Existing keys cannot
    /// be replaced; newer behavior requires a new build identity.
    pub fn register_agent(&self, build: AgentBuild, agent: Agent) -> Result<()> {
        self.register_agent_with_harness(build, agent, None)
    }

    /// Add a retained build with its immutable Harness at host startup.
    pub fn register_agent_with_harness(
        &self,
        build: AgentBuild,
        agent: Agent,
        harness: Option<Harness>,
    ) -> Result<()> {
        let mut registry = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if registry.contains_key(&build) {
            return Err(AgentLoopError::config("agent build is already registered"));
        }
        registry.insert(
            build,
            Registration {
                agent,
                harness,
                environment: None,
            },
        );
        Ok(())
    }
    /// Whether this execution host retains the exact executable registration.
    pub fn has_agent_build(&self, build: &AgentBuild) -> bool {
        self.registration(build).is_ok()
    }

    /// Register a retained build with explicit host services. The reference
    /// pins the factory configuration without persisting closures or secrets.
    pub fn register_agent_with_environment(
        &self,
        build: AgentBuild,
        agent: Agent,
        harness: Option<Harness>,
        reference: impl Into<String>,
        factory: impl crate::hosting::EnvironmentFactory,
    ) -> Result<()> {
        let reference = reference.into();
        if reference.is_empty() || reference.len() > 512 {
            return Err(AgentLoopError::config("invalid Environment reference"));
        }
        let mut registry = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if registry.contains_key(&build) {
            return Err(AgentLoopError::config("agent build is already registered"));
        }
        registry.insert(
            build,
            Registration {
                agent,
                harness,
                environment: Some((reference, Arc::new(factory))),
            },
        );
        Ok(())
    }

    pub(crate) async fn hosted_environment(
        &self,
        session: SessionId,
    ) -> Result<crate::Environment> {
        let local_binding = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session)
            .filter(|entry| entry.creation.is_some())
            .and_then(|entry| entry.durable_binding.clone());
        let binding = match local_binding {
            Some(binding) => binding,
            None => self.session_binding(session).await?,
        };
        let registration = self.registration(&binding.behavior)?;
        let Some((reference, factory)) = registration.environment else {
            return Err(AgentLoopError::config(
                "no retained Environment factory is configured",
            ));
        };
        if binding.environment.as_ref() != Some(&reference) {
            return Err(AgentLoopError::config(
                "retained Environment does not match binding",
            ));
        }
        let environment = factory.open(session).await?;
        // A cutover retained the legacy workspace head (EVE-1298). A factory
        // that opens any other head would silently move the session to a new
        // workspace; fail instead.
        if let Ok(Some(imported)) = self
            .profile()?
            .record(session, crate::hosting::cutover::CUTOVER_WORKSPACE)
            .await
        {
            let imported: everruns_core::host::WorkspaceBinding = serde_json::from_value(imported)
                .map_err(|_| AgentLoopError::store("imported workspace binding is corrupt"))?;
            if environment.workspace_head().binding() != &imported {
                return Err(AgentLoopError::config(
                    "retained Environment does not reopen the imported workspace head",
                ));
            }
        }
        Ok(environment)
    }

    /// Read authoritative scheduling and durable wait state without activation.
    pub async fn session_status(
        &self,
        session: SessionId,
    ) -> Result<crate::hosting::HostingStatus> {
        self.profile()?.status(session).await
    }
    /// Read an authoritative binding without reconstructing executable code.
    pub async fn session_binding(&self, session: SessionId) -> Result<SessionBinding> {
        Ok(self.profile()?.status(session).await?.binding)
    }

    /// Inspect retained-code compatibility without claiming execution or
    /// changing suspension, history, accepted inputs or pending waits.
    pub async fn session_compatibility(&self, session: SessionId) -> Result<SessionCompatibility> {
        self.binding_compatibility(&self.session_binding(session).await?)
    }

    /// Retry a compatibility suspension after restoring the required code.
    /// The authority checks the unchanged binding and absence of an active
    /// epoch atomically. This never migrates builds or clears uncertain effects.
    /// Retain `command` across lost replies. Privileged hosts authorize callers.
    pub async fn retry_session(&self, session: SessionId, command: uuid::Uuid) -> Result<()> {
        let binding = self.session_binding(session).await?;
        let compatibility = self.binding_compatibility(&binding)?;
        if !compatibility.is_available() {
            return Err(AgentLoopError::config(compatibility.suspension_reason()));
        }
        self.profile()?
            .command(
                session,
                crate::hosting::spi::session_commands::SessionCommand::RetryExecution {
                    id: command.to_string(),
                    binding: serde_json::to_value(binding)
                        .map_err(|e| AgentLoopError::config(e.to_string()))?,
                },
            )
            .await?;
        self.binding(session).activate();
        Ok(())
    }
    /// Commit a session binding at a caller-retained identity. Retrying the
    /// same creation is safe even after losing the entire Engine process.
    pub async fn create_registered(
        &self,
        session: SessionId,
        build: &AgentBuild,
    ) -> Result<Session> {
        self.create_registered_with_metadata(session, build, serde_json::Value::Null)
            .await
    }

    /// Commit metadata with the immutable binding. Retry a lost response at
    /// the same session id with the same configuration, including metadata.
    pub async fn create_registered_with_metadata(
        &self,
        session: SessionId,
        build: &AgentBuild,
        metadata: serde_json::Value,
    ) -> Result<Session> {
        if serde_json::to_vec(&metadata)
            .map_err(|_| AgentLoopError::config("invalid creation metadata"))?
            .len()
            > 64 * 1024
        {
            return Err(AgentLoopError::config("creation metadata exceeds limit"));
        }
        let profile = self.profile()?;
        let registration = self.registration(build)?;
        let binding = registered_binding(build, &registration, metadata)?;
        if let Some((_, factory)) = &registration.environment {
            let environment = factory.open(session).await?;
            if let Some(harness) = &registration.harness {
                harness
                    .negotiate(&environment)
                    .map_err(|error| AgentLoopError::config(error.to_string()))?;
            }
        } else if registration.agent.requires_hosted_workspace() {
            return Err(AgentLoopError::config(
                "this Agent requires a retained Environment factory",
            ));
        }
        profile.create(session, &binding).await?;
        self.cache_hosted(session, binding)?;
        self.resume_hosted(session).await
    }

    pub(crate) fn profile(&self) -> Result<&Arc<dyn SessionHosting>> {
        self.inner
            .hosting
            .as_ref()
            .ok_or_else(|| AgentLoopError::config("Engine has no session hosting profile"))
    }

    pub(super) fn registration(&self, build: &AgentBuild) -> Result<Registration> {
        self.inner
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(build)
            .cloned()
            .ok_or_else(|| AgentLoopError::config("retained agent build is unavailable"))
    }

    fn binding_compatibility(&self, binding: &SessionBinding) -> Result<SessionCompatibility> {
        if binding.framework != SESSION_RUNTIME_VERSION {
            return Ok(SessionCompatibility::UnsupportedRuntimeVersion {
                required: binding.framework,
                supported: SESSION_RUNTIME_VERSION,
            });
        }
        let Ok(registration) = self.registration(&binding.behavior) else {
            return Ok(SessionCompatibility::MissingBuild {
                required: binding.behavior.clone(),
            });
        };
        let harness = registration
            .harness
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| AgentLoopError::config("invalid registered Harness"))?;
        if binding.harness != harness
            || binding.environment
                != registration
                    .environment
                    .as_ref()
                    .map(|(reference, _)| reference.clone())
        {
            return Ok(SessionCompatibility::ConfigurationUnavailable);
        }
        Ok(SessionCompatibility::Available)
    }

    pub(super) fn cache_hosted(&self, session: SessionId, binding: SessionBinding) -> Result<()> {
        let compatibility = self.binding_compatibility(&binding)?;
        if !compatibility.is_available() {
            return Err(AgentLoopError::config(compatibility.suspension_reason()));
        }
        let registration = self.registration(&binding.behavior)?;
        let mut sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = sessions.get_mut(&session) {
            if existing.durable_binding.as_ref() != Some(&binding) {
                return Err(AgentLoopError::config(
                    "session cache binding conflicts with authority",
                ));
            }
            existing.agent = Some(registration.agent);
            existing.harness = registration.harness;
        } else {
            sessions.insert(
                session,
                EngineSessionEntry {
                    agent: Some(registration.agent),
                    harness: registration.harness,
                    state: None,
                    selected_environment: None,
                    durable_binding: Some(binding),
                    provisional: false,
                    creation: None,
                },
            );
        }
        Ok(())
    }

    pub(crate) async fn resume_hosted(&self, session: SessionId) -> Result<Session> {
        let status = self.profile()?.status(session).await?;
        if self.cache_hosted(session, status.binding.clone()).is_err() {
            // Readers need authoritative identity and ports, not executable
            // code. A missing retained build is handled by activation, which
            // suspends execution without hiding history or existing outcomes.
            let harness = status
                .binding
                .harness
                .clone()
                .map(serde_json::from_value)
                .transpose()
                .map_err(|_| AgentLoopError::store("invalid retained Harness"))?;
            self.inner
                .sessions
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .entry(session)
                .or_insert(EngineSessionEntry {
                    agent: None,
                    harness,
                    state: None,
                    selected_environment: None,
                    durable_binding: Some(status.binding),
                    provisional: false,
                    creation: None,
                });
        }
        if let Some(state) = self.state(session) {
            return Ok(Session::from_inner(state));
        }
        // Synchronous process-local sessions retain their selected Environment
        // across frontend handle loss. Remote readers must not require it.
        let environment = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session)
            .and_then(|entry| entry.selected_environment.clone());
        let handle = Session::new(
            self.binding(session),
            environment,
            self.inner.observers.clone(),
        );
        self.remember_state(session, &handle);
        Ok(handle)
    }

    /// Host activation entry. An alarm or native scheduler supplies only the
    /// session identity; authority and the retained registry decide its code.
    pub async fn activate(&self, session: SessionId) -> Result<()> {
        let profile = self.profile()?;
        let config = everruns_core::host::session_actor::SessionActorConfig::new(
            uuid::Uuid::new_v4().to_string(),
        );
        let mut ran = false;
        loop {
            let Some(activation) = profile
                .claim(session, &config.holder, config.lease_ttl)
                .await?
            else {
                if !ran || !profile.process_scheduler() {
                    return Ok(());
                }
                let status = profile.status(session).await?;
                if status.suspended || (!status.runnable && status.wake_at.is_none()) {
                    return Ok(());
                }
                everruns_contracts::rt::sleep(Duration::from_millis(if status.runnable {
                    50
                } else {
                    1000
                }))
                .await;
                continue;
            };
            if activation.actor.session_id != session {
                return Err(AgentLoopError::config(
                    "activation belongs to another session",
                ));
            }
            let compatibility = self.binding_compatibility(&activation.binding)?;
            if !compatibility.is_available() {
                let reason = compatibility.suspension_reason();
                activation.epoch.suspend(reason).await?;
                // No runtime/effects were started. Release the claim without
                // consuming input so restored code can be retried immediately.
                activation
                    .epoch
                    .actor_store()
                    .release_actor(
                        session,
                        &config.holder,
                        activation.actor.fence,
                        activation.actor.consumed_seq,
                    )
                    .await?;
                return Err(AgentLoopError::config(reason));
            }
            self.cache_hosted(session, activation.binding.clone())?;
            let registration = self.registration(&activation.binding.behavior)?;
            crate::hosting::execution::run(
                self,
                session,
                registration.agent,
                registration.harness,
                activation,
                config.clone(),
            )
            .await?;
            if !profile.process_scheduler() {
                return Ok(());
            }
            ran = true;
        }
    }
}

/// The immutable binding a registered build creates. An Agent cannot choose a
/// second authority beneath the Engine profile.
pub(super) fn registered_binding(
    build: &AgentBuild,
    registration: &Registration,
    metadata: serde_json::Value,
) -> Result<SessionBinding> {
    if registration.agent.custom_backends().is_some() {
        return Err(AgentLoopError::config(
            "Agent backends conflict with Engine hosting",
        ));
    }
    #[cfg(feature = "local")]
    if registration.agent.local_config().is_some() {
        return Err(AgentLoopError::config(
            "Agent local profile conflicts with Engine hosting",
        ));
    }
    Ok(SessionBinding {
        behavior: build.clone(),
        harness: registration
            .harness
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|_| AgentLoopError::config("invalid Harness binding"))?,
        environment: registration
            .environment
            .as_ref()
            .map(|(reference, _)| reference.clone()),
        framework: SESSION_RUNTIME_VERSION,
        metadata,
    })
}

impl Engine {
    pub(super) fn is_hosted(&self, session: SessionId) -> bool {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session)
            .is_none_or(|entry| entry.durable_binding.is_some())
    }

    pub(super) fn create_volatile(&self, agent: Agent) -> Session {
        let session = SessionId::new();
        let build = AgentBuild::new(agent.name(), format!("process-{session}"));
        let reference = format!("process-environment-{session}");
        let factory = agent.environment_factory();
        self.register_agent_with_environment(
            build.clone(),
            agent.clone(),
            None,
            &reference,
            factory,
        )
        .expect("fresh process registration");
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(
                session,
                EngineSessionEntry {
                    agent: Some(agent),
                    harness: None,
                    state: None,
                    selected_environment: None,
                    durable_binding: Some(SessionBinding {
                        framework: SESSION_RUNTIME_VERSION,
                        behavior: build,
                        harness: None,
                        environment: Some(reference),
                        metadata: serde_json::Value::Null,
                    }),
                    provisional: true,
                    creation: Some(Arc::new(OnceCell::new())),
                },
            );
        let handle = Session::new(self.binding(session), None, self.inner.observers.clone());
        self.remember_state(session, &handle);
        handle
    }

    pub(super) async fn ensure_hosted_created(&self, session: SessionId) -> Result<()> {
        let creation = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&session)
            .and_then(|entry| entry.creation.clone());
        if let Some(creation) = creation {
            creation
                .get_or_try_init(|| async {
                    let binding = {
                        let mut sessions = self
                            .inner
                            .sessions
                            .lock()
                            .unwrap_or_else(|p| p.into_inner());
                        let entry = sessions
                            .get_mut(&session)
                            .ok_or_else(|| AgentLoopError::SessionNotFound(session))?;
                        entry.provisional = false;
                        entry
                            .durable_binding
                            .clone()
                            .ok_or_else(|| AgentLoopError::config("missing provisional binding"))?
                    };
                    self.profile()?.create(session, &binding).await
                })
                .await?;
            Ok(())
        } else {
            self.profile()?.status(session).await.map(|_| ())
        }
    }

    pub(super) fn bind_volatile_environment(
        &self,
        session: SessionId,
        environment: crate::Environment,
    ) -> std::result::Result<bool, SessionEnvironmentError> {
        let mut sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let entry = sessions
            .get_mut(&session)
            .ok_or(SessionEnvironmentError::Unavailable)?;
        if !entry.provisional {
            return Ok(false);
        }
        entry
            .agent
            .as_ref()
            .ok_or(SessionEnvironmentError::Unavailable)?
            .validate_session_environment(&environment)?;
        let binding = entry
            .durable_binding
            .as_mut()
            .ok_or(SessionEnvironmentError::Unavailable)?;
        entry.selected_environment = Some(environment.clone());
        let reference = format!("process-environment-{session}");
        let factory = Arc::new(move |_| {
            let environment = environment.clone();
            async move { Ok(environment) }
        });
        self.inner
            .registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(&binding.behavior)
            .ok_or(SessionEnvironmentError::Unavailable)?
            .environment = Some((reference.clone(), factory));
        binding.environment = Some(reference);
        Ok(true)
    }
}
