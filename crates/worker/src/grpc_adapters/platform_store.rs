//! Portable delegation views returned by the authorized server edge.

use super::*;

#[async_trait]
impl everruns_capabilities::PlatformStore for GrpcOrgAdapter {
    fn for_execution(&self, id: Uuid) -> Option<Arc<dyn everruns_capabilities::PlatformStore>> {
        let mut bound = self.clone();
        bound.input_message_id = Some(id);
        Some(Arc::new(bound))
    }

    async fn platform_discover(&self, arguments: serde_json::Value) -> Result<String> {
        self.invoke_platform_command_surface(
            proto::PlatformCommandSurfaceOperation::Discover,
            arguments,
        )
        .await
    }

    async fn platform_query(&self, arguments: serde_json::Value) -> Result<String> {
        self.invoke_platform_command_surface(
            proto::PlatformCommandSurfaceOperation::Query,
            arguments,
        )
        .await
    }

    async fn platform_execute(&self, arguments: serde_json::Value) -> Result<String> {
        self.invoke_platform_command_surface(
            proto::PlatformCommandSurfaceOperation::Execute,
            arguments,
        )
        .await
    }

    // =========================================================================
    // Harness Operations
    // =========================================================================

    async fn get_harness(
        &self,
        id: everruns_contracts::typed_id::HarnessId,
    ) -> Result<Option<everruns_core::HarnessDefinition>> {
        self.execute_runtime_lookup("get_harness", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    // =========================================================================
    // Agent Operations
    // =========================================================================

    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<everruns_core::AgentDefinition>> {
        self.execute_runtime_lookup("get_agent", serde_json::json!({ "id": id.to_string() }))
            .await
    }

    // =========================================================================
    // App Operations
    // =========================================================================

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn create_session_with_options(
        &self,
        request: everruns_capabilities::PlatformCreateSessionRequest,
    ) -> Result<everruns_core::ExecutionSession> {
        self.execute_runtime_command(
            "create_session",
            serde_json::json!({
                "harness_id": request.harness_id.to_string(),
                "agent_id": request.agent_id.map(|id| id.to_string()),
                "title": request.title,
                "goal": request.goal,
                "locale": request.locale,
                "tags": ["managed"],
                "capabilities": [],
                "tools": [],
                "mcp_servers": {},
                "initial_files": [],
                "blueprint_id": request.blueprint_id,
                "blueprint_config": request.blueprint_config,
                "parent_session_id": request.parent_session_id.map(|id| id.to_string()),
                "forked_from_session_id": request.forked_from_session_id.map(|id| id.to_string()),
                "budget_root_session_id": request.budget_root_session_id.map(|id| id.to_string()),
                "seed": request.seed,
            }),
        )
        .await
    }

    async fn get_session_by_id(
        &self,
        id: SessionId,
    ) -> Result<Option<everruns_core::ExecutionSession>> {
        self.execute_runtime_lookup(
            "get_session",
            serde_json::json!({ "session_id": id.to_string() }),
        )
        .await
    }

    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<everruns_contracts::typed_id::SessionParticipantId> {
        self.execute_runtime_command(
            "add_session_participant",
            serde_json::json!({
                "session_id": session_id.to_string(),
                "kind": "agent",
                "agent_id": agent_id.to_string(),
            }),
        )
        .await
    }

    // =========================================================================
    // Messaging
    // =========================================================================

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        let _: serde_json::Value = self
            .execute_platform_command(
                "create_message",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "message": {
                        "content": [{ "type": "text", "text": content }],
                    },
                }),
            )
            .await?;
        Ok(())
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<everruns_capabilities::PlatformMessage>> {
        let mut messages: Vec<RuntimeMessage> = self
            .execute_platform_command(
                "list_messages",
                serde_json::json!({
                    "session_id": session_id.to_string(),
                    "limit": limit.unwrap_or(10),
                }),
            )
            .await?;
        messages.retain(|message| {
            matches!(
                message.role,
                everruns_core::RuntimeMessageRole::User | everruns_core::RuntimeMessageRole::Agent
            )
        });

        Ok(messages
            .into_iter()
            .filter_map(|message| {
                let content = message
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        everruns_core::ContentPart::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if content.is_empty() {
                    return None;
                }
                Some(everruns_capabilities::PlatformMessage {
                    role: match message.role {
                        everruns_core::RuntimeMessageRole::User => "user".to_string(),
                        _ => "agent".to_string(),
                    },
                    content,
                    created_at: message.created_at,
                })
            })
            .collect())
    }

    // =========================================================================
    // Turn Management
    // =========================================================================

    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        timeout_secs: Option<u64>,
    ) -> Result<String> {
        let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(120));
        let start = std::time::Instant::now();
        let poll_interval = std::time::Duration::from_millis(500);

        loop {
            let session = self
                .get_session_by_id(session_id)
                .await?
                .ok_or_else(|| AgentLoopError::store("Session not found"))?;

            match session.status {
                everruns_core::session::SessionExecutionState::Idle => {
                    if let Some(status) = self.latest_terminal_turn_status(session_id).await? {
                        return Ok(status);
                    }
                    // Session status flips to idle independently from terminal
                    // turn-event persistence. Keep polling until the event lands
                    // so callers can distinguish successful and failed idle turns.
                }
                everruns_core::session::SessionExecutionState::WaitingForToolResults => {
                    return Ok("waiting_for_tool_results".to_string());
                }
                everruns_core::session::SessionExecutionState::Paused => {
                    return Ok("paused".to_string());
                }
                everruns_core::session::SessionExecutionState::Started
                | everruns_core::session::SessionExecutionState::Active => {}
            }

            if start.elapsed() > timeout {
                return Ok(format!("timeout (last status: {:?})", session.status));
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    // =========================================================================
    // Capabilities
    // =========================================================================

    // =========================================================================
    // UI Links
    // =========================================================================
}
