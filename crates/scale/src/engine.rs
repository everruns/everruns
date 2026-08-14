use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns::{Agent, Engine, EngineCreateError, InMemoryEngine, ResumeError, Session, SessionId};
use everruns_host::{EnvironmentBindingStore, HostBackends};
use sqlx::PgPool;

use crate::persistence::{PostgresEnvironmentBindingStore, PostgresEventLog};
use crate::{
    DurableTurnDriver, PortableAgent, PortableAgentError, Registry, RunController, migrate,
};

/// PostgreSQL-backed engine for explicitly portable agent submissions.
///
/// An arbitrary [`Agent`] can contain closures, provider drivers, event sinks,
/// workspace implementations, and lifecycle hooks. Consequently
/// [`Engine::try_create`] rejects every raw Agent with a typed error.
/// [`ScaleEngine::submit`] is the accepted path: it persists only
/// [`PortableAgent`] references and resolves their process-local
/// implementations from a startup [`Registry`].
#[derive(Clone)]
pub struct ScaleEngine {
    inner: Arc<ScaleEngineInner>,
}

struct ScaleEngineInner {
    pool: Option<PgPool>,
    registry: Registry,
    local: InMemoryEngine,
    controller: DurableTurnDriver,
    definitions: Mutex<HashMap<SessionId, PortableAgent>>,
    runtime_backends: Option<HostBackends>,
    binding_store: Option<Arc<dyn EnvironmentBindingStore>>,
}

impl fmt::Debug for ScaleEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScaleEngine")
            .field(
                "portable_sessions",
                &self.inner.definitions.lock().map(|v| v.len()).unwrap_or(0),
            )
            .finish_non_exhaustive()
    }
}

impl ScaleEngine {
    /// Initialize Scale and its versioned schema from a blank PostgreSQL DB.
    pub async fn new(pool: PgPool, registry: Registry) -> Result<Self, PortableAgentError> {
        migrate(&pool)
            .await
            .map_err(|error| PortableAgentError::Persistence(error.to_string()))?;
        let binding_store = Arc::new(PostgresEnvironmentBindingStore::new(pool.clone()));
        let runtime_backends =
            HostBackends::in_memory().with_event_log(Arc::new(PostgresEventLog::new(pool.clone())));
        Ok(Self {
            inner: Arc::new(ScaleEngineInner {
                pool: Some(pool.clone()),
                registry,
                local: InMemoryEngine::new(),
                controller: DurableTurnDriver::postgres(pool),
                definitions: Mutex::new(HashMap::new()),
                runtime_backends: Some(runtime_backends),
                binding_store: Some(binding_store),
            }),
        })
    }

    /// Build a Scale control plane over an already-adapted durable store.
    ///
    /// This is used by standalone workers whose PostgreSQL access is mediated
    /// by gRPC. Portable facade submission is unavailable because this form has
    /// no direct catalog pool.
    pub fn from_controller(controller: DurableTurnDriver) -> Self {
        Self {
            inner: Arc::new(ScaleEngineInner {
                pool: None,
                registry: Registry::new(),
                local: InMemoryEngine::new(),
                controller,
                definitions: Mutex::new(HashMap::new()),
                runtime_backends: None,
                binding_store: None,
            }),
        }
    }

    /// Build the embedded product control plane over a shared memory store.
    pub fn in_memory_controller(store: Arc<everruns_durable::InMemoryWorkflowEventStore>) -> Self {
        Self::from_controller(DurableTurnDriver::shared_in_memory(store))
    }

    #[cfg(test)]
    fn for_test(registry: Registry) -> Self {
        let mut engine = Self::from_controller(DurableTurnDriver::in_memory());
        Arc::get_mut(&mut engine.inner)
            .expect("new test engine has one owner")
            .registry = registry;
        engine
    }

