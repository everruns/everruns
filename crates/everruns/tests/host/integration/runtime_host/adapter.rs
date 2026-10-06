use super::*;

#[derive(Clone, Default)]
pub(in crate::integration) struct TestSessionStore {
    pub(in crate::integration) sessions: Arc<RwLock<HashMap<SessionId, ExecutionSession>>>,
    pub(in crate::integration) fail_status_writes: Arc<AtomicBool>,
}

impl TestSessionStore {
    pub(in crate::integration) async fn insert(&self, session: ExecutionSession) {
        self.sessions.write().await.insert(session.id, session);
    }

    async fn set_status(
        &self,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> everruns_contracts::error::Result<ExecutionSession> {
        if self.fail_status_writes.load(Ordering::SeqCst) {
            return Err(everruns_contracts::error::AgentLoopError::config(
                "injected session status failure",
            ));
        }
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(&session_id).expect("session exists");
        session.status = status;
        Ok(session.clone())
    }
}

#[async_trait]
impl SessionStore for TestSessionStore {
    async fn get_session(
        &self,
        session_id: SessionId,
    ) -> everruns_contracts::error::Result<Option<ExecutionSession>> {
        Ok(self.sessions.read().await.get(&session_id).cloned())
    }
}

#[async_trait]
impl SessionMutator for TestSessionStore {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> everruns_contracts::error::Result<ExecutionSession> {
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(&session_id).expect("session exists");
        session.title = Some(title);
        Ok(session.clone())
    }
}

#[derive(Clone)]
pub(in crate::integration) struct MockHostAdapter {
    pub(in crate::integration) capability_registry: CapabilityRegistry,
    pub(in crate::integration) driver_registry: DriverRegistry,
    pub(in crate::integration) harness_store: Arc<InMemoryHarnessStore>,
    pub(in crate::integration) agent_store: Arc<InMemoryAgentStore>,
    pub(in crate::integration) session_store: Arc<TestSessionStore>,
    pub(in crate::integration) message_store: Arc<InMemoryMessageRetriever>,
    pub(in crate::integration) provider_store: Arc<InMemoryProviderStore>,
    pub(in crate::integration) event_emitter: Arc<InMemoryEventEmitter>,
    pub(in crate::integration) file_store: Arc<InMemorySessionFileStore>,
    pub(in crate::integration) session_task_registry: Option<Arc<dyn SessionTaskRegistry>>,
}

#[async_trait]
impl RuntimeHostAdapter for MockHostAdapter {
    async fn set_session_status(
        &self,
        _org_id: i64,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> everruns_contracts::error::Result<()> {
        self.session_store.set_status(session_id, status).await?;
        Ok(())
    }

    async fn load_resolved_turn(
        &self,
        _org_id: i64,
        session_id: SessionId,
    ) -> everruns_contracts::error::Result<ResolvedTurnInputs> {
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .expect("session exists");
        let agent = match session.agent_id {
            Some(agent_id) => self.agent_store.get_agent(agent_id).await?,
            None => None,
        };
        let harness = self
            .harness_store
            .get_harness(session.harness_id)
            .await?
            .expect("harness exists");
        let snapshot =
            everruns_core::ResolvedExecutionSnapshot::project(&harness, agent.as_ref(), &session)?;
        Ok(ResolvedTurnInputs {
            snapshot,
            messages: self.message_store.load(session_id).await?,
            mcp_tool_definitions: vec![],
        })
    }

    fn capability_registry(&self) -> CapabilityRegistry {
        self.capability_registry.clone()
    }

    fn driver_registry(&self) -> DriverRegistry {
        self.driver_registry.clone()
    }

    fn harness_store(&self, _org_id: i64) -> Arc<dyn HarnessStore> {
        self.harness_store.clone()
    }

    fn agent_store(&self, _org_id: i64) -> Arc<dyn AgentStore> {
        self.agent_store.clone()
    }

    fn session_store(&self, _org_id: i64) -> Arc<dyn SessionStore> {
        self.session_store.clone()
    }

    fn session_mutator(&self, _org_id: i64) -> Arc<dyn SessionMutator> {
        self.session_store.clone()
    }

    fn provider_store(&self, _org_id: i64) -> Arc<dyn ProviderStore> {
        self.provider_store.clone()
    }

    fn message_store(&self) -> Arc<dyn everruns_core::MessageRetriever> {
        self.message_store.clone()
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        self.event_emitter.clone()
    }

    #[cfg(feature = "bashkit")]
    fn bash_hook_dispatcher(
        &self,
        org_id: i64,
    ) -> Arc<dyn everruns_core::hook_executor::BashHookDispatcher> {
        Arc::new(
            everruns_integrations::bashkit::BashkitShellHookDispatcher::new(
                self.file_store(org_id),
            ),
        )
    }

    fn file_store(&self, _org_id: i64) -> Arc<dyn SessionFileSystem> {
        self.file_store.clone()
    }

    fn session_task_registry(&self) -> Option<Arc<dyn SessionTaskRegistry>> {
        self.session_task_registry.clone()
    }
}
