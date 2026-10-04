//! An embedded runtime uses the stock subagents capability without constructing
//! a hosted agent, harness, session, participant, or ownership record.

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use everruns::{Agent, InMemoryEngine, Model};
    use everruns_contracts::error::{AgentLoopError, Result};
    use everruns_contracts::typed_id::{
        AgentId, HarnessId, SessionId, SessionParticipantId, WorkspaceId,
    };
    use everruns_core::execution_loading::SessionStore;
    use everruns_core::tool_context::ToolContext;
    use everruns_core::{
        AgentDefinition, Capability, ExecutionSession, HarnessDefinition, ToolExecutionResult,
    };
    use everruns_platform::capabilities::SubagentCapability;
    use everruns_platform::{
        PlatformCreateSessionRequest, PlatformMessage, PlatformStore, PlatformStoreSubagentDelegate,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// A runtime-owned catalog. Child turns run through the real Framework
    /// engine and simulated driver, rather than returning hardcoded messages.
    struct RuntimeStore {
        engine: InMemoryEngine,
        parent: ExecutionSession,
        sessions: Mutex<HashMap<SessionId, ExecutionSession>>,
        children: Mutex<HashMap<SessionId, everruns::Session>>,
        messages: Mutex<HashMap<SessionId, Vec<PlatformMessage>>>,
    }

    impl RuntimeStore {
        fn new() -> Self {
            let id = SessionId::new();
            Self {
                engine: InMemoryEngine::new(),
                parent: ExecutionSession::new(
                    id,
                    WorkspaceId::from_uuid(id.uuid()),
                    HarnessId::new(),
                ),
                sessions: Mutex::new(HashMap::new()),
                children: Mutex::new(HashMap::new()),
                messages: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl PlatformStore for RuntimeStore {
        async fn get_agent_by_id(&self, _id: AgentId) -> Result<Option<AgentDefinition>> {
            Ok(None)
        }
        async fn get_harness(&self, _id: HarnessId) -> Result<Option<HarnessDefinition>> {
            Ok(Some(HarnessDefinition::new(
                "embedded",
                "Solve the delegated task.",
            )))
        }
        async fn create_session_with_options(
            &self,
            request: PlatformCreateSessionRequest,
        ) -> Result<ExecutionSession> {
            let agent = Agent::builder()
                .instructions("Return the delegated answer.")
                .model(Model::simulated("42"))
                .build()
                .map_err(|error| AgentLoopError::tool(error.to_string()))?;
            let child = self.engine.create(agent);
            child
                .start()
                .await
                .map_err(|error| AgentLoopError::tool(error.to_string()))?;
            let id = child.session_id();
            let mut session =
                ExecutionSession::new(id, WorkspaceId::from_uuid(id.uuid()), request.harness_id);
            session.agent_id = request.agent_id;
            session.parent_session_id = request.parent_session_id;
            session.forked_from_session_id = request.forked_from_session_id;
            session.title = request.title;
            session.goal = request.goal;
            session.locale = request.locale;
            self.sessions.lock().unwrap().insert(id, session.clone());
            self.children.lock().unwrap().insert(id, child);
            Ok(session)
        }
        async fn get_session_by_id(&self, id: SessionId) -> Result<Option<ExecutionSession>> {
            if id == self.parent.id {
                return Ok(Some(self.parent.clone()));
            }
            Ok(self.sessions.lock().unwrap().get(&id).cloned())
        }
        async fn add_agent_session_participant(
            &self,
            _session_id: SessionId,
            _agent_id: AgentId,
        ) -> Result<SessionParticipantId> {
            Err(AgentLoopError::tool(
                "Embedded host has no participant catalog",
            ))
        }
        async fn send_message(&self, id: SessionId, content: &str) -> Result<()> {
            let child = self
                .children
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .ok_or_else(|| AgentLoopError::session_not_found(id))?;
            let turn = child
                .run(content)
                .await
                .map_err(|error| AgentLoopError::tool(error.to_string()))?;
            assert!(turn.success, "child execution must complete");
            self.messages.lock().unwrap().insert(
                id,
                vec![PlatformMessage {
                    role: "agent".into(),
                    content: turn.response,
                    created_at: chrono::Utc::now(),
                }],
            );
            self.sessions.lock().unwrap().get_mut(&id).unwrap().status =
                everruns_core::session::SessionExecutionState::Idle;
            Ok(())
        }
        async fn get_messages(
            &self,
            id: SessionId,
            _limit: Option<usize>,
        ) -> Result<Vec<PlatformMessage>> {
            Ok(self
                .messages
                .lock()
                .unwrap()
                .get(&id)
                .cloned()
                .unwrap_or_default())
        }
        async fn wait_for_idle(&self, id: SessionId, _timeout_secs: Option<u64>) -> Result<String> {
            if self.messages.lock().unwrap().contains_key(&id) {
                Ok("completed".into())
            } else {
                Err(AgentLoopError::tool("Child has not completed"))
            }
        }
    }

    #[async_trait]
    impl SessionStore for RuntimeStore {
        async fn get_session(&self, id: SessionId) -> Result<Option<ExecutionSession>> {
            self.get_session_by_id(id).await
        }
    }

    fn context(store: Arc<RuntimeStore>) -> ToolContext {
        let mut context = ToolContext::new(store.parent.id);
        context.subagent_delegate = Some(Arc::new(PlatformStoreSubagentDelegate(store.clone())));
        context.session_store = Some(store);
        context
    }

    #[tokio::test]
    async fn stock_subagents_execute_a_real_child_turn_with_only_runtime_views() {
        let store = Arc::new(RuntimeStore::new());
        let context = context(store.clone());
        let provider = SubagentCapability
            .delegation_target_with_config(&json!({}))
            .unwrap();
        let result = provider
            .tool
            .execute_with_context(
                json!({
                    "name": "Compute", "instructions": "What is six times seven?",
                    "target": {"type": "subagent"}, "mode": "foreground"
                }),
                &context,
            )
            .await;
        let ToolExecutionResult::Success(value) = result else {
            panic!("spawn failed: {result:?}");
        };
        assert_eq!(value["result"], "42");
        assert_eq!(value["status"], "completed");
        let child_id: SessionId = value["subagent_id"].as_str().unwrap().parse().unwrap();
        let child = store.get_session_by_id(child_id).await.unwrap().unwrap();
        assert_eq!(child.parent_session_id, Some(context.session_id));
        assert_eq!(child.harness_id, store.parent.harness_id);
        assert_eq!(child.title.as_deref(), Some("Compute"));
    }

    #[tokio::test]
    async fn stock_subagents_enforce_depth_before_creating_a_child() {
        let store = Arc::new(RuntimeStore::new());
        let mut context = context(store.clone());
        context.subagent_nesting_policy = context.subagent_nesting_policy.with_platform_default(0);
        let provider = SubagentCapability
            .delegation_target_with_config(&json!({}))
            .unwrap();
        let result = provider.tool.execute_with_context(json!({
            "name": "Denied", "instructions": "go", "target": {"type": "subagent"}, "mode": "foreground"
        }), &context).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(message) if message.contains("max_subagent_depth"))
        );
        assert!(store.children.lock().unwrap().is_empty());
    }
}