    /// Validate, resolve, persist, and create a portable session.
    ///
    /// Validation and registry resolution finish before any catalog or workflow
    /// write. A missing provider/tool/capability/hook error names both the
    /// component and its stable registration path.
    pub async fn submit(&self, definition: PortableAgent) -> Result<Session, PortableAgentError> {
        let agent = self.resolve(&definition)?;
        let session = self
            .inner
            .local
            .try_create(agent)
            .map_err(|error| PortableAgentError::InvalidDefinition(error.to_string()))?;
        let json = serde_json::to_value(&definition)
            .map_err(|error| PortableAgentError::InvalidDefinition(error.to_string()))?;
        if let Some(pool) = &self.inner.pool {
            sqlx::query(
                "INSERT INTO everruns_scale_sessions \
                 (session_id, definition_version, portable_agent) VALUES ($1, $2, $3) \
                 ON CONFLICT (session_id) DO NOTHING",
            )
            .bind(session.session_id().uuid())
            .bind(i16::try_from(definition.version).unwrap_or(i16::MAX))
            .bind(json)
            .execute(pool)
            .await
            .map_err(|error| PortableAgentError::Persistence(error.to_string()))?;
        }
        self.inner
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session.session_id(), definition);
        Ok(session)
    }

    fn resolve(&self, definition: &PortableAgent) -> Result<Agent, PortableAgentError> {
        match (&self.inner.runtime_backends, &self.inner.binding_store) {
            (Some(backends), Some(binding_store)) => self.inner.registry.resolve_with_backends(
                definition,
                backends.clone(),
                binding_store.clone(),
            ),
            _ => self.inner.registry.resolve(definition),
        }
    }

    /// Read the exact persisted portable definition for a Scale session.
    pub async fn definition(
        &self,
        session_id: SessionId,
    ) -> Result<Option<PortableAgent>, PortableAgentError> {
        if let Some(definition) = self
            .inner
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&session_id)
            .cloned()
        {
            return Ok(Some(definition));
        }
        let Some(pool) = self.inner.pool.as_ref() else {
            return Ok(None);
        };
        let value: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT portable_agent FROM everruns_scale_sessions WHERE session_id = $1",
        )
        .bind(session_id.uuid())
        .fetch_optional(pool)
        .await
        .map_err(|error| PortableAgentError::Persistence(error.to_string()))?;
        value
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| PortableAgentError::Persistence(error.to_string()))
    }
}

#[async_trait]
impl RunController for ScaleEngine {
    async fn start_run(
        &self,
        org_id: i64,
        session_id: SessionId,
        harness_id: everruns_provider::typed_id::HarnessId,
        agent_id: Option<everruns_provider::typed_id::AgentId>,
        input_message_id: everruns_provider::typed_id::MessageId,
        request_id: Option<String>,
    ) -> anyhow::Result<()> {
        self.inner
            .controller
            .start_run(
                org_id,
                session_id,
                harness_id,
                agent_id,
                input_message_id,
                request_id,
            )
            .await
    }

    async fn resume_after_tool_results(&self, session_id: SessionId) -> anyhow::Result<()> {
        self.inner
            .controller
            .resume_after_tool_results(session_id)
            .await
    }

    async fn cancel_run(&self, session_id: SessionId) -> anyhow::Result<()> {
        self.inner.controller.cancel_run(session_id).await
    }

    async fn is_running(&self, session_id: SessionId) -> bool {
        self.inner.controller.is_running(session_id).await
    }

    async fn active_count(&self) -> usize {
        self.inner.controller.active_count().await
    }
}

#[async_trait]
impl Engine for ScaleEngine {
    fn try_create(&self, _agent: Agent) -> Result<Session, EngineCreateError> {
        Err(EngineCreateError::PortableAgentRequired {
            engine: "ScaleEngine",
            component: "everruns::Agent (process-local implementation graph)",
            registration_path: "ScaleEngine::submit(PortableAgent)",
        })
    }

    fn create(&self, agent: Agent) -> Session {
        self.try_create(agent)
            .unwrap_or_else(|error| panic!("{error}"))
    }

