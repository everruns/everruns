//! Session Capability
//!
//! Provides session metadata tools:
//! - `write_session_title`: update session title
//! - `get_session_info`: return session id, title, and agent name (if any)

use super::{Capability, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Session capability - read/update session metadata.
pub struct SessionCapability;

impl Capability for SessionCapability {
    fn id(&self) -> &str {
        "session"
    }

    fn name(&self) -> &str {
        "Session"
    }

    fn description(&self) -> &str {
        "Read and update current session metadata like title and agent info."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("panel-left")
    }

    fn category(&self) -> Option<&str> {
        Some("Session")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(WriteSessionTitleTool),
            Box::new(GetSessionInfoTool),
        ]
    }
}

/// Tool: write_session_title
pub struct WriteSessionTitleTool;

#[async_trait]
impl Tool for WriteSessionTitleTool {
    fn name(&self) -> &str {
        "write_session_title"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Write Session Title")
    }

    fn description(&self) -> &str {
        "Update the current session title."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "New session title"
                }
            },
            "required": ["title"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "write_session_title requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let title = match arguments.get("title").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolExecutionResult::tool_error("Missing required parameter: title"),
        };

        let Some(mutator) = &context.session_mutator else {
            return ToolExecutionResult::tool_error(
                "Session mutator not available in this context",
            );
        };

        match mutator
            .update_session_title(context.session_id, title.clone())
            .await
        {
            Ok(session) => ToolExecutionResult::success(json!({
                "session_id": session.id.to_string(),
                "title": session.title,
                "updated": true,
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }
}

/// Tool: get_session_info
pub struct GetSessionInfoTool;

#[async_trait]
impl Tool for GetSessionInfoTool {
    fn name(&self) -> &str {
        "get_session_info"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Get Session Info")
    }

    fn description(&self) -> &str {
        "Get current session metadata: id, title, and agent name (if assigned)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "get_session_info requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(session_store) = &context.session_store else {
            return ToolExecutionResult::tool_error("Session store not available in this context");
        };

        let session = match session_store.get_session(context.session_id).await {
            Ok(Some(session)) => session,
            Ok(None) => return ToolExecutionResult::tool_error("Session not found"),
            Err(e) => return ToolExecutionResult::internal_error(e),
        };

        let agent_name =
            if let (Some(agent_id), Some(agent_store)) = (session.agent_id, &context.agent_store) {
                match agent_store.get_agent(agent_id).await {
                    Ok(Some(agent)) => Some(agent.name),
                    Ok(None) => None,
                    Err(e) => return ToolExecutionResult::internal_error(e),
                }
            } else {
                None
            };

        ToolExecutionResult::success(json!({
            "session_id": session.id.to_string(),
            "title": session.title,
            "locale": session.locale,
            "agent_name": agent_name,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentStatus};
    use crate::error::Result;
    use crate::session::{Session, SessionStatus};
    use crate::typed_id::{AgentId, HarnessId, ModelId, SessionId};
    use crate::{AgentCapabilityConfig, Tool};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct MockSessionStore {
        session: Arc<Mutex<Option<Session>>>,
    }

    #[async_trait]
    impl crate::traits::SessionStore for MockSessionStore {
        async fn get_session(&self, _session_id: SessionId) -> Result<Option<Session>> {
            Ok(self.session.lock().expect("poisoned").clone())
        }
    }

    #[derive(Clone)]
    struct MockSessionMutator {
        session: Arc<Mutex<Session>>,
    }

    #[async_trait]
    impl crate::traits::SessionMutator for MockSessionMutator {
        async fn update_session_title(
            &self,
            _session_id: SessionId,
            title: String,
        ) -> Result<Session> {
            let mut session = self.session.lock().expect("poisoned");
            session.title = Some(title);
            Ok(session.clone())
        }
    }

    struct MockAgentStore {
        agent: Option<Agent>,
    }

    #[async_trait]
    impl crate::traits::AgentStore for MockAgentStore {
        async fn get_agent(&self, _agent_id: AgentId) -> Result<Option<Agent>> {
            Ok(self.agent.clone())
        }
    }

    fn build_session(agent_id: Option<AgentId>) -> Session {
        Session {
            id: SessionId::new(),
            organization_id: "org_00000000000000000000000000000001".to_string(),
            harness_id: HarnessId::new(),
            agent_id,
            title: Some("Old title".to_string()),
            locale: None,
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: Some(ModelId::new()),
            capabilities: vec![],
            tools: vec![],
            status: SessionStatus::Idle,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: vec![],
            parent_session_id: None,
            subagent_name: None,
            subagent_task: None,
            subagent_status: None,
        }
    }

    #[tokio::test]
    async fn write_session_title_updates_title() {
        let session = build_session(None);
        let session_id = session.id;
        let mut context = ToolContext::new(session_id);
        context.session_mutator = Some(Arc::new(MockSessionMutator {
            session: Arc::new(Mutex::new(session)),
        }));

        let tool = WriteSessionTitleTool;
        let result = tool
            .execute_with_context(json!({"title": "New title"}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["title"], "New title");
                assert_eq!(value["updated"], true);
            }
            _ => panic!("expected success"),
        }
    }

    #[tokio::test]
    async fn get_session_info_returns_agent_name_when_assigned() {
        let agent_id = AgentId::new();
        let session = build_session(Some(agent_id));
        let session_id = session.id;

        let agent = Agent {
            public_id: agent_id,
            internal_id: agent_id.uuid(),
            name: "Research Agent".to_string(),
            description: Some("desc".to_string()),
            system_prompt: "prompt".to_string(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("session")],
            initial_files: vec![],
            tools: vec![],
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        };

        let context = ToolContext::new(session_id)
            .with_session_store(Arc::new(MockSessionStore {
                session: Arc::new(Mutex::new(Some(session))),
            }))
            .with_agent_store(Arc::new(MockAgentStore { agent: Some(agent) }));

        let tool = GetSessionInfoTool;
        let result = tool.execute_with_context(json!({}), &context).await;

        match result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["title"], "Old title");
                assert_eq!(value["agent_name"], "Research Agent");
            }
            _ => panic!("expected success"),
        }
    }
}