    async fn resume(&self, session_id: SessionId) -> Result<Session, ResumeError> {
        if let Ok(session) = self.inner.local.resume(session_id).await {
            return Ok(session);
        }
        let definition = self
            .definition(session_id)
            .await
            .map_err(|_| ResumeError::Unavailable)?
            .ok_or(ResumeError::SessionNotFound { session_id })?;
        let agent = self
            .resolve(&definition)
            .map_err(|_| ResumeError::Unavailable)?;
        self.inner
            .definitions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id, definition);
        Ok(self.inner.local.restore(session_id, agent))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ComponentKind, RegistrationPath};
    use everruns::{
        Agent, CancellationToken, Engine, EventStream, Model, RunOptions, SendDisposition,
        TurnStopReason,
    };
    use everruns_provider::runtime_provider::Provider;
    use everruns_test_support::{LlmSimConfig, LlmSimDriver};
    use std::time::Duration;

    fn path(value: &str) -> RegistrationPath {
        RegistrationPath::new(value).unwrap()
    }

    fn registry(response: &str) -> Registry {
        registry_with(LlmSimConfig::fixed(response.to_string()))
    }

    fn registry_with(config: LlmSimConfig) -> Registry {
        let mut registry = Registry::new();
        registry
            .register_provider(
                path("providers/llmsim/default"),
                Provider::new("llmsim", LlmSimDriver::new(config)),
            )
            .unwrap();
        registry
            .register_workspace(path("workspaces/test/default"), |builder| builder)
            .unwrap();
        registry
    }

    fn portable() -> PortableAgent {
        PortableAgent::builder(
            "Be concise.",
            "llmsim-model",
            path("providers/llmsim/default"),
            path("workspaces/test/default"),
        )
        .name("portable")
        .build()
        .unwrap()
    }

    fn drain_event_signature(mut stream: EventStream) -> Vec<(String, Option<i32>)> {
        let mut signature = Vec::new();
        while let Some(event) = stream.try_recv().expect("event stream remains lossless") {
            signature.push((event.event_type().to_string(), event.sequence()));
        }
        signature
    }

    #[test]
    fn raw_agent_is_rejected_before_submission() {
        let engine = ScaleEngine::for_test(registry("unused"));
        let local = Agent::builder()
            .instructions("local")
            .model(Model::simulated("local"))
            .build()
            .unwrap();
        assert!(matches!(
            engine.try_create(local),
            Err(EngineCreateError::PortableAgentRequired {
                engine: "ScaleEngine",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn missing_registration_names_component_and_path() {
        let engine = ScaleEngine::for_test(Registry::new());
        let error = match engine.submit(portable()).await {
            Ok(_) => panic!("missing provider registration was accepted"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            PortableAgentError::MissingRegistration {
                component: ComponentKind::Provider,
                registration_path: "providers/llmsim/default".into(),
            }
        );
    }

    #[tokio::test]
    async fn missing_workspace_registration_names_component_and_path() {
        let engine = ScaleEngine::for_test(registry("unused"));
        let definition = PortableAgent::builder(
            "Be concise.",
            "llmsim-model",
            path("providers/llmsim/default"),
            path("workspaces/test/missing"),
        )
        .build()
        .unwrap();
        let error = match engine.submit(definition).await {
            Ok(_) => panic!("missing workspace registration was accepted"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            PortableAgentError::MissingRegistration {
                component: ComponentKind::Workspace,
                registration_path: "workspaces/test/missing".into(),
            }
        );
    }

    #[tokio::test]
    async fn portable_and_in_memory_sessions_have_outcome_history_and_resume_parity() {
        let scale = ScaleEngine::for_test(registry("same outcome"));
        let scale_session = scale.submit(portable()).await.unwrap();
        let memory = everruns::InMemoryEngine::new();
        let memory_session = memory.create(
            Agent::builder()
                .instructions("Be concise.")
                .model(Model::simulated("same outcome"))
                .build()
                .unwrap(),
        );

        let scale_events = scale_session.events();
        let memory_events = memory_session.events();
        let scale_turn = scale_session.send_and_wait("hello").await.unwrap();
        let memory_turn = memory_session.send_and_wait("hello").await.unwrap();
        assert_eq!(scale_turn.response, memory_turn.response);
        assert_eq!(scale_turn.success, memory_turn.success);
        assert_eq!(scale_turn.stop_reason, memory_turn.stop_reason);

        let scale_history = scale_session.history().page().await.unwrap();
        let memory_history = memory_session.history().page().await.unwrap();
        assert_eq!(scale_history.messages.len(), memory_history.messages.len());
        for (scale_message, memory_message) in scale_history
            .messages
            .iter()
            .zip(memory_history.messages.iter())
        {
            assert_eq!(scale_message.role, memory_message.role);
            assert_eq!(scale_message.content, memory_message.content);
        }
        assert_eq!(
            drain_event_signature(scale_events),
            drain_event_signature(memory_events),
            "both engines expose the same canonical live event sequence"
        );

        let resumed = scale.resume(scale_session.session_id()).await.unwrap();
        assert_eq!(resumed.session_id(), scale_session.session_id());
        assert_eq!(
            resumed.send_and_wait("again").await.unwrap().response,
            "same outcome"
        );
    }

    #[tokio::test]
    async fn portable_and_in_memory_sessions_have_cancellation_parity() {
        let delay = Duration::from_secs(30);
        let scale = ScaleEngine::for_test(registry_with(
            LlmSimConfig::fixed("eventually").with_response_delay(delay),
        ));
        let scale_session = scale.submit(portable()).await.unwrap();
        let memory_session = everruns::InMemoryEngine::new().create(
            Agent::builder()
                .instructions("Be concise.")
                .model(Model::simulated_with_config(
                    LlmSimConfig::fixed("eventually").with_response_delay(delay),
                ))
                .build()
                .unwrap(),
        );

        let scale_token = CancellationToken::new();
        let memory_token = CancellationToken::new();
        for token in [scale_token.clone(), memory_token.clone()] {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                token.cancel();
            });
        }

        let (scale_turn, memory_turn) = tokio::join!(
            scale_session.run_with("hello", RunOptions::new().cancel_token(scale_token)),
            memory_session.run_with("hello", RunOptions::new().cancel_token(memory_token)),
        );
        for turn in [scale_turn.unwrap(), memory_turn.unwrap()] {
            assert!(!turn.success);
            assert_eq!(turn.stop_reason, TurnStopReason::Cancelled);
        }
    }

    #[tokio::test]
    async fn portable_and_in_memory_sessions_have_steering_parity() {
        let delay = Duration::from_millis(250);
        let scale = ScaleEngine::for_test(registry_with(
            LlmSimConfig::fixed("steered").with_response_delay(delay),
        ));
        let scale_session = scale.submit(portable()).await.unwrap();
        let memory_session = everruns::InMemoryEngine::new().create(
            Agent::builder()
                .instructions("Be concise.")
                .model(Model::simulated_with_config(
                    LlmSimConfig::fixed("steered").with_response_delay(delay),
                ))
                .build()
                .unwrap(),
        );

        let scale_first = scale_session.send("first").await.unwrap();
        let memory_first = memory_session.send("first").await.unwrap();
        let scale_second = scale_session.send("second").await.unwrap();
        let memory_second = memory_session.send("second").await.unwrap();
        assert_eq!(scale_first.disposition, SendDisposition::Started);
        assert_eq!(memory_first.disposition, SendDisposition::Started);
        assert_eq!(scale_second.disposition, SendDisposition::Steered);
        assert_eq!(memory_second.disposition, SendDisposition::Steered);

        let (scale_turn, memory_turn) = tokio::join!(scale_first.wait(), memory_first.wait());
        let scale_turn = scale_turn.unwrap();
        let memory_turn = memory_turn.unwrap();
        assert_eq!(scale_turn.response, memory_turn.response);
        assert_eq!(scale_turn.stop_reason, memory_turn.stop_reason);
    }
}
